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

/// Write `log/handoff/uuids.txt` and `urls.txt` (the directory is emptied
/// first). Returns the directory.
pub fn write(data: &Path, h: &Handoff) -> Result<PathBuf> {
    let dir = data.join("log").join("handoff");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("uuids.txt"), lines(&h.uuids))?;
    std::fs::write(dir.join("urls.txt"), lines(&h.urls))?;
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
    let res = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join("uuids.txt"), lines(&h.uuids)));
    match res {
        Ok(()) => log(&format!(
            "Hand-off: {} UUID(s) without detail/Basic UDI-DI -> {} (urls.txt unchanged)",
            h.uuids.len(),
            dir.join("uuids.txt").display()
        )),
        Err(e) => log(&format!("Hand-off: uuids.txt not written: {e}")),
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
    let code = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(|x| x.get("code"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let txt = |v: &serde_json::Value, k: &str| {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
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
            let uuid = txt(d, "uuid");
            if !new.contains(&uuid) {
                continue;
            }
            n += con.execute(
                "INSERT OR IGNORE INTO listing_cache (uuid, srn, manufacturer_name, primary_di, trade_name, risk_class, device_status, version_number, budi_version_number, listed_at, budi_ulid) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10)",
                rusqlite::params![
                    uuid,
                    txt(d, "manufacturerSrn"),
                    txt(d, "manufacturerName"),
                    txt(d, "primaryDi"),
                    txt(d, "tradeName"),
                    code(d, "riskClass"),
                    code(d, "deviceStatusType"),
                    d.get("versionNumber").and_then(|x| x.as_i64()),
                    now,
                    txt(d, "basicUdiDiDataUlid"),
                ],
            )?;
        }
    }
    Ok(n)
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
        std::fs::write(hdir.join("urls.txt"), lines(&urls))?;
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
    std::fs::write(hdir.join("uuids.txt"), lines(&h.uuids))?;
    std::fs::write(hdir.join("urls.txt"), "")?;
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
