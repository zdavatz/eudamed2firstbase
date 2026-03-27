mod api_detail;
mod api_json;
mod config;
mod eudamed;
mod eudamed_json;
mod firstbase;
mod mappings;
mod transform;
mod transform_api;
mod transform_detail;
mod transform_eudamed_json;
mod version_db;
mod scan;
mod swissdamed;
mod xlsx_export;

use anyhow::{Context, Result};
use chrono::Local;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

/// Default directory for cached Basic UDI-DI data
const BASIC_UDI_CACHE_DIR: &str = "/tmp/basic_udi_cache";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let config_path = Path::new("config.toml");
    let config = config::load_config(config_path)
        .context("Failed to load config.toml")?;

    match args.get(1).map(|s| s.as_str()) {
        Some("ndjson") => {
            // Process NDJSON file(s) from ndjson/ directory (listing format)
            let input_dir = args.get(2).map(|s| s.as_str()).unwrap_or("ndjson");
            process_ndjson(Path::new(input_dir), &config)
        }
        Some("eudamed_json") => {
            // Process individual EUDAMED JSON files (one-to-one)
            let input_dir = args.get(2).map(|s| s.as_str()).unwrap_or("eudamed_json");
            process_eudamed_json_dir(Path::new(input_dir), &config)
        }
        Some("scan") => {
            // Fast parallel scan of firstbase JSON files — outputs "filepath\tGTIN" per line
            let input_dir = args.get(2).map(|s| s.as_str()).unwrap_or("firstbase_json");
            scan::scan_dir(Path::new(input_dir))
        }
        Some("xlsx") => {
            // Convert detail NDJSON to XLSX
            let input_file = args.get(2).map(|s| s.as_str())
                .unwrap_or("ndjson/eudamed_10k_details.ndjson");
            let basic_udi_cache = load_basic_udi_cache(Path::new(BASIC_UDI_CACHE_DIR));
            if !basic_udi_cache.is_empty() {
                println!("Loaded {} Basic UDI-DI records from cache", basic_udi_cache.len());
            }
            let result = xlsx_export::ndjson_to_xlsx(Path::new(input_file), &basic_udi_cache)?;
            println!("  -> {}", result);
            Ok(())
        }
        Some("detail") => {
            // Process detail NDJSON, optionally merging with listing data
            let detail_file = args.get(2).map(|s| s.as_str())
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
            let dest = processed_dir.join(path.file_name().unwrap());
            if let Err(e) = std::fs::rename(path, &dest) {
                eprintln!("  Warning: could not move {} to processed/: {}", path.display(), e);
            }
        }
        println!("Moved {} file(s) to {}", processed_files.len(), processed_dir.display());
    }

    println!("\nProcessed {} XML file(s)", processed);
    Ok(())
}

fn process_xml_file(input_path: &Path, output_dir: &Path, config: &config::Config) -> Result<String> {
    let xml_content = std::fs::read_to_string(input_path)
        .context("Failed to read XML file")?;

    let response = eudamed::parse_pull_response(&xml_content)
        .context("Failed to parse EUDAMED XML")?;

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

    let file = std::fs::File::open(input_path)
        .context("Failed to open NDJSON file")?;
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
        println!("  Loaded {} listing records for merging", listing_index.len());
    }

    // Load Basic UDI-DI cache
    let basic_udi_cache = load_basic_udi_cache(Path::new(BASIC_UDI_CACHE_DIR));
    if !basic_udi_cache.is_empty() {
        println!("  Loaded {} Basic UDI-DI records from cache", basic_udi_cache.len());
    }

    let file = std::fs::File::open(detail_path)
        .with_context(|| format!("Failed to open {}", detail_path.display()))?;
    let reader = std::io::BufReader::new(file);

    // Read all lines first
    let lines: Vec<(usize, String)> = reader.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.ok()?;
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() { None } else { Some((i + 1, trimmed)) }
        })
        .collect();

    // Process lines in parallel
    let results: Vec<Result<firstbase::DraftItemDocument, (usize, String)>> = lines.par_iter()
        .map(|(line_num, trimmed)| {
            match api_detail::parse_api_detail(trimmed) {
                Ok(detail) => {
                    let uuid = detail.uuid.clone().unwrap_or_default();
                    let basic_udi = basic_udi_cache.get(&uuid);
                    let mut document = transform_detail::transform_detail_document(&detail, config, basic_udi, &uuid);

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
            trade_item
                .classification
                .additional_classifications
                .insert(
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
    let has_ema = trade_item.contact_information.iter().any(|c| c.contact_type.value == "EMA");
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
    let has_ear = trade_item.contact_information.iter().any(|c| c.contact_type.value == "EAR");
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
    let conn = version_db::open_db(db_path)
        .context("Failed to open version tracking DB")?;
    let existing_count = version_db::count_records(&conn)?;
    println!("Version DB: {} existing records ({})", existing_count, db_path.display());

    // Load Basic UDI-DI cache
    let cache_dir = Path::new(BASIC_UDI_CACHE_DIR);
    let mut basic_udi_cache = load_basic_udi_cache(cache_dir);
    if !basic_udi_cache.is_empty() {
        println!("Loaded {} Basic UDI-DI records from cache", basic_udi_cache.len());
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

            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();

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
                    let trade_item = transform_eudamed_json::transform_eudamed_device(&device, config);
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

    // Move successfully processed files to eudamed_json/processed/
    if !processed_files.is_empty() {
        std::fs::create_dir_all(&processed_dir)?;
        for path in &processed_files {
            let dest = processed_dir.join(path.file_name().unwrap());
            if let Err(e) = std::fs::rename(path, &dest) {
                eprintln!("  Warning: could not move {} to processed/: {}", path.display(), e);
            }
        }
        println!("Moved {} file(s) to {}", processed_files.len(), processed_dir.display());
    }

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
                eprintln!("  Warning: failed to read Basic UDI-DI response for {}: {}", uuid, e);
                return None;
            }
        },
        Err(e) => {
            eprintln!("  Warning: failed to fetch Basic UDI-DI for {}: {}", uuid, e);
            return None;
        }
    };

    // Save to cache
    let _ = std::fs::create_dir_all(cache_dir);
    let cache_path = cache_dir.join(format!("{}.json", uuid));
    if let Err(e) = std::fs::write(&cache_path, &body) {
        eprintln!("  Warning: failed to cache Basic UDI-DI for {}: {}", uuid, e);
    }

    match api_detail::parse_basic_udi_di(&body) {
        Ok(data) => Some(data),
        Err(e) => {
            eprintln!("  Warning: failed to parse Basic UDI-DI for {}: {}", uuid, e);
            None
        }
    }
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
    entries.par_iter()
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
