//! Gmail API email sending with file attachments.
//! Uses Google Service Account with domain-wide delegation.

use anyhow::{Context, Result};
use std::process::Command;

/// Default service account credentials (same as swissdamed2sqlite)
pub const DEFAULT_P12_KEY: &str = "swissdamed2sqlite-9dd3bf6717d4.p12";
pub const DEFAULT_SERVICE_EMAIL: &str =
    "swissdamed2sqlite@swissdamed2sqlite.iam.gserviceaccount.com";

/// Send an email with a file attachment via Gmail API.
pub fn send_email_with_attachment(
    p12_path: &str,
    service_email: &str,
    from_email: &str,
    to_email: &str,
    subject: &str,
    body_text: &str,
    attachment_path: &str,
) -> Result<()> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let url_engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    eprintln!("Sending {} via email to {} ...", attachment_path, to_email);

    let pem = extract_pem_from_p12(p12_path)?;
    let token = get_gmail_access_token(&pem, service_email, from_email)?;

    let file_name = std::path::Path::new(attachment_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(attachment_path);

    let file_content = std::fs::read(attachment_path)
        .with_context(|| format!("Cannot read {}", attachment_path))?;
    let encoded_attachment = engine.encode(&file_content);

    // Detect content type from extension
    let content_type = if file_name.ends_with(".csv") {
        "text/csv"
    } else if file_name.ends_with(".xlsx") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    } else if file_name.ends_with(".pdf") {
        "application/pdf"
    } else if file_name.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    };

    let boundary = "eudamed2firstbase_email_boundary";

    let raw_email = format!(
        "From: {from}\r\n\
         To: {to}\r\n\
         Subject: {subject}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: text/plain; charset=\"UTF-8\"\r\n\
         \r\n\
         {body}\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: {content_type}; name=\"{file_name}\"\r\n\
         Content-Disposition: attachment; filename=\"{file_name}\"\r\n\
         Content-Transfer-Encoding: base64\r\n\
         \r\n\
         {attachment}\r\n\
         --{boundary}--\r\n",
        from = from_email,
        to = to_email,
        subject = subject,
        boundary = boundary,
        body = body_text,
        content_type = content_type,
        file_name = file_name,
        attachment = encoded_attachment,
    );

    let encoded_message = url_engine.encode(raw_email.as_bytes());
    let payload = serde_json::json!({ "raw": encoded_message });

    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();

    let mut resp = agent
        .post("https://www.googleapis.com/gmail/v1/users/me/messages/send")
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .send(payload.to_string().as_bytes())?;

    let status = resp.status();
    let resp_body = resp.body_mut().read_to_string()?;

    if status.as_u16() >= 400 {
        anyhow::bail!("Gmail send failed ({}): {}", status, resp_body);
    }

    let result: serde_json::Value = serde_json::from_str(&resp_body)?;
    let id = result.get("id").and_then(|v| v.as_str()).unwrap_or("?");
    eprintln!("Email sent to {} (message id: {})", to_email, id);

    Ok(())
}

/// Extract PEM private key from .p12 file using openssl CLI.
fn extract_pem_from_p12(p12_path: &str) -> Result<String> {
    // Try with -legacy flag first (OpenSSL 3.x), fall back without it (LibreSSL/older)
    let output = Command::new("openssl")
        .args([
            "pkcs12", "-in", p12_path, "-nocerts", "-nodes",
            "-passin", "pass:notasecret", "-legacy",
        ])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => Command::new("openssl")
            .args([
                "pkcs12", "-in", p12_path, "-nocerts", "-nodes",
                "-passin", "pass:notasecret",
            ])
            .output()?,
    };

    if !output.status.success() {
        anyhow::bail!(
            "openssl pkcs12 failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}

/// Get Gmail API access token via JWT/service account.
fn get_gmail_access_token(
    pem_key: &str,
    service_email: &str,
    sub_email: &str,
) -> Result<String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::Serialize;

    #[derive(Serialize)]
    struct Claims {
        iss: String,
        scope: String,
        aud: String,
        exp: u64,
        iat: u64,
        sub: String,
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let claims = Claims {
        iss: service_email.to_string(),
        scope: "https://www.googleapis.com/auth/gmail.send".to_string(),
        aud: "https://oauth2.googleapis.com/token".to_string(),
        iat: now,
        exp: now + 3600,
        sub: sub_email.to_string(),
    };

    let header = Header::new(Algorithm::RS256);
    let key = EncodingKey::from_rsa_pem(pem_key.as_bytes())?;
    let jwt = encode(&header, &claims, &key)?;

    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();

    let form_body = format!(
        "grant_type={}&assertion={}",
        "urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer",
        jwt
    );

    let mut resp = agent
        .post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form_body.as_bytes())?;

    let body: serde_json::Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
    body.get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("No access_token in Gmail response: {}", body))
}
