use crate::api_detail::{ApiDeviceDetail, BasicUdiDiData, Substance, CmrSubstance};
use crate::config::Config;
use crate::firstbase::*;
use crate::mappings;
use chrono::Local;

/// Transform a full API device detail record into a firstbase TradeItem.
/// Optional `basic_udi` provides real MDR mandatory fields from the Basic UDI-DI level.
pub fn transform_detail_device(device: &ApiDeviceDetail, config: &Config, basic_udi: Option<&BasicUdiDiData>) -> TradeItem {
    let now = Local::now();
    let now_str = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    // Use version_date for lastChangeDateTime (avoids G572 "future date" error)
    let last_change = device.version_date.as_ref()
        .filter(|d| !d.is_empty())
        .cloned()
        .unwrap_or_else(|| now_str.clone());

    let gtin = device.gtin();

    // --- Device status ---
    let status_code = device
        .status_code()
        .map(|s| mappings::device_status_to_gs1(&s).to_string())
        .unwrap_or_default();

    // --- Production identifiers ---
    // MDR/IVDR require at least one (097.013). Default to BATCH_NUMBER when EUDAMED has none.
    let mut production_ids: Vec<CodeValue> = device
        .production_identifiers()
        .into_iter()
        .map(|id| CodeValue { value: id })
        .collect();
    if production_ids.is_empty() {
        production_ids.push(CodeValue { value: "BATCH_NUMBER".to_string() });
    }

    // --- Sterility ---
    let sterility = build_sterility(device, config);

    // --- Reusability ---
    let reusability = build_reusability(device);

    // --- Contacts ---
    let mut contacts = build_contacts(device);

    // Add manufacturer contact from Basic UDI-DI (if not already present)
    let has_ema = contacts.iter().any(|c| c.contact_type.value == "EMA");
    if !has_ema {
        if let Some(ref mfr) = basic_udi.and_then(|b| b.manufacturer.as_ref()) {
            if let Some(ref srn) = mfr.srn {
                contacts.push(TradeItemContactInformation {
                contact_type: CodeValue { value: "EMA".to_string() },
                party_identification: vec![AdditionalPartyIdentification {
                    type_code: "SRN".to_string(),
                    value: srn.clone(),
                }],
                contact_name: mfr.name.clone(),
                addresses: Vec::new(),
                communication_channels: Vec::new(),
            });
            }
        }
    }

    // Add authorised representative contact from Basic UDI-DI (if not already present)
    let has_ear = contacts.iter().any(|c| c.contact_type.value == "EAR");
    if !has_ear {
        if let Some(ref ar) = basic_udi.and_then(|b| b.authorised_representative.as_ref()) {
            if let Some(ref srn) = ar.srn {
                contacts.push(TradeItemContactInformation {
                    contact_type: CodeValue { value: "EAR".to_string() },
                    party_identification: vec![AdditionalPartyIdentification {
                        type_code: "SRN".to_string(),
                        value: srn.clone(),
                    }],
                    contact_name: ar.name.clone(),
                    addresses: Vec::new(),
                    communication_channels: Vec::new(),
                });
            }
        }
    }

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

    // --- Non-GS1 primary DI → additional identification (GDSN only allows GS1 as Gtin) ---
    if !device.is_gs1_primary() {
        let agency = device.primary_di_agency().unwrap_or_default();
        let type_code = mappings::issuing_agency_to_type_code(&agency).to_string();
        let code = device.primary_di_code();
        if !code.is_empty() {
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code,
                value: code,
            });
        }
    }

    // --- Secondary DI → additional identification as GTIN_14 ---
    if let Some(ref secondary) = device.secondary_di {
        if let Some(ref code) = secondary.code {
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: "GTIN_14".to_string(),
                value: code.clone(),
            });
        }
    }

    // --- EMDN/CND nomenclature → additional classification system 88 ---
    let mut all_classifications = Vec::new();

    // Risk class from Basic UDI-DI → classification system 76
    if let Some(ref rc) = basic_udi.and_then(|b| b.risk_class_code()) {
        all_classifications.push(AdditionalClassification {
            system_code: CodeValue { value: "76".to_string() },
            values: vec![AdditionalClassificationValue {
                code_value: mappings::risk_class_refdata_to_gs1(rc).to_string(),
            }],
        });
    }

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
    let healthcare_module = build_healthcare_module(device, basic_udi);

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
    // Use risk class from Basic UDI-DI to determine MDR vs IVDR
    let reg_act = basic_udi
        .and_then(|b| b.risk_class_code())
        .map(|rc| mappings::regulation_from_risk_class_refdata(&rc).to_string())
        .unwrap_or_else(|| "MDR".to_string());
    let regulated_trade_item_module = Some(RegulatedTradeItemModule {
        info: vec![RegulatoryInformation {
            act: reg_act,
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
        target_sector: vec!["UDI_REGISTRY".to_string()],
        chemical_regulation_module,
        healthcare_item_module: healthcare_module,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: Some(bool_str(basic_udi.and_then(|b| b.implantable).unwrap_or(false))),
                device_count,
                direct_marking,
                measuring_function: Some(basic_udi.and_then(|b| b.measuring_function).unwrap_or(false)),
                is_active: Some(basic_udi.and_then(|b| b.active).unwrap_or(false)),
                administer_medicine: Some(basic_udi.and_then(|b| b.administering_medicine).unwrap_or(false)),
                is_medicinal_product: Some(basic_udi.and_then(|b| b.medicinal_product).unwrap_or(false)),
                is_reprocessed: device.reprocessed,
                is_reusable_surgical: Some(basic_udi.and_then(|b| b.reusable).unwrap_or(false)),
                production_identifier_types: production_ids,
                annex_xvi_types: Vec::new(),
                multi_component_type: Some(CodeValue {
                    value: basic_udi
                        .and_then(|b| b.multi_component_code())
                        .unwrap_or_else(|| "DEVICE".to_string()),
                }),
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
            last_change,
            effective: now_str.clone(),
            publication: now_str,
        },
        global_model_info: vec![GlobalModelInformation {
            number: basic_udi
                .and_then(|b| b.basic_udi.as_ref())
                .and_then(|di| di.code.clone())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| device.primary_di_code()), // fallback to primary DI
            descriptions: {
                // 097.025: GlobalModelDescription with languageCode 'en' is required
                let mut descs: Vec<LangValue> = trade_names
                    .iter()
                    .map(|(lang, text)| LangValue {
                        language_code: lang.clone(),
                        value: text.clone(),
                    })
                    .collect();
                let has_en = descs.iter().any(|d| d.language_code == "en");
                if !has_en {
                    // Fall back to first available trade name, or Basic UDI-DI device name
                    let fallback = trade_names
                        .first()
                        .map(|(_, text)| text.clone())
                        .or_else(|| basic_udi.and_then(|b| b.device_name.clone()))
                        .unwrap_or_else(|| device.primary_di_code());
                    descs.insert(0, LangValue {
                        language_code: "en".to_string(),
                        value: fallback,
                    });
                }
                descs
            },
        }],
        gtin,
        additional_identification,
        referenced_trade_items,
    }
}

fn build_sterility(device: &ApiDeviceDetail, _config: &Config) -> Option<SterilityInformation> {
    let sterile = device.sterile?;
    let sterilization = device.sterilization.unwrap_or(false);

    let manufacturer_sterilisation = vec![CodeValue {
        value: if sterile {
            "UNSPECIFIED".to_string()
        } else {
            "NOT_STERILISED".to_string()
        },
    }];

    let prior_to_use = vec![CodeValue {
        value: if sterilization {
            "UNSPECIFIED".to_string()
        } else {
            "NO_STERILISATION_REQUIRED".to_string()
        },
    }];

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
        if max.is_some() {
            Some(ReusabilityInformation {
                reusability_type: CodeValue {
                    value: "LIMITED_REUSABLE".to_string(),
                },
                max_cycles: max,
            })
        } else {
            Some(ReusabilityInformation {
                reusability_type: CodeValue {
                    value: "REUSABLE".to_string(),
                },
                max_cycles: None,
            })
        }
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

fn build_healthcare_module(device: &ApiDeviceDetail, basic_udi: Option<&BasicUdiDiData>) -> Option<HealthcareItemInformationModule> {
    let clinical_sizes = build_clinical_sizes(device);
    let storage_handling = build_storage_handling(device);
    let clinical_warnings = build_clinical_warnings(device);
    let contains_latex = Some(device.latex.map(|b| bool_str(b)).unwrap_or_else(|| "FALSE".to_string()));

    Some(HealthcareItemInformationModule {
        info: HealthcareItemInformation {
            human_blood_derivative: Some(bool_str(basic_udi.and_then(|b| b.human_product).unwrap_or(false))),
            contains_latex,
            human_tissue: Some(bool_str(basic_udi.and_then(|b| b.human_tissues).unwrap_or(false))),
            animal_tissue: Some(basic_udi.and_then(|b| b.animal_tissues).unwrap_or(false)),
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

            let mut descriptions = extract_descriptions(&shc.description);
            // 097.074: Some SHC codes require a description; use code as placeholder
            if descriptions.is_empty() {
                descriptions.push(LangValue {
                    language_code: "en".to_string(),
                    value: gs1_code.clone(),
                });
            }

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
    // Determine which country is the "original placed" market
    let original_iso2 = device.placed_on_the_market.as_ref()
        .and_then(|c| c.iso2_code.as_ref())
        .map(|s| s.as_str());

    let mut original_countries = Vec::new();
    let mut additional_countries = Vec::new();

    let markets = device.market_info_link.as_ref()
        .and_then(|m| m.ms_where_available.as_ref());

    if let Some(markets) = markets {
        for ma in markets {
            let iso2 = match ma.country.as_ref().and_then(|c| c.iso2_code.as_ref()) {
                Some(c) => c,
                None => continue,
            };
            // Skip GB/XI — not valid GDSN market countries post-Brexit (G541)
            if !mappings::is_valid_gdsn_market_country(iso2) {
                continue;
            }
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
    }

    // 097.020: ON_MARKET requires exactly one ORIGINAL_PLACED country.
    // If no match from msWhereAvailable, use placedOnTheMarket directly.
    if original_countries.is_empty() {
        if let Some(iso2) = original_iso2 {
            if mappings::is_valid_gdsn_market_country(iso2) {
                let numeric = mappings::country_alpha2_to_numeric(iso2);
                original_countries.push(SalesConditionCountry {
                    country_code: CodeValue { value: numeric.to_string() },
                    start_datetime: String::new(),
                    end_datetime: None,
                });
            }
        }
    }

    // Last resort: use the first additional country as ORIGINAL_PLACED
    if original_countries.is_empty() && !additional_countries.is_empty() {
        original_countries.push(additional_countries.remove(0));
    }

    // Ensure only one country in ORIGINAL_PLACED (097.020: only one allowed)
    while original_countries.len() > 1 {
        additional_countries.push(original_countries.pop().unwrap());
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
    // Skip self-references (G641 error)
    if gtin == device.primary_di_code() {
        return Vec::new();
    }
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
