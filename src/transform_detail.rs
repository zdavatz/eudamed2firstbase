use crate::api_detail::ApiDeviceDetail;
use crate::config::Config;
use crate::firstbase::*;
use crate::mappings;
use chrono::Local;

/// Transform a full API device detail record into a firstbase TradeItem.
pub fn transform_detail_device(device: &ApiDeviceDetail, config: &Config) -> TradeItem {
    let now = Local::now();
    let now_str = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    let gtin = device.gtin();

    // --- Risk class from CND nomenclatures or linked data ---
    // The detail endpoint doesn't directly expose risk class, but we can
    // derive it from the linked basic UDI data if available. For now,
    // we leave it empty (it was populated from the listing data).
    let additional_classifications = Vec::new();

    // --- Device status ---
    let status_code = device
        .status_code()
        .map(|s| mappings::device_status_to_gs1(&s).to_string())
        .unwrap_or_default();

    // --- Production identifiers ---
    let production_ids: Vec<CodeValue> = device
        .production_identifiers()
        .into_iter()
        .map(|id| CodeValue { value: id })
        .collect();

    // --- Sterility ---
    let sterility = build_sterility(device, config);

    // --- Reusability ---
    let reusability = build_reusability(device);

    // --- Contacts (manufacturer/AR not available in detail, will be merged from listing) ---
    let contacts = Vec::new();

    // --- Trade name / description ---
    let trade_names = device.trade_name_texts();
    let additional_descs = device.additional_description_texts();
    let description_module = if !trade_names.is_empty() || !additional_descs.is_empty() {
        Some(TradeItemDescriptionModule {
            info: TradeItemDescriptionInformation {
                descriptions: trade_names
                    .iter()
                    .map(|(lang, text)| LangValue {
                        language_code: lang.clone(),
                        value: text.clone(),
                    })
                    .collect(),
                additional_descriptions: additional_descs
                    .iter()
                    .map(|(lang, text)| LangValue {
                        language_code: lang.clone(),
                        value: text.clone(),
                    })
                    .collect(),
            },
        })
    } else {
        None
    };

    // --- Reference → additional identification ---
    let mut additional_identification = Vec::new();
    if let Some(ref reference) = device.reference {
        if reference != "-" && !reference.is_empty() {
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: "MANUFACTURER_PART_NUMBER".to_string(),
                value: reference.clone(),
            });
        }
    }

    // --- EMDN/CND nomenclature → additional classification system 88 ---
    let mut emdn_classifications = Vec::new();
    if let Some(ref cnds) = device.cnd_nomenclatures {
        for cnd in cnds {
            if let Some(ref code) = cnd.code {
                emdn_classifications.push(AdditionalClassification {
                    system_code: CodeValue {
                        value: "88".to_string(),
                    },
                    values: vec![AdditionalClassificationValue {
                        code_value: code.clone(),
                    }],
                });
            }
        }
    }

    let mut all_classifications = additional_classifications;
    all_classifications.extend(emdn_classifications);

    // --- Healthcare item module (clinical sizes, storage, warnings, latex, tissue) ---
    let healthcare_module = build_healthcare_module(device);

    // --- Referenced file module (IFU URL) ---
    let referenced_file_module = device.additional_information_url.as_ref().map(|url| {
        ReferencedFileDetailInformationModule {
            headers: vec![ReferencedFileHeader {
                media_source_gln: None,
                mime_type: None,
                file_type: CodeValue {
                    value: "IFU".to_string(),
                },
                format_name: None,
                file_name: None,
                uri: url.clone(),
                is_primary: "TRUE".to_string(),
            }],
        }
    });

    // --- Sales module (market availability) ---
    let sales_module = build_sales_module(device);

    // --- Base quantity → device count ---
    let device_count = device.base_quantity;

    TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["HEALTHCARE".to_string()],
        chemical_regulation_module: None,
        healthcare_item_module: healthcare_module,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: None, // Not in detail endpoint directly
                device_count,
                direct_marking: Vec::new(),
                measuring_function: None,
                is_active: None,
                administer_medicine: None,
                is_medicinal_product: None,
                is_reprocessed: device.reprocessed,
                is_reusable_surgical: None,
                production_identifier_types: production_ids,
                annex_xvi_types: Vec::new(),
                multi_component_type: None,
                eu_status: CodeValue {
                    value: status_code,
                },
                reusability,
                sterility,
            },
        },
        referenced_file_module: referenced_file_module,
        regulated_trade_item_module: None,
        sales_module,
        description_module,
        is_base_unit: true,
        is_despatch_unit: false,
        is_orderable_unit: true,
        unit_descriptor: CodeValue {
            value: "BASE_UNIT_OR_EACH".to_string(),
        },
        trade_channel_code: Vec::new(),
        information_provider: InformationProvider {
            gln: config.provider.gln.clone(),
            party_name: config.provider.party_name.clone(),
        },
        classification: GdsnClassification {
            segment_code: config.gpc.segment_code.clone(),
            class_code: config.gpc.class_code.clone(),
            family_code: config.gpc.family_code.clone(),
            category_code: config.gpc.category_code.clone(),
            category_name: config.gpc.category_name.clone(),
            additional_classifications: all_classifications,
        },
        next_lower_level: None,
        target_market: TargetMarketObj {
            country_code: CodeValue {
                value: config.target_market.country_code.clone(),
            },
        },
        contact_information: contacts,
        synchronisation_dates: TradeItemSynchronisationDates {
            last_change: now_str.clone(),
            effective: now_str.clone(),
            publication: now_str,
        },
        global_model_info: vec![GlobalModelInformation {
            number: String::new(), // Will be merged from listing data (basicUdi)
            descriptions: Vec::new(),
        }],
        gtin,
        additional_identification,
    }
}

fn build_sterility(device: &ApiDeviceDetail, config: &Config) -> Option<SterilityInformation> {
    let sterile = device.sterile?;
    let sterilization = device.sterilization.unwrap_or(false);

    let manufacturer_sterilisation = if sterile {
        vec![CodeValue {
            value: config
                .sterilisation_method
                .clone()
                .unwrap_or_else(|| "UNSPECIFIED".to_string()),
        }]
    } else {
        vec![CodeValue {
            value: "NOT_STERILISED".to_string(),
        }]
    };

    let prior_to_use = if sterilization {
        vec![CodeValue {
            value: "STERILISE_BEFORE_USE".to_string(),
        }]
    } else {
        Vec::new()
    };

    Some(SterilityInformation {
        manufacturer_sterilisation,
        prior_to_use,
    })
}

fn build_reusability(device: &ApiDeviceDetail) -> Option<ReusabilityInformation> {
    let single_use = device.single_use?;

    if single_use {
        Some(ReusabilityInformation {
            reusability_type: CodeValue {
                value: "SINGLE_USE".to_string(),
            },
            max_cycles: None,
        })
    } else {
        let max = device.max_number_of_reuses;
        Some(ReusabilityInformation {
            reusability_type: CodeValue {
                value: "LIMITED_REUSABLE".to_string(),
            },
            max_cycles: max,
        })
    }
}

fn build_healthcare_module(device: &ApiDeviceDetail) -> Option<HealthcareItemInformationModule> {
    let clinical_sizes = build_clinical_sizes(device);
    let storage_handling = build_storage_handling(device);
    let clinical_warnings = build_clinical_warnings(device);
    let contains_latex = device.latex.map(|b| bool_str(b));

    // Only produce the module if there's something to put in it
    if clinical_sizes.is_empty()
        && storage_handling.is_empty()
        && clinical_warnings.is_empty()
        && contains_latex.is_none()
    {
        return None;
    }

    Some(HealthcareItemInformationModule {
        info: HealthcareItemInformation {
            human_blood_derivative: None,
            contains_latex,
            human_tissue: None,
            animal_tissue: None,
            storage_handling,
            clinical_sizes,
            clinical_warnings,
        },
    })
}

fn build_clinical_sizes(device: &ApiDeviceDetail) -> Vec<ClinicalSizeOutput> {
    let sizes = match device.clinical_sizes.as_ref() {
        Some(s) if !s.is_empty() => s,
        _ => return Vec::new(),
    };

    sizes
        .iter()
        .filter_map(|cs| {
            let type_code_raw = cs.size_type.as_ref()?.code.as_ref()?;
            let cst_code = extract_cst_code(type_code_raw);
            let gs1_type = mappings::clinical_size_type_to_gs1(&cst_code);

            let precision_raw = cs
                .precision
                .as_ref()
                .and_then(|p| p.code.as_ref())
                .map(|c| extract_last_segment(c))
                .unwrap_or_else(|| "TEXT".to_string())
                .to_uppercase();

            let precision_code = match precision_raw.as_str() {
                "TEXT" => "TEXT",
                "EXACT" | "VALUE" => "EXACT",
                "APPROXIMATELY" | "APPROX" => "APPROXIMATELY",
                "RANGE" => "RANGE",
                other => other,
            };

            // Build measurement values
            let unit_code = cs
                .metric_of_measurement
                .as_ref()
                .and_then(|m| m.code.as_ref())
                .map(|c| {
                    let mu_code = extract_mu_code(c);
                    mappings::measurement_unit_to_gs1(&mu_code).to_string()
                })
                .unwrap_or_default();

            let mut values = Vec::new();
            let mut maximums = Vec::new();

            if let Some(v) = cs.value {
                values.push(MeasurementValue {
                    unit_code: unit_code.clone(),
                    value: v,
                });
            } else if let Some(min) = cs.minimum_value {
                values.push(MeasurementValue {
                    unit_code: unit_code.clone(),
                    value: min,
                });
            }

            if let Some(max) = cs.maximum_value {
                maximums.push(MeasurementValue {
                    unit_code: unit_code.clone(),
                    value: max,
                });
            }

            Some(ClinicalSizeOutput {
                type_code: CodeValue {
                    value: gs1_type.to_string(),
                },
                values,
                maximums,
                precision: CodeValue {
                    value: precision_code.to_string(),
                },
                text: cs.text.clone(),
            })
        })
        .collect()
}

fn build_storage_handling(device: &ApiDeviceDetail) -> Vec<ClinicalStorageHandling> {
    let conditions = match device.storage_handling_conditions.as_ref() {
        Some(c) if !c.is_empty() => c,
        _ => return Vec::new(),
    };

    conditions
        .iter()
        .filter_map(|shc| {
            let type_code_raw = shc.type_code.as_ref()?;
            let shc_code = extract_shc_code(type_code_raw);
            let gs1_code = mappings::storage_handling_to_gs1(&shc_code);

            let descriptions = extract_descriptions(&shc.description);

            Some(ClinicalStorageHandling {
                type_code: CodeValue { value: gs1_code },
                descriptions,
            })
        })
        .collect()
}

fn build_clinical_warnings(device: &ApiDeviceDetail) -> Vec<ClinicalWarningOutput> {
    let warnings = match device.critical_warnings.as_ref() {
        Some(w) if !w.is_empty() => w,
        _ => return Vec::new(),
    };

    warnings
        .iter()
        .filter_map(|cw| {
            let type_code_raw = cw.type_code.as_ref()?;
            let cw_code = extract_last_segment(type_code_raw).to_uppercase();

            let descriptions = extract_descriptions(&cw.description);

            Some(ClinicalWarningOutput {
                agency_code: CodeValue {
                    value: "EUDAMED".to_string(),
                },
                warning_code: cw_code,
                descriptions,
            })
        })
        .collect()
}

fn build_sales_module(device: &ApiDeviceDetail) -> Option<SalesInformationModule> {
    let market_info = device.market_info_link.as_ref()?;
    let markets = market_info.ms_where_available.as_ref()?;
    if markets.is_empty() {
        return None;
    }

    let countries: Vec<SalesConditionCountry> = markets
        .iter()
        .filter_map(|ma| {
            let iso2 = ma.country.as_ref()?.iso2_code.as_ref()?;
            let numeric = mappings::country_alpha2_to_numeric(iso2);
            Some(SalesConditionCountry {
                country_code: CodeValue {
                    value: numeric.to_string(),
                },
                start_datetime: ma.start_date.clone().unwrap_or_default(),
                end_datetime: ma.end_date.clone(),
            })
        })
        .collect();

    if countries.is_empty() {
        return None;
    }

    Some(SalesInformationModule {
        sales: SalesInformation {
            conditions: vec![TargetMarketSalesCondition {
                condition_code: CodeValue {
                    value: "UNRESTRICTED".to_string(),
                },
                countries,
            }],
        },
    })
}

// --- Helper functions ---

fn bool_str(b: bool) -> String {
    if b {
        "TRUE".to_string()
    } else {
        "FALSE".to_string()
    }
}

/// Extract CST code: "refdata.clinical-size-type.CST19" → "CST19"
fn extract_cst_code(code: &str) -> String {
    code.rsplit('.').next().unwrap_or(code).to_uppercase()
}

/// Extract MU code: "refdata.clinical-size-measurement-unit.MU50" → "MU50"
fn extract_mu_code(code: &str) -> String {
    code.rsplit('.').next().unwrap_or(code).to_uppercase()
}

/// Extract SHC code: "refdata.storage-handling-conditions-type.SHC099" → "SHC099"
fn extract_shc_code(code: &str) -> String {
    code.rsplit('.').next().unwrap_or(code).to_uppercase()
}

/// Extract last segment: "refdata.something.value" → "value"
fn extract_last_segment(code: &str) -> String {
    code.rsplit('.').next().unwrap_or(code).to_string()
}

/// Extract multilang descriptions from a MultiLangText
fn extract_descriptions(
    mlt: &Option<crate::api_detail::MultiLangText>,
) -> Vec<LangValue> {
    mlt.as_ref()
        .and_then(|t| t.texts.as_ref())
        .map(|texts| {
            texts
                .iter()
                .filter_map(|lt| {
                    let lang = lt.language.as_ref()?.iso_code.clone()?;
                    let text = lt.text.clone()?;
                    if text.is_empty() {
                        return None;
                    }
                    Some(LangValue {
                        language_code: lang,
                        value: text,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
