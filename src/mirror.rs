//! `mirror` subcommand — build and refresh a **complete local mirror** of the
//! public EUDAMED device data as SQLite.
//!
//! Three phases, each independently runnable and each **resumable**:
//!
//! 1. `--crawl`   — page the public listing endpoint (`GET /devices/udiDiData?page=N&size=300`)
//!                  into table `devices_listing`. 300 is the server-side cap: asking for
//!                  500/1000 still returns 300, so ~10.8k pages cover the ~3.25 M devices.
//!                  Completed pages are checkpointed in `crawl_pages`, so an interrupted
//!                  run continues where it stopped and a later run picks up devices added
//!                  in the meantime (the corpus grew by ~17k during one 5 h crawl).
//! 2. `--details` — for each device, fetch the per-device detail
//!                  (`GET /devices/udiDiData/{uuid}`) **and** its Basic-UDI record
//!                  (`GET /devices/basicUdiData/udiDiData/{uuid}`) into `device_details`,
//!                  both stored verbatim as JSON (lossless). The listing alone carries
//!                  ~10 usable fields; the detail pair carries the regulated attribute set
//!                  (CND nomenclature, storage/handling, critical warnings, clinical sizes,
//!                  sterile/latex/single-use, manufacturer + AR with SRN, risk class,
//!                  MDR/IVDR characteristics, certificates). Restrict the working set with
//!                  `--gtin-file` when only a known GTIN list matters.
//! 3. `--flatten` — parse the stored JSON pair into the flat, queryable
//!                  `device_details_flat` (one row per device, ~70 TEXT columns).
//!                  Pure local work, no network; safe to re-run after a parser change.
//!
//! Rate limiting is the shared `download::eudamed_get` choke-point (paced by
//! `RateLimiter`, honors `Retry-After`), so the mirror survives EUDAMED's
//! ~60 req/60 s budget without dropping pages.

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::download::{app_data_dir, eudamed_agent, eudamed_get, RateLimiter};

const LIST_URL: &str = "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData";
const DETAIL_URL: &str = "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData";
const BASIC_URL: &str = "https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData";

/// Server-side page-size cap. Requesting more silently yields 300.
const PAGE_SIZE: u32 = 300;

/// Columns of `devices_listing`, mirroring the listing payload's useful fields.
const LIST_COLS: &[&str] = &[
    "primaryDi",
    "basicUdi",
    "uuid",
    "ulid",
    "riskClass",
    "tradeName",
    "manufacturerName",
    "manufacturerSrn",
    "deviceStatusType",
    "versionNumber",
    "deviceName",
    "deviceModel",
    "lastUpdateDate",
    "reference",
    "containerPackageCount",
    "mfOrPrSrn",
    "applicableLegislation",
    "authorisedRepresentativeSrn",
    "authorisedRepresentativeName",
    "sterile",
    "multiComponent",
    "latestVersion",
];

/// Columns of `device_details_flat` — the parsed union of the detail and
/// Basic-UDI payloads.
const FLAT_COLS: &[&str] = &[
    // identity
    "uuid",
    "primaryDi",
    "basicUdiCode",
    "issuingAgency",
    "reference",
    "productDesigner",
    // naming
    "tradeName",
    "deviceName",
    "additionalDescription",
    "additionalInformationUrl",
    // classification
    "riskClass",
    "legislation",
    "deviceCriterion",
    "specialDeviceType",
    "cndCodes",
    "cndTerms",
    // manufacturer / authorised representative
    "manufacturerName",
    "manufacturerSrn",
    "manufacturerCountry",
    "manufacturerAddress",
    "arName",
    "arSrn",
    "arCountry",
    "arAddress",
    // market
    "placedOnTheMarketCountry",
    "marketCountries",
    "marketCountryCount",
    "deviceStatus",
    // MDR / IVDR characteristics (Basic-UDI record)
    "active",
    "implantable",
    "measuringFunction",
    "multiComponent",
    "reusable",
    "sutures",
    "administeringMedicine",
    "animalTissues",
    "humanTissues",
    "medicinalProduct",
    "microbialSubstances",
    "medicalPurpose",
    "companionDiagnostics",
    "reagent",
    "instrument",
    "kit",
    "selfTesting",
    "nearPatientTesting",
    "professionalTesting",
    // detail characteristics
    "sterile",
    "sterilization",
    "latex",
    "singleUse",
    "reprocessed",
    "maxNumberOfReuses",
    "baseQuantity",
    "unitOfUse",
    "directMarking",
    "directMarkingDi",
    "secondaryDi",
    "containedItem",
    "annexXVI",
    "cmrSubstance",
    "endocrineDisruptor",
    "clinicalSizes",
    "storageConditions",
    "criticalWarnings",
    "storageSymbol",
    // UDI-PI flags
    "piBatch",
    "piSerial",
    "piMfgDate",
    "piExpiryDate",
    "piSoftware",
    // certificates
    "certificateCount",
    "certificateNumbers",
    // versioning
    "versionDate",
    "lastUpdated",
    "versionNumber",
];

/// Options parsed from the command line.
pub struct MirrorOpts {
    pub crawl: bool,
    pub details: bool,
    pub flatten: bool,
    pub db_path: PathBuf,
    pub gtin_file: Option<PathBuf>,
    pub threads: usize,
    /// Milliseconds between paced requests (aggregate across threads).
    pub rate_ms: u64,
}

/// Parse `mirror` arguments. `--all` turns on all three phases; with no phase
/// flag at all we also do all three, since a bare `mirror` most plausibly means
/// "give me the whole mirror".
pub fn parse_args(args: &[String]) -> Result<MirrorOpts> {
    let has = |name: &str| args.iter().any(|a| a == name);
    let val = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .filter(|s| !s.starts_with("--"))
            .cloned()
    };

    let all = has("--all");
    let (mut crawl, mut details, mut flatten) =
        (has("--crawl"), has("--details"), has("--flatten"));
    if all || (!crawl && !details && !flatten) {
        crawl = true;
        details = true;
        flatten = true;
    }

    let db_path = match val("--db") {
        Some(p) => PathBuf::from(p),
        None => {
            let dir = app_data_dir().join("db");
            std::fs::create_dir_all(&dir).ok();
            dir.join(format!("eudamed_{}.db", today_stamp()))
        }
    };

    Ok(MirrorOpts {
        crawl,
        details,
        flatten,
        db_path,
        gtin_file: val("--gtin-file").map(PathBuf::from),
        threads: val("--threads")
            .and_then(|s| s.parse().ok())
            .unwrap_or(8)
            .clamp(1, 32),
        rate_ms: val("--rate-ms").and_then(|s| s.parse().ok()).unwrap_or(120),
    })
}

fn today_stamp() -> String {
    // DD.MM.YYYY to match the sibling tools' date-stamped outputs.
    chrono::Local::now().format("%d.%m.%Y").to_string()
}

pub fn run(opts: &MirrorOpts) -> Result<()> {
    eprintln!("[mirror] db: {}", opts.db_path.display());
    if let Some(parent) = opts.db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(&opts.db_path)
        .with_context(|| format!("opening {}", opts.db_path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    ensure_schema(&conn)?;

    // `Connection` is `Send` but not `Sync`, so the worker threads share it
    // through a mutex that OWNS it (a `Mutex<&Connection>` would not be `Sync`).
    // Writes are short (one page / one device at a time) and SQLite serialises
    // them anyway, so the lock is not the bottleneck — the paced HTTP is.
    let conn = Mutex::new(conn);

    if opts.crawl {
        crawl_listing(&conn, opts)?;
    }
    if opts.details {
        fetch_details(&conn, opts)?;
    }
    if opts.flatten {
        flatten_details(&conn)?;
    }
    Ok(())
}

/// Run `f` with the shared connection locked. A poisoned mutex is recovered
/// rather than propagated: a panicked worker must not strand a long crawl.
fn with_conn<T>(conn: &Mutex<Connection>, f: impl FnOnce(&Connection) -> T) -> T {
    let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
    f(&guard)
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    let cols = LIST_COLS
        .iter()
        .map(|c| format!("\"{c}\" TEXT"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS devices_listing ({cols}, rawJson TEXT);
         CREATE TABLE IF NOT EXISTS crawl_pages (page INTEGER PRIMARY KEY, items INTEGER, fetched_at TEXT);
         CREATE INDEX IF NOT EXISTS idx_listing_di ON devices_listing(primaryDi);
         CREATE TABLE IF NOT EXISTS device_details (
             uuid TEXT PRIMARY KEY, primaryDi TEXT, detailJson TEXT, basicJson TEXT, fetched_at TEXT);
         CREATE INDEX IF NOT EXISTS idx_details_di ON device_details(primaryDi);"
    ))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 1 — listing crawl
// ---------------------------------------------------------------------------

fn crawl_listing(conn: &Mutex<Connection>, opts: &MirrorOpts) -> Result<()> {
    let agent = eudamed_agent();
    let limiter = RateLimiter::new(Duration::from_millis(opts.rate_ms));

    let probe = eudamed_get(&agent, &limiter, &format!("{LIST_URL}?page=0&size=1"), 5)
        .map_err(|e| anyhow::anyhow!("probing listing size: {e}"))?;
    let total = serde_json::from_str::<Value>(&probe)
        .ok()
        .and_then(|v| v.get("totalElements").and_then(|t| t.as_u64()))
        .unwrap_or(0);
    let pages = total.div_ceil(PAGE_SIZE as u64) as u32;

    let done: HashSet<u32> = with_conn(conn, |c| -> Result<HashSet<u32>> {
        let mut st = c.prepare("SELECT page FROM crawl_pages")?;
        let rows = st.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.filter_map(|r| r.ok()).map(|p| p as u32).collect())
    })?;
    let todo: Vec<u32> = (0..pages).filter(|p| !done.contains(p)).collect();
    eprintln!(
        "[crawl] total={total} pages={pages} done={} todo={}",
        done.len(),
        todo.len()
    );
    if todo.is_empty() {
        return Ok(());
    }

    let counter = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);
    let started = Instant::now();
    let total_todo = todo.len();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads)
        .build()?;
    pool.install(|| {
        use rayon::prelude::*;
        todo.par_iter().for_each(|&page| {
            let url = format!("{LIST_URL}?page={page}&size={PAGE_SIZE}");
            let body = match eudamed_get(&agent, &limiter, &url, 6) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[crawl] page {page} failed: {e}");
                    errors.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            let content = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("content").cloned())
                .and_then(|c| c.as_array().cloned())
                .unwrap_or_default();

            with_conn(conn, |c| {
                if let Err(e) = insert_listing_page(c, page, &content) {
                    eprintln!("[crawl] page {page} not stored: {e}");
                }
            });

            let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 100 == 0 {
                let el = started.elapsed().as_secs_f64();
                let rate = n as f64 / el.max(0.001);
                let eta = (total_todo - n) as f64 / rate / 60.0;
                eprintln!(
                    "[crawl] {n}/{total_todo} pages  err={}  {:.1}min  ETA {:.0}min",
                    errors.load(Ordering::Relaxed),
                    el / 60.0,
                    eta
                );
            }
        });
    });

    let (rows, ok) = with_conn(conn, |c| -> Result<(i64, i64)> {
        Ok((
            c.query_row("SELECT COUNT(*) FROM devices_listing", [], |r| r.get(0))?,
            c.query_row("SELECT COUNT(*) FROM crawl_pages", [], |r| r.get(0))?,
        ))
    })?;
    eprintln!(
        "[crawl] done: pages_ok={ok}/{pages} rows={rows} errors={}",
        errors.load(Ordering::Relaxed)
    );
    Ok(())
}

/// Store one page's devices and its checkpoint **atomically**. Both must land
/// together: rows without a checkpoint would be re-fetched and duplicated on the
/// next run, a checkpoint without rows would silently lose 300 devices.
fn insert_listing_page(conn: &Connection, page: u32, content: &[Value]) -> Result<()> {
    let placeholders = std::iter::repeat_n("?", LIST_COLS.len() + 1)
        .collect::<Vec<_>>()
        .join(",");
    let cols = LIST_COLS
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("INSERT INTO devices_listing ({cols}, rawJson) VALUES ({placeholders})");

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        {
            let mut st = conn.prepare_cached(&sql)?;
            for item in content {
                let mut vals: Vec<Option<String>> =
                    LIST_COLS.iter().map(|c| scalar(item.get(*c))).collect();
                vals.push(Some(item.to_string()));
                st.execute(rusqlite::params_from_iter(vals.iter()))?;
            }
        }
        conn.prepare_cached("INSERT OR REPLACE INTO crawl_pages VALUES (?,?,datetime('now'))")?
            .execute(rusqlite::params![page as i64, content.len() as i64])?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Flatten a JSON value to a single scalar string, unwrapping the common
/// `{"code": …}` / `{"name": …}` reference-data wrappers EUDAMED uses.
fn scalar(v: Option<&Value>) -> Option<String> {
    let v = v?;
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Object(map) => {
            for key in ["code", "name", "text", "value"] {
                if let Some(inner) = map.get(key) {
                    if inner.is_string() || inner.is_number() {
                        return Some(
                            inner
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| inner.to_string()),
                        );
                    }
                }
            }
            Some(v.to_string())
        }
        Value::Array(_) => Some(v.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Phase 2 — per-device detail + Basic-UDI fetch
// ---------------------------------------------------------------------------

fn fetch_details(conn: &Mutex<Connection>, opts: &MirrorOpts) -> Result<()> {
    let filter: Option<HashSet<String>> = match &opts.gtin_file {
        Some(p) => {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            let set: HashSet<String> = text
                .lines()
                .filter_map(|l| normalize_gtin(l.trim()))
                .collect();
            eprintln!(
                "[details] GTIN filter: {} codes from {}",
                set.len(),
                p.display()
            );
            Some(set)
        }
        None => None,
    };

    let done: HashSet<String> = with_conn(conn, |c| -> Result<HashSet<String>> {
        let mut st = c.prepare("SELECT uuid FROM device_details WHERE detailJson IS NOT NULL")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })?;

    let todo: Vec<(String, String)> = with_conn(conn, |c| -> Result<Vec<(String, String)>> {
        let mut st = c.prepare(
            "SELECT primaryDi, uuid FROM devices_listing WHERE primaryDi IS NOT NULL AND uuid IS NOT NULL",
        )?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut seen = HashSet::new();
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|(di, uuid)| {
                if done.contains(uuid) || !seen.insert(uuid.clone()) {
                    return false;
                }
                match &filter {
                    Some(f) => normalize_gtin(di).map(|k| f.contains(&k)).unwrap_or(false),
                    None => true,
                }
            })
            .collect())
    })?;
    eprintln!(
        "[details] todo={} (already stored: {})",
        todo.len(),
        done.len()
    );
    if todo.is_empty() {
        return Ok(());
    }

    let agent = eudamed_agent();
    let limiter = RateLimiter::new(Duration::from_millis(opts.rate_ms));
    let counter = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);
    let started = Instant::now();
    let total_todo = todo.len();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads)
        .build()?;
    pool.install(|| {
        use rayon::prelude::*;
        todo.par_iter().for_each(|(di, uuid)| {
            let detail = eudamed_get(
                &agent,
                &limiter,
                &format!("{DETAIL_URL}/{uuid}?languageIso2Code=en"),
                5,
            );
            let detail = match detail {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("[details] {di} ({uuid}) failed: {e}");
                    errors.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            };
            // The Basic-UDI record carries risk class, legislation, manufacturer,
            // AR and the MDR/IVDR characteristic flags. A miss here is not fatal:
            // the detail row is still worth storing.
            let basic = eudamed_get(
                &agent,
                &limiter,
                &format!("{BASIC_URL}/{uuid}?languageIso2Code=en"),
                4,
            )
            .ok();

            with_conn(conn, |c| {
                if let Err(e) = c.execute(
                    "INSERT OR REPLACE INTO device_details VALUES (?,?,?,?,datetime('now'))",
                    rusqlite::params![uuid, di, detail, basic],
                ) {
                    eprintln!("[details] {di} not stored: {e}");
                }
            });

            let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 250 == 0 {
                let el = started.elapsed().as_secs_f64();
                let rate = n as f64 / el.max(0.001);
                let eta = (total_todo - n) as f64 / rate / 60.0;
                eprintln!(
                    "[details] {n}/{total_todo}  err={}  {:.1}min  ETA {:.0}min",
                    errors.load(Ordering::Relaxed),
                    el / 60.0,
                    eta
                );
            }
        });
    });

    let stored: i64 = with_conn(conn, |c| {
        c.query_row("SELECT COUNT(*) FROM device_details", [], |r| r.get(0))
    })?;
    eprintln!(
        "[details] done: stored={stored} errors={}",
        errors.load(Ordering::Relaxed)
    );
    Ok(())
}

/// Canonical GTIN key: digits only, leading zeros stripped, so EAN-13 and
/// GTIN-14 spellings of the same article compare equal.
pub fn normalize_gtin(s: &str) -> Option<String> {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ---------------------------------------------------------------------------
// Phase 3 — flatten stored JSON into queryable columns
// ---------------------------------------------------------------------------

fn flatten_details(shared: &Mutex<Connection>) -> Result<()> {
    let guard = shared.lock().unwrap_or_else(|e| e.into_inner());
    let conn: &Connection = &guard;
    let cols = FLAT_COLS
        .iter()
        .map(|c| format!("\"{c}\" TEXT"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS device_details_flat;
         CREATE TABLE device_details_flat ({cols});"
    ))?;

    let rows: Vec<(String, String, Option<String>, Option<String>)> = {
        let mut st =
            conn.prepare("SELECT uuid, primaryDi, detailJson, basicJson FROM device_details")?;
        let it = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        it.filter_map(|r| r.ok()).collect()
    };

    let placeholders = std::iter::repeat_n("?", FLAT_COLS.len())
        .collect::<Vec<_>>()
        .join(",");
    let colnames = FLAT_COLS
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("INSERT INTO device_details_flat ({colnames}) VALUES ({placeholders})");

    conn.execute_batch("BEGIN")?;
    {
        let mut st = conn.prepare(&sql)?;
        for (uuid, di, dj, bj) in &rows {
            let d: Option<Value> = dj.as_deref().and_then(|s| serde_json::from_str(s).ok());
            let b: Option<Value> = bj.as_deref().and_then(|s| serde_json::from_str(s).ok());
            let vals = flatten_one(uuid, di, d.as_ref(), b.as_ref());
            st.execute(rusqlite::params_from_iter(vals.iter()))?;
        }
    }
    conn.execute_batch("COMMIT")?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_flat_di ON device_details_flat(primaryDi)")?;
    eprintln!("[flatten] {} rows -> device_details_flat", rows.len());
    Ok(())
}

/// `refdata.risk-class.class-iia` → `class-iia`; unwraps `{"code": …}` first.
fn code(v: Option<&Value>) -> String {
    let raw = match v {
        Some(Value::Object(m)) => m
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    raw.rsplit('.').next().unwrap_or("").to_string()
}

/// Best available string from EUDAMED's multilingual `{texts: [...]}` shape,
/// preferring the default language, then German, then whatever is present.
fn text(v: Option<&Value>) -> String {
    let Some(v) = v else { return String::new() };
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    let Some(obj) = v.as_object() else {
        return String::new();
    };
    if let Some(s) = obj.get("textByDefaultLanguage").and_then(|t| t.as_str()) {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let texts = obj.get("texts").and_then(|t| t.as_array());
    let Some(texts) = texts else {
        return String::new();
    };
    let pick = |iso: &str| -> Option<String> {
        texts.iter().find_map(|t| {
            let lang = t.get("language")?.get("isoCode")?.as_str()?;
            if lang == iso {
                t.get("text")?.as_str().map(str::to_string)
            } else {
                None
            }
        })
    };
    pick("de")
        .or_else(|| pick("en"))
        .or_else(|| {
            texts
                .iter()
                .find_map(|t| t.get("text").and_then(|s| s.as_str()).map(str::to_string))
        })
        .unwrap_or_default()
}

fn yes_no(v: Option<&Value>) -> String {
    match v.and_then(|x| x.as_bool()) {
        Some(true) => "Ja".into(),
        Some(false) => "Nein".into(),
        None => String::new(),
    }
}

fn num(v: Option<&Value>) -> String {
    match v {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn di_code(v: Option<&Value>) -> String {
    match v {
        Some(Value::Object(m)) => m
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn flatten_one(uuid: &str, di: &str, d: Option<&Value>, b: Option<&Value>) -> Vec<String> {
    use std::collections::HashMap;
    let mut f: HashMap<&str, String> = FLAT_COLS.iter().map(|c| (*c, String::new())).collect();
    let set = |f: &mut HashMap<&str, String>, k: &'static str, v: String| {
        f.insert(k, v);
    };

    if let Some(d) = d {
        let g = |k: &str| d.get(k);
        set(
            &mut f,
            "uuid",
            d.get("uuid")
                .and_then(|v| v.as_str())
                .unwrap_or(uuid)
                .to_string(),
        );
        set(&mut f, "primaryDi", {
            let c = di_code(g("primaryDi"));
            if c.is_empty() {
                di.to_string()
            } else {
                c
            }
        });
        set(
            &mut f,
            "issuingAgency",
            code(g("primaryDi").and_then(|p| p.get("issuingAgency"))),
        );
        set(&mut f, "reference", num(g("reference")));
        set(&mut f, "productDesigner", text(g("productDesigner")));
        set(&mut f, "tradeName", text(g("tradeName")));
        set(
            &mut f,
            "additionalDescription",
            text(g("additionalDescription")),
        );
        set(
            &mut f,
            "additionalInformationUrl",
            g("additionalInformationUrl")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        );

        // CND / EMDN nomenclature — the closest thing EUDAMED has to a product category.
        if let Some(cnds) = g("cndNomenclatures").and_then(|v| v.as_array()) {
            set(
                &mut f,
                "cndCodes",
                cnds.iter()
                    .filter_map(|c| c.get("code").and_then(|x| x.as_str()))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            set(
                &mut f,
                "cndTerms",
                cnds.iter()
                    .map(|c| text(c.get("description")))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }

        set(
            &mut f,
            "placedOnTheMarketCountry",
            g("placedOnTheMarket")
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        );
        if let Some(ms) = g("marketInfoLink")
            .and_then(|m| m.get("msWhereAvailable"))
            .and_then(|v| v.as_array())
        {
            let mut iso: Vec<String> = ms
                .iter()
                .filter_map(|m| {
                    m.get("country")?
                        .get("iso2Code")?
                        .as_str()
                        .map(str::to_string)
                })
                .collect();
            iso.sort();
            iso.dedup();
            set(&mut f, "marketCountryCount", iso.len().to_string());
            set(&mut f, "marketCountries", iso.join("; "));
        }
        set(
            &mut f,
            "deviceStatus",
            code(g("deviceStatus").and_then(|s| s.get("type"))),
        );

        set(&mut f, "sterile", yes_no(g("sterile")));
        set(&mut f, "sterilization", yes_no(g("sterilization")));
        set(&mut f, "latex", yes_no(g("latex")));
        set(&mut f, "singleUse", yes_no(g("singleUse")));
        set(&mut f, "reprocessed", yes_no(g("reprocessed")));
        set(&mut f, "maxNumberOfReuses", num(g("maxNumberOfReuses")));
        set(&mut f, "baseQuantity", num(g("baseQuantity")));
        set(&mut f, "unitOfUse", di_code(g("unitOfUse")));
        set(&mut f, "directMarking", yes_no(g("directMarking")));
        set(&mut f, "directMarkingDi", di_code(g("directMarkingDi")));
        set(&mut f, "secondaryDi", di_code(g("secondaryDi")));
        set(
            &mut f,
            "containedItem",
            g("containedItem")
                .map(|v| truncate(&v.to_string(), 300))
                .unwrap_or_default(),
        );
        set(&mut f, "annexXVI", yes_no(g("annexXVIApplicable")));
        set(&mut f, "cmrSubstance", yes_no(g("cmrSubstance")));
        set(
            &mut f,
            "endocrineDisruptor",
            yes_no(g("endocrineDisruptor")),
        );

        if let Some(cs) = g("clinicalSizes").and_then(|v| v.as_array()) {
            let s = cs
                .iter()
                .map(|c| {
                    format!(
                        "{}:{}{}",
                        code(c.get("type")),
                        num(c.get("value")),
                        code(c.get("unitOfMeasure"))
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            set(&mut f, "clinicalSizes", truncate(&s, 500));
        }
        if let Some(sh) = g("storageHandlingConditions").and_then(|v| v.as_array()) {
            let s = sh
                .iter()
                .map(|c| {
                    let t = code(c.get("typeCode"));
                    let d = text(c.get("description"));
                    if d.is_empty() {
                        t
                    } else {
                        format!("{t}: {d}")
                    }
                })
                .collect::<Vec<_>>()
                .join("; ");
            set(&mut f, "storageConditions", truncate(&s, 500));
        }
        if let Some(cw) = g("criticalWarnings").and_then(|v| v.as_array()) {
            let s = cw
                .iter()
                .map(|c| code(c.get("typeCode")))
                .collect::<Vec<_>>()
                .join("; ");
            set(&mut f, "criticalWarnings", truncate(&s, 400));
        }
        set(&mut f, "storageSymbol", code(g("storageSymbol")));

        if let Some(pi) = g("udiPiType") {
            set(&mut f, "piBatch", yes_no(pi.get("batchNumber")));
            set(&mut f, "piSerial", yes_no(pi.get("serializationNumber")));
            set(&mut f, "piMfgDate", yes_no(pi.get("manufacturingDate")));
            set(&mut f, "piExpiryDate", yes_no(pi.get("expirationDate")));
            set(
                &mut f,
                "piSoftware",
                yes_no(pi.get("softwareIdentification")),
            );
        }
        set(&mut f, "versionDate", num(g("versionDate")));
        set(&mut f, "lastUpdated", num(g("lastUpdated")));
        set(&mut f, "versionNumber", num(g("versionNumber")));
    } else {
        set(&mut f, "uuid", uuid.to_string());
        set(&mut f, "primaryDi", di.to_string());
    }

    if let Some(b) = b {
        let g = |k: &str| b.get(k);
        set(&mut f, "basicUdiCode", di_code(g("basicUdi")));
        set(
            &mut f,
            "deviceName",
            g("deviceName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        );
        set(&mut f, "riskClass", code(g("riskClass")));
        set(&mut f, "legislation", code(g("legislation")));
        set(
            &mut f,
            "deviceCriterion",
            g("deviceCriterion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        );
        set(&mut f, "specialDeviceType", code(g("specialDeviceType")));

        // The actor payloads keep country and address FLAT (`countryName`,
        // `geographicalAddress`) rather than under a nested address object —
        // reading them as nested silently yields empty columns.
        let str_at = |o: &Value, keys: &[&str]| -> String {
            keys.iter()
                .find_map(|k| o.get(*k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
                .unwrap_or("")
                .to_string()
        };
        if let Some(m) = g("manufacturer") {
            set(&mut f, "manufacturerName", str_at(m, &["name"]));
            set(&mut f, "manufacturerSrn", str_at(m, &["srn"]));
            set(
                &mut f,
                "manufacturerCountry",
                str_at(m, &["countryName", "countryIso2Code"]),
            );
            set(
                &mut f,
                "manufacturerAddress",
                str_at(m, &["geographicalAddress"]),
            );
        }
        if let Some(ar) = g("authorisedRepresentative") {
            set(
                &mut f,
                "arName",
                str_at(ar, &["name", "authorisedRepresentativeName"]),
            );
            set(
                &mut f,
                "arSrn",
                str_at(ar, &["srn", "authorisedRepresentativeSrn"]),
            );
            set(&mut f, "arCountry", str_at(ar, &["countryName"]));
            set(
                &mut f,
                "arAddress",
                str_at(ar, &["address", "geographicalAddress"]),
            );
        }
        for key in [
            "active",
            "implantable",
            "measuringFunction",
            "multiComponent",
            "reusable",
            "sutures",
            "administeringMedicine",
            "animalTissues",
            "humanTissues",
            "medicinalProduct",
            "microbialSubstances",
            "medicalPurpose",
            "companionDiagnostics",
            "reagent",
            "instrument",
            "kit",
            "selfTesting",
            "nearPatientTesting",
            "professionalTesting",
        ] {
            f.insert(key, yes_no(g(key)));
        }
        if let Some(certs) = g("deviceCertificateInfoList").and_then(|v| v.as_array()) {
            if !certs.is_empty() {
                set(&mut f, "certificateCount", certs.len().to_string());
                let nums = certs
                    .iter()
                    .filter_map(|c| c.get("certificateNumber").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                set(&mut f, "certificateNumbers", truncate(&nums, 300));
            }
        }
    }

    FLAT_COLS
        .iter()
        .map(|c| f.get(*c).cloned().unwrap_or_default())
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtin_normalization_matches_ean13_and_gtin14() {
        // The same article spelled as EAN-13 and GTIN-14 must collapse to one key,
        // otherwise a retail GTIN never matches its EUDAMED registration.
        assert_eq!(
            normalize_gtin("7640127798065"),
            normalize_gtin("07640127798065")
        );
        assert_eq!(normalize_gtin("07640127798065").unwrap(), "7640127798065");
        // Separators and placeholder rows must not produce bogus keys.
        assert_eq!(normalize_gtin("7640-127-798065").unwrap(), "7640127798065");
        assert_eq!(normalize_gtin("00000000000000"), None);
        assert_eq!(normalize_gtin(""), None);
    }

    #[test]
    fn code_strips_refdata_prefix() {
        let v: Value = serde_json::json!({"code": "refdata.risk-class.class-iia"});
        assert_eq!(code(Some(&v)), "class-iia");
        assert_eq!(code(None), "");
    }

    #[test]
    fn text_prefers_default_then_german() {
        let v: Value = serde_json::json!({
            "texts": [
                {"language": {"isoCode": "en"}, "text": "Wrist orthosis"},
                {"language": {"isoCode": "de"}, "text": "Handgelenkorthese"}
            ],
            "textByDefaultLanguage": null
        });
        assert_eq!(text(Some(&v)), "Handgelenkorthese");

        // A single unlabelled text (language: null) still has to come through —
        // EUDAMED uses that shape for allLanguagesApplicable entries.
        let v2: Value = serde_json::json!({
            "texts": [{"language": null, "text": "ORTHO X W/ BRACE XL"}],
            "textByDefaultLanguage": null
        });
        assert_eq!(text(Some(&v2)), "ORTHO X W/ BRACE XL");
    }

    #[test]
    fn flatten_reads_both_payloads() {
        let detail: Value = serde_json::json!({
            "uuid": "u-1",
            "primaryDi": {"code": "04046445699214", "issuingAgency": {"code": "refdata.issuing-agency.gs1"}},
            "tradeName": {"texts": [{"language": null, "text": "VenoTrain soft"}]},
            "cndNomenclatures": [{"code": "M030405", "description": {"textByDefaultLanguage": "WRIST-HAND ORTHOSES"}}],
            "sterile": false,
            "latex": true,
            "criticalWarnings": [{"typeCode": "refdata.critical-warnings-type.CW018"}],
            "udiPiType": {"batchNumber": true, "expirationDate": null}
        });
        let basic: Value = serde_json::json!({
            "riskClass": {"code": "refdata.risk-class.class-i"},
            "legislation": {"code": "refdata.applicable-legislation.mdr"},
            "manufacturer": {"name": "Bauerfeind AG", "srn": "DE-MF-000012345"},
            "implantable": false,
            "reusable": true
        });
        let vals = flatten_one("u-1", "04046445699214", Some(&detail), Some(&basic));
        let idx = |name: &str| FLAT_COLS.iter().position(|c| *c == name).unwrap();

        assert_eq!(vals[idx("primaryDi")], "04046445699214");
        assert_eq!(vals[idx("issuingAgency")], "gs1");
        assert_eq!(vals[idx("tradeName")], "VenoTrain soft");
        assert_eq!(vals[idx("cndCodes")], "M030405");
        assert_eq!(vals[idx("cndTerms")], "WRIST-HAND ORTHOSES");
        assert_eq!(vals[idx("riskClass")], "class-i");
        assert_eq!(vals[idx("legislation")], "mdr");
        assert_eq!(vals[idx("manufacturerName")], "Bauerfeind AG");
        assert_eq!(vals[idx("criticalWarnings")], "CW018");
        assert_eq!(vals[idx("piBatch")], "Ja");
        // A present-but-false flag must read "Nein", not blank: "no latex" and
        // "not stated" are different answers for a compliance question.
        assert_eq!(vals[idx("sterile")], "Nein");
        assert_eq!(vals[idx("latex")], "Ja");
        assert_eq!(vals[idx("implantable")], "Nein");
        assert_eq!(vals[idx("reusable")], "Ja");
        // Absent field stays empty rather than being invented.
        assert_eq!(vals[idx("piExpiryDate")], "");
        assert_eq!(vals.len(), FLAT_COLS.len());
    }

    #[test]
    fn actor_country_and_address_are_read_flat() {
        // EUDAMED keeps actor country/address as FLAT keys on the actor object.
        // Reading them as a nested `address.country.name` yields empty columns
        // without any error — this test is what catches that regression.
        let detail: Value = serde_json::json!({"uuid": "u-3", "primaryDi": {"code": "1"}});
        let basic: Value = serde_json::json!({
            "manufacturer": {
                "name": "Promedics Orthopaedics Limited",
                "srn": "GB-MF-000008558",
                "countryName": "United Kingdom (ex Northern Ireland)",
                "countryIso2Code": "UK",
                "geographicalAddress": "Block 7 Gareloch Road PA14 5XH Port Glasgow"
            },
            "authorisedRepresentative": {
                "name": "Dolsan AG",
                "srn": "CH-AR-000001724",
                "countryName": "Switzerland"
            }
        });
        let vals = flatten_one("u-3", "1", Some(&detail), Some(&basic));
        let idx = |name: &str| FLAT_COLS.iter().position(|c| *c == name).unwrap();
        assert_eq!(
            vals[idx("manufacturerCountry")],
            "United Kingdom (ex Northern Ireland)"
        );
        assert_eq!(
            vals[idx("manufacturerAddress")],
            "Block 7 Gareloch Road PA14 5XH Port Glasgow"
        );
        assert_eq!(vals[idx("manufacturerSrn")], "GB-MF-000008558");
        assert_eq!(vals[idx("arName")], "Dolsan AG");
        assert_eq!(vals[idx("arCountry")], "Switzerland");
    }

    #[test]
    fn flatten_survives_a_missing_basic_payload() {
        // Basic-UDI fetches are allowed to fail; the detail row must still flatten.
        let detail: Value = serde_json::json!({"uuid": "u-2", "primaryDi": {"code": "123"}});
        let vals = flatten_one("u-2", "123", Some(&detail), None);
        let idx = |name: &str| FLAT_COLS.iter().position(|c| *c == name).unwrap();
        assert_eq!(vals[idx("primaryDi")], "123");
        assert_eq!(vals[idx("riskClass")], "");
    }
}
