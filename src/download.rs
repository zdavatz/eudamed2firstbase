//! Shared download module for EUDAMED data.
//! Used by both the GUI and the CLI `download` subcommand.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
    pub max_retries: u32,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            srns: Vec::new(),
            limit: None,
            data_dir: app_data_dir().join(DEFAULT_DATA_DIR),
            parallel_threads: 10,
            max_retries: 3,
        }
    }
}

/// Summary of what was downloaded.
pub struct DownloadResult {
    /// All UUIDs from the listing (with version numbers).
    pub uuid_versions: Vec<(String, Option<u32>)>,
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
        self.uuid_versions.iter().map(|(u, _)| u.clone()).collect()
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

    let uuid_versions = download_listings(EUDAMED_BASE_URL, &config.srns, config.limit, progress)?;

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

    log(&format!("{} UUIDs extracted from listings", uuid_versions.len()));

    // --- Step 2: Pre-download version check ---
    let db_dir = app_data_dir().join("db");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("version_tracking.db");
    let conn = crate::version_db::open_db(&db_path)?;

    let (need_download, unchanged_skipped) =
        filter_unchanged(&uuid_versions, &conn, progress);

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
    );
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
    );
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
            );
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
            );
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

/// Download paginated listings from EUDAMED, filtered by SRN.
/// Returns Vec<(uuid, Option<version_number>)>, deduplicated.
fn download_listings(
    base_url: &str,
    srns: &[String],
    limit: Option<usize>,
    progress: &dyn DownloadProgress,
) -> anyhow::Result<Vec<(String, Option<u32>)>> {
    let mut all_entries: Vec<(String, Option<u32>)> = Vec::new();

    for srn in srns {
        progress.on_event(DownloadEvent::Log(format!(
            "Downloading listing for SRN {}...",
            srn
        )));
        let mut page = 0;
        let mut srn_count = 0;

        loop {
            let url = format!(
                "{}?page={}&pageSize={}&srn={}&iso2Code=en&languageIso2Code=en",
                base_url, page, DEFAULT_PAGE_SIZE, srn
            );

            let resp = ureq::get(&url).call()?;
            let body: String = resp.into_body().read_to_string()?;

            let json: serde_json::Value = serde_json::from_str(&body)?;
            let content = json.get("content").and_then(|c| c.as_array());

            if let Some(items) = content {
                if items.is_empty() {
                    break;
                }

                let total_pages = json.get("totalPages").and_then(|t| t.as_u64()).unwrap_or(1);
                let total_elements = json.get("totalElements").and_then(|t| t.as_u64());

                for item in items {
                    if let Some(uuid) = item.get("uuid").and_then(|u| u.as_str()) {
                        let version = item
                            .get("versionNumber")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as u32);
                        all_entries.push((uuid.to_string(), version));
                        srn_count += 1;

                        if let Some(lim) = limit {
                            if srn_count >= lim {
                                break;
                            }
                        }
                    }
                }

                progress.on_event(DownloadEvent::Log(format!(
                    "  Listing page {}/{} — {} devices so far{}",
                    page + 1,
                    total_pages,
                    srn_count,
                    total_elements.map(|t| format!(" (of {} total)", t)).unwrap_or_default()
                )));

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

        progress.on_event(DownloadEvent::Log(format!(
            "  SRN {}: {} devices",
            srn, srn_count
        )));
    }

    // Deduplicate by UUID (keep highest version)
    all_entries.sort_by(|a, b| a.0.cmp(&b.0));
    all_entries.dedup_by(|a, b| {
        if a.0 == b.0 {
            if a.1 > b.1 {
                b.1 = a.1;
            }
            true
        } else {
            false
        }
    });

    Ok(all_entries)
}

/// Pre-download version check: compare listing versionNumber against version DB.
/// Returns (uuids_needing_download, count_unchanged).
fn filter_unchanged(
    uuid_versions: &[(String, Option<u32>)],
    conn: &rusqlite::Connection,
    _progress: &dyn DownloadProgress,
) -> (Vec<String>, usize) {
    let mut need_download = Vec::new();
    let mut unchanged = 0;

    for (uuid, listing_version) in uuid_versions {
        let db_version = crate::version_db::get_version(conn, uuid)
            .ok()
            .flatten()
            .and_then(|r| r.udi_version);
        if let Some(db_ver) = db_version {
            if Some(db_ver) == *listing_version {
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
) -> (usize, usize) {
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
        .unwrap()
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
                                    if last_reported.compare_exchange(prev, count, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                                        progress.on_event(DownloadEvent::Log(format!(
                                            "  {} {}/{} downloaded",
                                            prefix, count, total_need
                                        )));
                                    }
                                }
                                if let Ok(mut guard) = dl_log.lock() {
                                    if let Some(ref mut f) = *guard {
                                        let ts =
                                            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
                                        let _ =
                                            writeln!(f, "{} {} {}.json", ts, prefix, uuid);
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

    (downloaded.load(Ordering::Relaxed), cached)
}
