// Hide console window on Windows when running as GUI
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actors;
mod api_detail;
mod api_json;
mod config;
mod download;
mod eudamed;
mod eudamed_json;
mod firstbase;
mod gui;
mod installer;
mod mail;
mod mappings;
mod scan;
mod sheet;
mod swissdamed;
mod transform;
mod transform_api;
mod transform_detail;
mod transform_eudamed_json;
mod update;
mod version_db;
mod whatsapp;
mod xlsx_export;

use anyhow::{Context, Result};
use chrono::Local;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

/// Default directory for cached Basic UDI-DI data
const BASIC_UDI_CACHE_DIR: &str = "eudamed_json/basic";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // No arguments → launch GUI
    if args.len() <= 1 {
        gui::run_gui().map_err(|e| anyhow::anyhow!("GUI error: {}", e))?;
        return Ok(());
    }

    // "gui" subcommand also launches GUI
    if args.get(1).map(|s| s.as_str()) == Some("gui") {
        gui::run_gui().map_err(|e| anyhow::anyhow!("GUI error: {}", e))?;
        return Ok(());
    }

    let config_path = Path::new("config.toml");
    let config = config::load_config(config_path).context("Failed to load config.toml")?;

    match args.get(1).map(|s| s.as_str()) {
        Some("sync-srns") => {
            // Refresh the SRN worklist from the eudamed2firstbase_SRN Google Sheet.
            // Usage: cargo run sync-srns [outfile]   (default: srns_sheet.txt)
            // Picks up newly added SRNs so the nightly `check` covers them without
            // anyone editing the file by hand. On any sheet-read error the existing
            // file is left untouched (a transient API hiccup must never wipe the
            // worklist) and the command exits non-zero.
            let out_path = args
                .get(2)
                .filter(|s| !s.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| "srns_sheet.txt".to_string());

            let config_path = std::path::Path::new("config.toml");
            let config_path = if config_path.exists() {
                config_path.to_path_buf()
            } else {
                download::app_data_dir().join("config.toml")
            };
            let cfg = config::load_config(&config_path)?;

            let srns = match sheet::fetch_srns(&cfg) {
                Ok(s) if !s.is_empty() => s,
                Ok(_) => {
                    eprintln!(
                        "sync-srns: sheet returned 0 valid SRNs — keeping existing {} unchanged.",
                        out_path
                    );
                    std::process::exit(2);
                }
                Err(e) => {
                    eprintln!(
                        "sync-srns: ERROR reading sheet ({e}) — keeping existing {} unchanged.",
                        out_path
                    );
                    std::process::exit(2);
                }
            };

            let old: Vec<String> = std::fs::read_to_string(&out_path)
                .map(|s| s.split_whitespace().map(|x| x.to_string()).collect())
                .unwrap_or_default();
            let old_set: std::collections::HashSet<&String> = old.iter().collect();
            let new_set: std::collections::HashSet<&String> = srns.iter().collect();
            let added: Vec<&String> = srns.iter().filter(|s| !old_set.contains(*s)).collect();
            let removed: Vec<&String> = old.iter().filter(|s| !new_set.contains(*s)).collect();

            std::fs::write(&out_path, format!("{}\n", srns.join("\n")))
                .with_context(|| format!("Failed to write {}", out_path))?;
            eprintln!(
                "sync-srns: wrote {} SRNs to {} (+{} new, -{} removed)",
                srns.len(),
                out_path,
                added.len(),
                removed.len()
            );
            if !added.is_empty() {
                eprintln!(
                    "  new SRNs: {}",
                    added
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if !removed.is_empty() {
                eprintln!(
                    "  removed SRNs: {}",
                    removed
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Ok(())
        }
        Some("sync-gtins") => {
            // Refresh the customer GTIN worklist from the eudamed2firstbase_GTIN
            // Google Sheet tab. Usage: cargo run sync-gtins [outfile] (default
            // gtins_sheet.txt). The nightly `check --gtin-file gtins_sheet.txt`
            // then covers newly added GTINs. Same safety as sync-srns: on any
            // sheet-read error or a zero-GTIN result the existing file is left
            // untouched (a transient API hiccup must never wipe the worklist) and
            // the command exits non-zero.
            let out_path = args
                .get(2)
                .filter(|s| !s.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| "gtins_sheet.txt".to_string());

            let config_path = std::path::Path::new("config.toml");
            let config_path = if config_path.exists() {
                config_path.to_path_buf()
            } else {
                download::app_data_dir().join("config.toml")
            };
            let cfg = config::load_config(&config_path)?;

            let gtins = match sheet::fetch_gtins(&cfg) {
                Ok(g) if !g.is_empty() => g,
                Ok(_) => {
                    eprintln!(
                        "sync-gtins: sheet returned 0 valid GTINs — keeping existing {} unchanged.",
                        out_path
                    );
                    std::process::exit(2);
                }
                Err(e) => {
                    eprintln!(
                        "sync-gtins: ERROR reading sheet ({e}) — keeping existing {} unchanged.",
                        out_path
                    );
                    std::process::exit(2);
                }
            };

            let old: Vec<String> = std::fs::read_to_string(&out_path)
                .map(|s| s.split_whitespace().map(|x| x.to_string()).collect())
                .unwrap_or_default();
            let old_set: std::collections::HashSet<&String> = old.iter().collect();
            let new_set: std::collections::HashSet<&String> = gtins.iter().collect();
            let added: Vec<&String> = gtins.iter().filter(|g| !old_set.contains(*g)).collect();
            let removed: Vec<&String> = old.iter().filter(|g| !new_set.contains(*g)).collect();

            std::fs::write(&out_path, format!("{}\n", gtins.join("\n")))
                .with_context(|| format!("Failed to write {}", out_path))?;
            eprintln!(
                "sync-gtins: wrote {} GTINs to {} (+{} new, -{} removed)",
                gtins.len(),
                out_path,
                added.len(),
                removed.len()
            );
            if !added.is_empty() {
                eprintln!(
                    "  new GTINs: {}",
                    added
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if !removed.is_empty() {
                eprintln!(
                    "  removed GTINs: {}",
                    removed
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Ok(())
        }
        Some("sync-actors") => {
            // Refresh the EUDAMED actor registry (SRN -> manufacturer/AR name +
            // country/address) into the `actors` table. Re-runnable (upsert).
            // Usage: cargo run sync-actors [--rate-ms N]   (default 1050 ms/req)
            // Linked to devices via actors.srn = listing_cache.srn.
            let rate_ms: u64 = args
                .iter()
                .position(|a| a == "--rate-ms")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(2000); // ~30/min — the /eos sustained ceiling; caps even
                                  // fast empty-country probes so we never exceed budget and 429.
                                  // threads=1 by default: /eos tolerates only ~30/min sustained, and ANY
                                  // multi-thread burst eventually trips a 429 → 60s Retry-After lockstep
                                  // (all threads sleep in sync, throughput collapses to ~5/min). A single
                                  // thread self-limits via page latency, can't lockstep (nothing to sync
                                  // with — a lone 429 is just absorbed), and with per-country pagination
                                  // keeping pages fast (~2s) it sustains ~30/min — which is the /eos
                                  // ceiling anyway, so 1 thread is both the safest AND the fastest choice.
            let threads: usize = args
                .iter()
                .position(|a| a == "--threads")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);

            let db_path = download::app_data_dir().join("db/version_tracking.db");
            let conn = version_db::open_db(&db_path)?;
            match actors::sync_actors(conn, rate_ms, threads) {
                Ok((fetched, total)) => {
                    println!("sync-actors: {} of {} actors synced", fetched, total);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("sync-actors: ERROR {e}");
                    std::process::exit(1);
                }
            }
        }
        Some("check") => {
            // Check SRNs for updates, download changed, convert, and push to Firstbase
            // Usage: cargo run check /tmp/srn_update [--threads N]
            // The SRN worklist file is OPTIONAL: it's the first positional
            // (non-flag) argument. Omit it to run a GTIN-ONLY check (then
            // --gtin-file is required) — the customer GTIN worklist is pushed on
            // its own, skipping the ~30-min SRN listing pass entirely.
            let srn_file: Option<&String> = args.get(2).filter(|a| !a.starts_with("--"));
            let threads: Option<usize> = args
                .iter()
                .position(|a| a == "--threads")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok());

            // Optional customer GTIN worklist (--gtin-file <path>): downloaded in a
            // SECOND, SEQUENTIAL pass after the SRN pass. EUDAMED's ~60-req/60-s
            // budget is shared per-IP across ALL device endpoints (listing + detail
            // + basic), so the SRN and GTIN passes must NOT run concurrently —
            // each run_download paces itself under the ceiling, and running them
            // back-to-back keeps the aggregate under it too. One GTIN per line,
            // '#' comments tolerated.
            let gtins: Vec<String> = args
                .iter()
                .position(|a| a == "--gtin-file")
                .and_then(|i| args.get(i + 1))
                .map(|path| {
                    std::fs::read_to_string(path)
                        .map(|s| {
                            s.lines()
                                .map(|l| l.trim().to_string())
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();

            let srns: Vec<String> = match srn_file {
                Some(f) => std::fs::read_to_string(f)?
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                None => Vec::new(),
            };
            if srns.is_empty() && gtins.is_empty() {
                match srn_file {
                    Some(f) => eprintln!("No SRNs found in {}", f),
                    None => {
                        eprintln!(
                            "Usage: eudamed2firstbase check <srn_file> [--gtin-file <gtins>] [--threads N]"
                        );
                        eprintln!(
                            "       eudamed2firstbase check --gtin-file <gtins> [--threads N]   (GTIN-only, no SRNs)"
                        );
                        eprintln!(
                            "  Checks SRNs and/or a GTIN worklist for updates, downloads changed,"
                        );
                        eprintln!("  converts to firstbase JSON, and pushes to GS1 Firstbase API.");
                    }
                }
                std::process::exit(1);
            }

            // Persistent "pending push" list: UUIDs converted this env but NOT yet
            // confirmed delivered to GS1. `check` unions this into every push scope;
            // a successful transport push clears the file, a FAILED transport push
            // (503 / token / network) leaves it — so the NEXT nightly `check`
            // automatically re-pushes the stranded devices. (Root cause it fixes:
            // the convert step indexes udi_versions BEFORE the push, so once a device
            // is converted the version-check no longer flags it as changed; without
            // this list a push that failed after convert would strand those devices
            // forever.) `check --push-only` re-pushes the same list immediately,
            // without waiting for the next nightly. NOTE: only TRANSPORT failures are
            // auto-retried; per-item validation rejects (097.xxx) return Ok and are
            // NOT re-pushed nightly (they are data problems, not outages).
            let pending_uuids_file = download::app_data_dir()
                .join("log")
                .join("pending_push_uuids.txt");
            let read_uuid_set = |path: &std::path::Path| -> std::collections::HashSet<String> {
                std::fs::read_to_string(path)
                    .map(|s| {
                        s.lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect()
                    })
                    .unwrap_or_default()
            };

            if args.iter().any(|a| a == "--push-only") {
                // Retry-only path: skip listing/download/convert entirely and
                // re-push exactly the pending list. Use after a transient GS1 outage
                // (e.g. token 503) to deliver now instead of waiting for 01:00.
                eprintln!("=== check --push-only: re-pushing pending (undelivered) devices ===");
                let uuids = read_uuid_set(&pending_uuids_file);
                if uuids.is_empty() {
                    eprintln!(
                        "No pending push list at {} (or empty). Nothing to retry.",
                        pending_uuids_file.display()
                    );
                    return Ok(());
                }
                eprintln!(
                    "Loaded {} pending UUID(s) to re-push from {}",
                    uuids.len(),
                    pending_uuids_file.display()
                );

                let config_path = download::app_data_dir().join("config.toml");
                let config_path = if config_path.exists() {
                    config_path
                } else {
                    std::path::PathBuf::from("config.toml")
                };
                let fb_config = config::load_config(&config_path)?;
                let pushed_ok = push_changed_to_firstbase(&fb_config, &uuids, &srns, &gtins)?;
                if pushed_ok {
                    let _ = std::fs::remove_file(&pending_uuids_file);
                    eprintln!("Cleared pending push list (delivered to GS1).");
                } else {
                    eprintln!(
                        "Push did not reach GS1 — pending list kept at {} for the next retry/nightly.",
                        pending_uuids_file.display()
                    );
                }
                return Ok(());
            }

            // Devices still owed from a prior failed push (carried into this run's
            // push scope below).
            let pending_prev = read_uuid_set(&pending_uuids_file);

            match srn_file {
                Some(f) => eprintln!("=== Check {} SRNs from {} ===", srns.len(), f),
                None => eprintln!(
                    "=== GTIN-only check: {} GTIN(s), no SRN worklist ===",
                    gtins.len()
                ),
            }

            // Step 1: Download (with version check). Skip the SRN listing pass
            // entirely when there is no SRN worklist (GTIN-only) — start from an
            // empty result and let the GTIN pass below fill it.
            let progress = download::StderrProgress;
            let mut result = if srns.is_empty() {
                download::DownloadResult::default()
            } else {
                let mut dl_config = download::DownloadConfig {
                    srns: srns.clone(),
                    limit: None,
                    ..Default::default()
                };
                if let Some(t) = threads {
                    dl_config.parallel_threads = t;
                    dl_config.detail_threads = t;
                    dl_config.listing_threads = t;
                }
                download::run_download(&dl_config, &progress)?
            };

            // Second pass: the GTIN worklist, downloaded SEQUENTIALLY after the SRN
            // pass (never concurrently — the per-IP rate budget is shared; see the
            // --gtin-file note above). Each GTIN resolves via the primaryDi filter,
            // is written to listing_cache with its real manufacturerSrn, and yields
            // the same (uuid, version, budi) tuples → merged into the SRN result so
            // the convert + push steps treat SRN- and GTIN-sourced devices
            // uniformly. Devices already covered by an SRN are de-duplicated.
            if !gtins.is_empty() {
                eprintln!(
                    "\n=== Second pass: checking {} GTIN(s) from --gtin-file (sequential) ===",
                    gtins.len()
                );
                let mut gcfg = download::DownloadConfig {
                    gtins: gtins.clone(),
                    limit: None,
                    ..Default::default()
                };
                if let Some(t) = threads {
                    gcfg.parallel_threads = t;
                    gcfg.detail_threads = t;
                    gcfg.listing_threads = t;
                }
                let gresult = download::run_download(&gcfg, &progress)?;
                let mut seen: std::collections::HashSet<String> =
                    result.need_download.iter().cloned().collect();
                for u in gresult.need_download {
                    if seen.insert(u.clone()) {
                        result.need_download.push(u);
                    }
                }
                let mut seen_v: std::collections::HashSet<String> = result
                    .uuid_versions
                    .iter()
                    .map(|(u, ..)| u.clone())
                    .collect();
                for tup in gresult.uuid_versions {
                    if seen_v.insert(tup.0.clone()) {
                        result.uuid_versions.push(tup);
                    }
                }
            }

            // Devices owed from a prior failed push: keep only those whose converted
            // firstbase_json/<uuid>.json still exists on disk (an already-accepted one
            // was moved to processed/ and is no longer owed).
            let fb_output_dir = download::app_data_dir().join("firstbase_json");
            let owed: std::collections::HashSet<String> = pending_prev
                .iter()
                .filter(|u| fb_output_dir.join(format!("{}.json", u)).exists())
                .cloned()
                .collect();
            if !owed.is_empty() {
                eprintln!(
                    "{} device(s) still owed from a prior failed push — will re-push this run.",
                    owed.len()
                );
            }

            if result.need_download.is_empty() && owed.is_empty() {
                eprintln!(
                    "\nNo updates found. All {} devices unchanged.",
                    result.uuid_versions.len()
                );
                // A stale pending file (its devices gone from disk) is cleared so it
                // does not linger; nothing to deliver.
                if !pending_prev.is_empty() {
                    let _ = std::fs::remove_file(&pending_uuids_file);
                }
                return Ok(());
            }
            if result.need_download.is_empty() {
                eprintln!(
                    "\nNo new/changed devices; re-pushing {} owed device(s) only.",
                    owed.len()
                );
            } else {
                eprintln!(
                    "\n{} new/changed devices (of {} total)",
                    result.need_download.len(),
                    result.uuid_versions.len()
                );
            }

            // Step 2: Convert (reuse existing firstbase pipeline)
            eprintln!("\n=== Converting to firstbase JSON ===");
            let data_dir = download::app_data_dir().join(download::DEFAULT_DATA_DIR);
            let detail_dir = data_dir.join("detail");
            let basic_dir = data_dir.join("basic");

            let config_path = download::app_data_dir().join("config.toml");
            let config_path = if config_path.exists() {
                config_path
            } else {
                std::path::PathBuf::from("config.toml")
            };
            let fb_config = config::load_config(&config_path)?;

            let basic_udi_cache = load_basic_udi_cache(&basic_dir);
            eprintln!(
                "Loaded {} Basic UDI-DI records from cache",
                basic_udi_cache.len()
            );

            let db_path = download::app_data_dir()
                .join("db")
                .join("version_tracking.db");
            let conn = version_db::open_db(&db_path)?;
            let output_dir = download::app_data_dir().join("firstbase_json");
            let _ = std::fs::create_dir_all(&output_dir);

            let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
            let total = result.need_download.len();
            let converted = std::sync::atomic::AtomicUsize::new(0);
            let reported = std::sync::atomic::AtomicUsize::new(0);
            let conn = std::sync::Mutex::new(conn);
            result.need_download.par_iter().for_each(|uuid| {
                let detail_path = detail_dir.join(format!("{}.json", uuid));
                if !detail_path.exists() {
                    return;
                }
                let json_content = match std::fs::read_to_string(&detail_path) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let device: api_detail::ApiDeviceDetail = match serde_json::from_str(&json_content)
                {
                    Ok(d) => d,
                    Err(_) => return,
                };
                let basic_udi = basic_udi_cache.get(uuid);
                let doc = transform_detail::transform_detail_document(
                    &device, &fb_config, basic_udi, uuid,
                );
                let draft_doc = firstbase::DraftItemDocument { draft_item: doc };
                let out = serde_json::to_string_pretty(&draft_doc)
                    .expect("Failed to serialize firstbase doc");
                let out_path = output_dir.join(format!("{}.json", uuid));
                let _ = std::fs::write(&out_path, &out);

                let mut version_rec = version_db::extract_detail_versions(&json_content);
                version_rec.last_synced = Some(now_str.clone());
                // Merge the Basic UDI-DI versionNumber from the basic JSON — the
                // detail JSON carries no BUDI version, so without this the upsert
                // overwrites udi_versions.budi_version with NULL every run. The next
                // check would then see (DB budi=None, listing budi=Some) in
                // filter_unchanged → "new BUDI data" → re-download + re-push the
                // ENTIRE worklist (a self-perpetuating mass re-push). Mirrors the
                // merge in process_eudamed_json_dir.
                let basic_path = basic_dir.join(format!("{}.json", uuid));
                if let Ok(budi_json) = std::fs::read_to_string(&basic_path) {
                    version_db::merge_budi_versions(&mut version_rec, &budi_json);
                }
                if let Ok(c) = conn.lock() {
                    let _ = version_db::upsert_version(&c, &version_rec);
                }

                let done = converted.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let prev = reported.load(std::sync::atomic::Ordering::Relaxed);
                if done >= prev + 1000
                    && reported
                        .compare_exchange(
                            prev,
                            done,
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed,
                        )
                        .is_ok()
                {
                    eprintln!("  Converted {}/{}...", done, total);
                }
            });
            let converted = converted.load(std::sync::atomic::Ordering::Relaxed);
            eprintln!("Converted {} devices to firstbase_json/", converted);

            if converted == 0 && owed.is_empty() {
                eprintln!("Nothing to push.");
                return Ok(());
            }

            // Step 3: push scope = this run's new/changed UUIDs ∪ devices still owed
            // from a prior failed push. Scoped to exactly these files (never the whole
            // firstbase_json/ backlog, so unrelated leftover rejects — HIBC/IFA,
            // 097.095-blocked — are not dragged in). Record the scope as the pending
            // list BEFORE pushing (survives a crash/failure), then clear it ONLY if
            // the push actually reached GS1; a transport failure keeps it so the next
            // nightly `check` (or `check --push-only`) re-pushes automatically.
            let mut push_scope: std::collections::HashSet<String> =
                result.need_download.iter().cloned().collect();
            push_scope.extend(owed.iter().cloned());
            if let Some(parent) = pending_uuids_file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let uuid_list = push_scope.iter().cloned().collect::<Vec<_>>().join("\n");
            if let Err(e) = std::fs::write(&pending_uuids_file, uuid_list) {
                eprintln!(
                    "Warning: could not record pending push list at {}: {}",
                    pending_uuids_file.display(),
                    e
                );
            }

            // GS1 report body lists the pushed SRNs. GTIN-worklist devices resolve
            // to their real manufacturer SRN in listing_cache, which is NOT in the
            // SRN worklist file — add those so an accepted customer GTIN device shows
            // up under "SRNs ok" instead of silently missing. (The report CSVs
            // already carry per-device SRN via the listing_cache join.)
            let mut report_srns: Vec<String> = srns.clone();
            if !gtins.is_empty() {
                if let Ok(c) = conn.lock() {
                    if let Ok(mut stmt) = c.prepare("SELECT srn FROM listing_cache WHERE uuid = ?1")
                    {
                        let mut extra: Vec<String> = Vec::new();
                        let existing: std::collections::HashSet<&String> = srns.iter().collect();
                        for uuid in &push_scope {
                            if let Ok(srn) = stmt.query_row([uuid], |r| r.get::<_, String>(0)) {
                                if !srn.is_empty()
                                    && !existing.contains(&srn)
                                    && !extra.contains(&srn)
                                {
                                    extra.push(srn);
                                }
                            }
                        }
                        extra.sort();
                        report_srns.extend(extra);
                    }
                }
            }
            let pushed_ok =
                push_changed_to_firstbase(&fb_config, &push_scope, &report_srns, &gtins)?;
            if pushed_ok {
                let _ = std::fs::remove_file(&pending_uuids_file);
            } else {
                eprintln!(
                    "Push did not reach GS1 — {} device(s) kept in the pending list ({}); \
                     the next nightly check (or `check --push-only`) will re-push them.",
                    push_scope.len(),
                    pending_uuids_file.display()
                );
            }
            Ok(())
        }
        Some("download") => {
            // Download from EUDAMED API (replaces download.sh)
            let (srns, mut gtins, limit, threads) = parse_download_args(&args[2..]);
            // --gtin-file <path>: one GTIN (UDI-DI primary code) per line, # comments ok.
            if let Some(pos) = args.iter().position(|a| a == "--gtin-file") {
                let file = args
                    .get(pos + 1)
                    .ok_or_else(|| anyhow::anyhow!("--gtin-file requires a path argument"))?;
                gtins.extend(
                    std::fs::read_to_string(file)?
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|s| !s.is_empty() && !s.starts_with('#')),
                );
            }
            if srns.is_empty() && gtins.is_empty() && limit.is_none() {
                eprintln!(
                    "Usage: eudamed2firstbase download [--N] [--srn <SRN> ...] [--gtin <GTIN> ...] [--gtin-file <file>] [--threads N] [--convert]"
                );
                eprintln!("  --N                Number of products per SRN (e.g. --10, --100)");
                eprintln!("  --srn <SRN> ...    Filter by manufacturer/AR SRN(s)");
                eprintln!("  --gtin <GTIN> ...  Fetch specific device(s) by UDI-DI primary code (GTIN); takes precedence over --srn");
                eprintln!("  --gtin-file <file> Read GTINs from a file (one per line)");
                eprintln!(
                    "  --threads N        Parallel threads for listings/lookups and downloads"
                );
                std::process::exit(1);
            }
            let mut dl_config = download::DownloadConfig {
                srns,
                gtins,
                limit,
                ..Default::default()
            };
            if let Some(t) = threads {
                dl_config.parallel_threads = t;
                dl_config.detail_threads = t;
                dl_config.listing_threads = t;
            }
            let progress = download::StderrProgress;
            let result = download::run_download(&dl_config, &progress)?;
            if result.uuid_versions.is_empty() {
                eprintln!("No devices found.");
            } else {
                eprintln!(
                    "\nDone: {} UUIDs, {} new/changed, {} unchanged, {} detail downloaded, {} basic downloaded",
                    result.uuid_versions.len(),
                    result.need_download.len(),
                    result.unchanged_skipped,
                    result.detail_downloaded,
                    result.basic_downloaded,
                );
            }
            // Auto-convert if --convert flag present.
            // process_eudamed_json_dir resolves all paths (input detail dir, output
            // firstbase_json/, db/, basic cache) relative to the CWD, but the download
            // above wrote under app_data_dir(). Run the convert from app_data_dir() so
            // input/output/db all line up with where the data lives (and where the
            // push step later reads firstbase_json/).
            if args.iter().any(|a| a == "--convert") {
                eprintln!("\n=== Converting to firstbase JSON ===");
                std::env::set_current_dir(download::app_data_dir())
                    .context("Failed to chdir to app data dir for convert")?;
                process_eudamed_json_dir(Path::new("eudamed_json/detail"), &config)?;
            }
            Ok(())
        }
        Some("ndjson") => {
            // Process NDJSON file(s) from ndjson/ directory (listing format)
            let input_dir = args.get(2).map(|s| s.as_str()).unwrap_or("ndjson");
            process_ndjson(Path::new(input_dir), &config)
        }
        Some("firstbase") | Some("eudamed2firstbase") | Some("eudamed_json") => {
            // Convert EUDAMED JSON → GS1 Firstbase JSON
            let input_dir = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("eudamed_json/detail");
            process_eudamed_json_dir(Path::new(input_dir), &config)
        }
        Some("swissdamed") => {
            // Convert EUDAMED JSON → Swissdamed JSON (almost 1:1 mapping)
            let detail_dir = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("eudamed_json/detail");
            let basic_dir = args
                .get(3)
                .map(|s| s.as_str())
                .unwrap_or("eudamed_json/basic");
            process_swissdamed(Path::new(detail_dir), Path::new(basic_dir))
        }
        Some("mailto") => {
            // Send one or more files as email attachments via Gmail API.
            // Credentials default to [gmail] in config.toml; --p12 overrides the key path.
            // Usage: cargo run mailto <file> [<file2> ...] --to <email> [--from <email>]
            //        [--subject <text>] [--body <text>] [--p12 <key>] [--max-bytes <N>]
            // Files are attached in the order given. With --max-bytes, files whose
            // cumulative raw size would exceed N are skipped (the first file is always
            // kept), so listing the small error report first and the large log last
            // means an oversized log is dropped and only the report is sent.
            let mut files: Vec<String> = Vec::new();
            let mut to = None;
            let mut subject = None;
            let mut from: Option<String> = None;
            let mut body: Option<String> = None;
            let mut max_bytes: Option<u64> = None;
            // Default p12 path from config; CLI --p12 flag overrides.
            let mut p12 = config.gmail.p12_key.clone();
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--to" => {
                        i += 1;
                        to = args.get(i).cloned();
                    }
                    "--subject" => {
                        i += 1;
                        subject = args.get(i).cloned();
                    }
                    "--from" => {
                        i += 1;
                        from = args.get(i).cloned();
                    }
                    "--body" => {
                        i += 1;
                        body = args.get(i).cloned();
                    }
                    "--p12" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            p12 = v.clone();
                        }
                    }
                    "--max-bytes" => {
                        i += 1;
                        max_bytes = args.get(i).and_then(|v| v.parse::<u64>().ok());
                    }
                    other if !other.starts_with("--") => {
                        files.push(args[i].clone());
                    }
                    _ => {}
                }
                i += 1;
            }
            if files.is_empty() {
                eprintln!("Usage: eudamed2firstbase mailto <file> [<file2> ...] --to <email> [--from <email>] [--subject <text>] [--body <text>] [--p12 <key>] [--max-bytes <N>]");
                std::process::exit(1);
            }
            let to = to.unwrap_or_else(|| {
                eprintln!("--to <email> is required");
                std::process::exit(1);
            });
            let from = from.unwrap_or_else(|| {
                eprintln!("--from <email> is required");
                std::process::exit(1);
            });
            let subject = subject.unwrap_or_else(|| {
                format!(
                    "eudamed2firstbase: {}",
                    std::path::Path::new(&files[0])
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&files[0])
                )
            });
            // No body by default (the report speaks for itself); --body overrides.
            let body = body.unwrap_or_default();

            // Size guard: skip files that would push the cumulative raw size over
            // --max-bytes; always keep the first (highest-priority) file.
            let send_files: Vec<String> = if let Some(limit) = max_bytes {
                let mut total: u64 = 0;
                let mut kept: Vec<String> = Vec::new();
                for f in &files {
                    let sz = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                    if kept.is_empty() || total + sz <= limit {
                        total += sz;
                        kept.push(f.clone());
                    } else {
                        eprintln!(
                            "Skipping attachment {} ({} bytes) — over --max-bytes {} limit",
                            f, sz, limit
                        );
                    }
                }
                kept
            } else {
                files.clone()
            };

            if p12.is_empty() {
                anyhow::bail!(
                    "Gmail p12 key path not configured. \
                     Add `p12_key = \"/path/to/key.p12\"` under [gmail] in config.toml \
                     (see config.sample.toml), or pass --p12 <path> on the command line."
                );
            }
            let service_email = config.gmail.service_email.clone();
            if service_email.is_empty() {
                anyhow::bail!(
                    "Gmail service account email not configured. \
                     Add `service_email = \"name@project.iam.gserviceaccount.com\"` \
                     under [gmail] in config.toml (see config.sample.toml)."
                );
            }
            mail::send_email_with_attachments(
                &p12,
                &service_email,
                &from,
                &to,
                &subject,
                &body,
                &send_files,
            )?;
            Ok(())
        }
        Some("whatsapp") => {
            // Send a file (PDF, HTML, image, …) via WhatsApp using Baileys.
            // Usage: cargo run whatsapp <file> --group <jid> [--caption <text>]
            //        cargo run whatsapp --list-groups
            if args.get(2).map(|s| s.as_str()) == Some("--list-groups")
                || args.get(2).map(|s| s.as_str()) == Some("--pair")
            {
                whatsapp::list_groups_streaming(|ev| match ev {
                    whatsapp::WhatsappEvent::Line(l) => println!("{}", l),
                    whatsapp::WhatsappEvent::Qr(_) => {
                        // QR is also printed as ASCII by qrcode-terminal — ignore sentinel.
                    }
                })?;
                return Ok(());
            }
            if args.get(2).map(|s| s.as_str()) == Some("--list-contacts") {
                let filter = args.get(3).map(|s| s.as_str());
                whatsapp::list_contacts_streaming(filter, |ev| match ev {
                    whatsapp::WhatsappEvent::Line(l) => println!("{}", l),
                    whatsapp::WhatsappEvent::Qr(_) => {}
                })?;
                return Ok(());
            }

            let mut file = None;
            let mut jid = None;
            let mut caption: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--group" | "--jid" | "--to" => {
                        i += 1;
                        jid = args.get(i).cloned();
                    }
                    "--caption" => {
                        i += 1;
                        caption = args.get(i).cloned();
                    }
                    _ if file.is_none() => {
                        file = Some(args[i].clone());
                    }
                    _ => {}
                }
                i += 1;
            }
            let file = file.unwrap_or_else(|| {
                eprintln!(
                    "Usage: eudamed2firstbase whatsapp <file> --group <jid> [--caption <text>]"
                );
                eprintln!("       eudamed2firstbase whatsapp --list-groups");
                std::process::exit(1);
            });
            let jid = jid.unwrap_or_else(|| {
                eprintln!("--group <jid> is required (e.g. 120363401234567890@g.us)");
                std::process::exit(1);
            });
            let caption = caption.unwrap_or_else(|| {
                std::path::Path::new(&file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&file)
                    .to_string()
            });
            let out = whatsapp::send(&jid, &file, &caption)?;
            print!("{}", out);
            Ok(())
        }
        Some("count") => {
            // Count devices per SRN from EUDAMED API (parallel)
            // Usage: cargo run count SRN1 SRN2 ...
            //    or: cargo run count --file srns.txt
            //    or: cargo run count --xlsx file.xlsx [col]
            let srns: Vec<String> = if args.get(2).map(|s| s.as_str()) == Some("--file") {
                let path = args.get(3).expect("Usage: count --file <file>");
                std::fs::read_to_string(path)?
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|s| !s.is_empty() && s.contains("-MF-"))
                    .collect()
            } else if args.get(2).map(|s| s.as_str()) == Some("--xlsx") {
                let path = args.get(3).expect("Usage: count --xlsx <file.xlsx> [col]");
                let col: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4); // default col D=4
                count_srns_xlsx(path, col)?
            } else {
                args[2..].iter().map(|s| s.to_string()).collect()
            };
            if srns.is_empty() {
                anyhow::bail!("No SRNs provided");
            }
            let unique: Vec<String> = {
                let mut s: Vec<String> = srns.clone();
                s.sort();
                s.dedup();
                s
            };
            eprintln!("Querying {} unique SRNs (10 parallel)...", unique.len());

            let agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(std::time::Duration::from_secs(10)))
                .build()
                .new_agent();

            let results: Vec<(String, i64)> = unique.par_iter().map(|srn| {
                let url = format!(
                    "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData?page=0&pageSize=1&srn={}&iso2Code=en&languageIso2Code=en",
                    srn
                );
                let mut count = -1i64;
                for attempt in 1..=3 {
                    match agent.get(&url).call() {
                        Ok(mut resp) => {
                            let body = resp.body_mut().read_to_string().unwrap_or_default();
                            count = serde_json::from_str::<serde_json::Value>(&body)
                                .ok()
                                .and_then(|v| v.get("totalElements")?.as_i64())
                                .unwrap_or(-1);
                            break;
                        }
                        Err(_) if attempt < 3 => {
                            std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
                        }
                        Err(_) => {}
                    }
                }
                (srn.clone(), count)
            }).collect();

            let result_map: HashMap<String, i64> = results.into_iter().collect();

            // If --xlsx mode, write back to the file
            if args.get(2).map(|s| s.as_str()) == Some("--xlsx") {
                let path = args
                    .get(3)
                    .ok_or_else(|| anyhow::anyhow!("--xlsx requires a file path argument"))?;
                let col: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
                write_counts_xlsx(path, col, &result_map)?;
                let total: i64 = result_map.values().filter(|&&v| v >= 0).sum();
                eprintln!(
                    "Done. {} SRNs, {} total devices. Written to {}",
                    result_map.len(),
                    total,
                    path
                );
            } else {
                // Print TSV to stdout
                for (srn, count) in &result_map {
                    println!(
                        "{}\t{}",
                        srn,
                        if *count >= 0 {
                            count.to_string()
                        } else {
                            "ERROR".to_string()
                        }
                    );
                }
                let total: i64 = result_map.values().filter(|&&v| v >= 0).sum();
                eprintln!("Done. {} SRNs, {} total devices.", result_map.len(), total);
            }
            Ok(())
        }
        Some("regenerate") => {
            // Re-run transform_detail + DraftItemDocument wrap over every detail file
            // in the app-data detail dir, writing to firstbase_json in parallel.
            // Unlike `check`, this ignores version tracking — every file is rewritten.
            let data_dir = download::app_data_dir();
            let config_path = data_dir.join("config.toml");
            let fb_config = if config_path.exists() {
                config::load_config(&config_path)?
            } else {
                config
            };

            // Optional `--uuid-file <path>`: reconvert only the listed UUIDs
            // (convert-only, no push) — a scoped counterpart to the full rewrite.
            let uuid_filter: Option<std::collections::HashSet<String>> = args
                .iter()
                .position(|a| a == "--uuid-file")
                .and_then(|i| args.get(i + 1))
                .map(|path| {
                    std::fs::read_to_string(path)
                        .map(|s| {
                            s.lines()
                                .map(|l| l.trim().to_string())
                                .filter(|l| !l.is_empty())
                                .collect()
                        })
                        .unwrap_or_default()
                });

            match &uuid_filter {
                Some(set) => eprintln!(
                    "Regenerating {} firstbase JSON file(s) (scoped) in parallel...",
                    set.len()
                ),
                None => eprintln!("Regenerating all firstbase JSON files in parallel..."),
            }
            let (converted, errors) =
                reconvert_uuids_from_detail(uuid_filter.as_ref(), &data_dir, &fb_config);
            eprintln!("Done: {} regenerated, {} errors.", converted, errors);
            Ok(())
        }
        Some("repush-srn") => {
            // Restore firstbase JSON files for the given SRN(s) from processed/
            // back to firstbase_json/, then push via gui::push_to_firstbase.
            // Bypasses the udi_versions unchanged-skip — lets you re-send a set
            // of devices to Firstbase when the regular pipeline considers them
            // already synced.
            //
            // Usage:
            //   cargo run repush-srn DE-MF-000005190 [SRN2 ...]
            //   cargo run repush-srn --file srns.txt
            //   cargo run repush-srn --reconvert DE-MF-000005190
            //
            // With --reconvert, the SRN's UUIDs are first re-converted from
            // eudamed_json/detail/<uuid>.json (using the latest converter logic),
            // bypassing udi_versions; the resulting fresh JSONs are written to
            // firstbase_json/ and pushed. Use this after a converter change
            // (e.g. v1.0.43 added DescriptionShort) when the EUDAMED detail
            // JSON itself hasn't changed but you want the new GS1 fields live
            // in Firstbase.
            //
            // Env: FIRSTBASE_EMAIL, FIRSTBASE_PASSWORD, FIRSTBASE_PUBLISH_GLN
            //      (publish_gln falls back to config.toml's [provider].publish_gln)
            eprintln!(
                "eudamed2firstbase v{} — repush-srn",
                env!("CARGO_PKG_VERSION")
            );

            // --force-reload (CLI mirror of GUI Mode 6 / StaleCleaner): force-
            // refetch detail + Basic UDI-DI fresh from EUDAMED before converting,
            // healing stale/incomplete cached files. Implies --reconvert.
            let force_reload = args.iter().any(|a| a == "--force-reload");
            let reconvert = force_reload || args.iter().any(|a| a == "--reconvert");
            let srns: Vec<String> = if let Some(pos) = args.iter().position(|a| a == "--file") {
                let file = args
                    .get(pos + 1)
                    .ok_or_else(|| anyhow::anyhow!("--file requires a path argument"))?;
                std::fs::read_to_string(file)?
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|s| !s.is_empty() && !s.starts_with('#'))
                    .collect()
            } else {
                args.iter()
                    .skip(2)
                    .filter(|a| !a.starts_with("--"))
                    .cloned()
                    .collect()
            };

            if srns.is_empty() {
                eprintln!("Usage: eudamed2firstbase repush-srn [--reconvert|--force-reload] <SRN1> [SRN2 ...]");
                eprintln!("   or: eudamed2firstbase repush-srn [--reconvert|--force-reload] --file <srns.txt>");
                eprintln!("   --force-reload: refetch detail + Basic UDI-DI fresh from EUDAMED (Mode 6 / StaleCleaner), implies --reconvert");
                std::process::exit(1);
            }
            eprintln!(
                "SRNs: {}{}",
                srns.join(", "),
                if reconvert { "  (--reconvert)" } else { "" }
            );

            let data_dir = download::app_data_dir();
            let firstbase_dir = data_dir.join("firstbase_json");
            let processed_dir = firstbase_dir.join("processed");
            let db_path = data_dir.join("db").join("version_tracking.db");
            if !db_path.exists() {
                eprintln!("No DB at {}. Run Download first.", db_path.display());
                std::process::exit(1);
            }
            std::fs::create_dir_all(&firstbase_dir)?;

            let conn = version_db::open_db(&db_path)?;
            let placeholders = srns.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT uuid FROM listing_cache WHERE srn IN ({})",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::ToSql> =
                srns.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let uuids: std::collections::HashSet<String> = stmt
                .query_map(&params[..], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);

            // --uuid-file: restrict the push scope to exactly these UUIDs (a subset
            // of the SRNs' devices) — e.g. to re-push only the previously-rejected
            // handful instead of every device of the SRN. The SRNs are still used
            // for the report/log; the scope becomes (SRN UUIDs ∩ file UUIDs).
            let uuids: std::collections::HashSet<String> =
                if let Some(pos) = args.iter().position(|a| a == "--uuid-file") {
                    let file = args
                        .get(pos + 1)
                        .ok_or_else(|| anyhow::anyhow!("--uuid-file requires a path argument"))?;
                    let want: std::collections::HashSet<String> = std::fs::read_to_string(file)?
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|s| !s.is_empty() && !s.starts_with('#'))
                        .collect();
                    let scoped: std::collections::HashSet<String> =
                        uuids.intersection(&want).cloned().collect();
                    eprintln!(
                        "--uuid-file: scope restricted to {} UUID(s) ({} in file)",
                        scoped.len(),
                        want.len()
                    );
                    scoped
                } else {
                    uuids
                };

            if uuids.is_empty() {
                eprintln!("No UUIDs in listing_cache for these SRNs. Run Download first.");
                std::process::exit(1);
            }
            eprintln!(
                "Found {} UUID(s) in listing_cache for {} SRN(s)",
                uuids.len(),
                srns.len()
            );

            // Target environment: FIRSTBASE_ENV=Production switches the CLI push
            // to the GS1 production endpoint; anything else (incl. unset) = Test.
            let fb_env = match std::env::var("FIRSTBASE_ENV").as_deref() {
                Ok("Production") | Ok("production") | Ok("PROD") | Ok("prod") => {
                    gui::FirstbaseEnv::Production
                }
                _ => gui::FirstbaseEnv::Test,
            };
            let env_label = match fb_env {
                gui::FirstbaseEnv::Production => "Production",
                gui::FirstbaseEnv::Test => "Test",
            };

            // Issue #10: skip NO_LONGER devices already ACCEPTED in this env.
            let (uuids, skipped) =
                version_db::filter_skip_no_longer_accepted(&conn, &uuids, env_label);
            if skipped > 0 {
                eprintln!(
                    "Skipping {} NO_LONGER UUID(s) already ACCEPTED in {} (G485 mitigation, Issue #10)",
                    skipped, env_label
                );
            }
            if uuids.is_empty() {
                eprintln!(
                    "Nothing to push: all {} UUID(s) were skipped as NO_LONGER + already ACCEPTED",
                    skipped
                );
                return Ok(());
            }

            // --- Force-reload (optional, Mode 6 / StaleCleaner) ---
            // Refetch detail + Basic UDI-DI fresh from EUDAMED, overwriting any
            // stale/incomplete cache, before the reconvert below reads them.
            if force_reload {
                eprintln!(
                    "Force-reloading detail + Basic UDI-DI for {} UUID(s) from EUDAMED...",
                    uuids.len()
                );
                let stats = force_reload_eudamed(&uuids, &data_dir, &|s: &str| eprintln!("{}", s));
                eprintln!(
                    "Force-reload: {} detail fresh; Basic UDI-DI {} already-complete (skipped), {} refetched of {} attempted",
                    stats.detail_ok, stats.skipped_complete, stats.basic_ok, stats.refetch_attempted
                );
                if stats.basic_missing() > 0 {
                    eprintln!(
                        "  {} of the attempted refetches failed — reasons: {} (404 = no record, kept old file; 429 = throttled)",
                        stats.basic_missing(),
                        stats.breakdown()
                    );
                }
            }

            // --- Reconvert (optional) ---
            // With --reconvert, re-run transform_detail for the matching UUIDs
            // before pushing. Writes fresh firstbase_json/<uuid>.json so the
            // next push picks up the latest converter logic. Bypasses
            // udi_versions cache by going straight from eudamed_json/detail/.
            if reconvert {
                let config_path = data_dir.join("config.toml");
                let fb_config = if config_path.exists() {
                    config::load_config(&config_path)?
                } else {
                    config.clone()
                };
                eprintln!(
                    "Reconverting {} UUID(s) from eudamed_json/detail/...",
                    uuids.len()
                );
                let (rc_converted, rc_errors) =
                    reconvert_uuids_from_detail(Some(&uuids), &data_dir, &fb_config);
                eprintln!(
                    "Reconvert: {} written to firstbase_json/, {} errors",
                    rc_converted, rc_errors
                );
            }

            // Count how many matching UUIDs are already in firstbase_json/ — BEFORE restoring
            let mut already_present = 0usize;
            if let Ok(entries) = std::fs::read_dir(&firstbase_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Some(stem) = path.file_stem() {
                            let stem_s = stem.to_string_lossy();
                            if uuids.contains(stem_s.as_ref()) {
                                already_present += 1;
                            }
                        }
                    }
                }
            }

            // Restore matching files from processed/ to firstbase_json/.
            // With --reconvert, only fall back to processed/ for UUIDs whose
            // fresh reconvert didn't produce a firstbase_json/<uuid>.json
            // (e.g. no detail file on disk for that UUID).
            let mut restored = 0usize;
            if let Ok(entries) = std::fs::read_dir(&processed_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Some(stem) = path.file_stem() {
                            let stem_s = stem.to_string_lossy();
                            if uuids.contains(stem_s.as_ref()) {
                                if let Some(name) = path.file_name() {
                                    let dest = firstbase_dir.join(name);
                                    if reconvert && dest.exists() {
                                        continue;
                                    }
                                    if std::fs::rename(&path, &dest).is_ok() {
                                        restored += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let total = restored + already_present;
            eprintln!(
                "Restored {} from processed/, {} already in firstbase_json/ (total {} to push)",
                restored, already_present, total
            );
            if total == 0 {
                eprintln!(
                    "No matching files found in processed/ or firstbase_json/. Run Download & Convert first, or try `regenerate`."
                );
                std::process::exit(1);
            }

            // --- Push via gui::push_to_firstbase (same path as the GUI + `check`) ---
            let email = std::env::var("FIRSTBASE_EMAIL").unwrap_or_default();
            let password = std::env::var("FIRSTBASE_PASSWORD").unwrap_or_default();
            let publish_gln = match std::env::var("FIRSTBASE_PUBLISH_GLN") {
                Ok(v) if !v.is_empty() => v,
                _ if !config.provider.publish_gln.is_empty() => config.provider.publish_gln.clone(),
                _ => {
                    eprintln!(
                        "Set FIRSTBASE_PUBLISH_GLN or add publish_gln under [provider] in config.toml."
                    );
                    std::process::exit(1);
                }
            };
            if email.is_empty() || password.is_empty() {
                eprintln!("Set FIRSTBASE_EMAIL and FIRSTBASE_PASSWORD to push.");
                std::process::exit(1);
            }

            let settings = gui::Settings {
                firstbase_email: email,
                firstbase_password: password,
                publish_to_gln: publish_gln,
                provider_gln: config.provider.gln.clone(),
                firstbase_env: fb_env,
                ..Default::default()
            };
            eprintln!(
                "Firstbase environment: {} ({})",
                env_label,
                settings.firstbase_env.api_base()
            );
            let log_fn = |msg: &str| {
                eprintln!("{}", msg);
            };
            // SRN-scoped (CLI mirror of Mode 4/5/6): push ONLY this run's UUIDs,
            // not the whole firstbase_json/ backlog of other SRNs.
            match gui::push_to_firstbase(&settings, &log_fn, Some(&uuids)) {
                Ok((accepted, rejected)) => {
                    eprintln!("\nDone: {} accepted, {} rejected.", accepted, rejected);
                    // After a Production push, auto-email the GS1 report (errors-only
                    // CSV + full HTML log) to GS1. Never fail the run on a mail error.
                    if matches!(settings.firstbase_env, gui::FirstbaseEnv::Production) {
                        // repush-srn is SRN-scoped — no GTIN worklist.
                        if let Err(e) =
                            send_gs1_prod_report(&config, accepted, rejected, &srns, &[])
                        {
                            eprintln!("Auto-report to GS1 failed (non-fatal): {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("\nPush failed: {}", e);
                }
            }
            Ok(())
        }
        Some("gs1-report") => {
            // Manually (re)send the GS1 Production push report (errors CSV +
            // devices CSV + HTML log) for the latest Production session. Mirrors
            // the automatic send after a `repush-srn` Production push.
            // Usage: gs1-report [<accepted> <rejected>] [SRN ...] [--file <srns.txt>] [--gtin-file <gtins.txt>]
            //   The SRNs (positional after the two counts, or --file) are listed in
            //   the mail body. --gtin-file adds the separate GTIN-worklist updates
            //   CSV. GS1_REPORT_TO / GS1_REPORT_FROM / GS1_REPORT_DISABLE apply.
            let gtins: Vec<String> = args
                .iter()
                .position(|a| a == "--gtin-file")
                .and_then(|i| args.get(i + 1))
                .map(|path| {
                    std::fs::read_to_string(path)
                        .map(|s| {
                            s.lines()
                                .map(|l| l.trim().to_string())
                                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let accepted = args.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let rejected = args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let srns: Vec<String> = if let Some(pos) = args.iter().position(|a| a == "--file") {
                let file = args
                    .get(pos + 1)
                    .ok_or_else(|| anyhow::anyhow!("--file requires a path argument"))?;
                std::fs::read_to_string(file)?
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|s| !s.is_empty() && !s.starts_with('#'))
                    .collect()
            } else {
                // Positional SRNs after the two count args. Skip flags AND the
                // value that follows a value-taking flag (--gtin-file <path>), so
                // the path is not mistaken for an SRN in the mail body.
                let mut out = Vec::new();
                let mut skip_next = false;
                for a in args.iter().skip(4) {
                    if skip_next {
                        skip_next = false;
                        continue;
                    }
                    if a == "--gtin-file" || a == "--file" {
                        skip_next = true;
                        continue;
                    }
                    if a.starts_with("--") {
                        continue;
                    }
                    out.push(a.clone());
                }
                out
            };
            send_gs1_prod_report(&config, accepted, rejected, &srns, &gtins)?;
            Ok(())
        }
        Some("status") => {
            // Live snapshot of EUDAMED ingest + Firstbase push state.
            // Reads the version DB (WAL mode, safe alongside a running `check`).
            let data_dir = download::app_data_dir();
            let db_path = data_dir.join("db").join("version_tracking.db");
            if !db_path.exists() {
                eprintln!("No DB at {}. Nothing to report yet.", db_path.display());
                return Ok(());
            }
            let conn = version_db::open_db(&db_path)?;

            let q_one = |sql: &str| -> rusqlite::Result<i64> {
                conn.query_row(sql, [], |r| r.get::<_, i64>(0))
            };
            let q_str = |sql: &str| -> rusqlite::Result<String> {
                conn.query_row(sql, [], |r| r.get::<_, String>(0))
            };

            let listing_rows = q_one("SELECT COUNT(*) FROM listing_cache").unwrap_or(0);
            let listing_srns = q_one("SELECT COUNT(DISTINCT srn) FROM listing_cache").unwrap_or(0);
            let listing_latest =
                q_str("SELECT MAX(listed_at) FROM listing_cache").unwrap_or_default();

            let udi_total = q_one("SELECT COUNT(*) FROM udi_versions").unwrap_or(0);
            let udi_hour = q_one(
                "SELECT COUNT(*) FROM udi_versions WHERE last_synced >= datetime('now','-1 hour')",
            )
            .unwrap_or(0);
            let udi_day = q_one(
                "SELECT COUNT(*) FROM udi_versions WHERE last_synced >= datetime('now','-1 day')",
            )
            .unwrap_or(0);
            let udi_latest = q_str("SELECT MAX(last_synced) FROM udi_versions").unwrap_or_default();

            let detail_count = std::fs::read_dir(data_dir.join("eudamed_json/detail"))
                .map(|it| it.count())
                .unwrap_or(0);
            let basic_count = std::fs::read_dir(data_dir.join("eudamed_json/basic"))
                .map(|it| it.count())
                .unwrap_or(0);
            let fb_count = std::fs::read_dir(data_dir.join("firstbase_json"))
                .map(|it| {
                    it.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_file())
                        .count()
                })
                .unwrap_or(0);

            let push_total = q_one("SELECT COUNT(*) FROM push_log").unwrap_or(0);
            let push_accepted =
                q_one("SELECT COUNT(*) FROM push_log WHERE status='ACCEPTED'").unwrap_or(0);
            let push_rejected =
                q_one("SELECT COUNT(*) FROM push_log WHERE status='REJECTED'").unwrap_or(0);
            let has_env_col = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('push_session') WHERE name='firstbase_env'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            let session_sql = if has_env_col {
                "SELECT id, session_ts, COALESCE(firstbase_env,''), total_accepted, total_rejected \
                 FROM push_session ORDER BY id DESC LIMIT 1"
            } else {
                "SELECT id, session_ts, '' AS firstbase_env, total_accepted, total_rejected \
                 FROM push_session ORDER BY id DESC LIMIT 1"
            };
            let last_session = conn
                .query_row(session_sql, [], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })
                .ok();

            println!("=== eudamed2firstbase status ===");
            println!("Data dir: {}", data_dir.display());
            println!();
            println!("[EUDAMED ingest]");
            println!(
                "  listing_cache     {} rows across {} SRNs (latest {})",
                listing_rows, listing_srns, listing_latest
            );
            println!("  detail files      {} on disk", detail_count);
            println!("  basic  files      {} on disk", basic_count);
            println!(
                "  udi_versions      {} rows (last hour: {}, last 24h: {}, latest {})",
                udi_total, udi_hour, udi_day, udi_latest
            );
            println!();
            println!("[Firstbase]");
            println!("  firstbase_json    {} files awaiting push", fb_count);
            println!(
                "  push_log          {} total ({} accepted, {} rejected)",
                push_total, push_accepted, push_rejected
            );
            if let Some((id, at, env, acc, rej)) = last_session {
                println!(
                    "  last push session id={} env={} at {} -> {} accepted, {} rejected",
                    id, env, at, acc, rej
                );
            } else {
                println!("  last push session  (none yet)");
            }
            Ok(())
        }
        Some("scan") => {
            // Fast parallel scan of firstbase JSON files — outputs "filepath\tGTIN" per line
            let input_dir = args.get(2).map(|s| s.as_str()).unwrap_or("firstbase_json");
            scan::scan_dir(Path::new(input_dir))
        }
        Some("xlsx") => {
            // Convert detail NDJSON to XLSX
            let input_file = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("ndjson/eudamed_10k_details.ndjson");
            let basic_udi_cache = load_basic_udi_cache(Path::new(BASIC_UDI_CACHE_DIR));
            if !basic_udi_cache.is_empty() {
                println!(
                    "Loaded {} Basic UDI-DI records from cache",
                    basic_udi_cache.len()
                );
            }
            let result = xlsx_export::ndjson_to_xlsx(Path::new(input_file), &basic_udi_cache)?;
            println!("  -> {}", result);
            Ok(())
        }
        Some("detail") => {
            // Process detail NDJSON, optionally merging with listing data
            let detail_file = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("ndjson/eudamed_10k_details.ndjson");
            let listing_file = args.get(3).map(|s| s.as_str());
            process_detail_ndjson(Path::new(detail_file), listing_file.map(Path::new), &config)
        }
        Some("xml") | None => {
            // Original XML mode (default)
            process_xml_dir(&config)
        }
        Some(other) => {
            // Check if it's a file path
            let path = Path::new(other);
            if path.exists() && path.extension().map(|e| e == "ndjson").unwrap_or(false) {
                process_ndjson_file(path, &config)
            } else if path.exists() && path.extension().map(|e| e == "xml").unwrap_or(false) {
                let output_dir = Path::new("firstbase_json");
                std::fs::create_dir_all(output_dir)?;
                let output = process_xml_file(path, output_dir, &config)?;
                println!("  -> {}", output);
                Ok(())
            } else {
                eprintln!("Usage: eudamed2firstbase [xml|ndjson [dir]|detail <details.ndjson> [listing.ndjson]|eudamed_json [dir]]");
                eprintln!("       eudamed2firstbase <file.ndjson>");
                eprintln!("       eudamed2firstbase <file.xml>");
                std::process::exit(1);
            }
        }
    }
}

/// After a Production push, email a report to GS1: a separate errors-only CSV
/// (the rejected GTINs + their GS1 codes) plus the full HTML push log. If the
/// HTML log would push the message over the size limit it is dropped and only
/// the errors-only CSV is sent. Configurable via env:
///   GS1_REPORT_TO    (else config.toml [gs1_report] to; comma-separated)
///   GS1_REPORT_FROM  (else config.toml [gs1_report] from)
///   GS1_REPORT_DISABLE=1  to skip entirely
/// No mail addresses are hardcoded — they live in the gitignored config.toml.
/// Gmail credentials come from `[gmail]` in config.toml (p12 + service account).
/// Never fails the push: returns Err only to be logged as non-fatal by the caller.
/// Build the Firstbase push settings (credentials from env; recipient GLN and
/// target env from env, falling back to config.toml) and push ONLY `uuids`
/// (scoped). On a Production push, auto-email the GS1 report. Shared by `check`
/// and `check --push-only`.
///
/// Returns `Ok(true)` when the push actually REACHED GS1 (`push_to_firstbase`
/// returned Ok — even if some items were validation-rejected), and `Ok(false)`
/// on a transport failure (503 / token / network) or a config skip (missing
/// creds/GLN) where nothing was delivered. Callers use this to decide whether to
/// clear the pending-push list. Errors are logged, not propagated, so a retry
/// after an outage still exits cleanly.
fn push_changed_to_firstbase(
    fb_config: &config::Config,
    uuids: &std::collections::HashSet<String>,
    srns: &[String],
    gtin_worklist: &[String],
) -> anyhow::Result<bool> {
    eprintln!("\n=== Pushing to GS1 Firstbase API ===");
    let email = std::env::var("FIRSTBASE_EMAIL").unwrap_or_default();
    let password = std::env::var("FIRSTBASE_PASSWORD").unwrap_or_default();
    // Env var takes priority; fall back to publish_gln from config.toml.
    let publish_gln = match std::env::var("FIRSTBASE_PUBLISH_GLN") {
        Ok(v) if !v.is_empty() => v,
        _ if !fb_config.provider.publish_gln.is_empty() => fb_config.provider.publish_gln.clone(),
        _ => {
            eprintln!("Set FIRSTBASE_PUBLISH_GLN (recipient GLN) or add publish_gln under [provider] in config.toml. Skipping push.");
            return Ok(false);
        }
    };
    if email.is_empty() || password.is_empty() {
        eprintln!("Set FIRSTBASE_EMAIL and FIRSTBASE_PASSWORD to push. Skipping push.");
        return Ok(false);
    }

    // Target environment: FIRSTBASE_ENV=Production (anything else = Test),
    // mirroring repush-srn. Default Test keeps ad-hoc `check` runs safe.
    let fb_env = match std::env::var("FIRSTBASE_ENV").as_deref() {
        Ok("Production") | Ok("production") | Ok("PROD") | Ok("prod") => {
            gui::FirstbaseEnv::Production
        }
        _ => gui::FirstbaseEnv::Test,
    };

    // provider_gln comes from config.toml, not a hardcoded default.
    let settings = gui::Settings {
        firstbase_email: email,
        firstbase_password: password,
        publish_to_gln: publish_gln,
        provider_gln: fb_config.provider.gln.clone(),
        firstbase_env: fb_env,
        ..Default::default()
    };
    let log_fn = |msg: &str| {
        eprintln!("{}", msg);
    };
    match gui::push_to_firstbase(&settings, &log_fn, Some(uuids)) {
        Ok((accepted, rejected)) => {
            eprintln!("\nDone: {} accepted, {} rejected.", accepted, rejected);
            // After a Production push, auto-email the GS1 report (non-fatal — a
            // mail error never fails the run).
            if matches!(settings.firstbase_env, gui::FirstbaseEnv::Production) {
                if let Err(e) =
                    send_gs1_prod_report(fb_config, accepted, rejected, srns, gtin_worklist)
                {
                    eprintln!("Auto-report to GS1 failed (non-fatal): {}", e);
                }
            }
            // Reached GS1 (validation rejects are a separate, per-item concern).
            Ok(true)
        }
        Err(e) => {
            // Transport failure (503 / token / network) — nothing delivered.
            eprintln!("\nPush failed: {}", e);
            Ok(false)
        }
    }
}

fn send_gs1_prod_report(
    config: &config::Config,
    accepted: u32,
    rejected: u32,
    srns: &[String],
    gtin_worklist: &[String],
) -> anyhow::Result<()> {
    if std::env::var("GS1_REPORT_DISABLE")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        eprintln!("GS1 auto-report skipped (GS1_REPORT_DISABLE set).");
        return Ok(());
    }
    // Recipients/sender are NOT hardcoded (no mail addresses in source): env var
    // GS1_REPORT_TO / GS1_REPORT_FROM first, else the gitignored config.toml
    // `[gs1_report]` section.
    let to = std::env::var("GS1_REPORT_TO")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| config.gs1_report.to.clone());
    let from = std::env::var("GS1_REPORT_FROM")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| config.gs1_report.from.clone());
    if to.trim().is_empty() {
        eprintln!(
            "GS1 auto-report skipped: no recipient. Set GS1_REPORT_TO or [gs1_report] to \
             in config.toml."
        );
        return Ok(());
    }
    let p12 = config.gmail.p12_key.clone();
    let service_email = config.gmail.service_email.clone();
    if p12.is_empty() || service_email.is_empty() {
        eprintln!(
            "GS1 auto-report skipped: Gmail not configured ([gmail] p12_key/service_email \
             in config.toml)."
        );
        return Ok(());
    }

    let data_dir = download::app_data_dir();
    let prod_log_dir = data_dir.join("log").join("firstbase_prod");

    // --- Build two CSVs from the latest Production push session ---
    //   1) errors  CSV: one row per GS1 error (a device can have several)
    //   2) devices CSV: one row per rejected device (codes aggregated)
    let db_path = data_dir.join("db").join("version_tracking.db");
    let ts = chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let errors_csv = prod_log_dir.join(format!("rejects_errors_{}.csv", ts));
    let devices_csv = prod_log_dir.join(format!("rejects_devices_{}.csv", ts));
    let updates_csv = prod_log_dir.join(format!("updates_pushed_{}.csv", ts));
    // Separate attachment: only the accepted devices from the customer GTIN
    // worklist (gtin ∈ gtin_worklist). Written + attached only when the worklist
    // is non-empty AND ≥1 of its GTINs was accepted this session.
    let gtin_updates_csv = prod_log_dir.join(format!("updates_gtin_{}.csv", ts));
    let _ = std::fs::create_dir_all(&prod_log_dir);
    let esc = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));

    // Fetch every error row for the latest Production session.
    let mut error_rows: Vec<(String, String, String, String, String)> = Vec::new();
    // Accepted (successfully pushed) devices for this session, with the version
    // info + EUDAMED link so GS1 can locate the update in EUDAMED:
    // (srn, gtin, uuid, udi_version, budi_version, version_date).
    let mut accepted_rows: Vec<(String, String, String, String, String, String)> = Vec::new();
    // Push date for the subject, taken from the session timestamp (DD.MM.YYYY).
    let mut push_date = chrono::Local::now().format("%d.%m.%Y").to_string();
    {
        let conn = version_db::open_db(&db_path).context("open version DB for GS1 report")?;
        let session: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, COALESCE(session_ts,'') FROM push_session \
                 WHERE firstbase_env='Production' ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let session_id: Option<i64> = session.as_ref().map(|(id, _)| *id);
        // session_ts is ISO "YYYY-MM-DDT..." — reformat the date part to DD.MM.YYYY.
        if let Some((_, ts)) = &session {
            if ts.len() >= 10 {
                let (y, m, d) = (&ts[0..4], &ts[5..7], &ts[8..10]);
                push_date = format!("{}.{}.{}", d, m, y);
            }
        }
        if let Some(sid) = session_id {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(l.srn,''), COALESCE(e.gtin,''), COALESCE(e.error_code,''), \
                 COALESCE(e.attribute_name,''), COALESCE(e.error_description,'') \
                 FROM push_error e LEFT JOIN listing_cache l ON l.uuid = e.uuid \
                 WHERE e.session_id = ?1 ORDER BY l.srn, e.gtin, e.error_code",
            )?;
            let rows = stmt.query_map([sid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?;
            for row in rows {
                error_rows.push(row?);
            }

            // Accepted devices pushed in this session (the "updates" list), with
            // version info from listing_cache/udi_versions so GS1 can find the
            // exact version in EUDAMED. version_number matches EUDAMED's
            // `versionNumber`; udi_date is the EUDAMED version date.
            let mut astmt = conn.prepare(
                "SELECT COALESCE(l.srn,''), COALESCE(p.gtin,''), p.uuid, \
                 COALESCE(CAST(l.version_number AS TEXT),''), \
                 COALESCE(CAST(l.budi_version_number AS TEXT),''), \
                 COALESCE(v.udi_date,'') \
                 FROM push_log p \
                 LEFT JOIN listing_cache l ON l.uuid = p.uuid \
                 LEFT JOIN udi_versions v ON v.uuid = p.uuid \
                 WHERE p.pushed_at = (SELECT session_ts FROM push_session WHERE id = ?1) \
                 AND p.status = 'ACCEPTED' ORDER BY l.srn, p.gtin",
            )?;
            let arows = astmt.query_map([sid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            for row in arows {
                accepted_rows.push(row?);
            }
        }
    }

    // 1) errors CSV — one row per error.
    {
        let mut csv = String::from("srn,gtin,error_code,attribute,description\r\n");
        for (srn, gtin, code, attr, desc) in &error_rows {
            csv.push_str(&format!(
                "{},{},{},{},{}\r\n",
                esc(srn),
                esc(gtin),
                esc(code),
                esc(attr),
                esc(desc)
            ));
        }
        std::fs::write(&errors_csv, csv.as_bytes())
            .with_context(|| format!("write {}", errors_csv.display()))?;
    }

    // 2) devices CSV — one row per device, distinct codes joined, error count.
    let device_count;
    {
        // (srn, gtin) -> (distinct codes in first-seen order, total error count)
        let mut devices: Vec<(String, String, Vec<String>, usize)> = Vec::new();
        for (srn, gtin, code, _attr, _desc) in &error_rows {
            if let Some(d) = devices.iter_mut().find(|d| &d.0 == srn && &d.1 == gtin) {
                if !d.2.contains(code) {
                    d.2.push(code.clone());
                }
                d.3 += 1;
            } else {
                devices.push((srn.clone(), gtin.clone(), vec![code.clone()], 1));
            }
        }
        device_count = devices.len();
        let mut csv = String::from("srn,gtin,error_codes,error_count\r\n");
        for (srn, gtin, codes, count) in &devices {
            csv.push_str(&format!(
                "{},{},{},{}\r\n",
                esc(srn),
                esc(gtin),
                esc(&codes.join("; ")),
                count
            ));
        }
        std::fs::write(&devices_csv, csv.as_bytes())
            .with_context(|| format!("write {}", devices_csv.display()))?;
    }
    // 3) updates CSV — one row per ACCEPTED (successfully pushed) device, with
    //    the UDI-DI/Basic UDI-DI version, version date, and a direct EUDAMED
    //    link (the API URL resolves to that exact device and shows
    //    `versionNumber`/`versionDate`) so GS1 can verify the update in EUDAMED.
    {
        let mut csv =
            String::from("srn,gtin,udi_version,budi_version,version_date,eudamed_url\r\n");
        for (srn, gtin, uuid, udi_ver, budi_ver, ver_date) in &accepted_rows {
            let url = format!(
                "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/{}?languageIso2Code=en",
                uuid
            );
            csv.push_str(&format!(
                "{},{},{},{},{},{}\r\n",
                esc(srn),
                esc(gtin),
                esc(udi_ver),
                esc(budi_ver),
                esc(ver_date),
                esc(&url)
            ));
        }
        std::fs::write(&updates_csv, csv.as_bytes())
            .with_context(|| format!("write {}", updates_csv.display()))?;
    }
    // 4) GTIN-worklist updates CSV — the subset of the accepted devices whose
    //    GTIN is in the customer GTIN worklist, so distribution/GS1 can see the
    //    customer's own updates separately from the SRN-worklist ones. Same
    //    columns as updates_pushed. Only written when there is ≥1 such row.
    let gtin_update_count = if gtin_worklist.is_empty() {
        0
    } else {
        let worklist: std::collections::HashSet<&str> =
            gtin_worklist.iter().map(|g| g.as_str()).collect();
        let mut csv =
            String::from("srn,gtin,udi_version,budi_version,version_date,eudamed_url\r\n");
        let mut n = 0usize;
        for (srn, gtin, uuid, udi_ver, budi_ver, ver_date) in &accepted_rows {
            if !worklist.contains(gtin.as_str()) {
                continue;
            }
            n += 1;
            let url = format!(
                "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/{}?languageIso2Code=en",
                uuid
            );
            csv.push_str(&format!(
                "{},{},{},{},{},{}\r\n",
                esc(srn),
                esc(gtin),
                esc(udi_ver),
                esc(budi_ver),
                esc(ver_date),
                esc(&url)
            ));
        }
        if n > 0 {
            std::fs::write(&gtin_updates_csv, csv.as_bytes())
                .with_context(|| format!("write {}", gtin_updates_csv.display()))?;
        }
        n
    };
    eprintln!(
        "GS1 auto-report: {} error rows / {} devices / {} accepted ({} GTIN-worklist) -> {} , {} , {}",
        error_rows.len(),
        device_count,
        accepted_rows.len(),
        gtin_update_count,
        errors_csv.display(),
        devices_csv.display(),
        updates_csv.display()
    );

    // --- Locate the most recent Production HTML log ---
    let latest_html: Option<std::path::PathBuf> = std::fs::read_dir(&prod_log_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".log.html"))
                .unwrap_or(false)
        })
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());

    // --- Attachment list with size guard: both CSVs always; HTML only if it fits ---
    // Gmail caps a message at 25 MB; base64 inflates ~33%, so keep raw under ~18 MB.
    const MAX_TOTAL_RAW: u64 = 18 * 1024 * 1024;
    let mut attachments: Vec<String> = vec![
        updates_csv.to_string_lossy().to_string(),
        errors_csv.to_string_lossy().to_string(),
        devices_csv.to_string_lossy().to_string(),
    ];
    // The GTIN-worklist updates CSV (if any rows) goes right after the full
    // updates list — tiny, always kept.
    if gtin_update_count > 0 {
        attachments.insert(1, gtin_updates_csv.to_string_lossy().to_string());
    }
    let base_size = std::fs::metadata(&updates_csv)
        .map(|m| m.len())
        .unwrap_or(0)
        + std::fs::metadata(&errors_csv).map(|m| m.len()).unwrap_or(0)
        + std::fs::metadata(&devices_csv)
            .map(|m| m.len())
            .unwrap_or(0)
        + std::fs::metadata(&gtin_updates_csv)
            .map(|m| m.len())
            .unwrap_or(0);
    if let Some(html) = &latest_html {
        let html_size = std::fs::metadata(html).map(|m| m.len()).unwrap_or(0);
        if base_size + html_size <= MAX_TOTAL_RAW {
            attachments.push(html.to_string_lossy().to_string());
        } else {
            eprintln!(
                "GS1 auto-report: HTML log {} ({} bytes) too large — sending the two CSVs only.",
                html.display(),
                html_size
            );
        }
    } else {
        eprintln!("GS1 auto-report: no Production HTML log found — sending the two CSVs only.");
    }

    let total = accepted + rejected;
    let pct = if total > 0 {
        accepted as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    // Subject leads with the push date instead of a fixed phrase.
    let subject = format!(
        "{} — {} / {} ACCEPTED ({:.2}%)",
        push_date, accepted, total, pct
    );

    // Body separates the pushed SRNs into "ok" (no push errors) and "not-ok"
    // (>=1 rejected device). not-ok = distinct SRNs among the rejected devices;
    // ok = the caller's full pushed list minus not-ok (so a fully-accepted run
    // lists every SRN under "ok" and nothing under "not-ok"). For a manual
    // `gs1-report` resend without a pushed list, only "not-ok" can be shown.
    let mut not_ok: Vec<String> = error_rows
        .iter()
        .map(|(srn, ..)| srn.clone())
        .filter(|s| !s.is_empty())
        .collect();
    not_ok.sort();
    not_ok.dedup();
    let not_ok_set: std::collections::HashSet<&String> = not_ok.iter().collect();
    let mut ok: Vec<String> = srns
        .iter()
        .filter(|s| !s.is_empty() && !not_ok_set.contains(s))
        .cloned()
        .collect();
    ok.sort();
    ok.dedup();
    let mut body = String::new();
    if !ok.is_empty() {
        body.push_str(&format!("SRNs ok ({}):\n{}\n", ok.len(), ok.join("\n")));
    }
    if !not_ok.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&format!(
            "SRNs not-ok ({}):\n{}\n",
            not_ok.len(),
            not_ok.join("\n")
        ));
    }

    mail::send_email_with_attachments(
        &p12,
        &service_email,
        &from,
        &to,
        &subject,
        &body,
        &attachments,
    )?;
    eprintln!("GS1 auto-report sent to {}.", to);
    Ok(())
}

fn parse_download_args(
    args: &[String],
) -> (Vec<String>, Vec<String>, Option<usize>, Option<usize>) {
    let mut srns = Vec::new();
    let mut gtins = Vec::new();
    let mut limit = None;
    let mut threads = None;
    let mut i = 0;

    while i < args.len() {
        if args[i] == "--srn" {
            i += 1;
            while i < args.len() && !args[i].starts_with("--") {
                srns.push(args[i].clone());
                i += 1;
            }
        } else if args[i] == "--gtin" {
            i += 1;
            while i < args.len() && !args[i].starts_with("--") {
                gtins.push(args[i].clone());
                i += 1;
            }
        } else if args[i] == "--threads" {
            i += 1;
            if i < args.len() {
                threads = args[i].parse().ok();
                i += 1;
            }
        } else if args[i].starts_with("--") && args[i][2..].parse::<usize>().is_ok() {
            if let Ok(n) = args[i][2..].parse::<usize>() {
                limit = Some(n);
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    (srns, gtins, limit, threads)
}

fn process_xml_dir(config: &config::Config) -> Result<()> {
    let input_dir = Path::new("xml");
    let output_dir = Path::new("firstbase_json");
    let processed_dir = input_dir.join("processed");
    std::fs::create_dir_all(output_dir)?;

    let mut processed = 0;
    let mut processed_files = Vec::new();
    for entry in std::fs::read_dir(input_dir).context("Failed to read xml/ directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "xml").unwrap_or(false) {
            println!("Processing: {}", path.display());
            match process_xml_file(&path, output_dir, config) {
                Ok(output_path) => {
                    println!("  -> {}", output_path);
                    processed += 1;
                    processed_files.push(path);
                }
                Err(e) => {
                    eprintln!("  Error: {:#}", e);
                }
            }
        }
    }

    // Move successfully processed files to xml/processed/
    if !processed_files.is_empty() {
        std::fs::create_dir_all(&processed_dir)?;
        for path in &processed_files {
            let file_name = match path.file_name() {
                Some(n) => n,
                None => {
                    eprintln!(
                        "  Error: could not determine file name for {}",
                        path.display()
                    );
                    std::process::exit(1);
                }
            };
            let dest = processed_dir.join(file_name);
            if let Err(e) = std::fs::rename(path, &dest) {
                eprintln!(
                    "  Warning: could not move {} to processed/: {}",
                    path.display(),
                    e
                );
            }
        }
        println!(
            "Moved {} file(s) to {}",
            processed_files.len(),
            processed_dir.display()
        );
    }

    println!("\nProcessed {} XML file(s)", processed);
    Ok(())
}

fn process_xml_file(
    input_path: &Path,
    output_dir: &Path,
    config: &config::Config,
) -> Result<String> {
    let xml_content = std::fs::read_to_string(input_path).context("Failed to read XML file")?;

    let response =
        eudamed::parse_pull_response(&xml_content).context("Failed to parse EUDAMED XML")?;

    let document = transform::transform(&response, config)
        .context("Failed to transform to firstbase format")?;

    let now = Local::now();
    let filename = format!("firstbase_{}.json", now.format("%d.%m.%Y"));
    let output_path = output_dir.join(&filename);

    let json = serde_json::to_string_pretty(&document)?;
    std::fs::write(&output_path, json)?;

    Ok(output_path.display().to_string())
}

fn process_ndjson(input_dir: &Path, config: &config::Config) -> Result<()> {
    let output_dir = Path::new("firstbase_json");
    std::fs::create_dir_all(output_dir)?;

    let mut total_processed = 0;
    for entry in std::fs::read_dir(input_dir).context("Failed to read ndjson/ directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "ndjson").unwrap_or(false) {
            println!("Processing: {}", path.display());
            match process_ndjson_file(&path, config) {
                Ok(()) => {
                    total_processed += 1;
                }
                Err(e) => {
                    eprintln!("  Error: {:#}", e);
                }
            }
        }
    }

    println!("\nProcessed {} NDJSON file(s)", total_processed);
    Ok(())
}

fn process_ndjson_file(input_path: &Path, config: &config::Config) -> Result<()> {
    let output_dir = Path::new("firstbase_json");
    std::fs::create_dir_all(output_dir)?;

    let file = std::fs::File::open(input_path).context("Failed to open NDJSON file")?;
    let reader = std::io::BufReader::new(file);

    let mut trade_items = Vec::new();
    let mut errors = 0;
    let mut line_num = 0;

    for line in reader.lines() {
        line_num += 1;
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match api_json::parse_api_device(trimmed) {
            Ok(device) => {
                let trade_item = transform_api::transform_api_device(&device, config);
                let uuid = device.uuid.as_deref().unwrap_or("unknown");
                let document = firstbase::FirstbaseDocument {
                    trade_item,
                    children: Vec::new(),
                    identifier: format!("Draft_{}", uuid),
                };
                trade_items.push(firstbase::DraftItemDocument {
                    draft_item: document,
                });
            }
            Err(e) => {
                if errors < 5 {
                    eprintln!("  Line {}: {}", line_num, e);
                }
                errors += 1;
            }
        }
    }

    // Generate output filename
    let now = Local::now();
    let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
    let filename = format!("firstbase_{}_{}.json", stem, now.format("%d.%m.%Y"));
    let output_path = output_dir.join(&filename);

    let json = serde_json::to_string_pretty(&trade_items)?;
    std::fs::write(&output_path, &json)?;

    println!(
        "  -> {} ({} devices, {} errors, {})",
        output_path.display(),
        trade_items.len(),
        errors,
        format_size(json.len()),
    );

    Ok(())
}

/// Process detail NDJSON file, optionally merging with listing data for
/// fields not available in the detail endpoint (manufacturer SRN/name,
/// AR SRN/name, risk class, basic UDI).
fn process_detail_ndjson(
    detail_path: &Path,
    listing_path: Option<&Path>,
    config: &config::Config,
) -> Result<()> {
    let output_dir = Path::new("firstbase_json");
    std::fs::create_dir_all(output_dir)?;

    // Load listing data index if provided (keyed by GTIN / primaryDi)
    let listing_index = if let Some(lp) = listing_path {
        println!("Loading listing data from {}...", lp.display());
        load_listing_index(lp)?
    } else {
        // Try default listing file
        let default_listing = Path::new("ndjson/eudamed_10k.ndjson");
        if default_listing.exists() {
            println!("Loading listing data from {}...", default_listing.display());
            load_listing_index(default_listing)?
        } else {
            HashMap::new()
        }
    };

    if !listing_index.is_empty() {
        println!(
            "  Loaded {} listing records for merging",
            listing_index.len()
        );
    }

    // Load Basic UDI-DI cache
    let basic_udi_cache = load_basic_udi_cache(Path::new(BASIC_UDI_CACHE_DIR));
    if !basic_udi_cache.is_empty() {
        println!(
            "  Loaded {} Basic UDI-DI records from cache",
            basic_udi_cache.len()
        );
    }

    let file = std::fs::File::open(detail_path)
        .with_context(|| format!("Failed to open {}", detail_path.display()))?;
    let reader = std::io::BufReader::new(file);

    // Read all lines first
    let lines: Vec<(usize, String)> = reader
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.ok()?;
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some((i + 1, trimmed))
            }
        })
        .collect();

    // Process lines in parallel
    let results: Vec<Result<firstbase::DraftItemDocument, (usize, String)>> = lines
        .par_iter()
        .map(|(line_num, trimmed)| {
            match api_detail::parse_api_detail(trimmed) {
                Ok(detail) => {
                    let uuid = detail.uuid.clone().unwrap_or_default();
                    let basic_udi = basic_udi_cache.get(&uuid);
                    let mut document = transform_detail::transform_detail_document(
                        &detail, config, basic_udi, &uuid,
                    );

                    // Merge listing data (manufacturer, AR, risk class, basic UDI)
                    let gtin = &document.trade_item.gtin;
                    if let Some(listing) = listing_index.get(gtin) {
                        merge_listing_data(&mut document.trade_item, listing);
                    }

                    let draft_doc = firstbase::DraftItemDocument {
                        draft_item: document,
                    };

                    // Write individual file per UUID
                    if !uuid.is_empty() {
                        let individual_path = output_dir.join(format!("{}.json", uuid));
                        if let Ok(individual_json) = serde_json::to_string_pretty(&draft_doc) {
                            let _ = std::fs::write(&individual_path, &individual_json);
                        }
                    }

                    Ok(draft_doc)
                }
                Err(e) => Err((*line_num, format!("{}", e))),
            }
        })
        .collect();

    // Collect results preserving order
    let mut trade_items = Vec::new();
    let mut errors = 0;
    for result in results {
        match result {
            Ok(doc) => trade_items.push(doc),
            Err((line_num, e)) => {
                if errors < 10 {
                    eprintln!("  Line {}: {}", line_num, e);
                }
                errors += 1;
            }
        }
    }

    if errors > 10 {
        eprintln!("  ... and {} more errors", errors - 10);
    }

    let now = Local::now();
    let stem = detail_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let filename = format!("firstbase_{}_{}.json", stem, now.format("%d.%m.%Y"));
    let output_path = output_dir.join(&filename);

    let json = serde_json::to_string_pretty(&trade_items)?;
    std::fs::write(&output_path, &json)?;

    println!(
        "  -> {} ({} devices, {} errors, {})",
        output_path.display(),
        trade_items.len(),
        errors,
        format_size(json.len()),
    );

    Ok(())
}

/// Listing data we want to merge into detail-based records
struct ListingData {
    basic_udi: String,
    risk_class_code: Option<String>,
    manufacturer_srn: Option<String>,
    manufacturer_name: Option<String>,
    authorised_representative_srn: Option<String>,
    authorised_representative_name: Option<String>,
}

fn load_listing_index(path: &Path) -> Result<HashMap<String, ListingData>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut index = HashMap::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(device) = api_json::parse_api_device(trimmed) {
            if let Some(ref gtin) = device.primary_di {
                if !gtin.is_empty() {
                    index.insert(
                        gtin.clone(),
                        ListingData {
                            basic_udi: device.basic_udi.clone().unwrap_or_default(),
                            risk_class_code: device.risk_class_code(),
                            manufacturer_srn: device.manufacturer_srn.clone(),
                            manufacturer_name: device.manufacturer_name.clone(),
                            authorised_representative_srn: device
                                .authorised_representative_srn
                                .clone(),
                            authorised_representative_name: device
                                .authorised_representative_name
                                .clone(),
                        },
                    );
                }
            }
        }
    }

    Ok(index)
}

fn merge_listing_data(trade_item: &mut firstbase::TradeItem, listing: &ListingData) {
    // Set basic UDI as global model number 1:1 (v1.0.64, Maik's mapping): the
    // real GMN for MDR/IVDR, the `B-<GTIN>` placeholder for legacy. No local GMN
    // gate — EUDAMED validates GS1 identifiers at registration. (097.116 on
    // legacy B-<GTIN> is the open TEST-push question; see firstbase::build.)
    if !listing.basic_udi.is_empty() {
        if let Some(gmi) = trade_item.global_model_info.first_mut() {
            gmi.number = listing.basic_udi.clone();
        }
    }

    // Add risk class classification (system 76) if not already present
    if let Some(ref rc) = listing.risk_class_code {
        let gs1_risk = mappings::risk_class_to_gs1(rc);
        let has_risk_class = trade_item
            .classification
            .additional_classifications
            .iter()
            .any(|c| c.system_code.value == "76");
        if !has_risk_class {
            trade_item.classification.additional_classifications.insert(
                0,
                firstbase::AdditionalClassification {
                    system_code: firstbase::CodeValue {
                        value: "76".to_string(),
                    },
                    values: vec![firstbase::AdditionalClassificationValue {
                        code_value: gs1_risk.to_string(),
                    }],
                },
            );
        }

        // Update regulatory act based on risk class (MDR vs IVDR) — fixes 097.005
        let act = mappings::regulation_from_risk_class(rc);
        if let Some(ref mut module) = trade_item.regulated_trade_item_module {
            if let Some(reg) = module.info.first_mut() {
                reg.act = act.to_string();
            }
        }
    }

    // Add manufacturer contact (if not already added by Basic UDI-DI)
    let has_ema = trade_item
        .contact_information
        .iter()
        .any(|c| c.contact_type.value == "EMA");
    if !has_ema {
        if let Some(ref srn) = listing.manufacturer_srn {
            trade_item
                .contact_information
                .push(firstbase::TradeItemContactInformation {
                    contact_type: firstbase::CodeValue {
                        value: "EMA".to_string(),
                    },
                    party_identification: vec![firstbase::AdditionalPartyIdentification {
                        type_code: "SRN".to_string(),
                        value: srn.clone(),
                    }],
                    contact_name: listing.manufacturer_name.clone(),
                    addresses: Vec::new(),
                    communication_channels: Vec::new(),
                });
        }
    }

    // Add authorised representative contact (if not already added by Basic UDI-DI)
    let has_ear = trade_item
        .contact_information
        .iter()
        .any(|c| c.contact_type.value == "EAR");
    if !has_ear {
        if let Some(ref srn) = listing.authorised_representative_srn {
            trade_item
                .contact_information
                .push(firstbase::TradeItemContactInformation {
                    contact_type: firstbase::CodeValue {
                        value: "EAR".to_string(),
                    },
                    party_identification: vec![firstbase::AdditionalPartyIdentification {
                        type_code: "SRN".to_string(),
                        value: srn.clone(),
                    }],
                    contact_name: listing.authorised_representative_name.clone(),
                    addresses: Vec::new(),
                    communication_channels: Vec::new(),
                });
        }
    }
}

/// Process individual EUDAMED JSON files from a directory.
/// Each input file produces one output file (one-to-one mapping).
/// Uses version tracking DB to skip unchanged devices.
fn process_eudamed_json_dir(input_dir: &Path, config: &config::Config) -> Result<()> {
    let output_dir = Path::new("firstbase_json");
    let processed_dir = input_dir.join("processed");
    std::fs::create_dir_all(output_dir)?;

    // Open version tracking database
    let db_path = Path::new(version_db::VERSION_DB_PATH);
    let conn = version_db::open_db(db_path).context("Failed to open version tracking DB")?;
    let existing_count = version_db::count_records(&conn)?;
    println!(
        "Version DB: {} existing records ({})",
        existing_count,
        db_path.display()
    );

    // Load Basic UDI-DI cache
    let cache_dir = Path::new(BASIC_UDI_CACHE_DIR);
    let mut basic_udi_cache = load_basic_udi_cache(cache_dir);
    if !basic_udi_cache.is_empty() {
        println!(
            "Loaded {} Basic UDI-DI records from cache",
            basic_udi_cache.len()
        );
    }

    let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    let mut processed = 0;
    let mut skipped = 0;
    let mut errors = 0;
    let mut processed_files = Vec::new();
    let mut change_summary: HashMap<String, u32> = HashMap::new();

    for entry in std::fs::read_dir(input_dir).context("Failed to read eudamed_json/ directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            let json_content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;

            // Detect file type: UDI-DI level (has primaryDi with actual data) vs device level
            // Exclude "primaryDi":null and "primaryDi": null
            let is_udi_di = json_content.contains("\"primaryDi\"")
                && !json_content.contains("\"primaryDi\":null")
                && !json_content.contains("\"primaryDi\": null");

            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // --- Version tracking: extract versions and check for changes ---
            let mut version_rec = if is_udi_di {
                version_db::extract_detail_versions(&json_content)
            } else {
                // Device-level files: use hash-only tracking (no sub-section versions)
                let mut rec = version_db::VersionRecord::default();
                rec.uuid = stem.clone();
                rec.detail_hash = version_db::hash_json(&json_content);
                rec
            };

            // Merge BUDI versions if cache file exists
            if is_udi_di {
                let budi_cache_path = cache_dir.join(format!("{}.json", stem));
                if let Ok(budi_json) = std::fs::read_to_string(&budi_cache_path) {
                    version_db::merge_budi_versions(&mut version_rec, &budi_json);
                }
            }

            version_rec.last_synced = Some(now_str.clone());

            // Detect changes
            let changes = version_db::detect_changes(&conn, &version_rec)?;
            if !changes.has_any_change() {
                let out_path = output_dir.join(format!("{}.json", stem));
                if out_path.exists() {
                    // Output already on disk — real no-op skip.
                    skipped += 1;
                    processed_files.push(path);
                    continue;
                }
                // Hash/versions match DB but no output file exists. Happens when
                // `download` step indexed udi_versions *before* convert ran.
                // Fall through to actual conversion so the output is produced.
            }

            let change_label = changes.summary();
            *change_summary.entry(change_label.clone()).or_insert(0) += 1;

            // --- Convert ---
            let result: anyhow::Result<firstbase::FirstbaseDocument> = if is_udi_di {
                // UDI-DI level file — reuse existing api_detail parser/transformer
                // Fetch Basic UDI-DI on demand if not cached
                if !basic_udi_cache.contains_key(&stem) {
                    if let Some(data) = fetch_basic_udi_di(&stem, cache_dir) {
                        println!("  Fetched Basic UDI-DI for {}", stem);
                        basic_udi_cache.insert(stem.clone(), data);
                        // Re-merge BUDI versions after fetch
                        let budi_cache_path = cache_dir.join(format!("{}.json", stem));
                        if let Ok(budi_json) = std::fs::read_to_string(&budi_cache_path) {
                            version_db::merge_budi_versions(&mut version_rec, &budi_json);
                        }
                    }
                }
                api_detail::parse_api_detail(&json_content).map(|detail| {
                    let basic_udi = basic_udi_cache.get(&stem);
                    transform_detail::transform_detail_document(&detail, config, basic_udi, &stem)
                })
            } else {
                // Device level file (Basic UDI-DI)
                eudamed_json::parse_eudamed_json(&json_content).map(|device| {
                    let trade_item =
                        transform_eudamed_json::transform_eudamed_device(&device, config);
                    firstbase::FirstbaseDocument {
                        trade_item,
                        children: Vec::new(),
                        identifier: format!("Draft_{}", stem),
                    }
                })
            };

            match result {
                Ok(document) => {
                    let draft_doc = firstbase::DraftItemDocument {
                        draft_item: document,
                    };

                    let filename = path.file_name().unwrap_or_default().to_string_lossy();
                    let output_path = output_dir.join(filename.as_ref());

                    let json = serde_json::to_string_pretty(&draft_doc)?;
                    std::fs::write(&output_path, &json)?;

                    // Update version DB after successful conversion
                    version_db::upsert_version(&conn, &version_rec)?;

                    processed += 1;
                    processed_files.push(path);
                }
                Err(e) => {
                    eprintln!("  Error in {}: {:#}", path.display(), e);
                    errors += 1;
                }
            }
        }
    }

    // Files stay in eudamed_json/detail/ — version DB tracks what's been processed

    // Print change summary
    if !change_summary.is_empty() {
        println!("\nChange summary:");
        let mut sorted: Vec<_> = change_summary.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (label, count) in sorted {
            println!("  {:>5}x  {}", count, label);
        }
    }

    println!(
        "\nProcessed {} converted, {} skipped (unchanged), {} errors -> {}",
        processed,
        skipped,
        errors,
        output_dir.display()
    );
    Ok(())
}

/// Fetch Basic UDI-DI data from EUDAMED API and cache it.
/// Returns None on any failure (network, parse, etc.).
/// Fetch a device's Basic UDI-DI from the EUDAMED API on a cache miss.
///
/// **Hardened (v1.0.65):** retries with backoff and only caches/returns a body
/// that actually parses to a Basic UDI-DI carrying a non-empty `basic_udi.code`.
/// The old single-shot version returned `None` on any transient hiccup (timeout,
/// 429, 5xx) — under a 5000-device bulk Mode 5/Mode 0 reconvert that silently left
/// `basic_udi = None` for the unlucky devices, so their `GlobalModelInformation`
/// came out empty (no globalModelNumber, no globalModelDescription) → 097.025 on
/// every push (the FR/CH/BR 50-device reject batch). Parsing *before* caching also
/// stops an error page from being written to the cache and then counting as a
/// "present" basic file forever.
/// Why a Basic UDI-DI (re)fetch ended the way it did — captured so a bulk
/// force-reload can tell EUDAMED **throttling (HTTP 429)** from a **genuinely
/// absent record (HTTP 404)** from a **client-side timeout/connection error**,
/// instead of guessing. `Http(code)` is the last non-2xx status EUDAMED actually
/// returned; `Network` is a connection/timeout error (no HTTP response at all);
/// `EmptyBody` is a 2xx whose body carried no valid, code-carrying Basic UDI-DI.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BasicFetchReason {
    Ok,
    Http(u16),
    Network,
    EmptyBody,
}

/// Core fetch returning both the data (for callers that need it) and the failure
/// reason (for force-reload diagnostics). Parse-before-cache: only a valid,
/// code-carrying body is ever written to disk.
fn fetch_basic_udi_di_outcome(
    uuid: &str,
    cache_dir: &Path,
) -> (Option<api_detail::BasicUdiDiData>, BasicFetchReason) {
    let url = format!(
        "https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/{}?languageIso2Code=en",
        uuid
    );
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .new_agent();

    let mut last_reason = BasicFetchReason::Network;
    for attempt in 1..=4 {
        match agent.get(&url).call() {
            Ok(mut resp) => {
                // http_status_as_error(false) ⇒ 404/429/5xx arrive here as Ok with a
                // readable status, NOT as Err. Capture it so we can categorise.
                let status = resp.status().as_u16();
                // Capture Retry-After (seconds) BEFORE consuming the body. EUDAMED's
                // Basic-UDI endpoint is rate-limited to ~60 req/60s (measured) and
                // answers a 429 with `Retry-After: 60`; honoring that header is the
                // only backoff that actually clears the window — the old linear 1-3s
                // backoff could never win against a 60s throttle, which is why a
                // 50-thread burst lost 429×4978 of 5372 in Maik's v1.0.69 run.
                let retry_after_secs = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                match api_detail::parse_basic_udi_di(&body) {
                    Ok(data)
                        if data
                            .basic_udi
                            .as_ref()
                            .and_then(|b| b.code.as_ref())
                            .map(|c| !c.is_empty())
                            .unwrap_or(false) =>
                    {
                        // Only cache a body that is a real, code-carrying Basic UDI-DI.
                        let _ = std::fs::create_dir_all(cache_dir);
                        let cache_path = cache_dir.join(format!("{}.json", uuid));
                        if let Err(e) = std::fs::write(&cache_path, &body) {
                            eprintln!(
                                "  Warning: failed to cache Basic UDI-DI for {}: {}",
                                uuid, e
                            );
                        }
                        return (Some(data), BasicFetchReason::Ok);
                    }
                    _ => {
                        // EUDAMED answered but the body has no usable code. A non-2xx
                        // status is the real signal (429 throttle / 404 no record /
                        // 5xx server error); a 2xx here means an empty/partial body.
                        last_reason = if (200..300).contains(&status) {
                            BasicFetchReason::EmptyBody
                        } else {
                            BasicFetchReason::Http(status)
                        };
                        if attempt < 4 {
                            // On a 429 throttle, wait the server-stated Retry-After
                            // (capped at 70s); otherwise a short linear backoff.
                            let wait = if status == 429 {
                                retry_after_secs.unwrap_or(60).min(70)
                            } else {
                                attempt
                            };
                            std::thread::sleep(std::time::Duration::from_secs(wait));
                            continue;
                        }
                        eprintln!(
                            "  Warning: Basic UDI-DI for {} unusable after {} attempts (last HTTP {})",
                            uuid, attempt, status
                        );
                        return (None, last_reason);
                    }
                }
            }
            Err(e) => {
                last_reason = BasicFetchReason::Network;
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_secs(attempt));
                    continue;
                }
                eprintln!(
                    "  Warning: failed to fetch Basic UDI-DI for {} after {} attempts: {}",
                    uuid, attempt, e
                );
                return (None, last_reason);
            }
        }
    }
    (None, last_reason)
}

/// Public wrapper: fetch the Basic UDI-DI, returning just the data. Used by the
/// convert fetch-on-miss path. Force-reload calls [`fetch_basic_udi_di_outcome`]
/// directly so it can categorise why a refetch failed.
fn fetch_basic_udi_di(uuid: &str, cache_dir: &Path) -> Option<api_detail::BasicUdiDiData> {
    fetch_basic_udi_di_outcome(uuid, cache_dir).0
}

/// True if the cached Basic UDI-DI for `uuid` is missing, unparseable, or
/// incomplete (no `basic_udi.code` or no `device_name`) — i.e. it needs a fresh
/// refetch to heal the 097.013/097.025 reject drivers. A cached basic that
/// already carries a non-empty code AND deviceName is treated as current and
/// skipped.
///
/// This is the gate that keeps Mode 6 within EUDAMED's ~60-req/60s Basic-UDI
/// budget: refetching all 5372 every run blows the budget (429×4978), but only
/// the genuinely stale/missing handful actually need healing. A device whose
/// EUDAMED record is itself empty (e.g. FR-MF-000000602: no deviceName at the
/// source) will keep returning true and be refetched each run — harmless, it is
/// a small constant set, and the refetch simply confirms the source gap.
fn basic_needs_refetch(uuid: &str, basic_dir: &Path) -> bool {
    let path = basic_dir.join(format!("{}.json", uuid));
    match std::fs::read_to_string(&path) {
        Ok(body) => match api_detail::parse_basic_udi_di(&body) {
            Ok(d) => {
                let has_code = d
                    .basic_udi
                    .as_ref()
                    .and_then(|b| b.code.as_ref())
                    .map(|c| !c.trim().is_empty())
                    .unwrap_or(false);
                let has_name = d
                    .device_name
                    .as_ref()
                    .map(|n| !n.trim().is_empty())
                    .unwrap_or(false);
                !(has_code && has_name)
            }
            Err(_) => true,
        },
        Err(_) => true,
    }
}

/// Force-fetch the UDI-DI **detail** record from EUDAMED, overwriting any cached
/// `detail/<uuid>.json`. Mirrors [`fetch_basic_udi_di`]'s hardened shape: a 15 s
/// global timeout + a 4-attempt retry loop with linear backoff, and
/// parse-before-cache — the body is only written when it parses to an
/// `ApiDeviceDetail`, so an error page / partial body can never poison the cache.
///
/// Used by [`force_reload_eudamed`] (Mode 6). Unlike the fetch-on-miss path in
/// [`reconvert_uuids_from_detail`], this refetches **unconditionally**, so a
/// stale or incomplete cached file is replaced with the current EUDAMED record.
fn fetch_detail(uuid: &str, cache_dir: &Path) -> Option<api_detail::ApiDeviceDetail> {
    let url = format!(
        "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData/{}?languageIso2Code=en",
        uuid
    );
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .new_agent();

    for attempt in 1..=4 {
        match agent.get(&url).call() {
            Ok(mut resp) => {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                match serde_json::from_str::<api_detail::ApiDeviceDetail>(&body) {
                    Ok(data) => {
                        let _ = std::fs::create_dir_all(cache_dir);
                        let cache_path = cache_dir.join(format!("{}.json", uuid));
                        if let Err(e) = std::fs::write(&cache_path, &body) {
                            eprintln!("  Warning: failed to cache detail for {}: {}", uuid, e);
                        }
                        return Some(data);
                    }
                    Err(_) => {
                        if attempt < 4 {
                            std::thread::sleep(std::time::Duration::from_secs(attempt));
                            continue;
                        }
                        eprintln!(
                            "  Warning: detail for {} unparseable after {} attempts",
                            uuid, attempt
                        );
                        return None;
                    }
                }
            }
            Err(e) => {
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_secs(attempt));
                    continue;
                }
                eprintln!(
                    "  Warning: failed to fetch detail for {} after {} attempts: {}",
                    uuid, attempt, e
                );
                return None;
            }
        }
    }
    None
}

/// Result of a [`force_reload_eudamed`] run: fresh-record counts plus a breakdown
/// of why Basic UDI-DI refetches that did not succeed failed.
pub(crate) struct ForceReloadStats {
    pub requested: usize,
    pub detail_ok: usize,
    /// Basic UDI-DIs already complete in cache (code + deviceName) → not refetched.
    pub skipped_complete: usize,
    /// Basic UDI-DIs we actually tried to refetch (= requested - skipped_complete).
    pub refetch_attempted: usize,
    pub basic_ok: usize,
    pub basic_429: usize,
    pub basic_404: usize,
    pub basic_http_other: usize,
    pub basic_network: usize,
    pub basic_empty: usize,
}

impl ForceReloadStats {
    /// Basic UDI-DIs we tried to refetch but could not (kept their old file, if
    /// any). Already-complete basics that were intentionally skipped do NOT count
    /// here — only failures among the refetch-attempted set.
    pub fn basic_missing(&self) -> usize {
        self.refetch_attempted.saturating_sub(self.basic_ok)
    }

    /// One-line failure breakdown, e.g. `429×0, 404×4218, 5xx×0, timeout×0, empty×0`.
    /// 404 = no Basic UDI-DI record exists; 429 = EUDAMED throttled us.
    pub fn breakdown(&self) -> String {
        format!(
            "429×{}, 404×{}, 5xx/other×{}, timeout×{}, empty×{}",
            self.basic_429,
            self.basic_404,
            self.basic_http_other,
            self.basic_network,
            self.basic_empty,
        )
    }
}

/// Mode 6 (Force-Reload) helper: re-fetch **detail + Basic UDI-DI** from EUDAMED
/// for every UUID, overwriting the cached `eudamed_json/detail|basic/<uuid>.json`.
///
/// Why this exists: the fetch-on-miss safety net in
/// [`reconvert_uuids_from_detail`] only fills a *genuine* cache miss — it never
/// refreshes a file that is present but **stale or incomplete** (e.g. a Basic
/// UDI-DI cached before EUDAMED populated `deviceName`, which parses fine and so
/// is accepted as-is → empty `globalModelDescription` → 097.025). Mode 6 refetches
/// both records unconditionally and overwrites the cached file on success (a valid,
/// code-carrying body), healing stale, partial, and missing caches in one pass.
/// After this, a normal reconvert reads fresh data.
///
/// IMPORTANT — never delete the existing basic before the refetch: a refetch that
/// fails under EUDAMED throttling would leave the device with no basic at all →
/// `basic_udi=None` → mass 097.025. (That delete-first bug wiped 4218/5330 basics in
/// the v1.0.66 run on FR-MF-000000602 / CH-MF-000009933 / BR-MF-000014512 → 0
/// accepted, 969 rejected.) We keep the old file on failure; concurrency is bounded
/// to stay under EUDAMED's rate limit so the refetch actually succeeds.
///
/// Returns a [`ForceReloadStats`] with the fresh-record counts plus a breakdown
/// of *why* each Basic UDI-DI refetch that did not succeed failed, so the log can
/// state plainly whether it was EUDAMED throttling (429), absent records (404),
/// timeouts, or empty bodies — instead of the old "throttling or no record" guess.
fn force_reload_eudamed(
    uuids: &std::collections::HashSet<String>,
    data_dir: &Path,
    progress: &dyn Fn(&str),
) -> ForceReloadStats {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    let detail_dir = data_dir.join("eudamed_json/detail");
    let basic_dir = data_dir.join("eudamed_json/basic");
    let _ = std::fs::create_dir_all(&detail_dir);
    let _ = std::fs::create_dir_all(&basic_dir);

    let list: Vec<&String> = uuids.iter().collect();

    // --- DETAIL pass: 50 threads. The detail endpoint is NOT tightly rate-limited
    // (Maik's 2026-06-25 run refetched 5372/5372 detail fine at 50 threads), so we
    // keep the proven download width here.
    let detail_ok = AtomicUsize::new(0);
    let detail_pool = rayon::ThreadPoolBuilder::new().num_threads(50).build().ok();
    let detail_run = || {
        list.par_iter().for_each(|uuid| {
            if fetch_detail(uuid, &detail_dir).is_some() {
                detail_ok.fetch_add(1, Ordering::Relaxed);
            }
        });
    };
    match &detail_pool {
        Some(p) => p.install(detail_run),
        None => detail_run(),
    }
    let detail_ok = detail_ok.load(Ordering::Relaxed);
    progress(&format!(
        "Detail refetch: {}/{} fresh (50 threads)",
        detail_ok,
        list.len()
    ));

    // --- BASIC UDI-DI pass: RATE-LIMITED, not concurrent.
    // The Basic-UDI endpoint allows ~60 requests per rolling 60-second window
    // (measured 2026-06-25), then returns 429 + `Retry-After: 60`. At 50 threads it
    // blows that budget in ~1s → 429×4978 of 5372 in Maik's v1.0.69 run, so most
    // stale basics never healed → residual 097.025/097.054. Two fixes:
    //   (1) skip basics that are already complete (code + deviceName present) — only
    //       the genuinely stale/missing handful need healing, which keeps the request
    //       count far under the budget; and
    //   (2) refetch the rest SEQUENTIALLY paced at ~1 req/s, with
    //       `fetch_basic_udi_di_outcome` honoring the 429 `Retry-After` header.
    // Proven harness: 120 paced requests across 2+ rate windows = 0 throttles.
    // We still NEVER delete the old basic first — a failed refetch keeps the
    // existing (stale-but-parseable ≫ absent) file (the v1.0.66 delete-first bug
    // wiped 4218/5330 basics → 0 accepted, 969 rejected).
    let need: Vec<&String> = list
        .iter()
        .copied()
        .filter(|u| basic_needs_refetch(u, &basic_dir))
        .collect();
    let skipped_complete = list.len().saturating_sub(need.len());
    let refetch_attempted = need.len();
    progress(&format!(
        "Basic UDI-DI: {} already complete (skipped), {} to refetch at ≤1 req/s (EUDAMED ~60/min limit)...",
        skipped_complete, refetch_attempted
    ));

    let mut basic_ok = 0usize;
    let mut basic_429 = 0usize;
    let mut basic_404 = 0usize;
    let mut basic_http_other = 0usize;
    let mut basic_network = 0usize;
    let mut basic_empty = 0usize;

    // ~54 req/min (just under the 60/60s budget) — paced by request start time.
    let min_interval = Duration::from_millis(1100);
    let mut last_start: Option<Instant> = None;
    for (i, uuid) in need.iter().enumerate() {
        if let Some(t) = last_start {
            let elapsed = t.elapsed();
            if elapsed < min_interval {
                std::thread::sleep(min_interval - elapsed);
            }
        }
        last_start = Some(Instant::now());
        match fetch_basic_udi_di_outcome(uuid, &basic_dir).1 {
            BasicFetchReason::Ok => basic_ok += 1,
            BasicFetchReason::Http(429) => basic_429 += 1,
            BasicFetchReason::Http(404) => basic_404 += 1,
            BasicFetchReason::Http(_) => basic_http_other += 1,
            BasicFetchReason::Network => basic_network += 1,
            BasicFetchReason::EmptyBody => basic_empty += 1,
        }
        // Live monitoring: progress + throttle state every 25 (and at the end).
        if (i + 1) % 25 == 0 || i + 1 == refetch_attempted {
            progress(&format!(
                "  Basic UDI-DI refetch {}/{} — {} ok, {} throttled(429), {} no-record(404)",
                i + 1,
                refetch_attempted,
                basic_ok,
                basic_429,
                basic_404
            ));
        }
    }

    ForceReloadStats {
        requested: list.len(),
        detail_ok,
        skipped_complete,
        refetch_attempted,
        basic_ok,
        basic_429,
        basic_404,
        basic_http_other,
        basic_network,
        basic_empty,
    }
}

/// Convert EUDAMED JSON → Swissdamed JSON (almost 1:1 mapping)
fn process_swissdamed(detail_dir: &Path, basic_dir: &Path) -> Result<()> {
    use rayon::prelude::*;

    let output_dir = Path::new("swissdamed_json");
    std::fs::create_dir_all(output_dir)?;

    // Collect detail files
    let entries: Vec<_> = std::fs::read_dir(detail_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();

    println!(
        "Processing {} detail files from {}",
        entries.len(),
        detail_dir.display()
    );

    let results: Vec<_> = entries
        .par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_string_lossy().to_string();
            let basic_path = basic_dir.join(format!("{}.json", stem));

            // Read detail JSON
            let detail_json = std::fs::read_to_string(&path).ok()?;
            let device: api_detail::ApiDeviceDetail = serde_json::from_str(&detail_json).ok()?;

            // Read basic JSON
            let basic_json = std::fs::read_to_string(&basic_path).ok()?;
            let basic_udi: api_detail::BasicUdiDiData = serde_json::from_str(&basic_json).ok()?;

            // Determine endpoint and build payload
            let endpoint = swissdamed::legislation_endpoint(&basic_udi);
            let is_spp = basic_udi.is_spp();

            let payload = if is_spp {
                serde_json::to_string_pretty(&swissdamed::to_spp_dto(&device, &basic_udi)).ok()?
            } else {
                serde_json::to_string_pretty(&swissdamed::to_mdr_dto(&device, &basic_udi)).ok()?
            };

            // Write output
            let out_path = output_dir.join(format!("{}.json", stem));
            std::fs::write(&out_path, &payload).ok()?;

            Some((stem, endpoint.to_string()))
        })
        .collect();

    // Summary
    let mut endpoint_counts: HashMap<String, u32> = HashMap::new();
    for (_, endpoint) in &results {
        *endpoint_counts.entry(endpoint.clone()).or_insert(0) += 1;
    }

    println!("Converted {} files to swissdamed_json/", results.len());
    for (endpoint, count) in &endpoint_counts {
        println!("  {}: {}", endpoint, count);
    }

    Ok(())
}

/// Read SRNs from an xlsx file column (1-based)
fn count_srns_xlsx(path: &str, col: usize) -> Result<Vec<String>> {
    use calamine::{open_workbook, Reader, Xlsx};
    let mut workbook: Xlsx<_> =
        open_workbook(path).with_context(|| format!("Cannot open {}", path))?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No sheets in xlsx"))?;
    let range = workbook.worksheet_range(&sheet_name)?;
    let mut srns = Vec::new();
    for row in range.rows().skip(1) {
        // skip header
        if let Some(cell) = row.get(col - 1) {
            let s = cell.to_string().trim().to_string();
            if !s.is_empty() && s.contains("-MF-") {
                srns.push(s);
            }
        }
    }
    Ok(srns)
}

/// Write GTIN counts back to xlsx file: reads original, adds count column
fn write_counts_xlsx(path: &str, srn_col: usize, counts: &HashMap<String, i64>) -> Result<()> {
    use calamine::{open_workbook, Reader, Xlsx};
    use rust_xlsxwriter::Workbook;

    let mut workbook: Xlsx<_> = open_workbook(path)?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("XLSX file has no sheets: {}", path))?;
    let range = workbook.worksheet_range(&sheet_name)?;

    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();

    for (r, row) in range.rows().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            match cell {
                calamine::Data::String(s) => {
                    ws.write_string(r as u32, c as u16, s)?;
                }
                calamine::Data::Float(f) => {
                    ws.write_number(r as u32, c as u16, *f)?;
                }
                calamine::Data::Int(i) => {
                    ws.write_number(r as u32, c as u16, *i as f64)?;
                }
                calamine::Data::Bool(b) => {
                    ws.write_boolean(r as u32, c as u16, *b)?;
                }
                calamine::Data::DateTime(dt) => {
                    ws.write_string(r as u32, c as u16, &dt.to_string())?;
                }
                calamine::Data::DateTimeIso(s) => {
                    ws.write_string(r as u32, c as u16, s)?;
                }
                calamine::Data::DurationIso(s) => {
                    ws.write_string(r as u32, c as u16, s)?;
                }
                calamine::Data::Error(_) | calamine::Data::Empty => {}
            }
        }
        // Add count column
        if r == 0 {
            ws.write_string(r as u32, row.len() as u16, "GTIN_Count")?;
        } else {
            let srn = row
                .get(srn_col - 1)
                .map(|c| c.to_string().trim().to_string())
                .unwrap_or_default();
            if let Some(&count) = counts.get(&srn) {
                if count >= 0 {
                    ws.write_number(r as u32, row.len() as u16, count as f64)?;
                } else {
                    ws.write_string(r as u32, row.len() as u16, "ERROR")?;
                }
            }
        }
    }

    wb.save(path)?;
    Ok(())
}

/// Re-run transform_detail + DraftItemDocument wrap over every detail file in
/// `data_dir/eudamed_json/detail/`, writing to `data_dir/firstbase_json/`.
///
/// If `uuids_filter` is `Some`, only files whose stem matches one of the UUIDs
/// are processed. If `None`, every detail file is regenerated (used by the
/// `regenerate` subcommand).
///
/// Used by `regenerate` (no filter) and by `repush-srn --reconvert` /
/// GUI Mode 5 (filter to one or more SRNs' UUIDs). Ignores `udi_versions`
/// version tracking by design — the targeted files are unconditionally
/// rewritten so the latest converter logic (e.g. new GS1 fields like
/// `DescriptionShort` since v1.0.43) is applied even when the EUDAMED detail
/// JSON itself hasn't changed.
fn reconvert_uuids_from_detail(
    uuids_filter: Option<&std::collections::HashSet<String>>,
    data_dir: &Path,
    fb_config: &config::Config,
) -> (usize, usize) {
    let detail_dir = data_dir.join("eudamed_json/detail");
    let basic_dir = data_dir.join("eudamed_json/basic");
    let output_dir = data_dir.join("firstbase_json");
    let _ = std::fs::create_dir_all(&output_dir);

    let basic_udi_cache = load_basic_udi_cache(&basic_dir);

    let detail_files: Vec<std::path::PathBuf> = match std::fs::read_dir(&detail_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .filter(
                |p| match (uuids_filter, p.file_stem().and_then(|s| s.to_str())) {
                    (Some(set), Some(uuid)) => set.contains(uuid),
                    (None, _) => true,
                    _ => false,
                },
            )
            .collect(),
        Err(_) => return (0, 0),
    };

    let converted = std::sync::atomic::AtomicUsize::new(0);
    let errors = std::sync::atomic::AtomicUsize::new(0);

    detail_files.par_iter().for_each(|detail_path| {
        let uuid = detail_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let json_content = match std::fs::read_to_string(detail_path) {
            Ok(s) => s,
            Err(_) => {
                errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };
        let device: api_detail::ApiDeviceDetail = match serde_json::from_str(&json_content) {
            Ok(d) => d,
            Err(_) => {
                errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };
        // Safety net: fetch the Basic UDI-DI on demand when it is not cached.
        // Without it the converter emits bad defaults (globalModelNumber=GTIN,
        // no MODEL_NUMBER/globalModelDescription/EAR) that GS1 rejects with
        // 097.116/097.025/097.054.
        let fetched_basic;
        let basic_udi = match basic_udi_cache.get(&uuid) {
            Some(b) => Some(b),
            None => {
                fetched_basic = fetch_basic_udi_di(&uuid, &basic_dir);
                fetched_basic.as_ref()
            }
        };
        let doc = transform_detail::transform_detail_document(&device, fb_config, basic_udi, &uuid);
        let draft_doc = firstbase::DraftItemDocument { draft_item: doc };
        let out = match serde_json::to_string_pretty(&draft_doc) {
            Ok(s) => s,
            Err(_) => {
                errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };
        let out_path = output_dir.join(format!("{}.json", uuid));
        if std::fs::write(&out_path, &out).is_err() {
            errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        converted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });

    (
        converted.load(std::sync::atomic::Ordering::Relaxed),
        errors.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Load Basic UDI-DI cache: maps UDI-DI UUID → BasicUdiDiData
fn load_basic_udi_cache(cache_dir: &Path) -> HashMap<String, api_detail::BasicUdiDiData> {
    if !cache_dir.exists() {
        return HashMap::new();
    }
    let entries: Vec<_> = match std::fs::read_dir(cache_dir) {
        Ok(e) => e.filter_map(|e| e.ok()).collect(),
        Err(_) => return HashMap::new(),
    };
    entries
        .par_iter()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let uuid = path.file_stem()?.to_string_lossy().to_string();
                let content = std::fs::read_to_string(&path).ok()?;
                let data = api_detail::parse_basic_udi_di(&content).ok()?;
                Some((uuid, data))
            } else {
                None
            }
        })
        .collect()
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
