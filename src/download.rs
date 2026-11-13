//! Shared download module for EUDAMED data.
//! Used by both the GUI and the CLI `download` subcommand.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use rayon::prelude::*;

pub const EUDAMED_BASE_URL: &str = "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData";
pub const BASIC_UDI_BASE_URL: &str =
    "https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData";
pub const DEFAULT_PAGE_SIZE: usize = 300;
/// Attempts for a listing page whose answer is unusable (see `listing_page_check`).
const LISTING_BAD_PAGE_ATTEMPTS: usize = 4;
const LISTING_BAD_PAGE_WAIT_SECS: u64 = 15;

/// Parse one SRN listing page and reject answers that would end the
/// pagination early: unparseable body, no `content` array, or an empty
/// `content` on a page that the previous page said exists. An empty first
/// page is a genuine "SRN has no devices".
fn listing_page_check(
    body: &str,
    page: usize,
    known_total_pages: Option<u64>,
) -> Result<serde_json::Value, &'static str> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "unreadable answer (not JSON)")?;
    let items = json
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or("answer without device list")?;
    if items.is_empty() && page > 0 && known_total_pages.is_some_and(|t| (page as u64) < t) {
        return Err("empty page before the last page");
    }
    Ok(json)
}
pub const DEFAULT_DATA_DIR: &str = "eudamed_json";

/// Returns the application data directory.
/// Under macOS App Sandbox, uses the container directory.
/// Otherwise, uses the current working directory.
pub fn app_data_dir() -> PathBuf {
    // macOS sandbox: APP_SANDBOX_CONTAINER_ID env var is set
    if let Ok(container) = std::env::var("APP_SANDBOX_CONTAINER_ID") {
        if !container.is_empty() {
            if let Some(home) = std::env::var_os("HOME") {
                // Sandbox container maps HOME to ~/Library/Containers/<bundle-id>/Data
                let dir = PathBuf::from(home).join("eudamed2firstbase");
                let _ = std::fs::create_dir_all(&dir);
                return dir;
            }
        }
    }

    // All platforms: ~/eudamed2firstbase/
    // Windows: %USERPROFILE%\eudamed2firstbase\
    // Linux/macOS: ~/eudamed2firstbase/
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var_os("HOME");

    if let Some(home) = home {
        let dir = PathBuf::from(home).join("eudamed2firstbase");
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }

    // Fallback: current working directory
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Progress/log messages emitted during the download pipeline.
pub enum DownloadEvent {
    Log(String),
    Progress {
        step: String,
        detail: String,
    },
    /// Structured progress for a GUI status bar: `done` of `total` items finished
    /// in `phase` ("Listings" / "detail" / "basic"). CLI ignores it (the Log lines
    /// already cover stderr); the GUI renders it as a progress bar.
    Status {
        phase: String,
        done: usize,
        total: usize,
    },
}

/// Trait for receiving download progress. Implemented differently by GUI and CLI.
pub trait DownloadProgress: Send + Sync {
    fn on_event(&self, event: DownloadEvent);
}

/// CLI progress reporter — prints to stderr.
pub struct StderrProgress;

impl DownloadProgress for StderrProgress {
    fn on_event(&self, event: DownloadEvent) {
        match event {
            DownloadEvent::Log(msg) => eprintln!("{}", msg),
            DownloadEvent::Progress { step, detail } => {
                eprintln!("[{}] {}", step, detail);
            }
            // CLI already prints per-phase Log lines; the Status event exists for
            // the GUI progress bar, so the stderr reporter ignores it.
            DownloadEvent::Status { .. } => {}
        }
    }
}

/// Configuration for a download run.
pub struct DownloadConfig {
    pub srns: Vec<String>,
    /// UDI-DI primary codes (GTINs) to fetch directly via the `primaryDi` listing
    /// filter, bypassing SRN pagination. When non-empty this takes precedence over
    /// `srns` for the listing step. Each GTIN resolves to one device (or none).
    pub gtins: Vec<String>,
    pub limit: Option<usize>,
    pub data_dir: PathBuf,
    pub parallel_threads: usize,
    /// Detail-endpoint concurrency. (Pre-v1.0.72 this was kept high on the theory
    /// that detail was less throttled; v1.0.72 disproved that — EUDAMED's rate
    /// budget is SHARED across all device endpoints — so it now matches the others.
    /// Under the proactive `RateLimiter` the thread count is throughput-irrelevant
    /// anyway; a few threads just overlap network latency / survive a slow request.)
    pub detail_threads: usize,
    pub listing_threads: usize,
    pub max_retries: u32,
    /// Minimum interval between EUDAMED requests (ms), enforced globally by the
    /// shared `RateLimiter`. ~1050 ms ≈ 57 req/min, just under the measured shared
    /// ~60-req/60 s per-IP budget → steady throughput with ~0 throttles.
    pub rate_interval_ms: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            srns: Vec::new(),
            gtins: Vec::new(),
            limit: None,
            data_dir: app_data_dir().join(DEFAULT_DATA_DIR),
            // v1.0.72: EUDAMED's ~60-req/60 s budget is SHARED across listing +
            // detail + basic (per-IP), so throughput is governed by the proactive
            // RateLimiter (rate_interval_ms), NOT the thread counts. Keep modest,
            // uniform pools — a few threads to overlap latency / survive a slow
            // request; the limiter paces the aggregate to ~57/min regardless.
            parallel_threads: 6,
            detail_threads: 6,
            // Listing / GTIN answers take ~40 s (10.2026): with 6 threads only ~9
            // requests per minute were in flight. 12 keep the limiter busy; it still
            // spaces the starts, so the per-IP budget is never exceeded.
            listing_threads: 12,
            max_retries: 3,
            rate_interval_ms: default_rate_interval_ms(),
        }
    }
}

/// Summary of what was downloaded.
#[derive(Default)]
pub struct DownloadResult {
    /// All UUIDs from the listing (with version numbers).
    pub uuid_versions: Vec<(String, Option<u32>, Option<u32>)>,
    /// UUIDs that needed downloading (new or version-changed).
    pub need_download: Vec<String>,
    /// Number of unchanged UUIDs (skipped by version check).
    pub unchanged_skipped: usize,
    pub detail_downloaded: usize,
    pub detail_cached: usize,
    pub basic_downloaded: usize,
    pub basic_cached: usize,
}

impl DownloadResult {
    /// All UUIDs from the listing.
    pub fn all_uuids(&self) -> Vec<String> {
        self.uuid_versions
            .iter()
            .map(|(u, _, _)| u.clone())
            .collect()
    }
}

/// Run the full download pipeline (listings + version check + detail + basic).
pub fn run_download(
    config: &DownloadConfig,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<DownloadResult> {
    let detail_dir = config.data_dir.join("detail");
    let basic_dir = config.data_dir.join("basic");
    let log_dir = config.data_dir.join("log");
    std::fs::create_dir_all(&detail_dir)?;
    std::fs::create_dir_all(&basic_dir)?;
    std::fs::create_dir_all(&log_dir)?;

    let log = |msg: &str| {
        progress.on_event(DownloadEvent::Log(msg.to_string()));
    };

    // --- Step 1: Download listings ---
    progress.on_event(DownloadEvent::Progress {
        step: "Download".into(),
        detail: "Fetching listings from EUDAMED API...".into(),
    });

    // One shared limiter for the WHOLE run: EUDAMED's ~60/60 s budget is shared
    // across listing + detail + basic (per-IP), so a single bucket paces them all.
    let limiter = RateLimiter::new(std::time::Duration::from_millis(config.rate_interval_ms));

    // GTIN mode takes precedence: resolve each GTIN to its device via the
    // `primaryDi` filter (one request each) instead of paginating SRN listings.
    // Either path returns the same (uuid, version, budi_version) tuples, so the
    // rest of the pipeline (version check, detail/basic fetch, indexing) is shared.
    let uuid_versions = if !config.gtins.is_empty() {
        download_listings_by_gtin(
            EUDAMED_BASE_URL,
            &config.gtins,
            config.listing_threads,
            &limiter,
            progress,
        )?
    } else {
        download_listings(
            EUDAMED_BASE_URL,
            &config.srns,
            config.limit,
            config.listing_threads,
            &limiter,
            progress,
        )?
    };

    if uuid_versions.is_empty() {
        return Ok(DownloadResult {
            uuid_versions: Vec::new(),
            need_download: Vec::new(),
            unchanged_skipped: 0,
            detail_downloaded: 0,
            detail_cached: 0,
            basic_downloaded: 0,
            basic_cached: 0,
        });
    }

    log(&format!(
        "{} UUIDs extracted from listings",
        uuid_versions.len()
    ));

    // --- Step 2: Pre-download version check ---
    let db_dir = app_data_dir().join("db");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("version_tracking.db");
    let conn = crate::version_db::open_db(&db_path)?;

    let run_start = std::time::SystemTime::now();
    let hash_changed = take_hash_changes(&log);
    let (mut need_download, unchanged_skipped, stale) = filter_unchanged(
        &uuid_versions,
        &conn,
        &detail_dir,
        &basic_dir,
        &hash_changed,
        progress,
    );

    log(&format!(
        "Version check: {} new/changed, {} unchanged (skipping download)",
        need_download.len(),
        unchanged_skipped
    ));

    // --- Step 3: Open download log ---
    let download_log = Arc::new(Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("download.log"))
            .ok(),
    ));

    // --- Step 4: Download detail files ---
    let (detail_downloaded, detail_cached) = parallel_fetch(
        &need_download,
        &stale,
        &detail_dir,
        EUDAMED_BASE_URL,
        "detail",
        &download_log,
        config.detail_threads,
        config.max_retries,
        &limiter,
        progress,
    )?;
    log(&format!(
        "Details: {} downloaded, {} cached -> {}",
        detail_downloaded,
        detail_cached,
        detail_dir.display()
    ));

    // --- Step 5: Download Basic UDI-DI files ---
    // Devices that need no certificate (non-sterile MDR Class I / IVDR Class A)
    // take their Basic UDI-DI from the datalake rows of this run's listing, so
    // only the rest asks the public endpoint (07.11.2026).
    let from_datalake = write_datalake_basics(&need_download, &stale, &basic_dir);
    if from_datalake > 0 {
        log(&format!(
            "Basic UDI-DI: {} taken from the datalake (non-sterile MDR Class I / IVDR Class A)",
            from_datalake
        ));
    }
    // A datalake Basic UDI-DI written above for a stale device is fresh; only
    // the other stale devices still need a forced public refetch.
    let stale_public: HashSet<String> = stale
        .iter()
        .filter(|u| crate::datalake::basic_for(u).is_none())
        .cloned()
        .collect();
    let (basic_downloaded, basic_cached) = parallel_fetch(
        &need_download,
        &stale_public,
        &basic_dir,
        BASIC_UDI_BASE_URL,
        "basic",
        &download_log,
        config.parallel_threads,
        config.max_retries,
        &limiter,
        progress,
    )?;
    log(&format!(
        "Basic UDI-DI: {} downloaded, {} cached -> {}",
        basic_downloaded,
        basic_cached,
        basic_dir.display()
    ));

    // --- Step 6: Completeness check + retry ---
    let missing_detail = need_download
        .iter()
        .filter(|u| {
            let p = detail_dir.join(format!("{}.json", u));
            !cached_record_ok(&p)
        })
        .count();
    let missing_basic = need_download
        .iter()
        .filter(|u| {
            let p = basic_dir.join(format!("{}.json", u));
            !cached_record_ok(&p)
        })
        .count();

    if missing_detail > 0 || missing_basic > 0 {
        log(&format!(
            "Completeness: {} detail missing, {} basic missing — retrying...",
            missing_detail, missing_basic
        ));

        // Retry missing ones
        let missing_detail_uuids: Vec<String> = need_download
            .iter()
            .filter(|u| {
                let p = detail_dir.join(format!("{}.json", u));
                !cached_record_ok(&p)
            })
            .cloned()
            .collect();
        let missing_basic_uuids: Vec<String> = need_download
            .iter()
            .filter(|u| {
                let p = basic_dir.join(format!("{}.json", u));
                !cached_record_ok(&p)
            })
            .cloned()
            .collect();

        if !missing_detail_uuids.is_empty() {
            parallel_fetch(
                &missing_detail_uuids,
                &HashSet::new(),
                &detail_dir,
                EUDAMED_BASE_URL,
                "detail",
                &download_log,
                config.detail_threads,
                5, // more retries for completeness check
                &limiter,
                progress,
            )?;
        }
        if !missing_basic_uuids.is_empty() {
            parallel_fetch(
                &missing_basic_uuids,
                &HashSet::new(),
                &basic_dir,
                BASIC_UDI_BASE_URL,
                "basic",
                &download_log,
                config.parallel_threads,
                5,
                &limiter,
                progress,
            )?;
        }

        let still_missing_d = need_download
            .iter()
            .filter(|u| !cached_record_ok(&detail_dir.join(format!("{}.json", u))))
            .count();
        let still_missing_b = need_download
            .iter()
            .filter(|u| !cached_record_ok(&basic_dir.join(format!("{}.json", u))))
            .count();
        if still_missing_d > 0 || still_missing_b > 0 {
            log(&format!(
                "Still missing after retry: {} detail, {} basic",
                still_missing_d, still_missing_b
            ));
        } else {
            log("All files complete after retry");
        }
    }

    // With the public endpoint blocked, only devices with a usable detail and a
    // Basic UDI-DI file go on to indexing, convert and push. A stale cached detail
    // of a changed device must not be indexed (its version would hide the change)
    // nor reconverted and re-pushed, so a changed device needs a detail written in
    // THIS run. Any other device (new, or one whose cache was incomplete) may use
    // its cached detail: nothing says it is outdated, and only its Basic UDI-DI
    // was missing (11.11.2026: 5'725 details on disk were dropped every night
    // although their Basic UDI-DI arrived).
    if public_endpoint_blocked() {
        let total = need_download.len();
        need_download
            .retain(|u| usable_when_blocked(u, &stale, &detail_dir, &basic_dir, run_start));
        log(&format!(
            "WARNING: EUDAMED public endpoint blocked (429) — {} of {} changed device(s) could not be downloaded; they are retried next run",
            total - need_download.len(),
            total
        ));
    }

    // Datalake row hashes of changed devices are stored only once their detail
    // was written in this run and a Basic UDI-DI file is there, so a change
    // whose refetch failed is detected again next run.
    if !hash_changed.is_empty() {
        let mut refreshed: HashSet<String> = HashSet::new();
        for u in &need_download {
            let Some(h) = hash_changed.get(u) else {
                continue;
            };
            let fresh_detail = std::fs::metadata(detail_dir.join(format!("{}.json", u)))
                .and_then(|m| m.modified())
                .map(|t| t >= run_start)
                .unwrap_or(false);
            if fresh_detail && cached_record_ok(&basic_dir.join(format!("{}.json", u))) {
                set_row_hash(&conn, u, h);
                refreshed.insert(u.clone());
            }
        }
        let open: Vec<&String> = hash_changed
            .keys()
            .filter(|u| !refreshed.contains(*u))
            .collect();
        let file = update_outdated_list(&open, &refreshed);
        log(&format!(
            "Datalake row hash: {} of {} changed device(s) refreshed{}",
            refreshed.len(),
            hash_changed.len(),
            match &file {
                Some((n, path)) => format!(
                    "; {} changed device(s) still wait for a fresh detail (GS1 has the older version): {}",
                    n,
                    path.display()
                ),
                None => String::new(),
            }
        ));
    }

    // --- Step 7: Extract versions from downloaded detail files into udi_versions DB ---
    if !need_download.is_empty() {
        log(&format!(
            "Indexing {} downloaded detail files into version DB...",
            need_download.len()
        ));
        let version_count = index_detail_versions(&detail_dir, &need_download, &conn, progress)?;
        log(&format!(
            "Indexed {} detail file versions in udi_versions DB",
            version_count
        ));
    }

    // --- Step 8: Update budi_version from listing data ---
    // The listing provides basicUdiDataVersionNumber which is not in the detail JSON.
    // Store it in udi_versions so the next check can compare both versions.
    // EUDAMED's listing reports 0 there for every device (a placeholder, real
    // versions start at 1). Writing that 0 wiped the real Basic UDI-DI version the
    // deep-scan had stored, every night, for every listed device — so each
    // deep-scan saw the version as "unknown" again and reported ~99 % of its
    // slice as baseline-filled. Only a real version is written.
    {
        let tx = conn
            .unchecked_transaction()
            .context("Failed to open budi_version update transaction")?;
        for (uuid, _udi_ver, budi_ver) in &uuid_versions {
            if let Some(bv) = real_listing_version(*budi_ver) {
                let _ = tx.execute(
                    "UPDATE udi_versions SET budi_version=?1 WHERE uuid=?2",
                    rusqlite::params![bv, uuid],
                );
            }
        }
        tx.commit()
            .context("Failed to commit budi_version update transaction")?;
    }

    Ok(DownloadResult {
        uuid_versions,
        need_download,
        unchanged_skipped,
        detail_downloaded,
        detail_cached,
        basic_downloaded,
        basic_cached,
    })
}

/// Extract version numbers from specific detail JSON files and upsert into udi_versions.
/// Uses rayon for parallel JSON parsing, then batch-inserts into SQLite.
fn index_detail_versions(
    detail_dir: &Path,
    uuids: &[String],
    conn: &rusqlite::Connection,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<usize> {
    if uuids.is_empty() {
        return Ok(0);
    }

    // Parallel: read + parse version records
    let records: Vec<crate::version_db::VersionRecord> = uuids
        .par_iter()
        .filter_map(|uuid| {
            let path = detail_dir.join(format!("{}.json", uuid));
            let content = std::fs::read_to_string(&path).ok()?;
            let mut rec = crate::version_db::extract_detail_versions(&content);
            rec.last_synced = Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
            Some(rec)
        })
        .collect();

    // Batch insert into DB using transactions
    let mut count = 0;
    let batch_size = 10000;
    for chunk in records.chunks(batch_size) {
        let tx = conn
            .unchecked_transaction()
            .context("Failed to open version index transaction")?;
        for rec in chunk {
            if crate::version_db::upsert_version(&tx, rec).is_ok() {
                count += 1;
            }
        }
        tx.commit()
            .context("Failed to commit version index transaction")?;
        if records.len() > batch_size {
            progress.on_event(DownloadEvent::Log(format!(
                "  Indexed {}/{} versions...",
                count,
                records.len()
            )));
        }
    }

    Ok(count)
}

/// A Production push accepted these devices: index the versions of the detail
/// files that were delivered, so `check` sees them as known. The nightly does
/// this in its download step, but `repush-srn` and GUI pushes of imported files
/// skip that step; without it the next `check` treated the devices as new and
/// pushed them again (11.11.2026: ~5'640, 13.11.2026: 117 devices). Returns the
/// number of indexed records.
pub fn index_delivered_versions(uuids: &[String]) -> usize {
    struct Quiet;
    impl DownloadProgress for Quiet {
        fn on_event(&self, _: DownloadEvent) {}
    }
    if uuids.is_empty() {
        return 0;
    }
    let detail_dir = app_data_dir().join(DEFAULT_DATA_DIR).join("detail");
    let db_path = app_data_dir().join("db").join("version_tracking.db");
    let Ok(conn) = crate::version_db::open_db(&db_path) else {
        return 0;
    };
    index_detail_versions(&detail_dir, uuids, &conn, &Quiet).unwrap_or(0)
}

/// Bulk-index all detail files in a directory into udi_versions DB.
/// Skips files already indexed with matching hash. Used for initial population.
pub fn index_all_detail_versions(
    detail_dir: &Path,
    conn: &rusqlite::Connection,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<usize> {
    let files: Vec<std::path::PathBuf> = std::fs::read_dir(detail_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();

    if files.is_empty() {
        return Ok(0);
    }

    progress.on_event(DownloadEvent::Log(format!(
        "  Scanning {} detail files for version indexing...",
        files.len()
    )));

    // Get existing hashes for fast skip
    let existing_hashes: std::collections::HashMap<String, String> = {
        let mut stmt = conn
            .prepare("SELECT uuid, detail_hash FROM udi_versions WHERE detail_hash IS NOT NULL")
            .context("Failed to prepare existing-hashes query")?;
        let iter = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to execute existing-hashes query")?;
        iter.filter_map(|r| r.ok()).collect()
    };
    let existing_hashes = Arc::new(existing_hashes);
    let skipped = AtomicUsize::new(0);

    let records: Vec<crate::version_db::VersionRecord> = files
        .par_iter()
        .filter_map(|path| {
            let uuid = path.file_stem()?.to_str()?.to_string();
            let content = std::fs::read_to_string(path).ok()?;
            let hash = crate::version_db::hash_json(&content);
            if let Some(existing) = existing_hashes.get(&uuid) {
                if *existing == hash {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            }
            let mut rec = crate::version_db::extract_detail_versions(&content);
            rec.last_synced = Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
            Some(rec)
        })
        .collect();

    progress.on_event(DownloadEvent::Log(format!(
        "  {} new/changed, {} unchanged (skipped)",
        records.len(),
        skipped.load(Ordering::Relaxed)
    )));

    let mut count = 0;
    let batch_size = 10000;
    for chunk in records.chunks(batch_size) {
        let tx = conn
            .unchecked_transaction()
            .context("Failed to open version index transaction")?;
        for rec in chunk {
            if crate::version_db::upsert_version(&tx, rec).is_ok() {
                count += 1;
            }
        }
        tx.commit()
            .context("Failed to commit version index transaction")?;
        progress.on_event(DownloadEvent::Log(format!(
            "  Indexed {}/{} versions...",
            count,
            records.len()
        )));
    }

    Ok(count)
}

/// Create the listing_cache table if it doesn't exist.
fn ensure_listing_cache(conn: &rusqlite::Connection) {
    // The listing connections are opened raw (not via version_db::open_db), so
    // the datalake row-hash table must be created here too: on 08.11.2026 the
    // first nightly SRN pass ran before any open_db and every hash INSERT failed
    // silently (only the GTIN pass stored hashes).
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS datalake_row_hash (
            uuid TEXT PRIMARY KEY,
            hash TEXT NOT NULL,
            seen_at TEXT NOT NULL DEFAULT ''
        );",
    );
    let _ = conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS listing_cache (
            uuid TEXT PRIMARY KEY,
            srn TEXT NOT NULL DEFAULT '',
            manufacturer_name TEXT NOT NULL DEFAULT '',
            primary_di TEXT NOT NULL DEFAULT '',
            trade_name TEXT NOT NULL DEFAULT '',
            risk_class TEXT NOT NULL DEFAULT '',
            device_status TEXT NOT NULL DEFAULT '',
            version_number INTEGER,
            budi_version_number INTEGER,
            listed_at TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS srn_listing_size (
            srn TEXT PRIMARY KEY,
            total_elements INTEGER,
            total_pages INTEGER NOT NULL,
            measured_at TEXT NOT NULL DEFAULT ''
        );
    ",
    );
    // Migrations for existing DBs
    let _ = conn.execute(
        "ALTER TABLE listing_cache ADD COLUMN manufacturer_name TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE listing_cache ADD COLUMN budi_version_number INTEGER",
        [],
    );
    // ULID of the device's Basic UDI-DI record (listing field `basicUdiDiDataUlid`).
    // Devices sharing it share one Basic UDI-DI record, so the deep-scan fetches
    // that record once per ULID instead of once per device.
    let _ = conn.execute(
        "ALTER TABLE listing_cache ADD COLUMN budi_ulid TEXT NOT NULL DEFAULT ''",
        [],
    );
}

/// A version number from the listing, unless it is the `0` placeholder EUDAMED
/// reports for the Basic UDI-DI of every device (real versions start at 1).
fn real_listing_version(v: Option<u32>) -> Option<u32> {
    v.filter(|n| *n > 0)
}

/// Built-in pacing between two EUDAMED requests: ~57 req/min, just under the
/// ~60-req/60 s per-IP budget measured in June 2026.
pub const DEFAULT_RATE_INTERVAL_MS: u64 = 1050;

/// Pacing between two EUDAMED requests. `EUDAMED_RATE_MS` overrides the built-in
/// default without a rebuild, because EUDAMED changes its per-IP budget: on
/// 09.10.2026 only ~20 requests per 60 s got through (20 OK, then 429 +
/// `Retry-After: 60`), so pacing at 1050 ms ran into the penalty every round
/// (~15/min) while ~3200 ms stays under the budget (~19/min, no penalty).
pub fn default_rate_interval_ms() -> u64 {
    parse_rate_interval_ms(std::env::var("EUDAMED_RATE_MS").ok().as_deref())
}

/// A missing, unparseable or absurdly small value (< 50 ms) keeps the default.
fn parse_rate_interval_ms(value: Option<&str>) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms >= 50)
        .unwrap_or(DEFAULT_RATE_INTERVAL_MS)
}

/// Proactive global rate limiter (v1.0.72). EUDAMED throttles ALL device
/// endpoints (listing + detail + Basic-UDI) against a SINGLE shared ~60-req/60 s
/// per-IP budget — measured 2026-06-25: draining detail's 60-request budget then
/// immediately hitting basic 429s instantly. The v1.0.71 *reactive* approach
/// (fire at N threads → eat a 60 s `Retry-After` on each 429) collapsed
/// throughput to ~12/min (threads sleeping a window in lockstep). This limiter
/// instead hands every request a time slot spaced `interval` apart, so the
/// AGGREGATE rate across all threads/endpoints stays just under the budget and
/// 429s never happen: measured **steady 57/min, 0 throttles** across 2+ windows.
pub struct RateLimiter {
    next_slot: std::sync::Mutex<std::time::Instant>,
    interval: std::time::Duration,
}

impl RateLimiter {
    pub fn new(interval: std::time::Duration) -> Self {
        Self {
            next_slot: std::sync::Mutex::new(std::time::Instant::now()),
            interval,
        }
    }

    /// Block until this caller's paced slot. Slots are handed out `interval`
    /// apart; `.max(now)` caps catch-up after an idle gap (e.g. a phase boundary)
    /// to a SINGLE immediate request, never a burst that could trip the window.
    pub fn acquire(&self) {
        let wait = {
            let mut slot = self.next_slot.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            let mine = (*slot).max(now);
            *slot = mine + self.interval;
            mine.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

/// Upper bound for one EUDAMED request. Listing and `primaryDi` answers took
/// 13–43 s in October 2026 (detail / Basic UDI-DI stay under 1 s); the former
/// 30 s cut most of them off — on 21.10.2026 345 of 391 GTIN lookups and on
/// 19.10.2026 58 SRN list pages failed with `timeout: global`, so their devices
/// were not checked that night.
pub const EUDAMED_TIMEOUT_SECS: u64 = 120;

/// Windows desktop Chrome. The public UI (`ec.europa.eu/tools/eudamed/api/devices/…`)
/// answers automated clients with HTTP 429 and a notice to use the datalake API.
/// Live check 06.11.2026 on
/// `…/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en`
/// with `Accept: application/json, text/plain, */*` and
/// `Accept-Language: en-US,en;q=0.9`:
/// `User-Agent: eudamed2firstbase`, no User-Agent, and Macintosh Chrome 120/131
/// were 429 (`Retry-After: 300`). Windows Chrome 120, Windows Chrome 131 and
/// Windows Firefox 121 were 200 (uuid `522a2c98-…`, primaryDi `00195451000232`).
/// The same Windows Chrome 120 string also returned the Basic UDI-DI.
/// Listing requests do **not** use this header: they keep the client default so
/// a public-listing 429 still switches to the datalake. GitHub update checks
/// keep their own app User-Agent.
pub const PUBLIC_UI_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// `Accept` sent with [`PUBLIC_UI_USER_AGENT`] (the value from the 200 responses).
pub const PUBLIC_UI_ACCEPT: &str = "application/json, text/plain, */*";

/// `Accept-Language` sent with [`PUBLIC_UI_USER_AGENT`].
pub const PUBLIC_UI_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// Detail (`…/udiDiData/{uuid}`) and Basic UDI-DI (`…/basicUdiData/…`) record
/// URLs. A listing is `…/udiDiData` plus a query string and no further path
/// segment, so it does not match.
pub fn is_public_device_record_url(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url).trim_end_matches('/');
    path.contains("/basicUdiData/") || path.contains("/udiDiData/")
}

/// True when a body can be an EUDAMED device record: JSON object, not an HTML
/// page. On 07.11.2026 EUDAMED answered 3'728 detail/Basic UDI-DI requests
/// with 2xx and its "Server temporarily unavailable" HTML page; they were
/// written as `<uuid>.json`, then counted as cached and never refetched.
pub fn looks_like_json_record(body: &str) -> bool {
    body.trim_start().starts_with('{')
}

/// Error text for a 2xx answer that is not a JSON record (see
/// [`looks_like_json_record`]). Counts toward the circuit breaker like a 429.
const NOT_JSON_ERR: &str = "HTTP 2xx but not JSON (EUDAMED error page)";

/// Attach the Windows desktop browser headers the public UI accepts on a
/// device detail or Basic UDI-DI request. Per-request, so the same agent can
/// still list with ureq's own User-Agent.
pub fn with_public_ui_browser_headers<B>(req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    req.header("User-Agent", PUBLIC_UI_USER_AGENT)
        .header("Accept", PUBLIC_UI_ACCEPT)
        .header("Accept-Language", PUBLIC_UI_ACCEPT_LANGUAGE)
}

/// A ureq agent that surfaces HTTP status (so 429 / `Retry-After` are readable
/// instead of arriving as an opaque `Err`) with a generous global timeout.
/// The agent itself keeps ureq's default User-Agent. Device detail and Basic
/// UDI-DI calls override it per request via [`with_public_ui_browser_headers`].
pub fn eudamed_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(EUDAMED_TIMEOUT_SECS)))
        .build()
        .new_agent()
}

/// Network trace for a suspicious (empty) listing answer, written to the log so
/// a user behind a proxy can show where the answer came from (29.10.2026: GS1
/// got empty listings for SRNs that have devices). One extra request: HTTP
/// status, time, the proxy/cache headers, the addresses `ec.europa.eu`
/// resolves to, the proxy environment and the start of the body.
fn listing_trace(agent: &ureq::Agent, limiter: &RateLimiter, url: &str) -> Vec<String> {
    let mut out = vec![format!("    trace: GET {}", url)];
    let resolved = std::net::ToSocketAddrs::to_socket_addrs(&("ec.europa.eu", 443))
        .map(|a| a.map(|a| a.ip().to_string()).collect::<Vec<_>>().join(", "))
        .unwrap_or_else(|e| format!("DNS error: {}", e));
    out.push(format!("    trace: ec.europa.eu resolves to {}", resolved));
    for var in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "NO_PROXY",
    ] {
        if let Ok(v) = std::env::var(var) {
            out.push(format!("    trace: env {}={}", var, v));
        }
    }
    limiter.acquire();
    let start = std::time::Instant::now();
    match agent
        .get(url)
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .call()
    {
        Ok(mut resp) => {
            out.push(format!(
                "    trace: HTTP {} after {} ms",
                resp.status().as_u16(),
                start.elapsed().as_millis()
            ));
            for h in [
                "via",
                "x-cache",
                "x-cache-hits",
                "age",
                "server",
                "date",
                "cache-control",
                "x-forwarded-for",
                "x-proxy-id",
                "proxy-connection",
                "content-length",
                "content-type",
            ] {
                if let Some(v) = resp.headers().get(h).and_then(|v| v.to_str().ok()) {
                    out.push(format!("    trace: {}: {}", h, v));
                }
            }
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            let head: String = body
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(300)
                .collect();
            out.push(format!("    trace: body ({} bytes): {}", body.len(), head));
        }
        Err(e) => out.push(format!(
            "    trace: request failed after {} ms: {}",
            start.elapsed().as_millis(),
            e
        )),
    }
    out
}

/// GET `url`, returning the body on 2xx. On **HTTP 429 it honors the
/// `Retry-After` header** (waits the stated seconds, capped at 70) and retries;
/// on any other non-2xx or a network error it does a short linear backoff.
/// Returns `Err(reason)` only after `max_attempts`.
///
/// This is the single choke-point that makes every EUDAMED download — listings,
/// detail, basic — survive the endpoints' rate limit (~60 req / 60 s on the
/// listing + Basic-UDI endpoints, measured 2026-06-25). The old paths either
/// `break` on a 429 (listing pagination → truncation: DE-MF-000018836 got 40 of
/// 4795) or backed off 2–6 s (parallel_fetch → never clears a 60 s window →
/// "still missing"). Here a throttled request waits out the window and retries
/// rather than abandoning the page/file, so nothing is silently dropped.
pub fn eudamed_get(
    agent: &ureq::Agent,
    limiter: &RateLimiter,
    url: &str,
    max_attempts: u32,
) -> Result<String, String> {
    if public_endpoint_blocked() {
        return Err("EUDAMED public endpoint blocked (429), request skipped".into());
    }
    let res = eudamed_get_inner(agent, limiter, url, max_attempts);
    if note_public_outcome(url, &res) {
        eprintln!(
            "EUDAMED public device records blocked: {} detail/basic requests in a row ended in 429 after their retries — skipping all further public requests in this run",
            PUBLIC_BLOCK_AFTER
        );
    }
    res
}

/// What one finished `eudamed_get` meant for the circuit breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicOutcome {
    Ok,
    Http429,
    Other,
}

/// Counts only device-record 429s. A public listing answers 429 on purpose
/// (no browser User-Agent, so the run can switch to the datalake). Those
/// listing 429s must not arm this breaker: detail and Basic UDI-DI run later
/// in the same process and are the calls that carry the browser User-Agent.
/// Ten record 429s in a row still stop every further public request, so a
/// real burst of detail/basic failures does not hang the night.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PublicCircuit {
    streak: usize,
    blocked: bool,
}

impl PublicCircuit {
    const fn fresh() -> Self {
        Self {
            streak: 0,
            blocked: false,
        }
    }

    fn observe(mut self, url: &str, outcome: PublicOutcome) -> Self {
        if self.blocked || !is_public_device_record_url(url) {
            return self;
        }
        match outcome {
            PublicOutcome::Ok => self.streak = 0,
            PublicOutcome::Http429 => {
                self.streak += 1;
                if self.streak >= PUBLIC_BLOCK_AFTER {
                    self.blocked = true;
                }
            }
            PublicOutcome::Other => {}
        }
        self
    }
}

/// Circuit breaker (05.11.2026, narrowed 07.11.2026): a burst of detail/basic
/// 429s used to be indistinguishable from the public listing's M2M 429. The
/// nightly lists first, so ten listing 429s tripped the breaker and every
/// later detail/basic call returned "request skipped" without being sent.
/// Only [`is_public_device_record_url`] outcomes move the streak. Once it
/// trips, every further public request of this process fails at once.
const PUBLIC_BLOCK_AFTER: usize = 10;
static PUBLIC_CIRCUIT: Mutex<PublicCircuit> = Mutex::new(PublicCircuit::fresh());

fn note_public_outcome(url: &str, res: &Result<String, String>) -> bool {
    let outcome = match res {
        Ok(_) => PublicOutcome::Ok,
        // A 2xx HTML error page is as much a refusal as a 429.
        Err(e) if e.starts_with("HTTP 429") || e == NOT_JSON_ERR => PublicOutcome::Http429,
        Err(_) => PublicOutcome::Other,
    };
    let mut guard = PUBLIC_CIRCUIT.lock().unwrap_or_else(|e| e.into_inner());
    let was = guard.blocked;
    *guard = guard.observe(url, outcome);
    !was && guard.blocked
}

/// True once the circuit breaker tripped in this process.
pub fn public_endpoint_blocked() -> bool {
    PUBLIC_CIRCUIT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .blocked
}

#[cfg(test)]
fn reset_public_circuit() {
    *PUBLIC_CIRCUIT.lock().unwrap_or_else(|e| e.into_inner()) = PublicCircuit::fresh();
}

fn eudamed_get_inner(
    agent: &ureq::Agent,
    limiter: &RateLimiter,
    url: &str,
    max_attempts: u32,
) -> Result<String, String> {
    let mut last = String::from("no response");
    for attempt in 1..=max_attempts.max(1) {
        // Proactive pacing: wait for a budget slot BEFORE every request so the
        // aggregate stays under EUDAMED's shared ~60/60 s limit and 429s (and
        // their 60 s Retry-After stalls) are avoided rather than reacted to.
        limiter.acquire();
        // no-cache: a caching proxy on the user's side served GS1 an empty
        // listing for CH-MF-000016376 for hours after an EUDAMED outage
        // (29.10.2026, v1.0.110); this asks every cache on the way to refetch.
        // Browser headers only on a device record. Listing URLs stay on the
        // client default so a 429 still falls through to the datalake.
        let mut req = agent
            .get(url)
            .header("Cache-Control", "no-cache")
            .header("Pragma", "no-cache");
        if is_public_device_record_url(url) {
            req = with_public_ui_browser_headers(req);
        }
        match req.call() {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                if (200..300).contains(&status) {
                    let body = resp
                        .body_mut()
                        .read_to_string()
                        .map_err(|e| e.to_string())?;
                    // A record URL must answer with a JSON object. EUDAMED's
                    // HTML error page also comes with 2xx; retry it and never
                    // hand it on to be cached.
                    if is_public_device_record_url(url) && !looks_like_json_record(&body) {
                        last = NOT_JSON_ERR.to_string();
                        if attempt < max_attempts {
                            std::thread::sleep(std::time::Duration::from_secs(5 * attempt as u64));
                            continue;
                        }
                        return Err(last);
                    }
                    return Ok(body);
                }
                if status == 429 {
                    // EUDAMED's block (05.11.2026) carries a notice pointing to
                    // the datalake API. Waiting through Retry-After rounds does
                    // not lift it, so give up at once. A listing 429 switches
                    // the run to the datalake; it does not arm the detail/basic
                    // circuit breaker. A record 429 still does.
                    let blocked_notice = {
                        let ra = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string());
                        let body = resp.body_mut().read_to_string().unwrap_or_default();
                        (body.contains("machineToMachineApi"), ra)
                    };
                    if blocked_notice.0 {
                        return Err("HTTP 429 (EUDAMED block, M2M notice)".into());
                    }
                    let wait = blocked_notice
                        .1
                        .as_deref()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .unwrap_or(60)
                        .min(70);
                    last = format!("HTTP 429 (waited {}s)", wait);
                    if attempt < max_attempts {
                        std::thread::sleep(std::time::Duration::from_secs(wait));
                        continue;
                    }
                    return Err(last);
                }
                last = format!("HTTP {}", status);
                if attempt < max_attempts {
                    std::thread::sleep(std::time::Duration::from_secs(attempt as u64));
                    continue;
                }
                return Err(last);
            }
            Err(e) => {
                last = format!("network error: {}", e);
                if attempt < max_attempts {
                    std::thread::sleep(std::time::Duration::from_secs(attempt as u64));
                    continue;
                }
                return Err(last);
            }
        }
    }
    Err(last)
}

/// Write datalake devices to listing_cache (only those in `keep`, if given) and
/// return their (uuid, version, None) tuples. The Basic UDI-DI version is left
/// NULL: the datalake has none (and the listing's was a 0 placeholder anyway).
fn store_datalake_devices(
    conn: &Mutex<rusqlite::Connection>,
    devices: &[crate::datalake::DlDevice],
    keep: Option<&std::collections::HashSet<String>>,
    now: &str,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    let mut out = Vec::new();
    let Ok(db) = conn.lock() else {
        return out;
    };
    for d in devices {
        if let Some(k) = keep {
            if !k.contains(&d.uuid) && !k.contains(&d.primary_di) {
                continue;
            }
        }
        let _ = db.execute(
            "INSERT OR REPLACE INTO listing_cache (uuid, srn, manufacturer_name, primary_di, trade_name, risk_class, device_status, version_number, budi_version_number, listed_at, budi_ulid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10)",
            rusqlite::params![d.uuid, d.srn, d.manufacturer_name, d.primary_di, d.trade_name, d.risk_class, d.device_status, d.version, now, d.budi_ulid],
        );
        if !d.row_hash.is_empty() {
            let old: Option<String> = db
                .query_row(
                    "SELECT hash FROM datalake_row_hash WHERE uuid = ?1",
                    [d.uuid.as_str()],
                    |r| r.get(0),
                )
                .ok();
            match old {
                // First sight: baseline, not a change.
                None => set_row_hash(&db, &d.uuid, &d.row_hash),
                Some(o) if o != d.row_hash => note_hash_change(&d.uuid, &d.row_hash),
                Some(_) => {}
            }
        }
        out.push((d.uuid.clone(), d.version, None));
    }
    out
}

/// Devices whose datalake row hash changed in this process (uuid -> new hash).
static HASH_CHANGES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn note_hash_change(uuid: &str, hash: &str) {
    HASH_CHANGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(uuid.to_string(), hash.to_string());
}

/// Most hash changes one run treats as real (`EUDAMED_HASH_CHANGE_MAX`,
/// default 1'500 = the nightly GS1 portion). More than that in one night
/// looks like a format change on the datalake side, not 1'500 edits.
fn hash_change_max() -> usize {
    std::env::var("EUDAMED_HASH_CHANGE_MAX")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(1500)
}

/// Take the hash changes collected by the listing. Above
/// [`hash_change_max`] none is used and none is stored (so the warning
/// repeats until someone looks).
fn take_hash_changes(log: &dyn Fn(&str)) -> HashMap<String, String> {
    let changes = HASH_CHANGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_default();
    if changes.is_empty() {
        return changes;
    }
    let mut changes = changes;
    let accepted = accept_delivered_hashes(&mut changes);
    if accepted > 0 {
        log(&format!(
            "Datalake row hash: {} changed device(s) were already delivered in their current version (outdated list) — new hash stored, no refetch",
            accepted
        ));
    }
    if changes.is_empty() {
        return changes;
    }
    let max = hash_change_max();
    if changes.len() > max {
        log(&format!(
            "WARNING: datalake row hash changed for {} device(s) without a version bump (limit {}) — ignored this run; check whether the datalake changed its format",
            changes.len(),
            max
        ));
        return HashMap::new();
    }
    log(&format!(
        "Datalake row hash: {} device(s) changed without a version bump — refetching them",
        changes.len()
    ));
    changes
}

fn delivered_outdated_path() -> std::path::PathBuf {
    app_data_dir()
        .join("log")
        .join("delivered_outdated_uuids.txt")
}

fn read_uuid_set(path: &std::path::Path) -> std::collections::BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn write_uuid_set(path: &std::path::Path, set: &std::collections::BTreeSet<String>) {
    if set.is_empty() {
        let _ = std::fs::remove_file(path);
    } else {
        let body: String = set.iter().map(|u| format!("{u}\n")).collect();
        let _ = std::fs::write(path, body);
    }
}

/// A Production push accepted these devices. Those on the outdated list were
/// delivered with a detail supplied by hand (`repush-srn` with imported files),
/// so they move from `outdated_uuids.txt` to `delivered_outdated_uuids.txt`.
/// The next run that sees their changed datalake row stores the new row hash
/// instead of refetching (and re-pushing) them. Returns how many moved.
pub fn mark_outdated_delivered(accepted: &[String]) -> usize {
    let out_path = app_data_dir().join("log").join("outdated_uuids.txt");
    let mut outdated = read_uuid_set(&out_path);
    if outdated.is_empty() {
        return 0;
    }
    let moved: Vec<String> = accepted
        .iter()
        .filter(|u| outdated.contains(*u))
        .cloned()
        .collect();
    if moved.is_empty() {
        return 0;
    }
    let del_path = delivered_outdated_path();
    let mut delivered = read_uuid_set(&del_path);
    for u in &moved {
        outdated.remove(u);
        delivered.insert(u.clone());
    }
    write_uuid_set(&del_path, &delivered);
    write_uuid_set(&out_path, &outdated);
    moved.len()
}

/// Row-hash changes of devices in `delivered_outdated_uuids.txt`: store the
/// new hash, drop them from `changes` and from the file. A delivered device
/// whose row was not seen this run (datalake down) stays in the file.
fn accept_delivered_hashes(changes: &mut HashMap<String, String>) -> usize {
    let path = delivered_outdated_path();
    let mut delivered = read_uuid_set(&path);
    if delivered.is_empty() || changes.is_empty() {
        return 0;
    }
    let db_path = app_data_dir().join("db").join("version_tracking.db");
    let Ok(conn) = rusqlite::Connection::open(&db_path) else {
        return 0;
    };
    ensure_listing_cache(&conn);
    let hits: Vec<String> = changes
        .keys()
        .filter(|u| delivered.contains(*u))
        .cloned()
        .collect();
    for u in &hits {
        if let Some(h) = changes.remove(u) {
            set_row_hash(&conn, u, &h);
        }
        delivered.remove(u);
    }
    write_uuid_set(&path, &delivered);
    hits.len()
}

/// `log/outdated_uuids.txt`: devices whose datalake row changed but whose fresh
/// detail could not be fetched yet, so GS1 still has the older version. Each
/// run adds its `open` devices and drops the `refreshed` ones; the file is
/// removed when it gets empty. Returns the count and path while it exists.
fn update_outdated_list(
    open: &[&String],
    refreshed: &HashSet<String>,
) -> Option<(usize, std::path::PathBuf)> {
    let path = app_data_dir().join("log").join("outdated_uuids.txt");
    let mut set: std::collections::BTreeSet<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    for u in open {
        set.insert((*u).clone());
    }
    set.retain(|u| !refreshed.contains(u));
    if set.is_empty() {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    let body: String = set.iter().map(|u| format!("{u}\n")).collect();
    std::fs::write(&path, body).ok()?;
    Some((set.len(), path))
}

/// One device whose datalake row differs from what was last delivered.
pub struct OutdatedDevice {
    pub uuid: String,
    pub srn: String,
    pub gtin: String,
    /// `row-hash` (content changed without a version bump) or `version`.
    pub reason: &'static str,
}

/// `outdated`: compare the current datalake rows of the worklist with the
/// stored row hashes and versions, without downloading or pushing anything.
/// SRNs are read with one `MF_SRN` query each; with `gtin_devices` the devices
/// of other SRNs in `listing_cache` (the GTIN worklist) are looked up one by one
/// (`PRIMARY_DI`). Returns the outdated devices and the number of failed
/// queries (> 0 means the list is incomplete).
pub fn find_outdated(
    srns: &[String],
    gtin_devices: bool,
    threads: usize,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<(Vec<OutdatedDevice>, usize)> {
    let db_path = app_data_dir().join("db").join("version_tracking.db");
    let conn = rusqlite::Connection::open(&db_path).context("open version DB")?;
    ensure_listing_cache(&conn);
    let hashes: HashMap<String, String> = conn
        .prepare("SELECT uuid, hash FROM datalake_row_hash")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .flatten()
        .collect();
    let versions: HashMap<String, u32> = conn
        .prepare("SELECT uuid, udi_version FROM udi_versions WHERE udi_version IS NOT NULL")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .flatten()
        .collect();
    // uuid -> (srn, primary_di) for every worklist device.
    let known: HashMap<String, (String, String)> = conn
        .prepare("SELECT uuid, srn, primary_di FROM listing_cache")?
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .flatten()
        .collect();
    drop(conn);

    let srn_set: HashSet<&str> = srns.iter().map(|s| s.as_str()).collect();
    // Work items: (filter key, value, uuid to match or None for a whole SRN).
    let mut items: Vec<(&'static str, String, Option<String>)> =
        srns.iter().map(|s| ("MF_SRN", s.clone(), None)).collect();
    if gtin_devices {
        for (u, (srn, di)) in &known {
            if !srn_set.contains(srn.as_str()) && !di.is_empty() {
                items.push(("PRIMARY_DI", di.clone(), Some(u.clone())));
            }
        }
    }
    let total = items.len();
    let done = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let out: Mutex<Vec<OutdatedDevice>> = Mutex::new(Vec::new());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()
        .context("thread pool")?;
    let log_line = |m: String| progress.on_event(DownloadEvent::Log(m));
    pool.install(|| {
        items.par_iter().for_each(|(key, value, only)| {
            let q = crate::datalake::query_logged(key, value, None, &|m| log_line(m));
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            let q = match q {
                Ok(q) => q,
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    log_line(format!("  {key}={value}: datalake failed ({e})"));
                    return;
                }
            };
            let mut found = Vec::new();
            for d in &q.devices {
                if only.as_ref().is_some_and(|u| u != &d.uuid) {
                    continue;
                }
                let Some((srn, di)) = known.get(&d.uuid) else {
                    continue;
                };
                let reason = if d
                    .version
                    .zip(versions.get(&d.uuid))
                    .is_some_and(|(now, stored)| now > *stored)
                {
                    Some("version")
                } else if hashes.get(&d.uuid).is_some_and(|h| h != &d.row_hash) {
                    Some("row-hash")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    found.push(OutdatedDevice {
                        uuid: d.uuid.clone(),
                        srn: srn.clone(),
                        gtin: di.clone(),
                        reason,
                    });
                }
            }
            if n % 50 == 0 || n == total {
                log_line(format!("[outdated] {n}/{total} queries"));
            }
            out.lock().unwrap_or_else(|e| e.into_inner()).extend(found);
        });
    });
    let mut v = out.into_inner().unwrap_or_else(|e| e.into_inner());
    v.sort_by(|a, b| (&a.srn, &a.gtin).cmp(&(&b.srn, &b.gtin)));
    v.dedup_by(|a, b| a.uuid == b.uuid);
    Ok((v, failed.into_inner()))
}

fn set_row_hash(db: &rusqlite::Connection, uuid: &str, hash: &str) {
    let _ = db.execute(
        "INSERT OR REPLACE INTO datalake_row_hash (uuid, hash, seen_at) VALUES (?1, ?2, datetime('now'))",
        rusqlite::params![uuid, hash],
    );
}

/// SRN listing via the datalake (`MF_SRN` filter, 1'000 devices per page).
/// None = fall back to the public listing (datalake error, or no device under
/// this SRN — e.g. a PR SRN, which the `MF_SRN` filter may not cover).
/// Write the pending datalake notices ([`crate::datalake::take_notices`]) to the
/// run log, so the GUI shows them as well as the terminal.
fn log_datalake_notices(progress: &dyn DownloadProgress) {
    for n in crate::datalake::take_notices() {
        progress.on_event(DownloadEvent::Log(n));
    }
}

fn datalake_listing_for_srn(
    srn: &str,
    limit: Option<usize>,
    keep: Option<&std::collections::HashSet<String>>,
    max_pages: Option<u64>,
    conn: &Mutex<rusqlite::Connection>,
    progress: &dyn DownloadProgress,
) -> Option<SrnListing> {
    // `max_pages` counts public pages of 300; a datalake page holds 1'000.
    let dl_max = max_pages.map(|m| ((m * DEFAULT_PAGE_SIZE as u64).div_ceil(1000)).max(1) as usize);
    let q = crate::datalake::query_logged("MF_SRN", srn, dl_max, &|m| {
        progress.on_event(DownloadEvent::Log(m))
    });
    log_datalake_notices(progress);
    let q = match q {
        Ok(q) => q,
        Err(e) => {
            progress.on_event(DownloadEvent::Log(format!(
                "  SRN {}: datalake failed ({}){}",
                srn,
                e,
                if crate::datalake::forced() {
                    "; public listing not asked"
                } else {
                    ", asking the public listing"
                }
            )));
            return None;
        }
    };
    if q.devices.is_empty() && q.complete {
        // A forced datalake run must not probe the public UI to "confirm" an
        // empty SRN. That probe is a 429, and ten of them used to trip the
        // detail/basic circuit breaker before any device record was fetched.
        if crate::datalake::forced() {
            progress.on_event(DownloadEvent::Log(format!(
                "  SRN {}: no device in the datalake",
                srn
            )));
            return Some(SrnListing {
                entries: Vec::new(),
                total_pages: Some(0),
                complete: true,
            });
        }
        progress.on_event(DownloadEvent::Log(format!(
            "  SRN {}: no device in the datalake, asking the public listing",
            srn
        )));
        return None;
    }
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut devices = q.devices;
    if let Some(l) = limit {
        devices.truncate(l);
    }
    let seen = devices.len();
    let entries = store_datalake_devices(conn, &devices, keep, &now);
    let pages_300 = (seen as u64).div_ceil(DEFAULT_PAGE_SIZE as u64).max(1);
    if q.complete {
        if let Ok(db) = conn.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO srn_listing_size (srn, total_elements, total_pages, measured_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![srn, seen as i64, pages_300 as i64, now],
            );
        }
    }
    let of = if keep.is_some() {
        format!(" (worklist devices, of {} listed)", seen)
    } else {
        String::new()
    };
    progress.on_event(DownloadEvent::Log(format!(
        "  SRN {}: {} devices{} via datalake, {} page(s){}",
        srn,
        entries.len(),
        of,
        q.pages,
        if q.complete { "" } else { ", stopped early" }
    )));
    Some(SrnListing {
        entries,
        total_pages: q.complete.then_some(pages_300),
        complete: q.complete && limit.is_none(),
    })
}

/// The SRN's devices as `listing_cache` knows them from earlier runs, for a run
/// in which no listing source answers (10.11.2026: datalake 504 all day, public
/// listing 429). Their versions are the last listed ones, so no new change is
/// detected, but a device whose detail or Basic UDI-DI is still missing — or
/// whose version bump was seen earlier and not fetched yet — is retried instead
/// of waiting for the next night with a working listing. Never `complete`.
/// The Basic UDI-DI version stays None, as for a datalake listing.
fn cached_listing_for_srn(
    srn: &str,
    limit: Option<usize>,
    keep: Option<&std::collections::HashSet<String>>,
    conn: &Mutex<rusqlite::Connection>,
    progress: &dyn DownloadProgress,
) -> Option<SrnListing> {
    let mut entries: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    {
        let db = conn.lock().ok()?;
        let mut st = db
            .prepare(
                "SELECT uuid, primary_di, version_number FROM listing_cache WHERE srn = ?1 ORDER BY uuid",
            )
            .ok()?;
        let rows = st
            .query_map([srn], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<u32>>(2)?,
                ))
            })
            .ok()?;
        for (uuid, di, version) in rows.flatten() {
            if let Some(k) = keep {
                if !k.contains(&uuid) && !di.as_ref().is_some_and(|d| k.contains(d)) {
                    continue;
                }
            }
            entries.push((uuid, version, None));
        }
    }
    if let Some(l) = limit {
        entries.truncate(l);
    }
    if entries.is_empty() {
        return None;
    }
    progress.on_event(DownloadEvent::Log(format!(
        "  SRN {}: {} devices from the cached listing (no listing source answered) — pending devices are retried, new changes are not detected",
        srn,
        entries.len()
    )));
    Some(SrnListing {
        entries,
        total_pages: None,
        complete: false,
    })
}

/// A GTIN's devices as `listing_cache` knows them (see [`cached_listing_for_srn`]).
fn cached_listing_for_gtin(
    gtin: &str,
    conn: &Mutex<rusqlite::Connection>,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    let Ok(db) = conn.lock() else {
        return Vec::new();
    };
    let Ok(mut st) = db.prepare(
        "SELECT uuid, version_number FROM listing_cache WHERE primary_di IN (?1, ?2) ORDER BY uuid",
    ) else {
        return Vec::new();
    };
    let padded = format!("{:0>14}", gtin);
    st.query_map([gtin, padded.as_str()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<u32>>(1)?, None))
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// Download paginated listings for a single SRN. Returns entries found.
fn download_listing_for_srn(
    base_url: &str,
    srn: &str,
    limit: Option<usize>,
    conn: &Mutex<rusqlite::Connection>,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    download_listing_for_srn_opts(base_url, srn, limit, None, None, conn, limiter, progress).entries
}

/// Result of one SRN listing run.
struct SrnListing {
    entries: Vec<(String, Option<u32>, Option<u32>)>,
    /// The SRN's page count (at `size=300`), when page 0 was read.
    total_pages: Option<u64>,
    /// Every device of the SRN was seen (no failed page, no early stop).
    complete: bool,
}

/// SRN listing with two options for the GTIN worklist (04.11.2026):
/// - `keep`: only devices whose uuid or primaryDi is in the set are written to
///   `listing_cache` and returned; the other devices of the SRN are ignored.
/// - `max_pages`: stop after page 0 when the SRN has more pages than this (the
///   single GTIN lookups are cheaper then). Page 0 is still used.
/// Page 0 also stores the SRN's size in `srn_listing_size` for the next run.
#[allow(clippy::too_many_arguments)]
fn download_listing_for_srn_opts(
    base_url: &str,
    srn: &str,
    limit: Option<usize>,
    keep: Option<&std::collections::HashSet<String>>,
    max_pages: Option<u64>,
    conn: &Mutex<rusqlite::Connection>,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> SrnListing {
    if crate::datalake::enabled() {
        match datalake_listing_for_srn(srn, limit, keep, max_pages, conn, progress) {
            Some(res) => return res,
            // Forced datalake: an error (None) stops here. Falling through
            // would send the public listing request this mode exists to avoid.
            None if crate::datalake::forced() => {
                return cached_listing_for_srn(srn, limit, keep, conn, progress).unwrap_or(
                    SrnListing {
                        entries: Vec::new(),
                        total_pages: None,
                        complete: false,
                    },
                );
            }
            None => {}
        }
    }
    let mut entries: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    let mut page = 0;
    let mut srn_count = 0;
    // Devices seen on the listing, kept or not (= srn_count without `keep`).
    let mut srn_seen = 0usize;
    let mut stopped_early = false;
    let mut page_failed = false;
    let mut srn_new = 0usize;
    let mut srn_udi_bumped = 0usize;
    let mut srn_budi_bumped = 0usize;
    let mut srn_unchanged = 0usize;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let agent = eudamed_agent();
    let mut known_total_pages: Option<u64> = None;
    let mut known_total_elements: Option<u64> = None;
    // Set when the `size=300` listing came back empty: the SRN is asked once
    // more with v1.0.109's `pageSize=20` (a different URL, so no cached answer).
    let mut small_pages = false;

    loop {
        // `size`, not `pageSize`: EUDAMED caps `pageSize` at 20 but honours
        // `size` up to 300 (measured 09.10.2026: 46 instead of 680 pages for a
        // 13'583-device SRN) — 15x fewer listing requests out of the shared budget.
        let url = if small_pages {
            format!(
                "{}?page={}&pageSize=20&srn={}&iso2Code=en&languageIso2Code=en",
                base_url, page, srn
            )
        } else {
            format!(
                "{}?page={}&size={}&srn={}&iso2Code=en&languageIso2Code=en",
                base_url, page, DEFAULT_PAGE_SIZE, srn
            )
        };

        // Retry the page on a 429 (honoring Retry-After) instead of breaking — a
        // throttled page used to abort the whole SRN's pagination and truncate it
        // (DE-MF-000018836: 40 of 4795). Now pagination runs to completion.
        //
        // An answer that does not parse, has no `content`, or is empty before the
        // last known page is retried too (28.10.2026: during an EUDAMED outage
        // such an answer silently ended CH-MF-000016376 at 60 of 111 devices).
        let mut json: Option<serde_json::Value> = None;
        let mut last_err = String::new();
        for attempt in 0..LISTING_BAD_PAGE_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_secs(LISTING_BAD_PAGE_WAIT_SECS));
            }
            match eudamed_get(&agent, limiter, &url, 6) {
                Ok(b) => match listing_page_check(&b, page, known_total_pages) {
                    Ok(j) => {
                        json = Some(j);
                        break;
                    }
                    Err(e) => last_err = e.to_string(),
                },
                Err(e) => {
                    // eudamed_get already retried 429/network errors itself.
                    last_err = e.to_string();
                    break;
                }
            }
        }
        let json = match json {
            Some(j) => j,
            None => {
                progress.on_event(DownloadEvent::Log(format!(
                    "  SRN {} page {} failed after retries: {}",
                    srn, page, last_err
                )));
                // EUDAMED blocks the public listing: list this SRN (and the rest
                // of the run) via the datalake instead (v1.0.112).
                if crate::datalake::is_block_error(&last_err) && crate::datalake::fallback_allowed()
                {
                    crate::datalake::activate_fallback(&last_err);
                    log_datalake_notices(progress);
                    if let Some(res) =
                        datalake_listing_for_srn(srn, limit, keep, max_pages, conn, progress)
                    {
                        return res;
                    }
                }
                // Neither source answered: the devices known from earlier runs.
                if page == 0 {
                    if let Some(res) = cached_listing_for_srn(srn, limit, keep, conn, progress) {
                        return res;
                    }
                }
                page_failed = true;
                break;
            }
        };
        let content = json.get("content").and_then(|c| c.as_array());

        if let Some(items) = content {
            if items.is_empty() {
                let total = json.get("totalElements").and_then(|t| t.as_u64());
                if page == 0 && !small_pages {
                    progress.on_event(DownloadEvent::Log(format!(
                        "  SRN {}: empty answer for size={} (totalElements {}), asking again with pageSize=20",
                        srn,
                        DEFAULT_PAGE_SIZE,
                        total.map_or("missing".to_string(), |t| t.to_string())
                    )));
                    for line in listing_trace(&agent, limiter, &url) {
                        progress.on_event(DownloadEvent::Log(line));
                    }
                    small_pages = true;
                    continue;
                }
                if page == 0 {
                    progress.on_event(DownloadEvent::Log(format!(
                        "  SRN {}: EUDAMED lists no devices (totalElements {})",
                        srn,
                        total.map_or("missing".to_string(), |t| t.to_string())
                    )));
                }
                break;
            }

            let total_pages = json.get("totalPages").and_then(|t| t.as_u64()).unwrap_or(1);
            let total_elements = json.get("totalElements").and_then(|t| t.as_u64());
            known_total_pages = Some(total_pages);
            known_total_elements = total_elements;

            // Batch insert into DB + on-the-fly version classification
            let page_start = entries.len();
            if let Ok(db) = conn.lock() {
                if page == 0 {
                    let _ = db.execute(
                        "INSERT OR REPLACE INTO srn_listing_size (srn, total_elements, total_pages, measured_at) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![srn, total_elements.map(|t| t as i64), total_pages as i64, now],
                    );
                }
                for item in items {
                    if let Some(uuid) = item.get("uuid").and_then(|u| u.as_str()) {
                        srn_seen += 1;
                        if let Some(k) = keep {
                            let di = item.get("primaryDi").and_then(|v| v.as_str()).unwrap_or("");
                            if !k.contains(uuid) && !k.contains(di) {
                                continue;
                            }
                        }
                        let version = item
                            .get("versionNumber")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as u32);
                        let budi_version = item
                            .get("basicUdiDataVersionNumber")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as u32);
                        let primary_di =
                            item.get("primaryDi").and_then(|v| v.as_str()).unwrap_or("");
                        let trade_name =
                            item.get("tradeName").and_then(|v| v.as_str()).unwrap_or("");
                        let risk_class = item
                            .get("riskClass")
                            .and_then(|v| v.get("code"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let device_status = item
                            .get("deviceStatusType")
                            .and_then(|v| v.get("code"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let mfr_srn = item
                            .get("manufacturerSrn")
                            .and_then(|v| v.as_str())
                            .unwrap_or(srn);
                        let mfr_name = item
                            .get("manufacturerName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let budi_ulid = item
                            .get("basicUdiDiDataUlid")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let _ = db.execute(
                            "INSERT OR REPLACE INTO listing_cache (uuid, srn, manufacturer_name, primary_di, trade_name, risk_class, device_status, version_number, budi_version_number, listed_at, budi_ulid) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                            rusqlite::params![uuid, mfr_srn, mfr_name, primary_di, trade_name, risk_class, device_status, version, budi_version, now, budi_ulid],
                        );

                        entries.push((uuid.to_string(), version, budi_version));
                        srn_count += 1;

                        if let Some(lim) = limit {
                            if srn_count >= lim {
                                break;
                            }
                        }
                    }
                }

                // Per-page on-the-fly version classification against udi_versions.
                // Same lock/connection, so no extra acquisition. Fast: one row lookup
                // per UUID via a cached prepared statement.
                if let Ok(mut stmt) = db.prepare_cached(
                    "SELECT udi_version, budi_version FROM udi_versions WHERE uuid = ?1",
                ) {
                    for (uuid, listing_ver, listing_budi) in &entries[page_start..] {
                        let row: rusqlite::Result<(Option<i64>, Option<i64>)> =
                            stmt.query_row([uuid.as_str()], |r| Ok((r.get(0)?, r.get(1)?)));
                        match row {
                            Err(_) => srn_new += 1,
                            Ok((db_udi, db_budi)) => {
                                let udi_changed = matches!(
                                    (db_udi, listing_ver),
                                    (Some(a), Some(b)) if (*b as i64) > a
                                );
                                let budi_changed = matches!(
                                    (db_budi, listing_budi),
                                    (Some(a), Some(b)) if (*b as i64) > a
                                );
                                if udi_changed {
                                    srn_udi_bumped += 1;
                                } else if budi_changed {
                                    srn_budi_bumped += 1;
                                } else {
                                    srn_unchanged += 1;
                                }
                            }
                        }
                    }
                }
            }

            if page % 10 == 0 || page + 1 >= total_pages as usize {
                progress.on_event(DownloadEvent::Log(format!(
                    "  {} page {}/{} — {} devices{}",
                    srn,
                    page + 1,
                    total_pages,
                    srn_count,
                    total_elements
                        .map(|t| format!(" (of {} total)", t))
                        .unwrap_or_default()
                )));
            }

            if let Some(lim) = limit {
                if srn_count >= lim {
                    stopped_early = true;
                    break;
                }
            }
            if page == 0 && max_pages.is_some_and(|m| total_pages > m) {
                progress.on_event(DownloadEvent::Log(format!(
                    "  SRN {}: {} pages, more than its {} worklist GTIN(s) — single lookups instead",
                    srn,
                    total_pages,
                    max_pages.unwrap_or(0)
                )));
                stopped_early = true;
                break;
            }

            page += 1;
            if page >= total_pages as usize {
                break;
            }
        } else {
            break;
        }
    }

    if srn_count > 0 || page == 0 {
        let of = if keep.is_some() {
            format!(" (worklist devices, of {} listed)", srn_seen)
        } else {
            String::new()
        };
        progress.on_event(DownloadEvent::Log(format!(
            "  SRN {}: {} devices{}  [{}↑ new, {}↑ udi, {}↑ budi, {} same]",
            srn, srn_count, of, srn_new, srn_udi_bumped, srn_budi_bumped, srn_unchanged
        )));
    }
    let mut complete = !stopped_early && !page_failed;
    if let Some(total) = known_total_elements {
        if (srn_seen as u64) < total {
            complete = false;
            if !stopped_early {
                progress.on_event(DownloadEvent::Log(format!(
                    "  WARNING: SRN {} incomplete — {} of {} devices listed; run again for the rest",
                    srn, srn_seen, total
                )));
            }
        }
    }

    SrnListing {
        entries,
        total_pages: known_total_pages,
        complete,
    }
}

/// Download paginated listings from EUDAMED, filtered by SRN.
/// Parallel: 10 SRNs at a time. Writes each page to listing_cache DB.
/// Returns Vec<(uuid, Option<version_number>)>, deduplicated.
fn download_listings(
    base_url: &str,
    srns: &[String],
    limit: Option<usize>,
    listing_threads: usize,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<Vec<(String, Option<u32>, Option<u32>)>> {
    // Open DB for listing cache
    let db_dir = app_data_dir().join("db");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("version_tracking.db");
    let conn = rusqlite::Connection::open(&db_path)?;
    ensure_listing_cache(&conn);
    let conn = Mutex::new(conn);

    progress.on_event(DownloadEvent::Log(format!(
        "Downloading listings for {} SRNs ({} parallel)...",
        srns.len(),
        listing_threads
    )));

    let base_url_owned = base_url.to_string();
    let completed = AtomicUsize::new(0);
    let total_srns = srns.len();

    // Parallel listing downloads
    let all_entries: Vec<Vec<(String, Option<u32>, Option<u32>)>> = rayon::ThreadPoolBuilder::new()
        .num_threads(listing_threads)
        .build()
        .context("Failed to build rayon thread pool for listing downloads")?
        .install(|| {
            srns.par_iter()
                .map(|srn| {
                    let entries = download_listing_for_srn(
                        &base_url_owned,
                        srn,
                        limit,
                        &conn,
                        limiter,
                        progress,
                    );
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done % 100 == 0 || done == total_srns {
                        progress.on_event(DownloadEvent::Log(format!(
                            "[Listing] {}/{} SRNs completed",
                            done, total_srns
                        )));
                    }
                    progress.on_event(DownloadEvent::Status {
                        phase: "Listings".into(),
                        done,
                        total: total_srns,
                    });
                    entries
                })
                .collect()
        });

    let mut flat: Vec<(String, Option<u32>, Option<u32>)> =
        all_entries.into_iter().flatten().collect();

    // Deduplicate by UUID (keep highest versions)
    flat.sort_by(|a, b| a.0.cmp(&b.0));
    flat.dedup_by(|a, b| {
        if a.0 == b.0 {
            if a.1 > b.1 {
                b.1 = a.1;
            }
            if a.2 > b.2 {
                b.2 = a.2;
            }
            true
        } else {
            false
        }
    });

    Ok(flat)
}

/// Resolve a single GTIN (UDI-DI `primaryDi` code) to its listing entry via the
/// `primaryDi` filter, write it to listing_cache (with the real manufacturerSrn,
/// so the device can also be pushed by SRN later), and return its
/// (uuid, version, budi_version) tuple. Empty Vec when EUDAMED has no such device.
/// GTIN lookup via the datalake (`PRIMARY_DI` filter). None on a datalake error.
fn datalake_lookup_gtin(
    gtin: &str,
    conn: &Mutex<rusqlite::Connection>,
    now: &str,
    progress: &dyn DownloadProgress,
) -> Option<Vec<(String, Option<u32>, Option<u32>)>> {
    let q = crate::datalake::query_logged("PRIMARY_DI", gtin, Some(1), &|m| {
        progress.on_event(DownloadEvent::Log(m))
    });
    log_datalake_notices(progress);
    match q {
        Ok(q) => {
            let entries = store_datalake_devices(conn, &q.devices, None, now);
            if entries.is_empty() {
                progress.on_event(DownloadEvent::Log(format!(
                    "  GTIN {}: no device in EUDAMED (datalake)",
                    gtin
                )));
            } else {
                for (uuid, _, _) in &entries {
                    progress.on_event(DownloadEvent::Log(format!(
                        "  GTIN {} -> {} (datalake)",
                        gtin, uuid
                    )));
                }
            }
            Some(entries)
        }
        Err(e) => {
            progress.on_event(DownloadEvent::Log(format!(
                "  GTIN {}: datalake failed ({})",
                gtin, e
            )));
            None
        }
    }
}

fn download_listing_for_gtin(
    base_url: &str,
    gtin: &str,
    conn: &Mutex<rusqlite::Connection>,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    if crate::datalake::enabled() {
        match datalake_lookup_gtin(gtin, conn, &now, progress) {
            Some(entries) => return entries,
            None if crate::datalake::forced() => {
                let cached = cached_listing_for_gtin(gtin, conn);
                progress.on_event(DownloadEvent::Log(format!(
                    "  GTIN {}: public listing not asked, {} device(s) from the cached listing",
                    gtin,
                    cached.len()
                )));
                return cached;
            }
            None => {}
        }
    }
    let agent = eudamed_agent();
    let url = format!(
        "{}?page=0&pageSize=5&primaryDi={}&iso2Code=en&languageIso2Code=en",
        base_url, gtin
    );

    let body = match eudamed_get(&agent, limiter, &url, 6) {
        Ok(b) => b,
        Err(e) => {
            progress.on_event(DownloadEvent::Log(format!(
                "  GTIN {} lookup failed after retries: {}",
                gtin, e
            )));
            // EUDAMED blocks the public endpoint: ask the datalake (v1.0.112).
            if crate::datalake::is_block_error(&e) && crate::datalake::fallback_allowed() {
                crate::datalake::activate_fallback(&e);
                log_datalake_notices(progress);
                if let Some(entries) = datalake_lookup_gtin(gtin, conn, &now, progress) {
                    return entries;
                }
            }
            return Vec::new();
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    if let Some(items) = json.get("content").and_then(|c| c.as_array()) {
        if let Ok(db) = conn.lock() {
            for item in items {
                if let Some(uuid) = item.get("uuid").and_then(|u| u.as_str()) {
                    let version = item
                        .get("versionNumber")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    let budi_version = item
                        .get("basicUdiDataVersionNumber")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    let primary_di = item.get("primaryDi").and_then(|v| v.as_str()).unwrap_or("");
                    let trade_name = item.get("tradeName").and_then(|v| v.as_str()).unwrap_or("");
                    let risk_class = item
                        .get("riskClass")
                        .and_then(|v| v.get("code"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let device_status = item
                        .get("deviceStatusType")
                        .and_then(|v| v.get("code"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mfr_srn = item
                        .get("manufacturerSrn")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mfr_name = item
                        .get("manufacturerName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    let budi_ulid = item
                        .get("basicUdiDiDataUlid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    let _ = db.execute(
                        "INSERT OR REPLACE INTO listing_cache (uuid, srn, manufacturer_name, primary_di, trade_name, risk_class, device_status, version_number, budi_version_number, listed_at, budi_ulid) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        rusqlite::params![uuid, mfr_srn, mfr_name, primary_di, trade_name, risk_class, device_status, version, budi_version, now, budi_ulid],
                    );
                    entries.push((uuid.to_string(), version, budi_version));
                }
            }
        }
    }

    if entries.is_empty() {
        progress.on_event(DownloadEvent::Log(format!(
            "  GTIN {}: no device in EUDAMED",
            gtin
        )));
    } else {
        for (uuid, _, _) in &entries {
            progress.on_event(DownloadEvent::Log(format!("  GTIN {} -> {}", gtin, uuid)));
        }
    }
    entries
}

/// Resolve a list of GTINs to listing entries via the `primaryDi` filter (one
/// request per GTIN). Returns deduplicated (uuid, version, budi_version) tuples —
/// the same shape `download_listings` produces, so the rest of `run_download`
/// (version check, detail/basic fetch, indexing) is identical.
/// GTINs of one SRN that are read from that SRN's listing.
#[derive(Debug, PartialEq)]
struct SrnGroup {
    srn: String,
    gtins: Vec<String>,
    /// Known device uuids per GTIN (same order as `gtins`).
    gtin_uuids: Vec<Vec<String>>,
    /// All known uuids of the group.
    uuids: Vec<String>,
    /// Stop after page 0 when the SRN has more pages than this (size unknown).
    max_pages: Option<u64>,
}

#[derive(Debug, PartialEq)]
struct GtinPlan {
    by_srn: Vec<SrnGroup>,
    singles: Vec<String>,
}

/// `known`: primaryDi -> [(uuid, srn)] from listing_cache; `sizes`: srn -> pages
/// at `size=300` from srn_listing_size.
fn load_gtin_plan_inputs(
    conn: &rusqlite::Connection,
) -> (
    std::collections::HashMap<String, Vec<(String, String)>>,
    std::collections::HashMap<String, u64>,
) {
    let mut known: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();
    if let Ok(mut st) = conn.prepare("SELECT primary_di, uuid, srn FROM listing_cache") {
        if let Ok(rows) = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            for (di, uuid, srn) in rows.flatten() {
                if !di.is_empty() {
                    known.entry(di).or_default().push((uuid, srn));
                }
            }
        }
    }
    let mut sizes = std::collections::HashMap::new();
    if let Ok(mut st) = conn.prepare("SELECT srn, total_pages FROM srn_listing_size") {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for (srn, p) in rows.flatten() {
                sizes.insert(srn, p.max(0) as u64);
            }
        }
    }
    (known, sizes)
}

/// Decide per GTIN: SRN listing or single `primaryDi` lookup.
/// - Unknown GTIN (never resolved), or known under several SRNs: single.
/// - SRN with known size: listing when its pages <= its GTIN count.
/// - SRN with unknown size: listing for >= 2 GTINs, stopped after page 0 when
///   it turns out longer than the GTIN count; a lone GTIN stays single.
fn plan_gtin_lookups(
    gtins: &[String],
    known: &std::collections::HashMap<String, Vec<(String, String)>>,
    sizes: &std::collections::HashMap<String, u64>,
) -> GtinPlan {
    let mut seen = std::collections::HashSet::new();
    let mut singles = Vec::new();
    let mut groups: std::collections::BTreeMap<String, SrnGroup> =
        std::collections::BTreeMap::new();
    for g in gtins {
        if !seen.insert(g.clone()) {
            continue;
        }
        let padded = format!("{:0>14}", g);
        let rows = known.get(g).or_else(|| known.get(&padded));
        let rows: Vec<&(String, String)> = rows
            .map(|v| v.iter().filter(|(_, s)| !s.is_empty()).collect())
            .unwrap_or_default();
        let srns: std::collections::HashSet<&str> = rows.iter().map(|(_, s)| s.as_str()).collect();
        if rows.is_empty() || srns.len() != 1 {
            singles.push(g.clone());
            continue;
        }
        let srn = rows[0].1.clone();
        let uuids: Vec<String> = rows.iter().map(|(u, _)| u.clone()).collect();
        let grp = groups.entry(srn.clone()).or_insert_with(|| SrnGroup {
            srn,
            gtins: Vec::new(),
            gtin_uuids: Vec::new(),
            uuids: Vec::new(),
            max_pages: None,
        });
        grp.gtins.push(g.clone());
        grp.uuids.extend(uuids.iter().cloned());
        grp.gtin_uuids.push(uuids);
    }
    let mut by_srn = Vec::new();
    for (_, mut grp) in groups {
        let n = grp.gtins.len() as u64;
        match sizes.get(&grp.srn) {
            Some(&p) if p <= n => by_srn.push(grp),
            None if n >= 2 => {
                grp.max_pages = Some(n);
                by_srn.push(grp);
            }
            _ => singles.extend(grp.gtins),
        }
    }
    GtinPlan { by_srn, singles }
}

fn download_listings_by_gtin(
    base_url: &str,
    gtins: &[String],
    listing_threads: usize,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<Vec<(String, Option<u32>, Option<u32>)>> {
    let db_dir = app_data_dir().join("db");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("version_tracking.db");
    let conn = rusqlite::Connection::open(&db_path)?;
    ensure_listing_cache(&conn);

    // Plan (04.11.2026): a GTIN already resolved once has its device and SRN in
    // listing_cache. Such GTINs are grouped by SRN and read from the SRN listing
    // (300 devices per request) when that costs fewer requests than one lookup
    // per GTIN; the rest (new GTINs, small groups, long listings) stay single.
    let (known, sizes) = load_gtin_plan_inputs(&conn);
    let plan = plan_gtin_lookups(gtins, &known, &sizes);
    let conn = Mutex::new(conn);
    let base_url_owned = base_url.to_string();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(listing_threads)
        .build()
        .context("Failed to build rayon thread pool for GTIN lookups")?;

    let via_srn: usize = plan.by_srn.iter().map(|g| g.gtins.len()).sum();
    let est_pages: u64 = plan
        .by_srn
        .iter()
        .map(|g| sizes.get(&g.srn).copied().unwrap_or(1))
        .sum();
    progress.on_event(DownloadEvent::Log(format!(
        "GTIN plan: {} GTIN(s) via {} SRN listing(s) (~{} pages), {} single lookup(s)",
        via_srn,
        plan.by_srn.len(),
        est_pages,
        plan.singles.len()
    )));

    let mut flat: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    let mut singles = plan.singles.clone();
    if !plan.by_srn.is_empty() {
        let results: Vec<(Vec<(String, Option<u32>, Option<u32>)>, Vec<String>)> =
            pool.install(|| {
                plan.by_srn
                    .par_iter()
                    .map(|g| {
                        let mut keep: std::collections::HashSet<String> =
                            g.uuids.iter().cloned().collect();
                        for gt in &g.gtins {
                            keep.insert(gt.clone());
                            keep.insert(format!("{:0>14}", gt));
                        }
                        let res = download_listing_for_srn_opts(
                            &base_url_owned,
                            &g.srn,
                            None,
                            Some(&keep),
                            g.max_pages,
                            &conn,
                            limiter,
                            progress,
                        );
                        let seen: std::collections::HashSet<&str> =
                            res.entries.iter().map(|(u, ..)| u.as_str()).collect();
                        // A GTIN is done when one of its known devices was listed;
                        // otherwise (long listing, failed page, device gone from
                        // the SRN) it falls back to the single lookup.
                        let left: Vec<String> = g
                            .gtins
                            .iter()
                            .zip(&g.gtin_uuids)
                            .filter(|(_, us)| !us.iter().any(|u| seen.contains(u.as_str())))
                            .map(|(gt, _)| gt.clone())
                            .collect();
                        (res.entries, left)
                    })
                    .collect()
            });
        let mut fallback = 0usize;
        for (entries, left) in results {
            flat.extend(entries);
            fallback += left.len();
            singles.extend(left);
        }
        progress.on_event(DownloadEvent::Log(format!(
            "GTIN plan: SRN listings done, {} GTIN(s) not found there go to single lookups",
            fallback
        )));
    }

    progress.on_event(DownloadEvent::Log(format!(
        "Resolving {} GTIN(s) via primaryDi ({} parallel)...",
        singles.len(),
        listing_threads
    )));

    let completed = AtomicUsize::new(0);
    let total = singles.len();

    let all_entries: Vec<Vec<(String, Option<u32>, Option<u32>)>> = pool.install(|| {
        singles
            .par_iter()
            .map(|gtin| {
                let entries =
                    download_listing_for_gtin(&base_url_owned, gtin, &conn, limiter, progress);
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 100 == 0 || done == total {
                    progress.on_event(DownloadEvent::Log(format!(
                        "[GTIN] {}/{} resolved",
                        done, total
                    )));
                }
                progress.on_event(DownloadEvent::Status {
                    phase: "GTIN lookup".into(),
                    done,
                    total,
                });
                entries
            })
            .collect()
    });

    flat.extend(all_entries.into_iter().flatten());

    // Deduplicate by UUID (keep highest versions) — two GTINs could map to the
    // same device (rare), and the same device must not be downloaded twice.
    flat.sort_by(|a, b| a.0.cmp(&b.0));
    flat.dedup_by(|a, b| {
        if a.0 == b.0 {
            if a.1 > b.1 {
                b.1 = a.1;
            }
            if a.2 > b.2 {
                b.2 = a.2;
            }
            true
        } else {
            false
        }
    });

    Ok(flat)
}

/// Pre-download version check: compare listing versionNumber and basicUdiDataVersionNumber against version DB.
/// Returns (uuids_needing_download, count_unchanged).
///
/// A device is only treated as "unchanged" (and skipped) when its versions match
/// the DB *and* its cached `detail/<uuid>.json` and `basic/<uuid>.json` both exist
/// non-empty on disk. If either file is missing the cache is incomplete: convert
/// would fall back to bad defaults (globalModelNumber=GTIN, no
/// MODEL_NUMBER/globalModelDescription/EAR -> GS1 097.116/097.025/097.054). In
/// that case the device is re-downloaded and its stale version row is dropped so
/// the convert step reconverts instead of fast-path-skipping onto stale output.
fn filter_unchanged(
    uuid_versions: &[(String, Option<u32>, Option<u32>)],
    conn: &rusqlite::Connection,
    detail_dir: &Path,
    basic_dir: &Path,
    hash_changed: &HashMap<String, String>,
    _progress: &dyn DownloadProgress,
) -> (Vec<String>, usize, HashSet<String>) {
    let mut need_download = Vec::new();
    let mut unchanged = 0;
    // Changed devices whose cached files exist: they must be fetched again
    // although the files are there (a version bump or a datalake row-hash
    // change). parallel_fetch skips a cached file otherwise.
    let mut stale = HashSet::new();

    let file_present = |dir: &Path, uuid: &str| -> bool {
        let p = dir.join(format!("{}.json", uuid));
        std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false)
    };

    for (uuid, listing_version, budi_listing_version) in uuid_versions {
        if let Ok(Some(db_rec)) = crate::version_db::get_version(conn, uuid) {
            // Mirror the per-SRN listing classifier exactly: a device is
            // "changed" only on a strict version INCREASE (listing > DB, both
            // present). EUDAMED version numbers are monotonic, so equality or a
            // None on either side is NOT a change. The old logic treated any
            // mismatch — crucially (DB budi=None, listing budi=Some) — as
            // changed, which re-downloaded and re-pushed the ENTIRE worklist
            // whenever budi_version was unpopulated, while the classifier logged
            // those same devices as "same" (the 29355-new/0-unchanged vs all-same
            // discrepancy). Now the two agree.
            let udi_bumped = matches!(
                (db_rec.udi_version, listing_version),
                (Some(db), Some(listing)) if *listing > db
            );
            let budi_bumped = matches!(
                (db_rec.budi_version, budi_listing_version),
                (Some(db), Some(listing)) if *listing > db
            );
            let row_changed = hash_changed.contains_key(uuid);
            if udi_bumped || budi_bumped || row_changed {
                stale.insert(uuid.clone());
            }
            if !udi_bumped && !budi_bumped && !row_changed {
                if file_present(detail_dir, uuid) && file_present(basic_dir, uuid) {
                    unchanged += 1;
                    continue;
                }
                // No version bump but the cached files are missing — the local
                // cache is incomplete. Invalidate the stale version row and drop
                // any stale firstbase output built from the incomplete cache.
                // Step 7 (index_detail_versions) re-creates the version row after
                // re-download, so the row deletion alone would not force a rebuild
                // (the convert fast-path matches the unchanged detail hash);
                // removing the output file makes the convert step's
                // output-missing fallback rebuild it from the complete data.
                let _ = crate::version_db::delete_version(conn, uuid);
                let stale_output = app_data_dir()
                    .join("firstbase_json")
                    .join(format!("{}.json", uuid));
                let _ = std::fs::remove_file(&stale_output);
            }
        }
        need_download.push(uuid.clone());
    }

    (need_download, unchanged, stale)
}

/// With the public endpoint blocked: may device `u` go on to convert and push?
/// It needs a usable Basic UDI-DI file and a usable detail; a device in `stale`
/// (changed since its files were cached) needs a detail written at or after
/// `run_start`, any other device may use its cached detail.
fn usable_when_blocked(
    u: &str,
    stale: &HashSet<String>,
    detail_dir: &Path,
    basic_dir: &Path,
    run_start: std::time::SystemTime,
) -> bool {
    let d = detail_dir.join(format!("{}.json", u));
    if !cached_record_ok(&d) || !cached_record_ok(&basic_dir.join(format!("{}.json", u))) {
        return false;
    }
    if !stale.contains(u) {
        return true;
    }
    std::fs::metadata(&d)
        .and_then(|m| m.modified())
        .map(|t| t >= run_start)
        .unwrap_or(false)
}

/// Write the datalake Basic UDI-DI ([`crate::datalake::basic_for`]) of every
/// device in `uuids` that has no usable cached Basic UDI-DI. Returns the count.
fn write_datalake_basics(uuids: &[String], force: &HashSet<String>, basic_dir: &Path) -> usize {
    let mut n = 0;
    for u in uuids {
        let p = basic_dir.join(format!("{}.json", u));
        if cached_record_ok(&p) && !force.contains(u) {
            continue;
        }
        let Some(b) = crate::datalake::basic_for(u) else {
            continue;
        };
        let Ok(body) = serde_json::to_string_pretty(&b) else {
            continue;
        };
        if std::fs::write(&p, body).is_ok() {
            n += 1;
        }
    }
    n
}

/// deep-scan may skip the public Basic UDI-DI request of a device when the
/// cached records show it can carry no certificate — MDR Class I or IVDR
/// Class A, not sterile (detail `sterile` explicitly false), not a system /
/// procedure pack, no certificate in the cached Basic UDI-DI — because the
/// nightly datalake row hash already covers every other field of that record
/// (07.11.2026). The caller also requires a stored datalake row hash.
pub fn basic_covered_by_datalake(basic_body: &str, detail_body: &str) -> bool {
    let (Ok(b), Ok(d)) = (
        serde_json::from_str::<serde_json::Value>(basic_body),
        serde_json::from_str::<serde_json::Value>(detail_body),
    ) else {
        return false;
    };
    let code = |v: &serde_json::Value, k: &str| -> String {
        v.get(k)
            .and_then(|x| x.get("code"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let class_ok = matches!(
        (
            code(&b, "legislation").as_str(),
            code(&b, "riskClass").as_str()
        ),
        (
            "refdata.applicable-legislation.mdr",
            "refdata.risk-class.class-i"
        ) | (
            "refdata.applicable-legislation.ivdr",
            "refdata.risk-class.class-a"
        )
    );
    let spp = b
        .get("multiComponent")
        .and_then(|m| m.get("criterion"))
        .and_then(|c| c.as_str())
        .is_some_and(|c| c.eq_ignore_ascii_case("SPP"));
    let has_cert = b
        .get("deviceCertificateInfoListForDisplay")
        .and_then(|c| c.as_array())
        .is_some_and(|a| !a.is_empty());
    let sterile_false = d.get("sterile").and_then(|x| x.as_bool()) == Some(false);
    class_ok && !spp && !has_cert && sterile_false
}

/// A cached detail/basic file counts only when it starts like a JSON object.
/// An empty file or a saved HTML error page is fetched again.
pub fn cached_record_ok(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 64];
    let n = f.read(&mut head).unwrap_or(0);
    String::from_utf8_lossy(&head[..n])
        .trim_start()
        .starts_with('{')
}

/// Generic parallel download: fetch URL per UUID, save to dir, log to file.
/// A file in `force` is fetched even when cached; it is only overwritten
/// once the new answer arrived. Returns (downloaded_count, cached_count).
fn parallel_fetch(
    uuids: &[String],
    force: &HashSet<String>,
    target_dir: &Path,
    base_url: &str,
    log_prefix: &str,
    download_log: &Arc<Mutex<Option<std::fs::File>>>,
    threads: usize,
    max_retries: u32,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<(usize, usize)> {
    // Filter to only UUIDs not already on disk
    let need: Vec<&String> = uuids
        .iter()
        .filter(|uuid| {
            let p = target_dir.join(format!("{}.json", uuid));
            !cached_record_ok(&p) || force.contains(*uuid)
        })
        .collect();
    let cached = uuids.len() - need.len();

    progress.on_event(DownloadEvent::Progress {
        step: "Download".into(),
        detail: format!(
            "Downloading {} {} files ({} cached)...",
            need.len(),
            log_prefix,
            cached
        ),
    });

    let downloaded = AtomicUsize::new(0);
    let total_need = need.len();
    let base_url_owned = base_url.to_string();
    let target_dir_owned = target_dir.to_path_buf();
    let dl_log = download_log.clone();
    let prefix = log_prefix.to_string();
    let last_reported = AtomicUsize::new(0);
    let agent = eudamed_agent();

    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .context("Failed to build rayon thread pool for parallel fetch")?
        .install(|| {
            need.par_iter().for_each(|uuid| {
                let full_url = format!("{}/{}?languageIso2Code=en", base_url_owned, uuid);
                // Honors Retry-After on a 429 (replaces a 2-6 s linear backoff that
                // could never clear EUDAMED's ~60 s rate window → "still missing").
                if let Ok(body) = eudamed_get(&agent, limiter, &full_url, max_retries.max(4)) {
                    let path = target_dir_owned.join(format!("{}.json", uuid));
                    let _ = std::fs::write(&path, &body);
                    let count = downloaded.fetch_add(1, Ordering::Relaxed) + 1;
                    // Report progress every 10 files or at the end
                    let prev = last_reported.load(Ordering::Relaxed);
                    if count == total_need || count >= prev + 10 {
                        if last_reported
                            .compare_exchange(prev, count, Ordering::Relaxed, Ordering::Relaxed)
                            .is_ok()
                        {
                            progress.on_event(DownloadEvent::Log(format!(
                                "  {} {}/{} downloaded",
                                prefix, count, total_need
                            )));
                            progress.on_event(DownloadEvent::Status {
                                phase: prefix.clone(),
                                done: count,
                                total: total_need,
                            });
                        }
                    }
                    if let Ok(mut guard) = dl_log.lock() {
                        if let Some(ref mut f) = *guard {
                            let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
                            let _ = writeln!(f, "{} {} {}.json", ts, prefix, uuid);
                        }
                    }
                }
            });
        });

    Ok((downloaded.load(Ordering::Relaxed), cached))
}

#[cfg(test)]
mod gtin_plan_tests {
    use super::*;
    use std::collections::HashMap;

    fn known(rows: &[(&str, &str, &str)]) -> HashMap<String, Vec<(String, String)>> {
        let mut m: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (di, u, s) in rows {
            m.entry(di.to_string())
                .or_default()
                .push((u.to_string(), s.to_string()));
        }
        m
    }
    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unknown_gtins_and_lone_gtins_of_unknown_srns_stay_single() {
        let k = known(&[("00000000000001", "u1", "CH-MF-1")]);
        let p = plan_gtin_lookups(
            &v(&["00000000000001", "00000000000009"]),
            &k,
            &HashMap::new(),
        );
        assert!(p.by_srn.is_empty());
        assert_eq!(p.singles, v(&["00000000000009", "00000000000001"]));
    }

    #[test]
    fn short_listing_is_used_long_listing_is_not() {
        let k = known(&[
            ("01", "u1", "SHORT"),
            ("02", "u2", "SHORT"),
            ("03", "u3", "SHORT"),
            ("04", "u4", "LONG"),
            ("05", "u5", "LONG"),
        ]);
        let sizes: HashMap<String, u64> =
            [("SHORT".to_string(), 2), ("LONG".to_string(), 40)].into();
        let p = plan_gtin_lookups(&v(&["01", "02", "03", "04", "05"]), &k, &sizes);
        assert_eq!(p.by_srn.len(), 1);
        assert_eq!(p.by_srn[0].srn, "SHORT");
        assert_eq!(p.by_srn[0].gtins, v(&["01", "02", "03"]));
        assert_eq!(p.by_srn[0].max_pages, None);
        assert_eq!(p.singles, v(&["04", "05"]));
    }

    #[test]
    fn unknown_size_probes_with_the_gtin_count_as_page_cap() {
        let k = known(&[("01", "u1", "X"), ("02", "u2", "X"), ("02", "u2b", "X")]);
        let p = plan_gtin_lookups(&v(&["01", "02", "02"]), &k, &HashMap::new());
        assert_eq!(p.by_srn.len(), 1);
        let g = &p.by_srn[0];
        assert_eq!(g.max_pages, Some(2));
        assert_eq!(g.gtin_uuids, vec![v(&["u1"]), v(&["u2", "u2b"])]);
        assert!(p.singles.is_empty());
    }

    #[test]
    fn short_gtin_matches_its_14_digit_form_and_split_srns_stay_single() {
        let k = known(&[
            ("04012345678901", "u1", "A"),
            ("04012345678902", "u2", "A"),
            ("05000000000000", "u3", "A"),
            ("05000000000000", "u4", "B"),
        ]);
        let sizes: HashMap<String, u64> = [("A".to_string(), 1)].into();
        let p = plan_gtin_lookups(
            &v(&["4012345678901", "04012345678902", "05000000000000"]),
            &k,
            &sizes,
        );
        assert_eq!(p.by_srn[0].gtins, v(&["4012345678901", "04012345678902"]));
        assert_eq!(p.singles, v(&["05000000000000"]));
    }
}

#[cfg(test)]
mod rate_tests {
    #[test]
    fn listing_page_check_rejects_answers_that_truncate() {
        let ok = r#"{"content":[{"uuid":"a"}],"totalPages":6}"#;
        assert!(listing_page_check(ok, 3, Some(6)).is_ok());
        assert!(listing_page_check("<html>down</html>", 3, Some(6)).is_err());
        assert!(listing_page_check(r#"{"error":"x"}"#, 3, Some(6)).is_err());
        assert!(listing_page_check(r#"{"content":[]}"#, 3, Some(6)).is_err());
        // An SRN without devices answers with an empty first page.
        assert!(listing_page_check(r#"{"content":[],"totalPages":0}"#, 0, None).is_ok());
    }

    use super::*;

    #[test]
    fn listing_placeholder_never_counts_as_a_version() {
        assert_eq!(real_listing_version(Some(0)), None);
        assert_eq!(real_listing_version(None), None);
        assert_eq!(real_listing_version(Some(2)), Some(2));
    }

    #[test]
    fn rate_interval_override() {
        assert_eq!(parse_rate_interval_ms(None), DEFAULT_RATE_INTERVAL_MS);
        assert_eq!(parse_rate_interval_ms(Some("3200")), 3200);
        assert_eq!(parse_rate_interval_ms(Some(" 3200\n")), 3200);
        // Garbage or a value that would hammer the API keeps the default.
        assert_eq!(
            parse_rate_interval_ms(Some("fast")),
            DEFAULT_RATE_INTERVAL_MS
        );
        assert_eq!(parse_rate_interval_ms(Some("0")), DEFAULT_RATE_INTERVAL_MS);
        assert_eq!(parse_rate_interval_ms(Some("")), DEFAULT_RATE_INTERVAL_MS);
    }
}

#[cfg(test)]
mod public_ui_header_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    fn header_value<'a>(req: &'a str, name: &str) -> Option<&'a str> {
        req.lines().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.eq_ignore_ascii_case(name) {
                Some(v.trim())
            } else {
                None
            }
        })
    }

    /// Accept one HTTP/1 request and return the raw header block.
    fn capture_one_request(url_path_and_query: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                match sock.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let body = b"{\"ok\":true}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.write_all(body);
            String::from_utf8_lossy(&buf).into_owned()
        });

        let agent = eudamed_agent();
        let limiter = RateLimiter::new(Duration::from_millis(0));
        let url = format!("http://127.0.0.1:{port}{url_path_and_query}");
        let body = eudamed_get(&agent, &limiter, &url, 1).expect("local request");
        assert_eq!(body, "{\"ok\":true}");
        server.join().unwrap()
    }

    #[test]
    fn record_urls_are_detail_and_basic_not_listing_or_datalake() {
        assert!(is_public_device_record_url(
            "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en"
        ));
        assert!(is_public_device_record_url(
            "https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en"
        ));
        assert!(!is_public_device_record_url(
            "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData?page=0&size=300&srn=DE-MF-000000001&iso2Code=en&languageIso2Code=en"
        ));
        assert!(!is_public_device_record_url(
            "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/"
        ));
        assert!(!is_public_device_record_url(
            "https://api.datalake.sante.service.ec.europa.eu/eudamed/udi?format=json&MF_SRN=DE-MF-000000001"
        ));
    }

    #[test]
    fn browser_user_agent_is_the_windows_chrome_that_returned_200() {
        assert!(PUBLIC_UI_USER_AGENT.starts_with("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"));
        assert!(PUBLIC_UI_USER_AGENT.contains("Chrome/120.0.0.0"));
        assert!(!PUBLIC_UI_USER_AGENT.contains("Macintosh"));
        assert!(!PUBLIC_UI_USER_AGENT
            .to_ascii_lowercase()
            .contains("eudamed2firstbase"));
        assert_eq!(PUBLIC_UI_ACCEPT, "application/json, text/plain, */*");
        assert_eq!(PUBLIC_UI_ACCEPT_LANGUAGE, "en-US,en;q=0.9");
    }

    #[test]
    fn detail_and_basic_send_windows_browser_headers_listing_does_not() {
        let detail = capture_one_request(
            "/devices/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en",
        );
        assert_eq!(
            header_value(&detail, "user-agent"),
            Some(PUBLIC_UI_USER_AGENT)
        );
        assert_eq!(header_value(&detail, "accept"), Some(PUBLIC_UI_ACCEPT));
        assert_eq!(
            header_value(&detail, "accept-language"),
            Some(PUBLIC_UI_ACCEPT_LANGUAGE)
        );
        assert!(!detail.to_ascii_lowercase().contains("eudamed2firstbase"));

        let basic = capture_one_request(
            "/devices/basicUdiData/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en",
        );
        assert_eq!(
            header_value(&basic, "user-agent"),
            Some(PUBLIC_UI_USER_AGENT)
        );
        assert_eq!(header_value(&basic, "accept"), Some(PUBLIC_UI_ACCEPT));
        assert_eq!(
            header_value(&basic, "accept-language"),
            Some(PUBLIC_UI_ACCEPT_LANGUAGE)
        );

        let listing = capture_one_request(
            "/devices/udiDiData?page=0&size=300&srn=DE-MF-000000001&languageIso2Code=en",
        );
        let listing_ua = header_value(&listing, "user-agent").unwrap_or("");
        assert!(
            listing_ua.starts_with("ureq/"),
            "listing should keep ureq's User-Agent, got {listing_ua}"
        );
        assert!(!listing_ua.contains("Mozilla"));
        assert!(header_value(&listing, "accept-language").is_none());
    }
}

#[cfg(test)]
mod listing_breaker_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::time::Duration;

    struct Silent;
    impl DownloadProgress for Silent {
        fn on_event(&self, _: DownloadEvent) {}
    }

    #[test]
    fn raw_listing_connection_can_store_row_hashes() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        ensure_listing_cache(&conn);
        set_row_hash(&conn, "u1", "h1");
        let h: String = conn
            .query_row(
                "SELECT hash FROM datalake_row_hash WHERE uuid='u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(h, "h1");
    }

    #[test]
    fn blocked_endpoint_keeps_cached_details_of_unchanged_devices() {
        let dir = std::env::temp_dir().join(format!("e2f_blocked_{}", std::process::id()));
        let (detail, basic) = (dir.join("detail"), dir.join("basic"));
        std::fs::create_dir_all(&detail).unwrap();
        std::fs::create_dir_all(&basic).unwrap();
        for u in ["new", "changed", "nobasic"] {
            std::fs::write(detail.join(format!("{u}.json")), "{\"uuid\":1}").unwrap();
        }
        for u in ["new", "changed"] {
            std::fs::write(basic.join(format!("{u}.json")), "{\"code\":1}").unwrap();
        }
        // Run started after the files were written: all details are "old".
        let run_start = std::time::SystemTime::now() + Duration::from_secs(5);
        let stale: HashSet<String> = ["changed".to_string()].into();
        let ok = |u| usable_when_blocked(u, &stale, &detail, &basic, run_start);
        assert!(
            ok("new"),
            "a cached detail of an unchanged device is usable"
        );
        assert!(
            !ok("changed"),
            "a changed device needs a detail of this run"
        );
        assert!(!ok("nobasic"), "no Basic UDI-DI, no push");
        assert!(!ok("missing"));
        // A detail written during the run is usable for a changed device too.
        let earlier = std::time::SystemTime::now() - Duration::from_secs(60);
        assert!(usable_when_blocked(
            "changed", &stale, &detail, &basic, earlier
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_devices_with_cached_files_are_marked_stale() {
        let dir = std::env::temp_dir().join(format!("e2f_stale_{}", std::process::id()));
        let (detail, basic) = (dir.join("detail"), dir.join("basic"));
        std::fs::create_dir_all(&detail).unwrap();
        std::fs::create_dir_all(&basic).unwrap();
        let conn = crate::version_db::open_db(&dir.join("v.db")).unwrap();
        for u in ["same", "bumped", "rowhash"] {
            std::fs::write(detail.join(format!("{u}.json")), "{}").unwrap();
            std::fs::write(basic.join(format!("{u}.json")), "{}").unwrap();
            let rec = crate::version_db::VersionRecord {
                uuid: u.to_string(),
                udi_version: Some(1),
                last_synced: Some("2026-11-07T00:00:00".to_string()),
                ..Default::default()
            };
            crate::version_db::upsert_version(&conn, &rec).unwrap();
        }
        let listing = vec![
            ("same".to_string(), Some(1), None),
            ("bumped".to_string(), Some(2), None),
            ("rowhash".to_string(), Some(1), None),
        ];
        let mut changed = HashMap::new();
        changed.insert("rowhash".to_string(), "h2".to_string());
        let (need, unchanged, stale) =
            filter_unchanged(&listing, &conn, &detail, &basic, &changed, &Silent);
        assert_eq!(unchanged, 1);
        assert_eq!(need, vec!["bumped".to_string(), "rowhash".to_string()]);
        assert!(stale.contains("bumped") && stale.contains("rowhash") && !stale.contains("same"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The listing source is a process-wide environment variable: tests that
    /// set it must not run at the same time.
    static SOURCE_LOCK: Mutex<()> = Mutex::new(());

    struct ListingSourceGuard {
        prev: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ListingSourceGuard {
        fn datalake() -> Self {
            let lock = SOURCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("EUDAMED_LISTING_SOURCE").ok();
            // The process environment is the production switch. Restored on drop
            // so other tests keep seeing an unset source.
            unsafe { std::env::set_var("EUDAMED_LISTING_SOURCE", "datalake") };
            Self { prev, _lock: lock }
        }
    }

    impl Drop for ListingSourceGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var("EUDAMED_LISTING_SOURCE", v),
                    None => std::env::remove_var("EUDAMED_LISTING_SOURCE"),
                }
            }
            crate::datalake::set_test_udi_url(None);
        }
    }

    struct LocalApi {
        port: u16,
        hits: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl LocalApi {
        /// One server for both APIs. `/eudamed/udi` answers as the datalake
        /// (empty for `MF_SRN`, one device for `PRIMARY_DI`). Anything else is
        /// the public UI: 429 with the M2M notice for a listing, 200 for a
        /// device record.
        fn spawn() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let hits_t = hits.clone();
            let stop_t = stop.clone();
            let join = std::thread::spawn(move || {
                while !stop_t.load(AtomicOrdering::Relaxed) {
                    match listener.accept() {
                        Ok((mut sock, _)) => {
                            sock.set_nonblocking(false).unwrap();
                            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut buf = Vec::new();
                            let mut tmp = [0u8; 1024];
                            loop {
                                match sock.read(&mut tmp) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        buf.extend_from_slice(&tmp[..n]);
                                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            let req = String::from_utf8_lossy(&buf).into_owned();
                            let target = req
                                .lines()
                                .next()
                                .unwrap_or("")
                                .split_whitespace()
                                .nth(1)
                                .unwrap_or("")
                                .to_string();
                            hits_t.lock().unwrap().push(target.clone());
                            let (status, reason, body) = if target.contains("/eudamed/udi") {
                                if target.contains("PRIMARY_DI=") {
                                    (
                                        200,
                                        "OK",
                                        r#"{"value":[{"UUID":"522a2c98-6a71-4b20-8959-ad1287b2e776","PRIMARY_DI":"00195451000232","VERSION_NUMBER":2,"LATEST_VERSION":1,"MF_SRN":"US-MF-000016658","MF_NAME":"M","TRADE_NAME":"Shockwave","RISK_CLASS_ID":-204,"DEVICE_STATUS_TYPE_ID":-11,"BASIC_UDI_DATA_ULID":"01M4"}]}"#,
                                    )
                                } else {
                                    (200, "OK", r#"{"value":[]}"#)
                                }
                            } else if target.contains("/udiDiData/00000000-html") {
                                (
                                    200,
                                    "OK",
                                    "<!DOCTYPE html>\n<html><title>Server temporarily unavailable</title></html>",
                                )
                            } else if is_public_device_record_url(&format!(
                                "https://ec.europa.eu{target}"
                            )) {
                                (200, "OK", r#"{"ok":true}"#)
                            } else {
                                (
                                    429,
                                    "Too Many Requests",
                                    r#"{"message":"machineToMachineApi"}"#,
                                )
                            };
                            let resp = format!(
                                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nRetry-After: 300\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = sock.write_all(resp.as_bytes());
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                port,
                hits,
                stop,
                join: Some(join),
            }
        }
    }

    impl Drop for LocalApi {
        fn drop(&mut self) {
            self.stop.store(true, AtomicOrdering::Relaxed);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    #[test]
    fn html_error_page_with_200_is_an_error_not_a_record() {
        let api = LocalApi::spawn();
        let limiter = RateLimiter::new(Duration::from_millis(0));
        let agent = eudamed_agent();
        let detail = format!(
            "http://127.0.0.1:{}/devices/udiDiData/00000000-html?languageIso2Code=en",
            api.port
        );
        if public_endpoint_blocked() {
            return; // another test tripped the shared breaker
        }
        assert_eq!(
            eudamed_get(&agent, &limiter, &detail, 1).unwrap_err(),
            NOT_JSON_ERR
        );
    }

    #[test]
    fn deep_scan_skips_basic_only_for_certificate_free_class_i() {
        let basic = |risk: &str, leg: &str, extra: &str| {
            format!(
                r#"{{"riskClass":{{"code":"refdata.risk-class.{risk}"}},"legislation":{{"code":"refdata.applicable-legislation.{leg}"}}{extra}}}"#
            )
        };
        let not_sterile = r#"{"uuid":"x","sterile":false}"#;
        let sterile = r#"{"uuid":"x","sterile":true}"#;
        let unknown = r#"{"uuid":"x"}"#;
        assert!(basic_covered_by_datalake(
            &basic("class-i", "mdr", ""),
            not_sterile
        ));
        assert!(basic_covered_by_datalake(
            &basic("class-a", "ivdr", ""),
            not_sterile
        ));
        assert!(!basic_covered_by_datalake(
            &basic("class-i", "mdr", ""),
            sterile
        ));
        assert!(!basic_covered_by_datalake(
            &basic("class-i", "mdr", ""),
            unknown
        ));
        assert!(!basic_covered_by_datalake(
            &basic("class-iia", "mdr", ""),
            not_sterile
        ));
        assert!(!basic_covered_by_datalake(
            &basic("class-i", "mdd", ""),
            not_sterile
        ));
        let cert = r#","deviceCertificateInfoListForDisplay":[{"certificateNumber":"1"}]"#;
        assert!(!basic_covered_by_datalake(
            &basic("class-i", "mdr", cert),
            not_sterile
        ));
        let spp = r#","multiComponent":{"criterion":"SPP"}"#;
        assert!(!basic_covered_by_datalake(
            &basic("class-i", "mdr", spp),
            not_sterile
        ));
        assert!(!basic_covered_by_datalake("<!DOCTYPE html>", not_sterile));
    }

    #[test]
    fn saved_html_page_is_not_a_cached_record() {
        assert!(looks_like_json_record("  {\"uuid\":\"x\"}"));
        assert!(!looks_like_json_record("<!DOCTYPE html>"));
        let dir = std::env::temp_dir().join(format!("e2f_cache_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok.json");
        let html = dir.join("html.json");
        let empty = dir.join("empty.json");
        std::fs::write(&ok, "\n{\"uuid\":\"x\"}").unwrap();
        std::fs::write(&html, "<!DOCTYPE html>\n<html lang=\"en\">").unwrap();
        std::fs::write(&empty, "").unwrap();
        assert!(cached_record_ok(&ok));
        assert!(!cached_record_ok(&html));
        assert!(!cached_record_ok(&empty));
        assert!(!cached_record_ok(&dir.join("missing.json")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_429s_trip_the_breaker_listing_429s_do_not() {
        let listing = "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData?page=0&size=300&srn=DE-MF-000000001";
        let detail = "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en";
        let basic = "https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en";
        let mut circuit = PublicCircuit::fresh();
        for _ in 0..PUBLIC_BLOCK_AFTER {
            circuit = circuit.observe(listing, PublicOutcome::Http429);
        }
        assert!(!circuit.blocked);
        assert_eq!(circuit.streak, 0);
        for _ in 0..PUBLIC_BLOCK_AFTER - 1 {
            circuit = circuit.observe(detail, PublicOutcome::Http429);
        }
        assert!(!circuit.blocked);
        circuit = circuit.observe(basic, PublicOutcome::Http429);
        assert!(circuit.blocked);
        circuit = circuit.observe(detail, PublicOutcome::Ok);
        assert!(circuit.blocked);
    }

    #[test]
    fn listing_429s_still_send_the_later_detail_and_basic_requests() {
        reset_public_circuit();
        let api = LocalApi::spawn();
        let limiter = RateLimiter::new(Duration::from_millis(0));
        let agent = eudamed_agent();
        let listing = format!(
            "http://127.0.0.1:{}/devices/udiDiData?page=0&size=300&srn=DE-MF-000000001&languageIso2Code=en",
            api.port
        );
        for _ in 0..PUBLIC_BLOCK_AFTER {
            let err = eudamed_get(&agent, &limiter, &listing, 1).unwrap_err();
            assert!(
                err.starts_with("HTTP 429"),
                "listing 429 should be reported, not swallowed: {err}"
            );
        }
        assert!(
            !public_endpoint_blocked(),
            "listing 429s armed the detail/basic circuit breaker"
        );
        let detail = format!(
            "http://127.0.0.1:{}/devices/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en",
            api.port
        );
        let basic = format!(
            "http://127.0.0.1:{}/devices/basicUdiData/udiDiData/522a2c98-6a71-4b20-8959-ad1287b2e776?languageIso2Code=en",
            api.port
        );
        assert_eq!(
            eudamed_get(&agent, &limiter, &detail, 1).unwrap(),
            r#"{"ok":true}"#
        );
        assert_eq!(
            eudamed_get(&agent, &limiter, &basic, 1).unwrap(),
            r#"{"ok":true}"#
        );
        let hits = api.hits.lock().unwrap().clone();
        assert!(
            hits.iter().any(|h| h.contains("/udiDiData/522a2c98")),
            "detail was not sent: {hits:?}"
        );
        assert!(
            hits.iter()
                .any(|h| h.contains("/basicUdiData/udiDiData/522a2c98")),
            "basic was not sent: {hits:?}"
        );
        assert!(!public_endpoint_blocked());
    }

    #[test]
    fn datalake_listing_source_does_not_probe_the_public_listing() {
        let _source = ListingSourceGuard::datalake();
        let api = LocalApi::spawn();
        crate::datalake::set_test_udi_url(Some(format!(
            "http://127.0.0.1:{}/eudamed/udi",
            api.port
        )));
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        ensure_listing_cache(&conn);
        let conn = Mutex::new(conn);
        let limiter = RateLimiter::new(Duration::from_millis(0));
        let public_base = format!("http://127.0.0.1:{}/devices/udiDiData", api.port);

        let empty = download_listing_for_srn_opts(
            &public_base,
            "DE-MF-000000001",
            None,
            None,
            None,
            &conn,
            &limiter,
            &Silent,
        );
        assert!(empty.entries.is_empty());
        assert!(empty.complete);

        let found =
            download_listing_for_gtin(&public_base, "00195451000232", &conn, &limiter, &Silent);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "522a2c98-6a71-4b20-8959-ad1287b2e776");

        let hits = api.hits.lock().unwrap().clone();
        assert!(
            hits.iter().any(|h| h.contains("MF_SRN=")),
            "datalake SRN listing was not asked: {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h.contains("PRIMARY_DI=")),
            "datalake GTIN lookup was not asked: {hits:?}"
        );
        assert!(
            hits.iter().all(|h| !h.contains("udiDiData")),
            "public listing was probed: {hits:?}"
        );
    }

    /// 10.11.2026: the datalake answered 504 all day and the public listing
    /// 429. The devices known from earlier runs must still be handed to the
    /// version check, so pending ones are retried.
    #[test]
    fn datalake_outage_reads_the_cached_listing() {
        let _source = ListingSourceGuard::datalake();
        let api = LocalApi::spawn();
        crate::datalake::set_test_udi_url(Some(format!(
            "http://127.0.0.1:{}/eudamed/udi",
            api.port
        )));
        crate::datalake::trip_test_breaker();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        ensure_listing_cache(&conn);
        for (uuid, di, version) in [("u1", "04000000000011", 3), ("u2", "04000000000028", 1)] {
            conn.execute(
                "INSERT INTO listing_cache (uuid, srn, primary_di, version_number, listed_at) VALUES (?1, 'DE-MF-000000001', ?2, ?3, 'x')",
                rusqlite::params![uuid, di, version],
            )
            .unwrap();
        }
        let conn = Mutex::new(conn);
        let limiter = RateLimiter::new(Duration::from_millis(0));
        let public_base = format!("http://127.0.0.1:{}/devices/udiDiData", api.port);

        let all = download_listing_for_srn_opts(
            &public_base,
            "DE-MF-000000001",
            None,
            None,
            None,
            &conn,
            &limiter,
            &Silent,
        );
        assert_eq!(
            all.entries,
            vec![
                ("u1".to_string(), Some(3), None),
                ("u2".to_string(), Some(1), None)
            ]
        );
        assert!(!all.complete, "a cached listing is never complete");

        let keep: std::collections::HashSet<String> = ["04000000000028".to_string()].into();
        let kept = download_listing_for_srn_opts(
            &public_base,
            "DE-MF-000000001",
            None,
            Some(&keep),
            None,
            &conn,
            &limiter,
            &Silent,
        );
        assert_eq!(kept.entries, vec![("u2".to_string(), Some(1), None)]);

        let unknown = download_listing_for_srn_opts(
            &public_base,
            "DE-MF-000000002",
            None,
            None,
            None,
            &conn,
            &limiter,
            &Silent,
        );
        assert!(unknown.entries.is_empty());

        let gtin =
            download_listing_for_gtin(&public_base, "4000000000011", &conn, &limiter, &Silent);
        assert_eq!(gtin, vec![("u1".to_string(), Some(3), None)]);

        crate::datalake::set_test_breaker(false);
        let hits = api.hits.lock().unwrap().clone();
        assert!(hits.is_empty(), "no source may be asked: {hits:?}");
    }
}
