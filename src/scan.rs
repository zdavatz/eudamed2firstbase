//! Fast parallel scanner for firstbase JSON files.
//! Outputs one line per file: "FILEPATH GTIN" (or skips files without numeric GTIN).
//! Used by push_to_api.sh instead of per-file Python calls.

use rayon::prelude::*;
use std::path::Path;

pub fn scan_dir(input_dir: &Path) -> anyhow::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(input_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".json") && !name.starts_with("firstbase_")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let results: Vec<Option<String>> = entries
        .par_iter()
        .map(|entry| {
            let path = entry.path();
            let content = std::fs::read_to_string(&path).ok()?;

            // Fast string search for Gtin field
            let gtin = extract_gtin(&content)?;

            // Only numeric GTINs (skip HIBC/IFA)
            if !gtin.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }

            Some(format!("{}\t{}", path.display(), gtin))
        })
        .collect();

    let mut live = 0;
    let mut skipped = 0;
    let mut lines = Vec::new();

    for result in results {
        if let Some(line) = result {
            live += 1;
            lines.push(line);
        } else {
            skipped += 1;
        }
    }

    // Print summary to stderr, data to stdout
    eprintln!("Found {} JSON files in {}/ ({} live, {} skipped no GTIN)",
        live + skipped, input_dir.display(), live, skipped);

    for line in &lines {
        println!("{}", line);
    }

    Ok(())
}

/// Extract Gtin value from JSON string without full parsing (fast path)
fn extract_gtin(content: &str) -> Option<String> {
    // Look for "Gtin": "..." pattern in DraftItem.TradeItem
    let marker = "\"Gtin\"";
    let pos = content.find(marker)?;
    let after = &content[pos + marker.len()..];
    // Skip whitespace and colon
    let after = after.trim_start();
    let after = after.strip_prefix(':')?;
    let after = after.trim_start();
    // Extract quoted value
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    let gtin = &after[..end];
    if gtin.is_empty() {
        return None;
    }
    Some(gtin.to_string())
}
