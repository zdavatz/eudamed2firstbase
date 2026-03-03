use anyhow::{Context, Result};
use rust_xlsxwriter::{Format, Workbook};
use std::io::BufRead;
use std::path::Path;

use crate::api_detail;

/// Convert a detail NDJSON file to XLSX, writing to xlsx/<stem>.xlsx
pub fn ndjson_to_xlsx(input_path: &Path) -> Result<String> {
    let output_dir = Path::new("xlsx");
    std::fs::create_dir_all(output_dir)?;

    let file = std::fs::File::open(input_path)
        .with_context(|| format!("Failed to open {}", input_path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    // Header format
    let header_fmt = Format::new().set_bold();

    // Write headers
    let headers = [
        "UUID",
        "Primary DI",
        "Issuing Agency",
        "Trade Name (EN)",
        "Reference",
        "Device Status",
        "Sterile",
        "Single Use",
        "Latex",
        "Reprocessed",
        "Base Quantity",
        "Direct Marking",
        "Clinical Sizes",
        "Markets",
        "Additional Info URL",
        "Version Date",
    ];
    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string_with_format(0, col as u16, *header, &header_fmt)?;
    }

    let mut row: u32 = 1;
    let mut errors = 0;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match api_detail::parse_api_detail(trimmed) {
            Ok(detail) => {
                worksheet.write_string(row, 0, detail.uuid.as_deref().unwrap_or(""))?;

                if let Some(ref di) = detail.primary_di {
                    worksheet.write_string(row, 1, di.code.as_deref().unwrap_or(""))?;
                    if let Some(ref agency) = di.issuing_agency {
                        worksheet.write_string(row, 2, agency.code.as_deref().unwrap_or(""))?;
                    }
                }

                // Trade name: first English text, fallback to first available
                if let Some(ref tn) = detail.trade_name {
                    let text = tn.texts.as_ref()
                        .and_then(|texts| {
                            texts.iter()
                                .find(|t| t.language.as_ref()
                                    .and_then(|l| l.iso_code.as_deref())
                                    .map(|c| c == "en")
                                    .unwrap_or(false))
                                .or_else(|| texts.first())
                        })
                        .and_then(|t| t.text.as_deref())
                        .unwrap_or("");
                    worksheet.write_string(row, 3, text)?;
                }

                worksheet.write_string(row, 4, detail.reference.as_deref().unwrap_or(""))?;

                if let Some(ref status) = detail.device_status {
                    if let Some(ref st) = status.status_type {
                        let code = st.code.as_deref().unwrap_or("");
                        let short = code.strip_prefix("refdata.device-model-status.")
                            .unwrap_or(code);
                        worksheet.write_string(row, 5, short)?;
                    }
                }

                worksheet.write_string(row, 6, &bool_str(detail.sterile))?;
                worksheet.write_string(row, 7, &bool_str(detail.single_use))?;
                worksheet.write_string(row, 8, &bool_str(detail.latex))?;
                worksheet.write_string(row, 9, &bool_str(detail.reprocessed))?;

                if let Some(qty) = detail.base_quantity {
                    worksheet.write_number(row, 10, qty as f64)?;
                }

                worksheet.write_string(row, 11, &bool_str(detail.direct_marking))?;

                // Clinical sizes count
                let cs_count = detail.clinical_sizes.as_ref().map(|v| v.len()).unwrap_or(0);
                if cs_count > 0 {
                    worksheet.write_number(row, 12, cs_count as f64)?;
                }

                // Markets: comma-joined ISO2 codes
                if let Some(ref mil) = detail.market_info_link {
                    if let Some(ref markets) = mil.ms_where_available {
                        let codes: Vec<&str> = markets.iter()
                            .filter_map(|m| m.country.as_ref()
                                .and_then(|c| c.iso2_code.as_deref()))
                            .collect();
                        if !codes.is_empty() {
                            worksheet.write_string(row, 13, &codes.join(", "))?;
                        }
                    }
                }

                worksheet.write_string(row, 14, detail.additional_information_url.as_deref().unwrap_or(""))?;
                worksheet.write_string(row, 15, detail.version_date.as_deref().unwrap_or(""))?;

                row += 1;
            }
            Err(e) => {
                if errors < 5 {
                    eprintln!("  Line error: {}", e);
                }
                errors += 1;
            }
        }
    }

    // Auto-fit column widths
    worksheet.autofit();

    let stem = input_path.file_stem().unwrap_or_default().to_string_lossy();
    let output_path = output_dir.join(format!("{}.xlsx", stem));
    workbook.save(&output_path)?;

    Ok(format!(
        "{} ({} devices, {} errors)",
        output_path.display(),
        row - 1,
        errors
    ))
}

fn bool_str(val: Option<bool>) -> String {
    match val {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => String::new(),
    }
}
