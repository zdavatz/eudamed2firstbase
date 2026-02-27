mod config;
mod eudamed;
mod firstbase;
mod mappings;
mod transform;

use anyhow::{Context, Result};
use chrono::Local;
use std::path::Path;

fn main() -> Result<()> {
    let config_path = Path::new("config.toml");
    let config = config::load_config(config_path)
        .context("Failed to load config.toml")?;

    let input_dir = Path::new("xml");
    let output_dir = Path::new("json");

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(output_dir)?;

    // Process all XML files in input directory
    let mut processed = 0;
    for entry in std::fs::read_dir(input_dir).context("Failed to read xml/ directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "xml").unwrap_or(false) {
            println!("Processing: {}", path.display());
            match process_file(&path, output_dir, &config) {
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

    println!("\nProcessed {} file(s)", processed);
    Ok(())
}

fn process_file(input_path: &Path, output_dir: &Path, config: &config::Config) -> Result<String> {
    let xml_content = std::fs::read_to_string(input_path)
        .context("Failed to read XML file")?;

    let response = eudamed::parse_pull_response(&xml_content)
        .context("Failed to parse EUDAMED XML")?;

    let document = transform::transform(&response, config)
        .context("Failed to transform to firstbase format")?;

    // Generate output filename: firstbase_dd.mm.yyyy.json
    let now = Local::now();
    let filename = format!("firstbase_{}.json", now.format("%d.%m.%Y"));
    let output_path = output_dir.join(&filename);

    let json = serde_json::to_string_pretty(&document)?;
    std::fs::write(&output_path, json)?;

    Ok(output_path.display().to_string())
}
