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

/// Extract the main TradeItem Gtin (not from children or NextLowerLevel).
/// Finds all "Gtin" occurrences and returns the one that belongs to the
/// top-level TradeItem (before CatalogueItemChildItemLink, after the last
/// top-level key like TargetMarket/TargetSector).
fn extract_gtin(content: &str) -> Option<String> {
    let marker = "\"Gtin\"";
    // Find ALL Gtin occurrences and pick the right one
    // The main TradeItem Gtin is at the top level — typically the last "Gtin"
    // before "CatalogueItemChildItemLink" or end of TradeItem.
    let child_boundary = content.find("\"CatalogueItemChildItemLink\"")
        .unwrap_or(content.len());

    let mut last_gtin = None;
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(marker) {
        let abs_pos = search_from + pos;
        if abs_pos >= child_boundary {
            break;
        }
        // Extract the value
        let after = &content[abs_pos + marker.len()..];
        let after = after.trim_start();
        if let Some(after) = after.strip_prefix(':') {
            let after = after.trim_start();
            if let Some(after) = after.strip_prefix('"') {
                if let Some(end) = after.find('"') {
                    let val = &after[..end];
                    if !val.is_empty() {
                        last_gtin = Some(val.to_string());
                    }
                }
            }
        }
        search_from = abs_pos + marker.len();
    }
    last_gtin
}
