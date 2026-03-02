use crate::api_detail::{ApiDeviceDetail, Substance, CmrSubstance};
use crate::config::Config;
use crate::firstbase::*;
use crate::mappings;
use chrono::Local;

/// Transform a full API device detail record into a firstbase TradeItem.
pub fn transform_detail_device(device: &ApiDeviceDetail, config: &Config) -> TradeItem {
    let now = Local::now();
    let now_str = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    let gtin = device.gtin();

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

    // --- Contacts ---
    let contacts = build_contacts(device);

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

    // --- Secondary DI → additional identification ---
    if let Some(ref secondary) = device.secondary_di {
        if let Some(ref code) = secondary.code {
            let agency = secondary.issuing_agency.as_ref()
                .and_then(|a| a.code.as_ref())
                .map(|c| mappings::issuing_agency_to_type_code(c))
                .unwrap_or("GS1");
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: agency.to_string(),
                value: code.clone(),
            });
        }
    }

    // --- Unit of use → additional identification ---
    if let Some(ref uou) = device.unit_of_use {
        if let Some(ref code) = uou.code {
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: "UNIT_OF_USE_IDENTIFIER".to_string(),
                value: code.clone(),
            });
        }
    }

    // --- EMDN/CND nomenclature → additional classification system 88 ---
    let mut all_classifications = Vec::new();
    if let Some(ref cnds) = device.cnd_nomenclatures {
        for cnd in cnds {
            if let Some(ref code) = cnd.code {
                all_classifications.push(AdditionalClassification {
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

    // --- Healthcare item module (clinical sizes, storage, warnings, latex, tissue) ---
    let healthcare_module = build_healthcare_module(device);

    // --- Chemical regulation module (substances) ---
    let chemical_regulation_module = build_chemical_regulation_module(device);

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

    // --- Regulated trade item module (regulatory act + agency) ---
    let regulated_trade_item_module = Some(RegulatedTradeItemModule {
        info: vec![RegulatoryInformation {
            act: "MDR".to_string(),
            agency: "EU".to_string(),
        }],
    });

    // --- Sales module (market availability with ORIGINAL_PLACED distinction) ---
    let sales_module = build_sales_module(device);

    // --- Direct marking DI ---
    let direct_marking = build_direct_marking(device);

    // --- Related devices (REPLACED/REPLACED_BY) ---
    let referenced_trade_items = build_referenced_trade_items(device);

    // --- Base quantity → device count ---
    let device_count = device.base_quantity;

    TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["HEALTHCARE".to_string(), "UDI_REGISTRY".to_string()],
        chemical_regulation_module,
        healthcare_item_module: healthcare_module,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: None, // Basic UDI-DI level, not in UDI-DI JSON
                device_count,
                direct_marking,
                measuring_function: None, // Basic UDI-DI level
                is_active: None,          // Basic UDI-DI level
                administer_medicine: None, // Basic UDI-DI level
                is_medicinal_product: None, // Basic UDI-DI level
                is_reprocessed: device.reprocessed,
                is_reusable_surgical: None, // Basic UDI-DI level
                production_identifier_types: production_ids,
                annex_xvi_types: Vec::new(), // Type codes at Basic UDI-DI level
                multi_component_type: None,  // At Basic UDI-DI level
                is_new_device: device.new_device,
                eu_status: CodeValue {
                    value: status_code,
                },
                reusability,
                sterility,
            },
        },
        referenced_file_module,
        regulated_trade_item_module,
        sales_module,
        description_module,
        is_base_unit: true,
        is_despatch_unit: false,
        is_orderable_unit: true,
        unit_descriptor: CodeValue {
            value: "BASE_UNIT_OR_EACH".to_string(),
        },
        trade_channel_code: vec![CodeValue { value: "UDI_REGISTRY".to_string() }],
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
        referenced_trade_items,
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

/// Build contacts: product designer → EPD contact
fn build_contacts(device: &ApiDeviceDetail) -> Vec<TradeItemContactInformation> {
    let mut contacts = Vec::new();

    // Product designer → EPD contact
    if let Some(ref pd) = device.product_designer {
        if let Some(ref actor) = pd.oem_actor {
            // Registered actor with SRN
            let mut party_ids = Vec::new();
            if let Some(ref srn) = actor.srn {
                party_ids.push(AdditionalPartyIdentification {
                    type_code: "SRN".to_string(),
                    value: srn.clone(),
                });
            }

            let mut addresses = Vec::new();
            if let Some((street, number, postal, city)) = actor.structured_address() {
                let country_numeric = actor.country_iso2_code.as_ref()
                    .map(|c| mappings::country_alpha2_to_numeric(c).to_string())
                    .unwrap_or_default();
                addresses.push(StructuredAddress {
                    city,
                    country_code: CodeValue { value: country_numeric },
                    postal_code: postal,
                    street,
                    street_number: if number.is_empty() { None } else { Some(number) },
                });
            }

            let mut channels = Vec::new();
            if let Some(ref phone) = actor.telephone {
                if !phone.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue { value: "TELEPHONE".to_string() },
                            value: phone.clone(),
                        }],
                    });
                }
            }
            if let Some(ref email) = actor.electronic_mail {
                if !email.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue { value: "EMAIL".to_string() },
                            value: email.clone(),
                        }],
                    });
                }
            }

            contacts.push(TradeItemContactInformation {
                contact_type: CodeValue { value: "EPD".to_string() },
                party_identification: party_ids,
                contact_name: actor.name.clone(),
                addresses,
                communication_channels: channels,
            });
        } else if let Some(ref org) = pd.oem_organisation {
            // Non-registered organisation
            let mut addresses = Vec::new();
            if let Some((street, number, postal, city)) = org.structured_address() {
                let country_numeric = org.country_iso2()
                    .map(|c| mappings::country_alpha2_to_numeric(&c).to_string())
                    .unwrap_or_default();
                addresses.push(StructuredAddress {
                    city,
                    country_code: CodeValue { value: country_numeric },
                    postal_code: postal,
                    street,
                    street_number: if number.is_empty() { None } else { Some(number) },
                });
            }

            let mut channels = Vec::new();
            if let Some(ref phone) = org.telephone {
                if !phone.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue { value: "TELEPHONE".to_string() },
                            value: phone.clone(),
                        }],
                    });
                }
            }
            if let Some(ref email) = org.electronic_mail {
                if !email.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue { value: "EMAIL".to_string() },
                            value: email.clone(),
                        }],
                    });
                }
            }

            contacts.push(TradeItemContactInformation {
                contact_type: CodeValue { value: "EPD".to_string() },
                party_identification: Vec::new(),
                contact_name: org.name.clone(),
                addresses,
                communication_channels: channels,
            });
        }
    }

    contacts
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
                "EXACT" | "VALUE" => "VALUE",
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

/// Build sales module with ORIGINAL_PLACED vs ADDITIONAL_MARKET_AVAILABILITY distinction.
fn build_sales_module(device: &ApiDeviceDetail) -> Option<SalesInformationModule> {
    let market_info = device.market_info_link.as_ref()?;
    let markets = market_info.ms_where_available.as_ref()?;
    if markets.is_empty() {
        return None;
    }

    // Determine which country is the "original placed" market
    let original_iso2 = device.placed_on_the_market.as_ref()
        .and_then(|c| c.iso2_code.as_ref())
        .map(|s| s.as_str());

    let mut original_countries = Vec::new();
    let mut additional_countries = Vec::new();

    for ma in markets {
        let iso2 = match ma.country.as_ref().and_then(|c| c.iso2_code.as_ref()) {
            Some(c) => c,
            None => continue,
        };
        let numeric = mappings::country_alpha2_to_numeric(iso2);
        let country = SalesConditionCountry {
            country_code: CodeValue {
                value: numeric.to_string(),
            },
            start_datetime: ma.start_date.clone().unwrap_or_default(),
            end_datetime: ma.end_date.clone(),
        };

        if original_iso2 == Some(iso2.as_str()) {
            original_countries.push(country);
        } else {
            additional_countries.push(country);
        }
    }

    let mut conditions = Vec::new();
    if !original_countries.is_empty() {
        conditions.push(TargetMarketSalesCondition {
            condition_code: CodeValue {
                value: "ORIGINAL_PLACED".to_string(),
            },
            countries: original_countries,
        });
    }
    if !additional_countries.is_empty() {
        conditions.push(TargetMarketSalesCondition {
            condition_code: CodeValue {
                value: "ADDITIONAL_MARKET_AVAILABILITY".to_string(),
            },
            countries: additional_countries,
        });
    }

    if conditions.is_empty() {
        return None;
    }

    Some(SalesInformationModule {
        sales: SalesInformation { conditions },
    })
}

/// Build direct marking DI identifiers.
fn build_direct_marking(device: &ApiDeviceDetail) -> Vec<DirectPartMarking> {
    let di = match device.direct_marking_di.as_ref() {
        Some(di) => di,
        None => return Vec::new(),
    };
    let code = match di.code.as_ref() {
        Some(c) if !c.is_empty() => c,
        _ => return Vec::new(),
    };
    let agency = di.issuing_agency.as_ref()
        .and_then(|a| a.code.as_ref())
        .map(|c| mappings::issuing_agency_to_type_code(c))
        .unwrap_or("GS1");

    vec![DirectPartMarking {
        agency_code: agency.to_string(),
        value: code.clone(),
    }]
}

/// Build referenced trade items from linked UDI-DI view (REPLACED/REPLACED_BY).
fn build_referenced_trade_items(device: &ApiDeviceDetail) -> Vec<ReferencedTradeItem> {
    let link = match device.linked_udi_di_view.as_ref() {
        Some(l) => l,
        None => return Vec::new(),
    };
    let gtin = match link.udi_di.as_ref().and_then(|d| d.code.as_ref()) {
        Some(g) if !g.is_empty() => g.clone(),
        _ => return Vec::new(),
    };
    let type_code = match link.device_criterion.as_deref() {
        Some("LEGACY") => "REPLACED",
        Some("STANDARD") => "REPLACED_BY",
        _ => "REPLACED_BY",
    };
    vec![ReferencedTradeItem {
        type_code: CodeValue { value: type_code.to_string() },
        gtin,
    }]
}

/// Build chemical regulation module from substances.
fn build_chemical_regulation_module(device: &ApiDeviceDetail) -> Option<ChemicalRegulationInformationModule> {
    let mut who_chemicals = Vec::new();
    let mut echa_chemicals = Vec::new();

    // --- Medicinal product substances → WHO/INN/MEDICINAL_PRODUCT ---
    if let Some(ref subs) = device.medicinal_product_substances {
        for sub in subs {
            who_chemicals.push(build_substance_chemical(sub, "MEDICINAL_PRODUCT"));
        }
    }

    // --- Human product substances → WHO/INN/HUMAN_PRODUCT ---
    if let Some(ref subs) = device.human_product_substances {
        for sub in subs {
            who_chemicals.push(build_substance_chemical(sub, "HUMAN_PRODUCT"));
        }
    }

    // --- Endocrine disrupting substances → ECHA/ECICS/ENDOCRINE_SUBSTANCE ---
    if let Some(ref subs) = device.endocrine_disrupting_substances {
        for sub in subs {
            echa_chemicals.push(build_substance_chemical(sub, "ENDOCRINE_SUBSTANCE"));
        }
    }

    // --- CMR substances → ECHA/ECICS/CMR_SUBSTANCE ---
    if let Some(ref subs) = device.cmr_substances {
        for sub in subs {
            echa_chemicals.push(build_cmr_chemical(sub));
        }
    }

    let mut infos = Vec::new();

    // WHO substances first (following transform.rs sort order)
    if !who_chemicals.is_empty() {
        infos.push(ChemicalRegulationInformation {
            agency: "WHO".to_string(),
            regulations: vec![ChemicalRegulation {
                regulation_name: "INN".to_string(),
                chemicals: who_chemicals,
            }],
        });
    }

    // ECHA substances (endocrine before CMR)
    if !echa_chemicals.is_empty() {
        infos.push(ChemicalRegulationInformation {
            agency: "ECHA".to_string(),
            regulations: vec![ChemicalRegulation {
                regulation_name: "ECICS".to_string(),
                chemicals: echa_chemicals,
            }],
        });
    }

    if infos.is_empty() {
        None
    } else {
        Some(ChemicalRegulationInformationModule { infos })
    }
}

/// Build a RegulatedChemical from a Substance (medicinal/human/endocrine).
fn build_substance_chemical(sub: &Substance, chemical_type: &str) -> RegulatedChemical {
    let name_text = extract_substance_name(sub);
    let inn = sub.inn_code.as_ref().filter(|s| !s.is_empty()).cloned();

    // CAS identifier
    let cas_ref = sub.cas_number.as_ref()
        .filter(|s| !s.is_empty())
        .map(|cas| ChemicalIdentifierRef {
            agency_name: "CAS".to_string(),
            value: cas.clone(),
        });

    // EC identifier
    let ec_ref = sub.ec_number.as_ref()
        .filter(|s| !s.is_empty())
        .map(|ec| ChemicalIdentifierRef {
            agency_name: "EC".to_string(),
            value: ec.clone(),
        });

    // Use CAS if available, else EC
    let identifier_ref = cas_ref.or(ec_ref);

    // Description from name texts (when no INN/CAS/EC)
    let descriptions = if identifier_ref.is_none() && inn.is_none() {
        name_text.as_ref().map(|name| vec![LangValue {
            language_code: "en".to_string(),
            value: name.trim().to_string(),
        }]).unwrap_or_default()
    } else {
        Vec::new()
    };

    RegulatedChemical {
        identifier_ref,
        chemical_name: inn,
        descriptions,
        cmr_type: None,
        chemical_type: CodeValue { value: chemical_type.to_string() },
    }
}

/// Build a RegulatedChemical from a CmrSubstance.
fn build_cmr_chemical(sub: &CmrSubstance) -> RegulatedChemical {
    let name_text = sub.name.as_ref()
        .and_then(|t| t.texts.as_ref())
        .and_then(|texts| texts.first())
        .and_then(|lt| lt.text.clone());

    // CAS identifier
    let cas_ref = sub.cas_number.as_ref()
        .filter(|s| !s.is_empty())
        .map(|cas| ChemicalIdentifierRef {
            agency_name: "CAS".to_string(),
            value: cas.clone(),
        });

    // EC identifier
    let ec_ref = sub.ec_number.as_ref()
        .filter(|s| !s.is_empty())
        .map(|ec| ChemicalIdentifierRef {
            agency_name: "EC".to_string(),
            value: ec.clone(),
        });

    let identifier_ref = cas_ref.or(ec_ref);

    // CMR type code from cmr_substance_type
    let cmr_type = sub.cmr_substance_type.as_ref()
        .and_then(|t| t.code.as_ref())
        .map(|c| CodeValue { value: mappings::cmr_type_to_gs1(c) });

    // Description from name (when no CAS/EC identifier)
    let descriptions = if identifier_ref.is_none() {
        name_text.as_ref().map(|name| vec![LangValue {
            language_code: "en".to_string(),
            value: name.trim().to_string(),
        }]).unwrap_or_default()
    } else {
        Vec::new()
    };

    RegulatedChemical {
        identifier_ref,
        chemical_name: None,
        descriptions,
        cmr_type,
        chemical_type: CodeValue { value: "CMR_SUBSTANCE".to_string() },
    }
}

/// Extract the first text from a Substance's name field
fn extract_substance_name(sub: &Substance) -> Option<String> {
    sub.name.as_ref()
        .and_then(|t| t.texts.as_ref())
        .and_then(|texts| texts.first())
        .and_then(|lt| lt.text.clone())
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
