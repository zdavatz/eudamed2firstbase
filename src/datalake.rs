//! Listing source: the official EUDAMED Public API of the SANTE datalake
//! (`api.datalake.sante.service.ec.europa.eu/eudamed/udi`, 05.11.2026).
//!
//! Since 05.11.2026 ~12:00 CET the public endpoints (`ec.europa.eu/tools/eudamed/
//! api/devices/…`) answer every automated client with 429 and point to this API.
//! It needs no key and showed no rate limit (16 parallel, no 429), but it carries
//! a REDUCED, flat record: uuid, UDI-DI version, status, risk class, Basic UDI-DI,
//! manufacturer / AR SRN, package DIs and the MDR flags — no certificates, market
//! countries, package quantities, substances or addresses. So it replaces the
//! listing / GTIN lookups that drive the version check, NOT the detail + Basic
//! UDI-DI downloads the converter needs.
//!
//! Used automatically once the public listing answers with EUDAMED's block
//! (429 with the M2M notice, v1.0.112 — GUI users cannot set env vars);
//! `EUDAMED_LISTING_SOURCE=datalake` forces it, `=public` disables it. Versions were checked against
//! listing_cache / udi_versions: 90 of 90 overlapping devices equal.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

pub const UDI_URL: &str = "https://api.datalake.sante.service.ec.europa.eu/eudamed/udi";
/// Answers take 10-25 s for 1'000 rows; keep the timeout well above that.
const TIMEOUT_SECS: u64 = 180;
const ATTEMPTS: u32 = 4;
const RETRY_WAIT_SECS: u64 = 15;

/// Set once the public listing answered with EUDAMED's block (v1.0.112): the
/// rest of the process lists via the datalake. GUI users cannot set env vars.
static FALLBACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn source_env() -> String {
    std::env::var("EUDAMED_LISTING_SOURCE")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Listing via the datalake: forced by `EUDAMED_LISTING_SOURCE=datalake`, or
/// automatically after the public listing was blocked (unless `=public`).
pub fn enabled() -> bool {
    match source_env().as_str() {
        "datalake" => true,
        "public" => false,
        _ => {
            FALLBACK.load(std::sync::atomic::Ordering::Relaxed)
                || crate::download::public_endpoint_blocked()
        }
    }
}

/// May the listing switch to the datalake automatically? (not with `=public`)
pub fn fallback_allowed() -> bool {
    source_env() != "public"
}

/// Switch the rest of this process to the datalake listing (logged once).
pub fn activate_fallback(reason: &str) {
    if fallback_allowed() && !FALLBACK.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!(
            "Listing source: public EUDAMED listing blocked ({reason}) — switching to the EUDAMED datalake API"
        );
    }
}

/// An `eudamed_get` error that means "blocked / throttled" (429).
pub fn is_block_error(e: &str) -> bool {
    e.starts_with("HTTP 429") || e.contains("blocked (429)")
}

/// One device of the datalake `/udi` answer, mapped to listing_cache columns.
#[derive(Debug, Clone, PartialEq)]
pub struct DlDevice {
    pub uuid: String,
    pub primary_di: String,
    pub version: Option<u32>,
    pub srn: String,
    pub manufacturer_name: String,
    pub trade_name: String,
    /// EUDAMED refdata code, e.g. `refdata.risk-class.class-iia` ("" if unknown).
    pub risk_class: String,
    /// EUDAMED refdata code, e.g. `refdata.device-model-status.on-the-market`.
    pub device_status: String,
    pub budi_ulid: String,
}

/// DEVICE_STATUS_TYPE_ID -> listing code (ids from `/reference?CODE=DEVICE_STATUS_TYPE_ID`).
fn status_code(id: i64) -> &'static str {
    match id {
        -11 => "refdata.device-model-status.on-the-market",
        -12 => "refdata.device-model-status.no-longer-on-the-market",
        -790 => "refdata.device-model-status.not-intended-for-eu-market",
        _ => "",
    }
}

/// RISK_CLASS_ID -> listing code (ids from `/reference?CODE=RISK_CLASS_ID`).
fn risk_code(id: i64) -> &'static str {
    match id {
        -203 => "refdata.risk-class.class-i",
        -204 => "refdata.risk-class.class-iia",
        -205 => "refdata.risk-class.class-iib",
        -10 => "refdata.risk-class.class-iii",
        -199 => "refdata.risk-class.class-a",
        -200 => "refdata.risk-class.class-b",
        -201 => "refdata.risk-class.class-c",
        -202 => "refdata.risk-class.class-d",
        -154 => "refdata.risk-class.aimdd",
        -157 => "refdata.risk-class.ivd-general",
        -219 => "refdata.risk-class.ivd-devices-self-testing",
        -155 => "refdata.risk-class.ivd-annex-ii-list-a",
        -156 => "refdata.risk-class.ivd-annex-ii-list-b",
        _ => "",
    }
}

fn num(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_f64()).map(|f| f as i64)
}
fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Map one `/udi` row. None without a uuid.
pub fn parse_row(v: &Value) -> Option<DlDevice> {
    let uuid = text(v, "UUID");
    if uuid.is_empty() {
        return None;
    }
    Some(DlDevice {
        uuid,
        primary_di: text(v, "PRIMARY_DI"),
        version: num(v, "VERSION_NUMBER").and_then(|n| u32::try_from(n).ok()),
        srn: text(v, "MF_SRN"),
        manufacturer_name: text(v, "MF_NAME"),
        trade_name: text(v, "TRADE_NAME"),
        risk_class: num(v, "RISK_CLASS_ID")
            .map(risk_code)
            .unwrap_or("")
            .to_string(),
        device_status: num(v, "DEVICE_STATUS_TYPE_ID")
            .map(status_code)
            .unwrap_or("")
            .to_string(),
        budi_ulid: text(v, "BASIC_UDI_DATA_ULID"),
    })
}

/// Keep one row per uuid: the latest version (LATEST_VERSION = 1 rows first,
/// then the highest VERSION_NUMBER).
pub fn dedup_latest(rows: &[Value]) -> Vec<DlDevice> {
    let mut best: std::collections::BTreeMap<String, (bool, Option<u32>, DlDevice)> =
        std::collections::BTreeMap::new();
    for r in rows {
        let Some(d) = parse_row(r) else { continue };
        let latest = num(r, "LATEST_VERSION") == Some(1);
        let better = match best.get(&d.uuid) {
            None => true,
            Some((l, v, _)) => (latest, d.version) > (*l, *v),
        };
        if better {
            best.insert(d.uuid.clone(), (latest, d.version, d));
        }
    }
    best.into_values().map(|(_, _, d)| d).collect()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(TIMEOUT_SECS)))
        .build()
        .new_agent()
}

fn get_page(agent: &ureq::Agent, url: &str) -> Result<Value> {
    let mut last = String::new();
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(RETRY_WAIT_SECS * attempt as u64));
        }
        match agent.get(url).header("Cache-Control", "no-cache").call() {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                if status != 200 {
                    last = format!(
                        "HTTP {status}: {}",
                        body.chars().take(120).collect::<String>()
                    );
                    continue;
                }
                match serde_json::from_str::<Value>(&body) {
                    Ok(v) if v.get("value").and_then(|x| x.as_array()).is_some() => return Ok(v),
                    Ok(_) => last = "answer without `value`".into(),
                    Err(e) => last = format!("not JSON: {e}"),
                }
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(anyhow!("datalake {url}: {last}"))
}

/// Result of one filtered `/udi` query.
pub struct DlQuery {
    pub devices: Vec<DlDevice>,
    /// Pages read (1'000 rows each).
    pub pages: usize,
    /// The last page had no `nextLink` (every device of the filter was read).
    pub complete: bool,
}

/// `/udi?<key>=<value>`, following `nextLink`; stops after `max_pages` pages
/// (then `complete` is false). An error on a later page keeps what was read.
pub fn query(key: &str, value: &str, max_pages: Option<usize>) -> Result<DlQuery> {
    let agent = agent();
    let mut url = format!(
        "{UDI_URL}?format=json&api-version=v1.0&{key}={}",
        urlencode(value)
    );
    let mut rows: Vec<Value> = Vec::new();
    let mut pages = 0usize;
    let complete = loop {
        let page = match get_page(&agent, &url) {
            Ok(p) => p,
            Err(e) if pages == 0 => return Err(e),
            Err(_) => break false,
        };
        pages += 1;
        if let Some(a) = page.get("value").and_then(|x| x.as_array()) {
            rows.extend(a.iter().cloned());
        }
        match page.get("nextLink").and_then(|x| x.as_str()) {
            Some(next) if !next.is_empty() => {
                if max_pages.is_some_and(|m| pages >= m) {
                    break false;
                }
                url = next.to_string();
            }
            _ => break true,
        }
    };
    Ok(DlQuery {
        devices: dedup_latest(&rows),
        pages,
        complete,
    })
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uuid: &str, ver: f64, latest: f64) -> Value {
        serde_json::json!({
            "UUID": uuid, "PRIMARY_DI": "00840273201991", "VERSION_NUMBER": ver,
            "LATEST_VERSION": latest, "MF_SRN": "US-MF-000016658", "MF_NAME": "M",
            "TRADE_NAME": "T", "RISK_CLASS_ID": -204.0, "DEVICE_STATUS_TYPE_ID": -11.0,
            "BASIC_UDI_DATA_ULID": "01M4"
        })
    }

    #[test]
    fn row_maps_to_listing_codes() {
        let d = parse_row(&row("u1", 2.0, 1.0)).unwrap();
        assert_eq!(d.version, Some(2));
        assert_eq!(d.risk_class, "refdata.risk-class.class-iia");
        assert_eq!(d.device_status, "refdata.device-model-status.on-the-market");
        assert_eq!(d.srn, "US-MF-000016658");
    }

    #[test]
    fn dedup_keeps_the_latest_version_per_uuid() {
        let rows = vec![
            row("u1", 1.0, 0.0),
            row("u1", 2.0, 1.0),
            row("u2", 3.0, 1.0),
        ];
        let d = dedup_latest(&rows);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].uuid, "u1");
        assert_eq!(d[0].version, Some(2));
    }

    #[test]
    fn unknown_ids_give_empty_codes() {
        let mut r = row("u1", 1.0, 1.0);
        r["RISK_CLASS_ID"] = serde_json::json!(-1.0);
        r["DEVICE_STATUS_TYPE_ID"] = serde_json::Value::Null;
        let d = parse_row(&r).unwrap();
        assert_eq!(d.risk_class, "");
        assert_eq!(d.device_status, "");
    }

    #[test]
    fn block_errors_are_recognised() {
        assert!(is_block_error("HTTP 429 (EUDAMED block, M2M notice)"));
        assert!(is_block_error("HTTP 429 (waited 60s)"));
        assert!(is_block_error(
            "EUDAMED public endpoint blocked (429), request skipped"
        ));
        assert!(!is_block_error("HTTP 503"));
        assert!(!is_block_error("network error: timeout"));
    }
}
