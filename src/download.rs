//! Shared download module for EUDAMED data.
//! Used by both the GUI and the CLI `download` subcommand.

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

    let (need_download, unchanged_skipped) =
        filter_unchanged(&uuid_versions, &conn, &detail_dir, &basic_dir, progress);

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
    let (basic_downloaded, basic_cached) = parallel_fetch(
        &need_download,
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
            !p.exists() || std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(true)
        })
        .count();
    let missing_basic = need_download
        .iter()
        .filter(|u| {
            let p = basic_dir.join(format!("{}.json", u));
            !p.exists() || std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(true)
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
                !p.exists() || std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(true)
            })
            .cloned()
            .collect();
        let missing_basic_uuids: Vec<String> = need_download
            .iter()
            .filter(|u| {
                let p = basic_dir.join(format!("{}.json", u));
                !p.exists() || std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(true)
            })
            .cloned()
            .collect();

        if !missing_detail_uuids.is_empty() {
            parallel_fetch(
                &missing_detail_uuids,
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
            .filter(|u| !detail_dir.join(format!("{}.json", u)).exists())
            .count();
        let still_missing_b = need_download
            .iter()
            .filter(|u| !basic_dir.join(format!("{}.json", u)).exists())
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

/// A ureq agent that surfaces HTTP status (so 429 / `Retry-After` are readable
/// instead of arriving as an opaque `Err`) with a generous global timeout.
pub fn eudamed_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(EUDAMED_TIMEOUT_SECS)))
        .build()
        .new_agent()
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
    let mut last = String::from("no response");
    for attempt in 1..=max_attempts.max(1) {
        // Proactive pacing: wait for a budget slot BEFORE every request so the
        // aggregate stays under EUDAMED's shared ~60/60 s limit and 429s (and
        // their 60 s Retry-After stalls) are avoided rather than reacted to.
        limiter.acquire();
        match agent.get(url).call() {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                if (200..300).contains(&status) {
                    return resp.body_mut().read_to_string().map_err(|e| e.to_string());
                }
                if status == 429 {
                    let wait = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
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

/// Download paginated listings for a single SRN. Returns entries found.
fn download_listing_for_srn(
    base_url: &str,
    srn: &str,
    limit: Option<usize>,
    conn: &Mutex<rusqlite::Connection>,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    let mut entries: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    let mut page = 0;
    let mut srn_count = 0;
    let mut srn_new = 0usize;
    let mut srn_udi_bumped = 0usize;
    let mut srn_budi_bumped = 0usize;
    let mut srn_unchanged = 0usize;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let agent = eudamed_agent();
    let mut known_total_pages: Option<u64> = None;
    let mut known_total_elements: Option<u64> = None;

    loop {
        // `size`, not `pageSize`: EUDAMED caps `pageSize` at 20 but honours
        // `size` up to 300 (measured 09.10.2026: 46 instead of 680 pages for a
        // 13'583-device SRN) — 15x fewer listing requests out of the shared budget.
        let url = format!(
            "{}?page={}&size={}&srn={}&iso2Code=en&languageIso2Code=en",
            base_url, page, DEFAULT_PAGE_SIZE, srn
        );

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
                break;
            }
        };
        let content = json.get("content").and_then(|c| c.as_array());

        if let Some(items) = content {
            if items.is_empty() {
                break;
            }

            let total_pages = json.get("totalPages").and_then(|t| t.as_u64()).unwrap_or(1);
            let total_elements = json.get("totalElements").and_then(|t| t.as_u64());
            known_total_pages = Some(total_pages);
            known_total_elements = total_elements;

            // Batch insert into DB + on-the-fly version classification
            let page_start = entries.len();
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
                    break;
                }
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
        progress.on_event(DownloadEvent::Log(format!(
            "  SRN {}: {} devices  [{}↑ new, {}↑ udi, {}↑ budi, {} same]",
            srn, srn_count, srn_new, srn_udi_bumped, srn_budi_bumped, srn_unchanged
        )));
    }
    if let (Some(total), None) = (known_total_elements, limit) {
        if (srn_count as u64) < total {
            progress.on_event(DownloadEvent::Log(format!(
                "  WARNING: SRN {} incomplete — {} of {} devices listed; run again for the rest",
                srn, srn_count, total
            )));
        }
    }

    entries
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
fn download_listing_for_gtin(
    base_url: &str,
    gtin: &str,
    conn: &Mutex<rusqlite::Connection>,
    limiter: &RateLimiter,
    progress: &dyn DownloadProgress,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
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
    let conn = Mutex::new(conn);

    progress.on_event(DownloadEvent::Log(format!(
        "Resolving {} GTIN(s) via primaryDi ({} parallel)...",
        gtins.len(),
        listing_threads
    )));

    let base_url_owned = base_url.to_string();
    let completed = AtomicUsize::new(0);
    let total = gtins.len();

    let all_entries: Vec<Vec<(String, Option<u32>, Option<u32>)>> = rayon::ThreadPoolBuilder::new()
        .num_threads(listing_threads)
        .build()
        .context("Failed to build rayon thread pool for GTIN lookups")?
        .install(|| {
            gtins
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

    let mut flat: Vec<(String, Option<u32>, Option<u32>)> =
        all_entries.into_iter().flatten().collect();

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
    _progress: &dyn DownloadProgress,
) -> (Vec<String>, usize) {
    let mut need_download = Vec::new();
    let mut unchanged = 0;

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
            if !udi_bumped && !budi_bumped {
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

    (need_download, unchanged)
}

/// Generic parallel download: fetch URL per UUID, save to dir, log to file.
/// Returns (downloaded_count, cached_count).
fn parallel_fetch(
    uuids: &[String],
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
            !p.exists() || std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(true)
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
