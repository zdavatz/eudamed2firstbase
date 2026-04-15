// Hide console window on Windows when running as GUI
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_detail;
mod api_json;
mod config;
mod download;
mod eudamed;
mod eudamed_json;
mod firstbase;
mod gui;
mod mail;
mod mappings;
mod scan;
mod swissdamed;
mod transform;
mod transform_api;
mod transform_detail;
mod transform_eudamed_json;
mod version_db;
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
        Some("check") => {
            // Check SRNs for updates, download changed, convert, and push to Firstbase
            // Usage: cargo run check /tmp/srn_update [--threads N]
            let file = args.get(2).unwrap_or_else(|| {
                eprintln!("Usage: eudamed2firstbase check <srn_file> [--threads N]");
                eprintln!("  Reads SRNs from file, checks for updates, downloads changed,");
                eprintln!("  converts to firstbase JSON, and pushes to GS1 Firstbase API.");
                std::process::exit(1);
            });
            let threads: Option<usize> = args
                .iter()
                .position(|a| a == "--threads")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok());

            let srns: Vec<String> = std::fs::read_to_string(file)?
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if srns.is_empty() {
                eprintln!("No SRNs found in {}", file);
                std::process::exit(1);
            }
            eprintln!("=== Check {} SRNs from {} ===", srns.len(), file);

            // Step 1: Download (with version check)
            let mut dl_config = download::DownloadConfig {
                srns: srns.clone(),
                limit: None,
                ..Default::default()
            };
            if let Some(t) = threads {
                dl_config.parallel_threads = t;
                dl_config.listing_threads = t;
            }
            let progress = download::StderrProgress;
            let result = download::run_download(&dl_config, &progress)?;

            if result.need_download.is_empty() {
                eprintln!(
                    "\nNo updates found. All {} devices unchanged.",
                    result.uuid_versions.len()
                );
                return Ok(());
            }
            eprintln!(
                "\n{} new/changed devices (of {} total)",
                result.need_download.len(),
                result.uuid_versions.len()
            );

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
            let mut converted = 0;
            for uuid in &result.need_download {
                let detail_path = detail_dir.join(format!("{}.json", uuid));
                if !detail_path.exists() {
                    continue;
                }
                let json_content = match std::fs::read_to_string(&detail_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let device: api_detail::ApiDeviceDetail = match serde_json::from_str(&json_content)
                {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let basic_udi = basic_udi_cache.get(uuid);

                let doc = transform_detail::transform_detail_document(
                    &device, &fb_config, basic_udi, uuid,
                );
                let out = match serde_json::to_string_pretty(&doc) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Failed to serialize firstbase doc for {}: {}", uuid, e);
                        std::process::exit(1);
                    }
                };
                let out_path = output_dir.join(format!("{}.json", uuid));
                let _ = std::fs::write(&out_path, &out);
                converted += 1;

                // Update version DB
                let mut version_rec = version_db::extract_detail_versions(&json_content);
                version_rec.last_synced = Some(now_str.clone());
                let _ = version_db::upsert_version(&conn, &version_rec);
            }
            eprintln!("Converted {} devices to firstbase_json/", converted);

            if converted == 0 {
                eprintln!("Nothing to push.");
                return Ok(());
            }

            // Step 3: Push to Firstbase API
            eprintln!("\n=== Pushing to GS1 Firstbase API ===");
            let email = std::env::var("FIRSTBASE_EMAIL").unwrap_or_default();
            let password = std::env::var("FIRSTBASE_PASSWORD").unwrap_or_default();
            // Env var takes priority; fall back to publish_gln from config.toml.
            let publish_gln = match std::env::var("FIRSTBASE_PUBLISH_GLN") {
                Ok(v) if !v.is_empty() => v,
                _ if !fb_config.provider.publish_gln.is_empty() => {
                    fb_config.provider.publish_gln.clone()
                }
                _ => {
                    eprintln!("Set FIRSTBASE_PUBLISH_GLN (recipient GLN) or add publish_gln under [provider] in config.toml. Skipping push.");
                    return Ok(());
                }
            };
            if email.is_empty() || password.is_empty() {
                eprintln!("Set FIRSTBASE_EMAIL and FIRSTBASE_PASSWORD to push. Skipping push.");
                return Ok(());
            }

            // provider_gln comes from config.toml, not a hardcoded default.
            let settings = gui::Settings {
                firstbase_email: email,
                firstbase_password: password,
                publish_to_gln: publish_gln,
                provider_gln: fb_config.provider.gln.clone(),
                ..Default::default()
            };
            let log_fn = |msg: &str| {
                eprintln!("{}", msg);
            };
            match gui::push_to_firstbase(&settings, &log_fn) {
                Ok((accepted, rejected)) => {
                    eprintln!("\nDone: {} accepted, {} rejected.", accepted, rejected);
                }
                Err(e) => {
                    eprintln!("\nPush failed: {}", e);
                }
            }
            Ok(())
        }
        Some("download") => {
            // Download from EUDAMED API (replaces download.sh)
            let (srns, limit, threads) = parse_download_args(&args[2..]);
            if srns.is_empty() && limit.is_none() {
                eprintln!(
                    "Usage: eudamed2firstbase download [--N] [--srn <SRN> ...] [--threads N]"
                );
                eprintln!("  --N              Number of products per SRN (e.g. --10, --100)");
                eprintln!("  --srn <SRN> ...  Filter by manufacturer/AR SRN(s)");
                eprintln!(
                    "  --threads N      Parallel threads for listings (default 10) and downloads"
                );
                std::process::exit(1);
            }
            let mut dl_config = download::DownloadConfig {
                srns,
                limit,
                ..Default::default()
            };
            if let Some(t) = threads {
                dl_config.parallel_threads = t;
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
            // Auto-convert if --convert flag present
            if args.iter().any(|a| a == "--convert") {
                eprintln!("\n=== Converting to firstbase JSON ===");
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
            // Send file as email attachment via Gmail API.
            // Credentials default to [gmail] in config.toml; --p12 overrides the key path.
            // Usage: cargo run mailto <file> --to <email> [--from <email>] [--subject <text>] [--p12 <key>]
            let mut file = None;
            let mut to = None;
            let mut subject = None;
            let mut from: Option<String> = None;
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
                    "--p12" => {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            p12 = v.clone();
                        }
                    }
                    _ if file.is_none() => {
                        file = Some(args[i].clone());
                    }
                    _ => {}
                }
                i += 1;
            }
            let file = file.unwrap_or_else(|| {
                eprintln!("Usage: eudamed2firstbase mailto <file> --to <email> [--from <email>] [--subject <text>] [--p12 <key>]");
                std::process::exit(1);
            });
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
                    std::path::Path::new(&file)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&file)
                )
            });

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
            mail::send_email_with_attachment(
                &p12,
                &service_email,
                &from,
                &to,
                &subject,
                &format!(
                    "File attached: {}",
                    std::path::Path::new(&file)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&file)
                ),
                &file,
            )?;
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

fn parse_download_args(args: &[String]) -> (Vec<String>, Option<usize>, Option<usize>) {
    let mut srns = Vec::new();
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

    (srns, limit, threads)
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
    // Set basic UDI as global model number
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
                // Unchanged — skip conversion, just move file
                skipped += 1;
                processed_files.push(path);
                continue;
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
fn fetch_basic_udi_di(uuid: &str, cache_dir: &Path) -> Option<api_detail::BasicUdiDiData> {
    let url = format!(
        "https://ec.europa.eu/tools/eudamed/api/devices/basicUdiData/udiDiData/{}?languageIso2Code=en",
        uuid
    );
    let body = match ureq::get(&url).call() {
        Ok(resp) => match resp.into_body().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "  Warning: failed to read Basic UDI-DI response for {}: {}",
                    uuid, e
                );
                return None;
            }
        },
        Err(e) => {
            eprintln!(
                "  Warning: failed to fetch Basic UDI-DI for {}: {}",
                uuid, e
            );
            return None;
        }
    };

    // Save to cache
    let _ = std::fs::create_dir_all(cache_dir);
    let cache_path = cache_dir.join(format!("{}.json", uuid));
    if let Err(e) = std::fs::write(&cache_path, &body) {
        eprintln!(
            "  Warning: failed to cache Basic UDI-DI for {}: {}",
            uuid, e
        );
    }

    match api_detail::parse_basic_udi_di(&body) {
        Ok(data) => Some(data),
        Err(e) => {
            eprintln!(
                "  Warning: failed to parse Basic UDI-DI for {}: {}",
                uuid, e
            );
            None
        }
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
