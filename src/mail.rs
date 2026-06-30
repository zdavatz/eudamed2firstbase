//! Gmail API email sending with file attachments.
//! Uses Google Service Account with domain-wide delegation.
//!
//! Credentials are read from `config.toml` (`[gmail]` section).
//! See `config.sample.toml` for the expected format.

use anyhow::{Context, Result};
use std::process::Command;

/// MIME content type for a file name, by extension.
fn content_type_for(file_name: &str) -> &'static str {
    let n = file_name.to_ascii_lowercase();
    if n.ends_with(".csv") {
        "text/csv"
    } else if n.ends_with(".html") || n.ends_with(".htm") {
        "text/html"
    } else if n.ends_with(".xlsx") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    } else if n.ends_with(".pdf") {
        "application/pdf"
    } else if n.ends_with(".json") {
        "application/json"
    } else if n.ends_with(".txt") || n.ends_with(".log") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

/// Send an email with a single file attachment via Gmail API.
/// Thin wrapper over [`send_email_with_attachments`].
pub fn send_email_with_attachment(
    p12_path: &str,
    service_email: &str,
    from_email: &str,
    to_email: &str,
    subject: &str,
    body_text: &str,
    attachment_path: &str,
) -> Result<()> {
    send_email_with_attachments(
        p12_path,
        service_email,
        from_email,
        to_email,
        subject,
        body_text,
        &[attachment_path.to_string()],
    )
}

/// Send an email with one or more file attachments via Gmail API.
/// `body_text` may be empty (an empty text/plain part is still included so the
/// message is well-formed; the recipient sees no body text).
pub fn send_email_with_attachments(
    p12_path: &str,
    service_email: &str,
    from_email: &str,
    to_email: &str,
    subject: &str,
    body_text: &str,
    attachment_paths: &[String],
) -> Result<()> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let url_engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    eprintln!(
        "Sending {} attachment(s) via email to {} ...",
        attachment_paths.len(),
        to_email
    );

    let pem = extract_pem_from_p12(p12_path)?;
    let token = get_gmail_access_token(&pem, service_email, from_email)?;

    let boundary = "eudamed2firstbase_email_boundary";
    let subject = encode_header(subject);

    // Header + leading text/plain part (empty body allowed).
    let mut raw_email = format!(
        "From: {from}\r\n\
         To: {to}\r\n\
         Subject: {subject}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: text/plain; charset=\"UTF-8\"\r\n\
         \r\n\
         {body}\r\n",
        from = from_email,
        to = to_email,
        subject = subject,
        boundary = boundary,
        body = body_text,
    );

    // One MIME part per attachment.
    for path in attachment_paths {
        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path.as_str());
        let file_content = std::fs::read(path).with_context(|| format!("Cannot read {}", path))?;
        let encoded_attachment = engine.encode(&file_content);
        let content_type = content_type_for(file_name);

        raw_email.push_str(&format!(
            "\r\n--{boundary}\r\n\
             Content-Type: {content_type}; name=\"{file_name}\"\r\n\
             Content-Disposition: attachment; filename=\"{file_name}\"\r\n\
             Content-Transfer-Encoding: base64\r\n\
             \r\n\
             {attachment}\r\n",
            boundary = boundary,
            content_type = content_type,
            file_name = file_name,
            attachment = encoded_attachment,
        ));
    }
    raw_email.push_str(&format!("--{boundary}--\r\n", boundary = boundary));

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

/// RFC 2047 MIME encoded-word for header values containing non-ASCII characters.
/// Pure-ASCII strings pass through unchanged.
fn encode_header(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    format!("=?UTF-8?B?{}?=", engine.encode(s.as_bytes()))
}

/// Locate the `openssl` binary by checking known absolute paths before falling
/// back to the PATH-resolved name.  Using absolute paths prevents PATH-hijacking
/// attacks where a malicious binary named `openssl` appears earlier in PATH.
fn find_openssl() -> &'static str {
    // Well-known absolute paths, checked in priority order.
    const CANDIDATES: &[&str] = &[
        "/usr/bin/openssl",          // Linux / macOS system default
        "/opt/homebrew/bin/openssl", // macOS Homebrew (Apple Silicon)
        "/usr/local/bin/openssl",    // macOS Homebrew (Intel) / custom installs
        "/opt/local/bin/openssl",    // MacPorts
    ];
    for path in CANDIDATES {
        if std::path::Path::new(path).exists() {
            return path;
        }
    }
    // Last resort: let the OS resolve via PATH (Windows or unusual layouts).
    // Log a warning so operators notice.
    eprintln!(
        "Warning: openssl not found at known absolute paths; \
         falling back to PATH resolution (potential security risk). \
         Set OPENSSL_BIN env var or install openssl to /usr/bin/openssl."
    );
    "openssl"
}

/// Extract PEM private key from .p12 file using the openssl CLI.
pub(crate) fn extract_pem_from_p12(p12_path: &str) -> Result<String> {
    let openssl = std::env::var("OPENSSL_BIN").unwrap_or_else(|_| find_openssl().to_string());

    // Try with -legacy flag first (OpenSSL 3.x), fall back without it (LibreSSL/older).
    let output = Command::new(&openssl)
        .args([
            "pkcs12",
            "-in",
            p12_path,
            "-nocerts",
            "-nodes",
            "-passin",
            "pass:notasecret",
            "-legacy",
        ])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => Command::new(&openssl)
            .args([
                "pkcs12",
                "-in",
                p12_path,
                "-nocerts",
                "-nodes",
                "-passin",
                "pass:notasecret",
            ])
            .output()
            .with_context(|| format!("Failed to run openssl ({})", openssl))?,
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
fn get_gmail_access_token(pem_key: &str, service_email: &str, sub_email: &str) -> Result<String> {
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
        "urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer", jwt
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
