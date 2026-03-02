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

use anyhow::{Context, Result};
use chrono::Local;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

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
    std::fs::create_dir_all(output_dir)?;

    let mut processed = 0;
    for entry in std::fs::read_dir(input_dir).context("Failed to read xml/ directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "xml").unwrap_or(false) {
            println!("Processing: {}", path.display());
            match process_xml_file(&path, output_dir, config) {
                Ok(output_path) => {
                    println!("  -> {}", output_path);
                    processed += 1;
                }
                Err(e) => {
                    eprintln!("  Error: {:#}", e);
                }
            }
        }
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
                trade_items.push(firstbase::FirstbaseDocument {
                    trade_item,
                    children: Vec::new(),
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

    let file = std::fs::File::open(detail_path)
        .with_context(|| format!("Failed to open {}", detail_path.display()))?;
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

        match api_detail::parse_api_detail(trimmed) {
            Ok(detail) => {
                let mut trade_item = transform_detail::transform_detail_device(&detail, config);

                // Merge listing data (manufacturer, AR, risk class, basic UDI)
                let gtin = &trade_item.gtin;
                if let Some(listing) = listing_index.get(gtin) {
                    merge_listing_data(&mut trade_item, listing);
                }

                trade_items.push(firstbase::FirstbaseDocument {
                    trade_item,
                    children: Vec::new(),
                });
            }
            Err(e) => {
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
    }

    // Add manufacturer contact
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

    // Add authorised representative contact
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

/// Process individual EUDAMED JSON files from a directory.
/// Each input file produces one output file (one-to-one mapping).
fn process_eudamed_json_dir(input_dir: &Path, config: &config::Config) -> Result<()> {
    let output_dir = Path::new("firstbase_json");
    std::fs::create_dir_all(output_dir)?;

    let mut processed = 0;
    let mut errors = 0;

    for entry in std::fs::read_dir(input_dir).context("Failed to read eudamed_json/ directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            let json_content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;

            // Detect file type: UDI-DI level (has primaryDi) vs device level
            let is_udi_di = json_content.contains("\"primaryDi\"");

            let result = if is_udi_di {
                // UDI-DI level file — reuse existing api_detail parser/transformer
                api_detail::parse_api_detail(&json_content).map(|detail| {
                    transform_detail::transform_detail_device(&detail, config)
                })
            } else {
                // Device level file (Basic UDI-DI)
                eudamed_json::parse_eudamed_json(&json_content).map(|device| {
                    transform_eudamed_json::transform_eudamed_device(&device, config)
                })
            };

            match result {
                Ok(trade_item) => {
                    let document = firstbase::FirstbaseDocument {
                        trade_item,
                        children: Vec::new(),
                    };

                    let filename = path.file_name().unwrap_or_default().to_string_lossy();
                    let output_path = output_dir.join(filename.as_ref());

                    let json = serde_json::to_string_pretty(&document)?;
                    std::fs::write(&output_path, &json)?;

                    processed += 1;
                }
                Err(e) => {
                    eprintln!("  Error in {}: {:#}", path.display(), e);
                    errors += 1;
                }
            }
        }
    }

    println!(
        "Processed {} EUDAMED JSON file(s) ({} errors) -> {}",
        processed,
        errors,
        output_dir.display()
    );
    Ok(())
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
