//! Read the customer SRN worklist from a Google Sheet via the service account
//! configured under `[gmail]` / `[sheet]` in config.toml. Used by the
//! `sync-srns` subcommand so the nightly `check` always picks up newly added
//! SRNs without anyone editing `srns_sheet.txt` by hand.
//!
//! Read-only (scope `spreadsheets.readonly`, no domain-wide delegation needed —
//! the sheet is shared directly with the service-account email as Viewer).

use anyhow::{anyhow, Context, Result};

/// Mint a read-only Sheets access token for the service account (no `sub`
/// impersonation — the sheet is shared with the SA itself).
fn sheets_access_token(pem_key: &str, service_email: &str) -> Result<String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::Serialize;

    #[derive(Serialize)]
    struct Claims {
        iss: String,
        scope: String,
        aud: String,
        exp: u64,
        iat: u64,
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    let claims = Claims {
        iss: service_email.to_string(),
        scope: "https://www.googleapis.com/auth/spreadsheets.readonly".to_string(),
        aud: "https://oauth2.googleapis.com/token".to_string(),
        iat: now,
        exp: now + 3600,
    };

    let header = Header::new(Algorithm::RS256);
    let key = EncodingKey::from_rsa_pem(pem_key.as_bytes())
        .context("Failed to load service-account private key (PEM)")?;
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
        .ok_or_else(|| anyhow!("No access_token in Google token response: {}", body))
}

/// Validate the SRN shape EUDAMED uses, e.g. `DE-MF-000017808`,
/// `CH-AR-000012345`, `FR-PR-000009999`.
fn is_srn(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let (cc, kind, num) = (parts[0], parts[1], parts[2]);
    cc.len() == 2
        && cc.chars().all(|c| c.is_ascii_uppercase())
        && matches!(kind, "MF" | "AR" | "PR")
        && num.len() >= 6
        && num.chars().all(|c| c.is_ascii_digit())
}

/// Validate a GTIN (UDI-DI primary code): all ASCII digits, GS1 length 8..=14
/// (GTIN-8/12/13/14). EUDAMED's `primaryDi` filter expects the exact numeric
/// code; non-numeric primaries (HIBC/IFA) are not GTINs and are rejected here.
fn is_gtin(s: &str) -> bool {
    let len = s.len();
    (8..=14).contains(&len) && s.chars().all(|c| c.is_ascii_digit())
}

/// Read the first column of an A1 range from the configured Google Sheet as
/// trimmed, non-empty cell strings (in sheet order, no de-dup, no validation).
/// Shared by `fetch_srns` / `fetch_gtins`. Errors if `[sheet] spreadsheet_id`
/// or the `[gmail]` service-account fields are unset, or the API call fails.
fn fetch_first_column(config: &crate::config::Config, range: &str) -> Result<Vec<String>> {
    let sheet = &config.sheet;
    let gmail = &config.gmail;
    if sheet.spreadsheet_id.trim().is_empty() {
        return Err(anyhow!(
            "Set [sheet] spreadsheet_id in config.toml to sync from the Google Sheet"
        ));
    }
    if gmail.p12_key.trim().is_empty() || gmail.service_email.trim().is_empty() {
        return Err(anyhow!(
            "Set [gmail] p12_key and service_email in config.toml (the same service account reads the sheet)"
        ));
    }

    let pem = crate::mail::extract_pem_from_p12(&gmail.p12_key)?;
    let token = sheets_access_token(&pem, &gmail.service_email)?;

    let url = format!(
        "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
        sheet.spreadsheet_id,
        urlencode(range)
    );

    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();

    let mut resp = agent
        .get(&url)
        .header("Authorization", &format!("Bearer {}", token))
        .call()
        .context("Sheets values.get request failed")?;

    let status = resp.status();
    let text = resp.body_mut().read_to_string()?;
    if status != 200 {
        return Err(anyhow!("Sheets API returned HTTP {}: {}", status, text));
    }
    let body: serde_json::Value = serde_json::from_str(&text)?;
    let rows = body
        .get("values")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("Sheets response has no `values` array: {}", body))?;

    let mut out = Vec::new();
    for row in rows {
        let cell = row
            .as_array()
            .and_then(|r| r.first())
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !cell.is_empty() {
            out.push(cell);
        }
    }
    Ok(out)
}

/// Fetch the SRN list from the configured Google Sheet, de-duplicated,
/// preserving sheet order.
pub fn fetch_srns(config: &crate::config::Config) -> Result<Vec<String>> {
    let cells = fetch_first_column(config, &config.sheet.srn_range)?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cell in cells {
        let srn = cell.to_uppercase();
        if is_srn(&srn) && seen.insert(srn.clone()) {
            out.push(srn);
        }
    }
    Ok(out)
}

/// Fetch the customer GTIN worklist from the configured Google Sheet
/// (`[sheet] gtin_range`, default the `eudamed2firstbase_GTIN` tab),
/// de-duplicated, preserving sheet order. Invalid/non-numeric cells (header,
/// notes) are dropped.
pub fn fetch_gtins(config: &crate::config::Config) -> Result<Vec<String>> {
    let cells = fetch_first_column(config, &config.sheet.gtin_range)?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cell in cells {
        if is_gtin(&cell) && seen.insert(cell.clone()) {
            out.push(cell);
        }
    }
    Ok(out)
}

/// Minimal percent-encoding for the A1 range path segment (covers `!`, space,
/// `:` and anything non-alphanumeric except `-._~`).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srn_shape() {
        assert!(is_srn("DE-MF-000017808"));
        assert!(is_srn("CH-AR-000012345"));
        assert!(is_srn("FR-PR-000009999"));
        assert!(!is_srn("SRN")); // header
        assert!(!is_srn("DE-XX-000017808")); // bad kind
        assert!(!is_srn("DEU-MF-000017808")); // 3-letter country
        assert!(!is_srn("DE-MF-123")); // too short
        assert!(!is_srn("")); // empty
    }

    #[test]
    fn gtin_shape() {
        assert!(is_gtin("04034342074074")); // 14-digit GTIN-14
        assert!(is_gtin("7612345000435")); // 13-digit
        assert!(is_gtin("12345678")); // 8-digit GTIN-8
        assert!(!is_gtin("GTIN")); // header
        assert!(!is_gtin("H123ABC")); // HIBC / non-numeric
        assert!(!is_gtin("1234567")); // too short
        assert!(!is_gtin("040343420740745")); // 15 digits, too long
        assert!(!is_gtin("")); // empty
    }

    #[test]
    fn url_encodes_range() {
        assert_eq!(
            urlencode("eudamed2firstbase_SRN!B1:B"),
            "eudamed2firstbase_SRN%21B1%3AB"
        );
    }
}
