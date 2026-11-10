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
//!
//! Basic UDI-DI from the datalake (07.11.2026): every row read by [`query`]
//! that belongs to a non-sterile, non-SPP MDR Class I or IVDR Class A device
//! is turned into a Basic UDI-DI record in the public format
//! ([`basic_from_row`]); `run_download` writes it instead of asking the public
//! `basicUdiData` endpoint. Measured on the 52'851 cached public records: all
//! 18'305 such MDR Class I devices carry no certificate (the only field the
//! datalake lacks that the converter needs for them); sterile Class I devices
//! do (32 of 1'358), so they stay public. `EUDAMED_BASIC_SOURCE=public`
//! disables it.

use std::collections::HashMap;
use std::sync::Mutex;
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

/// The operator forced the datalake (`EUDAMED_LISTING_SOURCE=datalake`).
/// Unlike [`enabled`], this stays false after an automatic fallback, so a
/// public listing probe is still how an unset run discovers the block.
/// A forced run must not probe the public listing at all: those 429s are not
/// a signal, and they used to arm the detail/basic circuit breaker before
/// any device record was fetched.
pub fn forced() -> bool {
    source_env() == "datalake"
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
    /// Fingerprint of the row's content fields ([`row_hash`]).
    pub row_hash: String,
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

/// MULTI_COMPONENT_ID -> public refdata code (checked against cached public
/// records 07.11.2026: -116 system, -117 procedure-pack, -541 spp-procedure-pack).
fn multi_component_code(id: i64) -> Option<&'static str> {
    match id {
        -116 => Some("refdata.multi-component.system"),
        -117 => Some("refdata.multi-component.procedure-pack"),
        -540 => Some("refdata.multi-component.spp-system"),
        -541 => Some("refdata.multi-component.spp-procedure-pack"),
        _ => None,
    }
}

const LEGISLATION_MDR: i64 = -197;
const LEGISLATION_IVDR: i64 = -198;
const RISK_CLASS_I: i64 = -203;
const RISK_CLASS_A: i64 = -199;

/// Basic UDI-DI from the datalake is on unless `EUDAMED_BASIC_SOURCE=public`.
pub fn basic_enabled() -> bool {
    !std::env::var("EUDAMED_BASIC_SOURCE")
        .map(|v| v.trim().eq_ignore_ascii_case("public"))
        .unwrap_or(false)
}

fn flag(v: &Value, k: &str) -> Value {
    match num(v, k) {
        Some(1) => Value::Bool(true),
        Some(0) => Value::Bool(false),
        _ => Value::Null,
    }
}

fn opt_text(v: &Value, k: &str) -> Value {
    let t = text(v, k);
    if t.is_empty() {
        Value::Null
    } else {
        Value::String(t)
    }
}

/// A Basic UDI-DI record in the public `basicUdiData` format, built from one
/// datalake `/udi` row — only for devices that need no certificate: MDR
/// Class I or IVDR Class A, `STERILE` = 0, not a system/procedure pack (SPP;
/// its medical purpose comes in one language only). None for every other row,
/// which then keeps the public Basic UDI-DI request. Carries `"source":
/// "datalake"`; it has no version numbers and no `lastUpdated`.
pub fn basic_from_row(v: &Value) -> Option<Value> {
    let legislation = num(v, "APPLICABLE_LEGISLATION_ID")?;
    let risk = num(v, "RISK_CLASS_ID")?;
    let legislation_code = match (legislation, risk) {
        (LEGISLATION_MDR, RISK_CLASS_I) => "refdata.applicable-legislation.mdr",
        (LEGISLATION_IVDR, RISK_CLASS_A) => "refdata.applicable-legislation.ivdr",
        _ => return None,
    };
    if num(v, "STERILE") != Some(0) {
        return None;
    }
    let criterion = text(v, "DEVICE_CRITERION");
    if criterion.eq_ignore_ascii_case("SPP") {
        return None;
    }
    let basic_udi = text(v, "BASIC_UDI");
    let mf_srn = text(v, "MF_SRN");
    if basic_udi.is_empty() || mf_srn.is_empty() || text(v, "UUID").is_empty() {
        return None;
    }
    let multi_component = match num(v, "MULTI_COMPONENT_ID").and_then(multi_component_code) {
        Some(code) => {
            serde_json::json!({ "code": code, "criterion": opt_text(v, "DEVICE_CRITERION") })
        }
        None => Value::Null,
    };
    let ar_srn = text(v, "AR_SRN");
    let authorised_representative = if ar_srn.is_empty() {
        Value::Null
    } else {
        serde_json::json!({ "srn": ar_srn, "name": opt_text(v, "AR_NAME") })
    };
    Some(serde_json::json!({
        "source": "datalake",
        "uuid": opt_text(v, "BASIC_UDI_DATA_UUID"),
        "active": flag(v, "ACTIVE"),
        "implantable": flag(v, "IMPLANTABLE"),
        "sutures": flag(v, "SUTURES"),
        "measuringFunction": flag(v, "MEASURING_FUNCTION"),
        "reusable": flag(v, "REUSABLE"),
        "medicinalProduct": flag(v, "MEDICINAL_PRODUCT"),
        "administeringMedicine": flag(v, "ADMINISTERING_MEDICINE"),
        "humanTissues": flag(v, "HUMAN_TISSUES"),
        "animalTissues": flag(v, "ANIMAL_TISSUES"),
        "humanProduct": flag(v, "HUMAN_PRODUCT"),
        "deviceName": opt_text(v, "DEVICE_NAME"),
        "deviceModel": opt_text(v, "DEVICE_MODEL"),
        "multiComponent": multi_component,
        "riskClass": { "code": risk_code(risk) },
        "legislation": { "code": legislation_code, "legacyDirective": false },
        "basicUdi": { "code": basic_udi },
        "manufacturer": { "srn": mf_srn, "name": opt_text(v, "MF_NAME") },
        "authorisedRepresentative": authorised_representative,
    }))
}

/// Basic UDI-DI records built from the rows this process has read, by device
/// uuid. Filled by [`query`], taken by `run_download`.
static BASICS: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

/// Remember a Basic UDI-DI for the latest row of every uuid in `rows` (same
/// choice as [`dedup_latest`]).
fn remember_basics(rows: &[Value]) {
    if !basic_enabled() {
        return;
    }
    let mut best: HashMap<String, (bool, Option<i64>, &Value)> = HashMap::new();
    for r in rows {
        let uuid = text(r, "UUID");
        if uuid.is_empty() {
            continue;
        }
        let key = (
            num(r, "LATEST_VERSION") == Some(1),
            num(r, "VERSION_NUMBER"),
        );
        if best.get(&uuid).is_none_or(|(l, n, _)| key > (*l, *n)) {
            best.insert(uuid, (key.0, key.1, r));
        }
    }
    let mut guard = BASICS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    for (uuid, (_, _, r)) in best {
        match basic_from_row(r) {
            Some(b) => {
                map.insert(uuid, b);
            }
            None => {
                map.remove(&uuid);
            }
        }
    }
}

/// The datalake Basic UDI-DI for `uuid`, if this process read an eligible row.
pub fn basic_for(uuid: &str) -> Option<Value> {
    BASICS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()?
        .get(uuid)
        .cloned()
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

/// Row fields left out of [`row_hash`]: the row id and the version fields
/// (a version bump is detected through the version itself).
const HASH_SKIP: &[&str] = &["ID", "LATEST_VERSION", "VERSION_NUMBER"];

/// SHA-256 over every content field of one `/udi` row (sorted by name,
/// numbers and nulls as JSON). It changes when the Basic UDI-DI part,
/// manufacturer, AR, status or a package DI changes, even when
/// `VERSION_NUMBER` stays the same.
pub fn row_hash(v: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    if let Some(obj) = v.as_object() {
        let mut keys: Vec<&String> = obj
            .keys()
            .filter(|k| !HASH_SKIP.contains(&k.as_str()))
            .collect();
        keys.sort();
        for k in keys {
            h.update(k.as_bytes());
            h.update(b"=");
            h.update(obj[k].to_string().as_bytes());
            h.update(b"\n");
        }
    }
    format!("{:x}", h.finalize())
}

/// Map one `/udi` row. None without a uuid.
pub fn parse_row(v: &Value) -> Option<DlDevice> {
    let uuid = text(v, "UUID");
    if uuid.is_empty() {
        return None;
    }
    Some(DlDevice {
        row_hash: row_hash(v),
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

/// Base URL of `/udi`. Tests point this at a local server; production uses
/// [`UDI_URL`].
fn udi_url() -> String {
    #[cfg(test)]
    if let Some(url) = TEST_UDI_URL.with(|c| c.borrow().clone()) {
        return url;
    }
    UDI_URL.to_string()
}

#[cfg(test)]
thread_local! {
    static TEST_UDI_URL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Point `/udi` at a local server for one test thread. `None` restores the
/// real datalake URL.
#[cfg(test)]
pub fn set_test_udi_url(url: Option<String>) {
    TEST_UDI_URL.with(|c| *c.borrow_mut() = url);
}

/// `/udi?<key>=<value>`, following `nextLink`; stops after `max_pages` pages
/// (then `complete` is false). An error on a later page keeps what was read.
pub fn query(key: &str, value: &str, max_pages: Option<usize>) -> Result<DlQuery> {
    if down() {
        anyhow::bail!(
            "datalake not available: {DOWN_AFTER} requests in a row failed, not asked again in this run"
        );
    }
    let res = query_inner(key, value, max_pages);
    if breaker_counts() {
        use std::sync::atomic::Ordering::Relaxed;
        match &res {
            Ok(_) => DL_FAILS.store(0, Relaxed),
            Err(e) => {
                if DL_FAILS.fetch_add(1, Relaxed) + 1 == DOWN_AFTER {
                    eprintln!(
                        "WARNING: EUDAMED datalake not available ({e}) — {DOWN_AFTER} requests in a row failed; the rest of this run reads the cached listing"
                    );
                }
            }
        }
    }
    res
}

/// Datalake requests that failed in a row (any error before the first page).
static DL_FAILS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// After this many failed requests in a row the datalake is not asked again in
/// this process (10.11.2026: a 504 outage made every request wait 120 s, so a
/// run with 6'000 GTIN lookups would have crawled through timeouts for days).
const DOWN_AFTER: usize = 5;

#[cfg(test)]
thread_local! {
    static TEST_BREAKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests share the process-wide counter, so it only counts for a test that
/// asked for it.
#[cfg(test)]
pub fn set_test_breaker(on: bool) {
    TEST_BREAKER.with(|c| c.set(on));
    DL_FAILS.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Put the calling test into the state after [`DOWN_AFTER`] failed requests.
#[cfg(test)]
pub fn trip_test_breaker() {
    TEST_BREAKER.with(|c| c.set(true));
    DL_FAILS.store(DOWN_AFTER, std::sync::atomic::Ordering::Relaxed);
}

fn breaker_counts() -> bool {
    #[cfg(test)]
    {
        TEST_BREAKER.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// The datalake failed [`DOWN_AFTER`] times in a row in this process.
pub fn down() -> bool {
    breaker_counts() && DL_FAILS.load(std::sync::atomic::Ordering::Relaxed) >= DOWN_AFTER
}

fn query_inner(key: &str, value: &str, max_pages: Option<usize>) -> Result<DlQuery> {
    let agent = agent();
    let mut url = format!(
        "{}?format=json&api-version=v1.0&{key}={}",
        udi_url(),
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
    remember_basics(&rows);
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

    fn class_i_row() -> Value {
        serde_json::json!({
            "UUID": "d1", "PRIMARY_DI": "04250203936606", "VERSION_NUMBER": 1.0, "LATEST_VERSION": 1.0,
            "APPLICABLE_LEGISLATION_ID": -197.0, "RISK_CLASS_ID": -203.0, "STERILE": 0.0,
            "ACTIVE": 0.0, "IMPLANTABLE": 0.0, "REUSABLE": 1.0, "MEASURING_FUNCTION": null,
            "BASIC_UDI": "B-1", "BASIC_UDI_DATA_UUID": "b1", "DEVICE_NAME": "Bandage",
            "MF_SRN": "US-MF-000000001", "MF_NAME": "M", "AR_SRN": "IE-AR-000000001", "AR_NAME": "A",
            "MULTI_COMPONENT_ID": -116.0, "DEVICE_CRITERION": "STANDARD"
        })
    }

    #[test]
    fn basic_from_row_builds_a_public_shaped_record_for_class_i() {
        let b = basic_from_row(&class_i_row()).expect("eligible");
        let parsed = crate::api_detail::parse_basic_udi_di(&b.to_string()).unwrap();
        assert_eq!(
            parsed.risk_class_code().as_deref(),
            Some("refdata.risk-class.class-i")
        );
        assert_eq!(parsed.regulatory_act().as_deref(), Some("MDR"));
        assert_eq!(parsed.reusable, Some(true));
        assert_eq!(parsed.active, Some(false));
        assert_eq!(parsed.measuring_function, None);
        assert_eq!(
            parsed.basic_udi.and_then(|d| d.code).as_deref(),
            Some("B-1")
        );
        assert_eq!(
            parsed
                .authorised_representative
                .and_then(|a| a.srn)
                .as_deref(),
            Some("IE-AR-000000001")
        );
        assert_eq!(
            parsed.multi_component.and_then(|m| m.code).as_deref(),
            Some("refdata.multi-component.system")
        );
    }

    #[test]
    fn row_hash_sees_content_changes_but_not_id_or_version() {
        let base = class_i_row();
        let h = row_hash(&base);
        let mut moved = base.clone();
        moved["ID"] = serde_json::json!(42.0);
        moved["VERSION_NUMBER"] = serde_json::json!(2.0);
        moved["LATEST_VERSION"] = serde_json::json!(0.0);
        assert_eq!(row_hash(&moved), h);
        let mut new_ar = base.clone();
        new_ar["AR_SRN"] = serde_json::json!("NL-AR-000000002");
        assert_ne!(row_hash(&new_ar), h);
        let mut new_flag = base;
        new_flag["IMPLANTABLE"] = serde_json::json!(1.0);
        assert_ne!(row_hash(&new_flag), h);
        assert_eq!(parse_row(&class_i_row()).unwrap().row_hash, h);
    }

    #[test]
    fn basic_from_row_skips_devices_that_can_carry_certificates() {
        let mut sterile = class_i_row();
        sterile["STERILE"] = serde_json::json!(1.0);
        let mut unknown_sterile = class_i_row();
        unknown_sterile["STERILE"] = Value::Null;
        let mut class_iia = class_i_row();
        class_iia["RISK_CLASS_ID"] = serde_json::json!(-204.0);
        let mut legacy = class_i_row();
        legacy["APPLICABLE_LEGISLATION_ID"] = serde_json::json!(-53.0);
        let mut spp = class_i_row();
        spp["DEVICE_CRITERION"] = serde_json::json!("SPP");
        for r in [sterile, unknown_sterile, class_iia, legacy, spp] {
            assert!(basic_from_row(&r).is_none(), "{r}");
        }
    }
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
