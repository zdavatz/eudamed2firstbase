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
//!   / `import/*_basic` directory), plus `log/outdated_uuids.txt`.
//!
//! With `EUDAMED_HANDOFF_HOST` (`user@host`, set in the untracked nightly
//! wrapper — never commit the host) both lists are copied to that host:
//! `/tmp/uuid` and `/tmp/url` are deleted and recreated empty first, then
//! `uuids.txt` / `urls.txt` are copied into them.

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

    Ok(Handoff {
        urls: urls_for(&srn_counts),
        uuids: uuids.into_iter().collect(),
    })
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
    if host.starts_with('-') || host.contains(char::is_whitespace) {
        bail!("{HOST_ENV}: invalid host {host:?}");
    }
    let opts = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=30"];
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

/// Build, write and (with [`HOST_ENV`] set) upload. Errors are logged, never
/// fatal: the nightly goes on with its push.
pub fn run_handoff(data: &Path, srns: &[String], log: &dyn Fn(&str)) {
    let h = match build(data, srns) {
        Ok(h) => h,
        Err(e) => return log(&format!("Hand-off: lists not built: {e:#}")),
    };
    let dir = match write(data, &h) {
        Ok(d) => d,
        Err(e) => return log(&format!("Hand-off: lists not written: {e:#}")),
    };
    log(&format!(
        "Hand-off: {} listing URL(s), {} UUID(s) without detail/Basic UDI-DI -> {}",
        h.urls.len(),
        h.uuids.len(),
        dir.display()
    ));
    match std::env::var(HOST_ENV) {
        Ok(host) if !host.trim().is_empty() => match upload(host.trim(), &dir) {
            Ok(()) => log(&format!(
                "Hand-off: copied to the hand-off host ({REMOTE_UUID_DIR}/uuids.txt, {REMOTE_URL_DIR}/urls.txt; both dirs cleared first)"
            )),
            Err(e) => log(&format!("Hand-off: upload failed: {e:#}")),
        },
        _ => log(&format!("Hand-off: {HOST_ENV} not set, lists not copied")),
    }
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
        // S1: 3 cached devices -> 1 page; S2: newest total 601 -> 3 pages.
        assert_eq!(h.urls.len(), 1 + 3);
        let dir = write(&tmp, &h).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("uuids.txt")).unwrap(),
            "a\nb\nd\n"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
