//! Hand-off lists for the operator when the datalake fails (20.11.2026).
//!
//! Without the datalake the nightly runs on the cached listing: new devices
//! and version bumps are not found, and the public endpoints answer every
//! automated client with 429. The operator then downloads the records in a
//! browser. This module writes what to download:
//!
//! - `log/handoff/urls.txt`: the public listing URLs of every worklist SRN
//!   (size=300, `ceil(total / 300)` pages: the newest hand-downloaded
//!   `totalElements`, else the `listing_cache` count), for
//!   `listing-compare`;
//! - `log/handoff/uuids.txt`: known devices whose detail or Basic UDI-DI
//!   record is missing (neither in `eudamed_json/` nor in an `import/*_detail`
//!   / `import/*_basic` directory), plus `log/outdated_uuids.txt`, minus
//!   `log/no_basic_uuids.txt` (devices the operator checked by hand: EUDAMED
//!   has no Basic UDI-DI for them, so asking again every night is useless).
//!
//! With `EUDAMED_HANDOFF_HOST` (`user@host`, set in the untracked nightly
//! wrapper — never commit the host) both lists are copied to that host:
//! `/tmp/uuid` and `/tmp/url` are deleted and recreated empty first, then
//! `uuids.txt` / `urls.txt` are copied into them.
//!
//! [`collect_local`] (step 5, `handoff --collect <dir>`, called by
//! `drop_import.sh` for pages from the local `drop/url_down`) compares the
//! listing pages and writes the next lists (UUIDs, or still missing pages).
//! Nothing here downloads from EUDAMED or polls another host.
//!
//! Every list written into `log/handoff/` is recorded in the DB (21.11.2026,
//! [`put_list`]): `handoff_write` (one row per written file: time, file, item
//! count) and `handoff_item` (per file and line: first / last time handed out,
//! how often). The GTIN lookups keep their state in `handoff_gtin` (requested,
//! found, not found) — the files in `log/handoff/` are outputs only and are
//! rebuilt from the DB, since they get taken away (20./21.11.2026).

use anyhow::{bail, Context, Result};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::download::cached_record_ok;
use crate::listing_compare::listing_url;

/// Listing page size of the URLs (same as `listing-compare`).
const PAGE_SIZE: u64 = 300;
/// Directories on the hand-off host. Cleared before every copy.
pub const REMOTE_UUID_DIR: &str = "/tmp/uuid";
pub const REMOTE_URL_DIR: &str = "/tmp/url";
/// Env var with the hand-off host (`user@host`).
pub const HOST_ENV: &str = "EUDAMED_HANDOFF_HOST";

pub struct Handoff {
    pub urls: Vec<String>,
    pub uuids: Vec<String>,
}

/// URLs for `(srn, devices)`: pages 0..ceil(devices/300), at least one page.
fn urls_for(srn_counts: &[(String, u64)]) -> Vec<String> {
    let mut out = Vec::new();
    for (srn, n) in srn_counts {
        let pages = n.div_ceil(PAGE_SIZE).max(1);
        for p in 0..pages {
            out.push(listing_url(srn, p));
        }
    }
    out
}

/// UUIDs found as a usable record file in `dir` (`<uuid>.json`).
fn records_in(dir: &Path, into: &mut HashSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") || !cached_record_ok(&p) {
            continue;
        }
        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
            into.insert(stem.to_string());
        }
    }
}

/// Record directories of one kind (`detail` / `basic`): `eudamed_json/<kind>`
/// and every `import/*_<kind>*` directory (hand-downloaded, not yet pushed).
fn record_dirs(data: &Path, kind: &str) -> Vec<PathBuf> {
    let mut dirs = vec![data.join("eudamed_json").join(kind)];
    if let Ok(rd) = std::fs::read_dir(data.join("import")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && name.contains(&format!("_{kind}")) && !name.ends_with("_old") {
                dirs.push(e.path());
            }
        }
    }
    dirs
}

/// `totalElements` of the newest hand-downloaded first page of `srn`
/// (`import/*_listing/<SRN>_page0.json`; directory names start with the date).
fn last_total(data: &Path, srn: &str) -> Option<u64> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(data.join("import"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_listing"))
        })
        .collect();
    dirs.sort();
    dirs.iter().rev().find_map(|d| {
        let body = std::fs::read_to_string(d.join(format!("{srn}_page0.json"))).ok()?;
        serde_json::from_str::<serde_json::Value>(&body)
            .ok()?
            .get("totalElements")?
            .as_u64()
    })
}

/// Build both lists. `data` is the app data dir (`~/eudamed2firstbase`).
pub fn build(data: &Path, srns: &[String]) -> Result<Handoff> {
    let db = rusqlite::Connection::open_with_flags(
        data.join("db").join("version_tracking.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("open version_tracking.db")?;

    let mut srn_counts = Vec::new();
    {
        let mut st = db.prepare("SELECT COUNT(*) FROM listing_cache WHERE srn = ?1")?;
        let mut seen = HashSet::new();
        for s in srns {
            let s = s.trim();
            if s.is_empty() || !seen.insert(s.to_string()) {
                continue;
            }
            let n: u64 = st.query_row([s], |r| r.get(0))?;
            // The newest hand-downloaded totalElements is the best count: the
            // listing_cache count also holds devices no longer listed (20.11.:
            // BR-MF-000014512 3'992 cached vs 2'919 listed = 4 empty pages).
            // An SRN that grew since is caught by listing-compare (missing pages).
            let n = last_total(data, s).unwrap_or(n);
            srn_counts.push((s.to_string(), n));
        }
    }

    let mut detail = HashSet::new();
    for d in record_dirs(data, "detail") {
        records_in(&d, &mut detail);
    }
    let mut basic = HashSet::new();
    for d in record_dirs(data, "basic") {
        records_in(&d, &mut basic);
    }

    let mut uuids = BTreeSet::new();
    {
        let mut st = db.prepare("SELECT uuid FROM listing_cache")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        for u in rows.flatten() {
            if !detail.contains(&u) || !basic.contains(&u) {
                uuids.insert(u);
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string(data.join("log").join("outdated_uuids.txt")) {
        for l in s.lines() {
            let u = l.split('\t').next().unwrap_or("").trim();
            if !u.is_empty() && !u.starts_with('#') {
                uuids.insert(u.to_string());
            }
        }
    }

    for u in no_basic(data) {
        uuids.remove(&u);
    }

    Ok(Handoff {
        urls: urls_for(&srn_counts),
        uuids: uuids.into_iter().collect(),
    })
}

/// `log/no_basic_uuids.txt`: one UUID per line (tab + note allowed, `#`
/// comments). Devices without a Basic UDI-DI in EUDAMED (checked by the
/// operator, 20.11.2026: 6e7daa00…). The Basic guard still holds them back.
fn no_basic(data: &Path) -> HashSet<String> {
    std::fs::read_to_string(data.join("log").join("no_basic_uuids.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('\t').next().unwrap_or("").trim().to_string())
        .filter(|u| !u.is_empty() && !u.starts_with('#'))
        .collect()
}

fn lines(v: &[String]) -> String {
    v.iter().map(|l| format!("{l}\n")).collect()
}

/// Create the hand-off tables (the DB is opened raw, see `ensure_listing_cache`).
fn ensure_tables(con: &rusqlite::Connection) -> Result<()> {
    con.execute_batch(
        "CREATE TABLE IF NOT EXISTS handoff_write (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             written_at TEXT NOT NULL,
             file TEXT NOT NULL,
             items INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS handoff_item (
             file TEXT NOT NULL,
             item TEXT NOT NULL,
             first_at TEXT NOT NULL,
             last_at TEXT NOT NULL,
             times INTEGER NOT NULL DEFAULT 1,
             PRIMARY KEY (file, item));
         CREATE TABLE IF NOT EXISTS drop_received (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             received_at TEXT NOT NULL,
             kind TEXT NOT NULL,
             item TEXT NOT NULL,
             outcome TEXT NOT NULL,
             bytes INTEGER NOT NULL,
             path TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS idx_drop_received_item ON drop_received(item);
         CREATE TABLE IF NOT EXISTS handoff_gtin (
             gtin TEXT PRIMARY KEY,
             requested_at TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'requested',
             answered_at TEXT);",
    )?;
    Ok(())
}

fn open_db(data: &Path) -> Result<rusqlite::Connection> {
    let con = rusqlite::Connection::open(data.join("db").join("version_tracking.db"))?;
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    ensure_tables(&con)?;
    Ok(con)
}

/// `handoff --received <list>` (21.11.2026, `drop_import.sh`): record every
/// file taken from the drop folder in `drop_received` — one line per file,
/// `<kind>\t<outcome>\t<path>` (kind detail / basic / listing / gtin, outcome
/// imported / rejected, path where it was moved). `item` is the file name
/// without `.json` (UUID, `<SRN>_page<N>` or GTIN), so it joins `handoff_item`.
pub fn record_received(data: &Path, list: &Path) -> Result<usize> {
    let body = std::fs::read_to_string(list).with_context(|| format!("read {}", list.display()))?;
    let con = open_db(data)?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut n = 0;
    con.execute_batch("BEGIN")?;
    {
        let mut st = con.prepare_cached(
            "INSERT INTO drop_received (received_at, kind, item, outcome, bytes, path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for l in body.lines() {
            let mut f = l.splitn(3, '\t');
            let (Some(kind), Some(outcome), Some(path)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let p = Path::new(path);
            let item = p
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .trim_end_matches(".json")
                .to_string();
            let bytes = std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(-1);
            st.execute(rusqlite::params![now, kind, item, outcome, bytes, path])?;
            n += 1;
        }
    }
    con.execute_batch("COMMIT")?;
    Ok(n)
}

/// Lists whose entries are held back after a hand-out. `gtin_urls.txt` is a
/// large work list taken in portions, so it is always written in full (an
/// answered GTIN leaves it anyway); so are the answer lists.
const TODO_LISTS: [&str; 2] = ["uuids.txt", "urls.txt"];

/// Hours before an entry of a to-do list is handed out again
/// (`HANDOFF_RETRY_HOURS`, default 24). 21.11.2026: the 21 devices without a
/// Basic UDI-DI were put into `uuids.txt` 8 times in 4 hours (every poll),
/// their details came back 3 times and the Basic UDI-DI never.
fn retry_hours() -> i64 {
    std::env::var("HANDOFF_RETRY_HOURS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(24)
}

/// Write `log/handoff/<name>` and record what was handed out in the DB. For
/// the to-do lists, an entry handed out less than `retry_hours()` ago is held
/// back (it reappears once the time is up; an entry whose record came in is
/// no longer in `items` at all). Returns (written, held back).
fn put_list(data: &Path, name: &str, items: &[String]) -> Result<(usize, usize)> {
    let dir = data.join("log").join("handoff");
    std::fs::create_dir_all(&dir)?;
    let con = open_db(data)?;
    let now_t = chrono::Local::now();
    let now = now_t.format("%Y-%m-%d %H:%M:%S").to_string();
    let out: Vec<String> = if TODO_LISTS.contains(&name) {
        let cutoff = (now_t - chrono::Duration::hours(retry_hours()))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let mut st = con.prepare_cached(
            "SELECT 1 FROM handoff_item WHERE file = ?1 AND item = ?2 AND last_at > ?3",
        )?;
        let mut keep = Vec::with_capacity(items.len());
        for it in items {
            if !st.exists(rusqlite::params![name, it, cutoff])? {
                keep.push(it.clone());
            }
        }
        keep
    } else {
        items.to_vec()
    };
    std::fs::write(dir.join(name), lines(&out))
        .with_context(|| format!("write {}", dir.join(name).display()))?;
    con.execute_batch("BEGIN")?;
    con.execute(
        "INSERT INTO handoff_write (written_at, file, items) VALUES (?1, ?2, ?3)",
        rusqlite::params![now, name, out.len() as i64],
    )?;
    {
        let mut st = con.prepare_cached(
            "INSERT INTO handoff_item (file, item, first_at, last_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(file, item) DO UPDATE SET last_at = ?3, times = times + 1",
        )?;
        for it in &out {
            st.execute(rusqlite::params![name, it, now])?;
        }
    }
    con.execute_batch("COMMIT")?;
    Ok((out.len(), items.len() - out.len()))
}

/// Write `log/handoff/uuids.txt` and `urls.txt`; other files there stay.
/// Returns the directory.
pub fn write(data: &Path, h: &Handoff) -> Result<PathBuf> {
    // The directory is no longer emptied first (21.11.2026): that deleted the
    // GTIN lists next to these two files.
    let dir = data.join("log").join("handoff");
    put_list(data, "uuids.txt", &h.uuids)?;
    put_list(data, "urls.txt", &h.urls)?;
    Ok(dir)
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("{cmd:?}"))?;
    if !out.status.success() {
        bail!(
            "{:?} failed ({}): {}",
            cmd.get_program(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Clear `/tmp/uuid` and `/tmp/url` on `host`, then copy the two lists into
/// them. Nothing is copied when clearing fails.
pub fn upload(host: &str, dir: &Path) -> Result<()> {
    check_host(host)?;
    let opts = SSH_OPTS;
    run(Command::new("ssh").args(opts).arg(host).arg(format!(
        "rm -rf {REMOTE_UUID_DIR} {REMOTE_URL_DIR} && mkdir -p {REMOTE_UUID_DIR} {REMOTE_URL_DIR}"
    )))?;
    run(Command::new("scp")
        .args(opts)
        .arg(dir.join("uuids.txt"))
        .arg(format!("{host}:{REMOTE_UUID_DIR}/uuids.txt")))?;
    run(Command::new("scp")
        .args(opts)
        .arg(dir.join("urls.txt"))
        .arg(format!("{host}:{REMOTE_URL_DIR}/urls.txt")))?;
    Ok(())
}

/// Write the lists after a nightly `check`. `with_urls` (the datalake failed):
/// both lists. Otherwise (records could not be downloaded, e.g. public 429)
/// only `uuids.txt`; `urls.txt` stays as it is. Never copies anything to
/// another host. Errors are logged, never fatal.
pub fn run_handoff(data: &Path, srns: &[String], with_urls: bool, log: &dyn Fn(&str)) {
    let h = match build(data, srns) {
        Ok(h) => h,
        Err(e) => return log(&format!("Hand-off: lists not built: {e:#}")),
    };
    if with_urls {
        match write(data, &h) {
            Ok(dir) => log(&format!(
                "Hand-off: {} listing URL(s), {} UUID(s) without detail/Basic UDI-DI -> {}",
                h.urls.len(),
                h.uuids.len(),
                dir.display()
            )),
            Err(e) => log(&format!("Hand-off: lists not written: {e:#}")),
        }
        return;
    }
    let dir = data.join("log").join("handoff");
    let res = put_list(data, "uuids.txt", &h.uuids);
    match res {
        Ok((n, held)) => log(&format!(
            "Hand-off: {} UUID(s) without detail/Basic UDI-DI, {n} handed out, {held} held back (handed out < {} h ago) -> {} (urls.txt unchanged)",
            h.uuids.len(),
            retry_hours(),
            dir.join("uuids.txt").display()
        )),
        Err(e) => log(&format!("Hand-off: uuids.txt not written: {e:#}")),
    }
}

const SSH_OPTS: [&str; 4] = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=30"];

fn check_host(host: &str) -> Result<()> {
    if host.is_empty() || host.starts_with('-') || host.contains(char::is_whitespace) {
        bail!("{HOST_ENV}: invalid host {host:?}");
    }
    Ok(())
}

/// Expected `(SRN, page)` from the last hand-off `urls.txt`.
fn expected_pages(data: &Path) -> Vec<(String, u64)> {
    let body = std::fs::read_to_string(data.join("log").join("handoff").join("urls.txt"))
        .unwrap_or_default();
    body.lines()
        .filter_map(|u| {
            let q = u.split_once('?')?.1;
            let mut srn = None;
            let mut page = None;
            for kv in q.split('&') {
                match kv.split_once('=') {
                    Some(("srn", v)) => srn = Some(v.to_string()),
                    Some(("page", v)) => page = v.parse().ok(),
                    _ => {}
                }
            }
            Some((srn?, page?))
        })
        .collect()
}

/// Pages still to download: those `totalElements` requires for the SRNs that
/// have pages, plus every expected page of an SRN without any usable page.
fn still_missing(
    expected: &[(String, u64)],
    srns: &std::collections::BTreeMap<String, crate::listing_compare::SrnPages>,
    c: &crate::listing_compare::Comparison,
) -> Vec<(String, u64)> {
    let mut m: BTreeSet<(String, u64)> = c.missing_pages.iter().cloned().collect();
    for (s, p) in expected {
        if !srns.contains_key(s) {
            m.insert((s.clone(), *p));
        }
    }
    m.into_iter().collect()
}

/// Insert the new devices of a complete round into `listing_cache` (rows from
/// the listing pages; no Basic UDI-DI version — the listing's is always 0), so
/// the nightly hand-off keeps listing them until their records are there.
fn insert_new(db: &Path, dir: &Path, new: &HashSet<String>) -> Result<usize> {
    let con = rusqlite::Connection::open(db)?;
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut n = 0;
    for e in std::fs::read_dir(dir)?.flatten() {
        let Ok(body) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        for d in v
            .get("content")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
        {
            if new.contains(&txt(d, "uuid")) {
                n += insert_row(&con, d, &now)?;
            }
        }
    }
    Ok(n)
}

fn txt(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Insert one listing item (public listing page or `mirror` rawJson, same
/// shape) into `listing_cache`. Never overwrites a known device.
fn insert_row(con: &rusqlite::Connection, d: &serde_json::Value, now: &str) -> Result<usize> {
    let code = |k: &str| {
        d.get(k)
            .and_then(|x| x.get("code"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let uuid = txt(d, "uuid");
    if uuid.is_empty() {
        return Ok(0);
    }
    Ok(con.execute(
        "INSERT OR IGNORE INTO listing_cache (uuid, srn, manufacturer_name, primary_di, trade_name, risk_class, device_status, version_number, budi_version_number, listed_at, budi_ulid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10)",
        rusqlite::params![
            uuid,
            txt(d, "manufacturerSrn"),
            txt(d, "manufacturerName"),
            txt(d, "primaryDi"),
            txt(d, "tradeName"),
            code("riskClass"),
            code("deviceStatusType"),
            d.get("versionNumber").and_then(|x| x.as_i64()),
            now,
            txt(d, "basicUdiDiDataUlid"),
        ],
    )?)
}

/// The UUIDs of `file` (one per line) that are in `listing_cache`, with their
/// SRN, sorted by UUID. Used by `drop_import.sh` (`handoff --known`) to plan
/// the push: thousands of UUIDs in a shell `sqlite3` argument fail with
/// "Argument list too long" (20.11.2026, 6'752 UUIDs).
pub fn known_in_cache(data: &Path, file: &Path) -> Result<Vec<(String, String)>> {
    let wanted: BTreeSet<String> = std::fs::read_to_string(file)
        .with_context(|| format!("read {}", file.display()))?
        .lines()
        .map(|l| l.split('\t').next().unwrap_or("").trim().to_string())
        .filter(|u| !u.is_empty() && !u.starts_with('#'))
        .collect();
    let con = rusqlite::Connection::open_with_flags(
        data.join("db").join("version_tracking.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("open version_tracking.db")?;
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    let mut st = con.prepare_cached("SELECT srn FROM listing_cache WHERE uuid = ?1")?;
    let mut out = Vec::new();
    for u in wanted {
        if let Ok(srn) = st.query_row([&u], |r| r.get::<_, String>(0)) {
            out.push((u, srn));
        }
    }
    Ok(out)
}

/// Public GTIN lookup URL (same query as the nightly's GTIN pass). The
/// operator saves the answer as `<GTIN>.json` into `drop/gtin/`.
pub fn gtin_url(gtin: &str) -> String {
    format!(
        "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData?page=0&pageSize=5&primaryDi={gtin}&iso2Code=en&languageIso2Code=en"
    )
}

fn read_gtins(path: &Path) -> Result<BTreeSet<String>> {
    Ok(std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?
        .lines()
        .map(|l| l.split('\t').next().unwrap_or("").trim().to_string())
        .filter(|g| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit()))
        .collect())
}

/// GTIN hand-off state in `handoff_gtin` (21.11.2026). The first run imports
/// the earlier file state from `log/handoff_state/` (kept on disk).
fn migrate_state(data: &Path, con: &rusqlite::Connection) -> Result<()> {
    let n: i64 = con.query_row("SELECT COUNT(*) FROM handoff_gtin", [], |r| r.get(0))?;
    let st = data.join("log").join("handoff_state");
    if n > 0 || !st.exists() {
        return Ok(());
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let read = |f: &str| -> Result<BTreeSet<String>> {
        let p = st.join(f);
        if p.exists() {
            read_gtins(&p)
        } else {
            Ok(BTreeSet::new())
        }
    };
    request_gtins(con, &read("gtins_requested.txt")?, &now)?;
    answer_gtins(con, &read("gtins_not_found.txt")?, "not_found", &now)?;
    answer_gtins(con, &read("gtins_found.txt")?, "found", &now)?;
    Ok(())
}

fn request_gtins(con: &rusqlite::Connection, gtins: &BTreeSet<String>, now: &str) -> Result<()> {
    con.execute_batch("BEGIN")?;
    {
        let mut st = con.prepare_cached(
            "INSERT OR IGNORE INTO handoff_gtin (gtin, requested_at) VALUES (?1, ?2)",
        )?;
        for g in gtins {
            st.execute(rusqlite::params![g, now])?;
        }
    }
    con.execute_batch("COMMIT")?;
    Ok(())
}

/// Store answers; `found` always wins over an earlier `not_found`.
fn answer_gtins(
    con: &rusqlite::Connection,
    gtins: &BTreeSet<String>,
    status: &str,
    now: &str,
) -> Result<()> {
    con.execute_batch("BEGIN")?;
    {
        let mut st = con.prepare_cached(
            "INSERT INTO handoff_gtin (gtin, requested_at, status, answered_at) VALUES (?1, ?3, ?2, ?3)
             ON CONFLICT(gtin) DO UPDATE SET status = ?2, answered_at = ?3
             WHERE handoff_gtin.status <> 'found'",
        )?;
        for g in gtins {
            st.execute(rusqlite::params![g, status, now])?;
        }
    }
    con.execute_batch("COMMIT")?;
    Ok(())
}

/// Rebuild `log/handoff/gtin_urls.txt` (requested, unanswered, not in
/// listing_cache) and `gtins_found.txt` / `gtins_not_found.txt` from the DB.
/// Returns the number of URLs handed out (the rest is held back, see `put_list`).
fn write_gtin_lists(data: &Path, con: &rusqlite::Connection) -> Result<usize> {
    let pick = |sql: &str| -> Result<Vec<String>> {
        let mut st = con.prepare(sql)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    };
    let ask = pick(
        "SELECT gtin FROM handoff_gtin WHERE status = 'requested'
         AND gtin NOT IN (SELECT primary_di FROM listing_cache WHERE primary_di IS NOT NULL)
         ORDER BY gtin",
    )?;
    let urls: Vec<String> = ask.iter().map(|g| gtin_url(g)).collect();
    let (n_out, _) = put_list(data, "gtin_urls.txt", &urls)?;
    put_list(
        data,
        "gtins_found.txt",
        &pick("SELECT gtin FROM handoff_gtin WHERE status = 'found' ORDER BY gtin")?,
    )?;
    put_list(
        data,
        "gtins_not_found.txt",
        &pick("SELECT gtin FROM handoff_gtin WHERE status = 'not_found' ORDER BY gtin")?,
    )?;
    Ok(n_out)
}

/// `handoff --gtins <file> [--mirror <db>]` (20.11.2026): bring a GTIN list
/// into the hand-off. GTINs already in `listing_cache` are skipped; GTINs
/// found in the `mirror` database (an earlier full listing crawl,
/// `devices_listing.rawJson`) are inserted into `listing_cache`, so their
/// devices land in `uuids.txt`; the rest go to `log/handoff/gtin_urls.txt`
/// (public GTIN lookups for the browser, answers into `drop/gtin/`).
pub fn add_gtins(
    data: &Path,
    gtins_file: &Path,
    mirror: Option<&Path>,
    srns: &[String],
    log: &dyn Fn(&str),
) -> Result<()> {
    let gtins = read_gtins(gtins_file)?;
    let db = data.join("db").join("version_tracking.db");
    let con = rusqlite::Connection::open(&db)?;
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    let known: HashSet<String> = {
        let mut st = con.prepare("SELECT DISTINCT primary_di FROM listing_cache")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.flatten().collect()
    };
    let mirror = match mirror {
        Some(m) => Some(
            rusqlite::Connection::open_with_flags(m, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("open {}", m.display()))?,
        ),
        None => None,
    };
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let (mut in_cache, mut from_mirror, mut rows) = (0usize, 0usize, 0usize);
    let mut ask = BTreeSet::new();
    con.execute_batch("BEGIN")?;
    for g in &gtins {
        if known.contains(g) {
            in_cache += 1;
            continue;
        }
        let mut hit = 0;
        if let Some(m) = &mirror {
            let mut st =
                m.prepare_cached("SELECT rawJson FROM devices_listing WHERE primaryDi = ?1")?;
            let bodies = st.query_map([g], |r| r.get::<_, String>(0))?;
            for b in bodies.flatten() {
                let Ok(d) = serde_json::from_str::<serde_json::Value>(&b) else {
                    continue;
                };
                if d.get("latestVersion").and_then(|x| x.as_bool()) == Some(false) {
                    continue;
                }
                hit += 1;
                rows += insert_row(&con, &d, &now)?;
            }
        }
        if hit > 0 {
            from_mirror += 1;
        } else {
            ask.insert(g.clone());
        }
    }
    con.execute_batch("COMMIT")?;
    ensure_tables(&con)?;
    migrate_state(data, &con)?;
    request_gtins(
        &con,
        &gtins,
        &chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    )?;
    let n_urls = write_gtin_lists(data, &con)?;
    let dir = data.join("log").join("handoff");
    log(&format!(
        "GTINs: {} in the list, {} already in listing_cache, {} found in the mirror ({} device(s) added to listing_cache), {} to look up by hand -> {}",
        gtins.len(),
        in_cache,
        from_mirror,
        rows,
        n_urls,
        dir.join("gtin_urls.txt").display()
    ));
    run_handoff(data, srns, false, log);
    Ok(())
}

/// `handoff --collect-gtins <dir>` (20.11.2026, `drop_import.sh` for answers
/// from `drop/gtin/<GTIN>.json`): found devices go into `listing_cache` (and
/// so into `uuids.txt`), found / not-found GTINs into
/// `log/handoff/gtins_found.txt` / `gtins_not_found.txt`, and both leave
/// `gtin_urls.txt`. Unusable answers (not JSON, no `content`, EUDAMED's empty
/// `"pageable": "INSTANCE"` placeholder) stay in `gtin_urls.txt`.
pub fn collect_gtins(data: &Path, dir: &Path, srns: &[String], log: &dyn Fn(&str)) -> Result<()> {
    let db = data.join("db").join("version_tracking.db");
    let con = rusqlite::Connection::open(&db)?;
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let (mut found, mut not_found, mut bad) = (BTreeSet::new(), BTreeSet::new(), Vec::new());
    let mut rows = 0;
    con.execute_batch("BEGIN")?;
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(g) = name
            .strip_suffix(".json")
            .filter(|g| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let v = std::fs::read_to_string(e.path())
            .ok()
            .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok());
        let Some(v) = v else {
            bad.push(name);
            continue;
        };
        let Some(content) = v.get("content").and_then(|c| c.as_array()) else {
            bad.push(name);
            continue;
        };
        let hits: Vec<&serde_json::Value> = content
            .iter()
            .filter(|d| txt(d, "primaryDi") == g)
            .collect();
        if !hits.is_empty() {
            for d in hits {
                rows += insert_row(&con, d, &now)?;
            }
            found.insert(g.to_string());
        } else if v.get("pageable").and_then(|p| p.as_str()) == Some("INSTANCE") {
            bad.push(name);
        } else {
            not_found.insert(g.to_string());
        }
    }
    con.execute_batch("COMMIT")?;
    ensure_tables(&con)?;
    migrate_state(data, &con)?;
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    answer_gtins(&con, &not_found, "not_found", &ts)?;
    answer_gtins(&con, &found, "found", &ts)?;
    let left = write_gtin_lists(data, &con)?;
    let urls_path = data.join("log").join("handoff").join("gtin_urls.txt");
    log(&format!(
        "GTIN answers: {} found ({} device(s) added to listing_cache), {} not in EUDAMED, {} unusable{}; {} GTIN URL(s) left -> {}",
        found.len(),
        rows,
        not_found.len(),
        bad.len(),
        if bad.is_empty() { String::new() } else { format!(" ({})", bad.join(", ")) },
        left,
        urls_path.display()
    ));
    run_handoff(data, srns, false, log);
    Ok(())
}

/// Add UUIDs to `log/outdated_uuids.txt` (devices whose records must be
/// fetched again: version bump or status change in the listing).
fn add_outdated(data: &Path, uuids: &[String]) -> Result<()> {
    if uuids.is_empty() {
        return Ok(());
    }
    let path = data.join("log").join("outdated_uuids.txt");
    let mut set: BTreeSet<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    set.extend(uuids.iter().cloned());
    std::fs::write(&path, lines(&set.into_iter().collect::<Vec<_>>()))?;
    Ok(())
}

/// Step 5 of the hand-off (20.11.2026), local: compare the listing pages in
/// `dir` (moved there from the drop folder by `drop_import.sh`) and write
/// what to download next into `log/handoff/`:
///
/// - pages still missing (an SRN grew, a page unusable, an expected SRN
///   without any page) → `urls.txt` lists only those, `uuids.txt` stays;
/// - listing complete → new devices go into `listing_cache` (so the nightly
///   pushes them once their records are there), version bumps / status
///   changes into `log/outdated_uuids.txt`, `uuids.txt` gets every known
///   device without detail / Basic UDI-DI plus the outdated ones, and
///   `urls.txt` is emptied.
pub fn collect_local(data: &Path, dir: &Path, srns: &[String], log: &dyn Fn(&str)) -> Result<()> {
    let db = data.join("db").join("version_tracking.db");
    let tag = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("drop_listing")
        .to_string();
    let c = crate::listing_compare::run(dir, &db, &data.join("log"), &tag)?;
    let (pages, _) = crate::listing_compare::read_dir(dir)?;
    let missing = still_missing(&expected_pages(data), &pages, &c);
    let hdir = data.join("log").join("handoff");
    std::fs::create_dir_all(&hdir)?;
    if !missing.is_empty() {
        let urls: Vec<String> = missing.iter().map(|(s, p)| listing_url(s, *p)).collect();
        put_list(data, "urls.txt", &urls)?;
        log(&format!(
            "Listing: {} page(s) still missing -> {} (uuids.txt unchanged until the listing is complete)",
            urls.len(),
            hdir.join("urls.txt").display()
        ));
        return Ok(());
    }
    let new: HashSet<String> = c.new.iter().map(|d| d.uuid.clone()).collect();
    let inserted = insert_new(&db, dir, &new)?;
    let changed: Vec<String> = c
        .version_bumps
        .iter()
        .map(|(d, _)| d.uuid.clone())
        .chain(c.status_changes.iter().map(|(d, _)| d.uuid.clone()))
        .collect();
    add_outdated(data, &changed)?;
    let h = build(data, srns)?;
    put_list(data, "uuids.txt", &h.uuids)?;
    put_list(data, "urls.txt", &[])?;
    log(&format!(
        "Listing complete: {} new ({} added to listing_cache), {} version bump(s), {} status change(s); {} UUID(s) to download -> {}; urls.txt emptied",
        c.new.len(),
        inserted,
        c.version_bumps.len(),
        c.status_changes.len(),
        h.uuids.len(),
        hdir.join("uuids.txt").display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_per_srn() {
        let u = urls_for(&[("A".into(), 0), ("B".into(), 300), ("C".into(), 301)]);
        assert_eq!(u.len(), 1 + 1 + 2);
        assert!(u[0].contains("page=0&") && u[0].contains("srn=A&"));
        assert!(u[3].contains("page=1&") && u[3].contains("srn=C&"));
    }

    #[test]
    fn missing_records_and_import_dirs() {
        let tmp = std::env::temp_dir().join(format!("handoff_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for d in [
            "eudamed_json/detail",
            "eudamed_json/basic",
            "import/2026-11-20_detail",
            "import/2026-11-20_basic",
            "db",
            "log",
        ] {
            std::fs::create_dir_all(tmp.join(d)).unwrap();
        }
        let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE listing_cache (uuid TEXT PRIMARY KEY, srn TEXT);
             INSERT INTO listing_cache VALUES ('a','S1'),('b','S1'),('c','S1'),('d','S2');",
        )
        .unwrap();
        drop(db);
        let put = |p: &str, body: &str| std::fs::write(tmp.join(p), body).unwrap();
        // a: complete in eudamed_json; b: detail only; c: complete in import;
        // d: basic is an HTML page (not a record).
        put("eudamed_json/detail/a.json", "{}");
        put("eudamed_json/basic/a.json", "{}");
        put("eudamed_json/detail/b.json", "{}");
        put("import/2026-11-20_detail/c.json", "{}");
        put("import/2026-11-20_basic/c.json", "{}");
        put("eudamed_json/detail/d.json", "{}");
        put("eudamed_json/basic/d.json", "<html>");
        put("log/outdated_uuids.txt", "a\n");
        std::fs::create_dir_all(tmp.join("import/2026-11-17_listing")).unwrap();
        std::fs::create_dir_all(tmp.join("import/2026-11-20_listing")).unwrap();
        put(
            "import/2026-11-17_listing/S2_page0.json",
            r#"{"totalElements": 900}"#,
        );
        put(
            "import/2026-11-20_listing/S2_page0.json",
            r#"{"totalElements": 601}"#,
        );
        let h = build(&tmp, &["S1".into(), "S2".into(), "S1".into()]).unwrap();
        assert_eq!(h.uuids, vec!["a", "b", "d"]);
        // A device checked by hand as having no Basic UDI-DI is left out.
        put(
            "log/no_basic_uuids.txt",
            "# no Basic UDI-DI in EUDAMED\nb\tchecked 20.11.\n",
        );
        let h = build(&tmp, &["S1".into(), "S2".into(), "S1".into()]).unwrap();
        assert_eq!(h.uuids, vec!["a", "d"]);
        std::fs::remove_file(tmp.join("log/no_basic_uuids.txt")).unwrap();
        // S1: 3 cached devices -> 1 page; S2: newest total 601 -> 3 pages.
        assert_eq!(h.urls.len(), 1 + 3);
        let dir = write(&tmp, &h).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("uuids.txt")).unwrap(),
            "a\nd\n"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn known_in_cache_many_uuids() {
        let tmp = std::env::temp_dir().join(format!("handoff_known_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("db")).unwrap();
        let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
        db.execute_batch("CREATE TABLE listing_cache (uuid TEXT PRIMARY KEY, srn TEXT);")
            .unwrap();
        let mut list = String::new();
        for i in 0..10_000 {
            let u = format!("{i:08x}-0000-0000-0000-000000000000");
            if i % 2 == 0 {
                db.execute(
                    "INSERT INTO listing_cache VALUES (?1, ?2)",
                    [&u, &format!("S{}", i % 7)],
                )
                .unwrap();
            }
            list.push_str(&u);
            list.push('\n');
        }
        drop(db);
        std::fs::write(tmp.join("u.txt"), list).unwrap();
        let k = known_in_cache(&tmp, &tmp.join("u.txt")).unwrap();
        assert_eq!(k.len(), 5_000);
        assert_eq!(
            k[0],
            (
                "00000000-0000-0000-0000-000000000000".to_string(),
                "S0".to_string()
            )
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn gtins_from_mirror_and_drop_answers() {
        let tmp = std::env::temp_dir().join(format!("handoff_gtin_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for d in ["db", "log", "answers"] {
            std::fs::create_dir_all(tmp.join(d)).unwrap();
        }
        let lc = "CREATE TABLE listing_cache (uuid TEXT PRIMARY KEY, srn TEXT NOT NULL DEFAULT '', manufacturer_name TEXT NOT NULL DEFAULT '', primary_di TEXT NOT NULL DEFAULT '', trade_name TEXT NOT NULL DEFAULT '', risk_class TEXT NOT NULL DEFAULT '', device_status TEXT NOT NULL DEFAULT '', version_number INTEGER, budi_version_number INTEGER, listed_at TEXT NOT NULL DEFAULT '', budi_ulid TEXT NOT NULL DEFAULT '');
             INSERT INTO listing_cache (uuid, srn, primary_di) VALUES ('u1','S1','111');";
        let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
        db.execute_batch(lc).unwrap();
        drop(db);
        let m = rusqlite::Connection::open(tmp.join("mirror.db")).unwrap();
        m.execute_batch(
            r#"CREATE TABLE devices_listing (primaryDi TEXT, rawJson TEXT);
               INSERT INTO devices_listing VALUES
                 ('222','{"uuid":"u2","primaryDi":"222","manufacturerSrn":"S2","latestVersion":true,"basicUdiDiDataUlid":"B2"}'),
                 ('222','{"uuid":"u2old","primaryDi":"222","manufacturerSrn":"S2","latestVersion":false}');"#,
        )
        .unwrap();
        drop(m);
        std::fs::write(
            tmp.join("gtins.txt"),
            "111\n222\n333\n444\n555\nnot-a-gtin\n",
        )
        .unwrap();
        add_gtins(
            &tmp,
            &tmp.join("gtins.txt"),
            Some(&tmp.join("mirror.db")),
            &[],
            &|_| {},
        )
        .unwrap();
        let read = |p: &str| std::fs::read_to_string(tmp.join(p)).unwrap_or_default();
        // 111 known, 222 from the mirror (latest version only), 333/444/555 to ask.
        assert_eq!(read("log/handoff/uuids.txt"), "u1\nu2\n");
        assert_eq!(read("log/handoff/gtin_urls.txt").lines().count(), 3);
        assert!(read("log/handoff/gtin_urls.txt").contains("primaryDi=333&"));

        let put = |p: &str, b: &str| std::fs::write(tmp.join("answers").join(p), b).unwrap();
        put(
            "333.json",
            r#"{"content":[{"uuid":"u3","primaryDi":"333","manufacturerSrn":"S3"}],"totalElements":1}"#,
        );
        put(
            "444.json",
            r#"{"content":[],"totalElements":0,"pageable":{"pageNumber":0}}"#,
        );
        put(
            "555.json",
            r#"{"content":[],"totalElements":0,"pageable":"INSTANCE"}"#,
        );
        let age = || {
            let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
            db.execute(
                "UPDATE handoff_item SET last_at = '2000-01-01 00:00:00'",
                [],
            )
            .unwrap();
        };
        // Handed out long ago, so everything still missing goes out again.
        age();
        collect_gtins(&tmp, &tmp.join("answers"), &[], &|_| {}).unwrap();
        assert_eq!(read("log/handoff/uuids.txt"), "u1\nu2\nu3\n");
        assert_eq!(read("log/handoff/gtins_found.txt"), "333\n");
        assert_eq!(read("log/handoff/gtins_not_found.txt"), "444\n");
        // The INSTANCE placeholder is no answer: 555 stays to be asked again.
        let left = read("log/handoff/gtin_urls.txt");
        assert_eq!(left.lines().count(), 1);
        assert!(left.contains("primaryDi=555&"));
        // Lists taken out of log/handoff/ come back from log/handoff_state/.
        for f in ["gtin_urls.txt", "gtins_found.txt", "gtins_not_found.txt"] {
            std::fs::remove_file(tmp.join("log/handoff").join(f)).unwrap();
        }
        std::fs::create_dir_all(tmp.join("empty")).unwrap();
        collect_gtins(&tmp, &tmp.join("empty"), &[], &|_| {}).unwrap();
        assert_eq!(read("log/handoff/gtin_urls.txt"), left);
        assert_eq!(read("log/handoff/gtins_found.txt"), "333\n");
        assert_eq!(read("log/handoff/gtins_not_found.txt"), "444\n");
        let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
        let n: i64 = db
            .query_row(
                "SELECT items FROM handoff_write WHERE file = 'gtin_urls.txt' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let times: i64 = db
            .query_row(
                "SELECT times FROM handoff_item WHERE file = 'gtin_urls.txt' AND item LIKE '%primaryDi=555&%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(times, 3);
        drop(db);
        let a = tmp.join("answers/333.json");
        std::fs::write(
            tmp.join("recv.txt"),
            format!("gtin\timported\t{}\nbad line\n", a.display()),
        )
        .unwrap();
        assert_eq!(record_received(&tmp, &tmp.join("recv.txt")).unwrap(), 1);
        let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
        let (item, bytes): (String, i64) = db
            .query_row("SELECT item, bytes FROM drop_received", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(item, "333");
        assert!(bytes > 0);
        drop(db);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn todo_entries_held_back_until_retry() {
        let tmp = std::env::temp_dir().join(format!("handoff_retry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("db")).unwrap();
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            put_list(&tmp, "uuids.txt", &v(&["a", "b"])).unwrap(),
            (2, 0)
        );
        // Same entries again: held back; a new one goes out at once.
        assert_eq!(
            put_list(&tmp, "uuids.txt", &v(&["a", "b", "c"])).unwrap(),
            (1, 2)
        );
        let read = |p: &str| std::fs::read_to_string(tmp.join("log/handoff").join(p)).unwrap();
        assert_eq!(read("uuids.txt"), "c\n");
        // Answer lists are always written in full.
        put_list(&tmp, "gtins_found.txt", &v(&["1"])).unwrap();
        assert_eq!(
            put_list(&tmp, "gtins_found.txt", &v(&["1"])).unwrap(),
            (1, 0)
        );
        // Once the retry time is up, the entries come back.
        let db = rusqlite::Connection::open(tmp.join("db/version_tracking.db")).unwrap();
        db.execute(
            "UPDATE handoff_item SET last_at = '2000-01-01 00:00:00'",
            [],
        )
        .unwrap();
        drop(db);
        assert_eq!(
            put_list(&tmp, "uuids.txt", &v(&["a", "b", "c"])).unwrap(),
            (3, 0)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn expected_and_still_missing() {
        let tmp = std::env::temp_dir().join(format!("handoff_exp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("log/handoff")).unwrap();
        let urls = urls_for(&[("A".into(), 301), ("B".into(), 10)]);
        std::fs::write(tmp.join("log/handoff/urls.txt"), lines(&urls)).unwrap();
        let exp = expected_pages(&tmp);
        assert_eq!(
            exp,
            vec![
                ("A".to_string(), 0),
                ("A".to_string(), 1),
                ("B".to_string(), 0)
            ]
        );
        // A has pages, listing_compare reports page 2 missing (it grew);
        // B has no usable page at all -> its expected page is missing too.
        let mut srns = std::collections::BTreeMap::new();
        srns.insert("A".to_string(), crate::listing_compare::SrnPages::default());
        let c = crate::listing_compare::Comparison {
            missing_pages: vec![("A".to_string(), 2)],
            ..Default::default()
        };
        assert_eq!(
            still_missing(&exp, &srns, &c),
            vec![("A".to_string(), 2), ("B".to_string(), 0)]
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
