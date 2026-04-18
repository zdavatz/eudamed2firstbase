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
    Progress { step: String, detail: String },
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
        }
    }
}

/// Configuration for a download run.
pub struct DownloadConfig {
    pub srns: Vec<String>,
    pub limit: Option<usize>,
    pub data_dir: PathBuf,
    pub parallel_threads: usize,
    pub listing_threads: usize,
    pub max_retries: u32,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            srns: Vec::new(),
            limit: None,
            data_dir: app_data_dir().join(DEFAULT_DATA_DIR),
            parallel_threads: 50,
            listing_threads: 50,
            max_retries: 3,
        }
    }
}

/// Summary of what was downloaded.
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

    let uuid_versions = download_listings(
        EUDAMED_BASE_URL,
        &config.srns,
        config.limit,
        config.listing_threads,
        progress,
    )?;

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

    let (need_download, unchanged_skipped) = filter_unchanged(&uuid_versions, &conn, progress);

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
        config.parallel_threads,
        config.max_retries,
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
                config.parallel_threads,
                5, // more retries for completeness check
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
    {
        let tx = conn
            .unchecked_transaction()
            .context("Failed to open budi_version update transaction")?;
        for (uuid, _udi_ver, budi_ver) in &uuid_versions {
            if let Some(bv) = budi_ver {
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
}

/// Download paginated listings for a single SRN. Returns entries found.
fn download_listing_for_srn(
    base_url: &str,
    srn: &str,
    limit: Option<usize>,
    conn: &Mutex<rusqlite::Connection>,
    progress: &dyn DownloadProgress,
) -> Vec<(String, Option<u32>, Option<u32>)> {
    let mut entries: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    let mut page = 0;
    let mut srn_count = 0;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    loop {
        let url = format!(
            "{}?page={}&pageSize={}&srn={}&iso2Code=en&languageIso2Code=en",
            base_url, page, DEFAULT_PAGE_SIZE, srn
        );

        let resp = match ureq::get(&url).call() {
            Ok(r) => r,
            Err(e) => {
                progress.on_event(DownloadEvent::Log(format!(
                    "  SRN {} page {} error: {}",
                    srn, page, e
                )));
                break;
            }
        };
        let body: String = match resp.into_body().read_to_string() {
            Ok(b) => b,
            Err(_) => break,
        };

        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(_) => break,
        };
        let content = json.get("content").and_then(|c| c.as_array());

        if let Some(items) = content {
            if items.is_empty() {
                break;
            }

            let total_pages = json.get("totalPages").and_then(|t| t.as_u64()).unwrap_or(1);
            let total_elements = json.get("totalElements").and_then(|t| t.as_u64());

            // Batch insert into DB
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

                        let _ = db.execute(
                            "INSERT OR REPLACE INTO listing_cache (uuid, srn, manufacturer_name, primary_di, trade_name, risk_class, device_status, version_number, budi_version_number, listed_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            rusqlite::params![uuid, mfr_srn, mfr_name, primary_di, trade_name, risk_class, device_status, version, budi_version, now],
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
            "  SRN {}: {} devices",
            srn, srn_count
        )));
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
                    let entries =
                        download_listing_for_srn(&base_url_owned, srn, limit, &conn, progress);
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done % 100 == 0 || done == total_srns {
                        progress.on_event(DownloadEvent::Log(format!(
                            "[Listing] {}/{} SRNs completed",
                            done, total_srns
                        )));
                    }
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

/// Pre-download version check: compare listing versionNumber and basicUdiDataVersionNumber against version DB.
/// Returns (uuids_needing_download, count_unchanged).
fn filter_unchanged(
    uuid_versions: &[(String, Option<u32>, Option<u32>)],
    conn: &rusqlite::Connection,
    _progress: &dyn DownloadProgress,
) -> (Vec<String>, usize) {
    let mut need_download = Vec::new();
    let mut unchanged = 0;

    for (uuid, listing_version, budi_listing_version) in uuid_versions {
        if let Ok(Some(db_rec)) = crate::version_db::get_version(conn, uuid) {
            let udi_match = match (db_rec.udi_version, listing_version) {
                (Some(db), Some(listing)) => db == *listing,
                (None, None) => true,
                _ => false,
            };
            let budi_match = match (db_rec.budi_version, budi_listing_version) {
                (Some(db), Some(listing)) => db == *listing,
                (None, None) => true,
                (None, Some(_)) => false, // new BUDI data available
                _ => false,
            };
            if udi_match && budi_match {
                unchanged += 1;
                continue;
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

    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .context("Failed to build rayon thread pool for parallel fetch")?
        .install(|| {
            need.par_iter().for_each(|uuid| {
                let full_url = format!("{}/{}?languageIso2Code=en", base_url_owned, uuid);
                for attempt in 1..=max_retries {
                    match ureq::get(&full_url).call() {
                        Ok(resp) => {
                            if let Ok(body) = resp.into_body().read_to_string() {
                                let path = target_dir_owned.join(format!("{}.json", uuid));
                                let _ = std::fs::write(&path, &body);
                                let count = downloaded.fetch_add(1, Ordering::Relaxed) + 1;
                                // Report progress every 10 files or at the end
                                let prev = last_reported.load(Ordering::Relaxed);
                                if count == total_need || count >= prev + 10 {
                                    if last_reported
                                        .compare_exchange(
                                            prev,
                                            count,
                                            Ordering::Relaxed,
                                            Ordering::Relaxed,
                                        )
                                        .is_ok()
                                    {
                                        progress.on_event(DownloadEvent::Log(format!(
                                            "  {} {}/{} downloaded",
                                            prefix, count, total_need
                                        )));
                                    }
                                }
                                if let Ok(mut guard) = dl_log.lock() {
                                    if let Some(ref mut f) = *guard {
                                        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
                                        let _ = writeln!(f, "{} {} {}.json", ts, prefix, uuid);
                                    }
                                }
                            }
                            return;
                        }
                        Err(_) if attempt < max_retries => {
                            std::thread::sleep(std::time::Duration::from_secs(
                                (attempt as u64) * 2,
                            ));
                        }
                        Err(_) => {}
                    }
                }
            });
        });

    Ok((downloaded.load(Ordering::Relaxed), cached))
}
