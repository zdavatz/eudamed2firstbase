use crate::config::Config;
use crate::eudamed::*;
use crate::firstbase::*;
use crate::mappings;
use anyhow::{Context, Result};
use std::collections::HashMap;

pub fn transform(response: &PullResponse, config: &Config) -> Result<FirstbaseDocument> {
    let device = &response.device;
    let basic_udi = device.mdr_basic_udi.as_ref().context("Missing MDRBasicUDI")?;
    let udidi = device.mdr_udidi_data.as_ref().context("Missing MDRUDIDIData")?;

    let base_unit_di = udidi.identifier.as_ref()
        .and_then(|id| id.di_code.as_deref())
        .context("Missing UDI-DI identifier")?;
    let basic_udi_di = basic_udi.identifier.as_ref()
        .and_then(|id| id.di_code.as_deref())
        .unwrap_or("");

    // Build the base unit trade item (with all device detail)
    let base_trade_item = build_base_unit(basic_udi, udidi, config)?;

    // Build packaging hierarchy
    let (top_gtin, hierarchy) = build_packaging_hierarchy(udidi, base_unit_di)?;

    if hierarchy.is_empty() {
        // No packages - base unit is the root
        return Ok(FirstbaseDocument {
            trade_item: base_trade_item,
            children: vec![],
        });
    }

    // Build nested structure from outermost package down to base unit
    build_nested_document(
        &hierarchy,
        &top_gtin,
        base_unit_di,
        base_trade_item,
        basic_udi_di,
        config,
    )
}

#[derive(Debug)]
struct PackageInfo {
    gtin: String,
    child_di: String,
    quantity: u32,
}

fn build_packaging_hierarchy(udidi: &MdrUdidiData, _base_unit_di: &str) -> Result<(String, Vec<PackageInfo>)> {
    if udidi.packages.is_empty() {
        return Ok((String::new(), vec![]));
    }

    let mut pkg_list: Vec<PackageInfo> = Vec::new();
    let mut child_dis: Vec<String> = Vec::new();

    for pkg in &udidi.packages {
        let gtin = pkg.identifier.as_ref()
            .and_then(|id| id.di_code.as_deref())
            .unwrap_or("")
            .to_string();
        let child_di = pkg.child.as_ref()
            .and_then(|id| id.di_code.as_deref())
            .unwrap_or("")
            .to_string();
        let qty = pkg.number_of_items.unwrap_or(1);

        child_dis.push(child_di.clone());
        pkg_list.push(PackageInfo { gtin, child_di, quantity: qty });
    }

    // The outermost package is the one whose DI is never referenced as a child
    let top_gtin = pkg_list.iter()
        .find(|p| !child_dis.contains(&p.gtin))
        .map(|p| p.gtin.clone())
        .unwrap_or_default();

    Ok((top_gtin, pkg_list))
}

fn build_nested_document(
    hierarchy: &[PackageInfo],
    top_gtin: &str,
    base_unit_di: &str,
    base_trade_item: TradeItem,
    basic_udi_di: &str,
    config: &Config,
) -> Result<FirstbaseDocument> {
    // Map from parent DI → PackageInfo
    let pkg_map: HashMap<&str, &PackageInfo> = hierarchy.iter()
        .map(|p| (p.gtin.as_str(), p))
        .collect();

    // Build from bottom up: find the chain from top to base
    let mut chain: Vec<&PackageInfo> = Vec::new();
    let mut current = top_gtin;
    loop {
        if let Some(pkg) = pkg_map.get(current) {
            chain.push(pkg);
            if pkg.child_di == base_unit_di {
                break;
            }
            current = &pkg.child_di;
        } else {
            break;
        }
    }

    // Build the innermost child link (base unit)
    let mut inner_link = CatalogueItemChildItemLink {
        quantity: chain.last().map(|p| p.quantity).unwrap_or(1),
        catalogue_item: CatalogueItem {
            identifier: generate_uuid(),
            trade_item: base_trade_item,
            children: vec![],
        },
    };

    // Wrap in intermediate packages (from second-to-last to second)
    for i in (0..chain.len().saturating_sub(1)).rev() {
        let pkg = chain[i];
        let child_pkg = chain[i + 1];

        let intermediate_trade_item = build_packaging_trade_item(
            &child_pkg.gtin,
            Some(&NextLowerLevel {
                quantity_of_children: 1,
                total_quantity: child_pkg.quantity,
                child_items: vec![ChildTradeItem {
                    quantity: child_pkg.quantity,
                    gtin: child_pkg.child_di.clone(),
                }],
            }),
            basic_udi_di,
            config,
            false,
        );

        inner_link = CatalogueItemChildItemLink {
            quantity: pkg.quantity,
            catalogue_item: CatalogueItem {
                identifier: generate_uuid(),
                trade_item: intermediate_trade_item,
                children: vec![inner_link],
            },
        };
    }

    // Top-level trade item (outermost package)
    let top_pkg = chain.first().unwrap();
    let top_next_lower = Some(NextLowerLevel {
        quantity_of_children: 1,
        total_quantity: top_pkg.quantity,
        child_items: vec![ChildTradeItem {
            quantity: top_pkg.quantity,
            gtin: top_pkg.child_di.clone(),
        }],
    });

    let top_trade_item = build_packaging_trade_item(
        top_gtin,
        top_next_lower.as_ref(),
        basic_udi_di,
        config,
        true,
    );

    Ok(FirstbaseDocument {
        trade_item: top_trade_item,
        children: vec![inner_link],
    })
}

fn build_packaging_trade_item(
    gtin: &str,
    next_lower: Option<&NextLowerLevel>,
    basic_udi_di: &str,
    config: &Config,
    is_top_level: bool,
) -> TradeItem {
    TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["UDI_REGISTRY".to_string()],
        chemical_regulation_module: None,
        healthcare_item_module: None,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                eu_status: CodeValue { value: "ON_MARKET".to_string() },
                ..Default::default()
            },
        },
        referenced_file_module: None,
        regulated_trade_item_module: None,
        sales_module: None,
        description_module: None,
        is_base_unit: false,
        is_despatch_unit: is_top_level,
        is_orderable_unit: true,
        unit_descriptor: CodeValue { value: "CASE".to_string() },
        trade_channel_code: vec![],
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
            additional_classifications: vec![],
        },
        next_lower_level: next_lower.map(|nl| NextLowerLevel {
            quantity_of_children: nl.quantity_of_children,
            total_quantity: nl.total_quantity,
            child_items: nl.child_items.iter().map(|c| ChildTradeItem {
                quantity: c.quantity,
                gtin: c.gtin.clone(),
            }).collect(),
        }),
        target_market: TargetMarketObj {
            country_code: CodeValue { value: config.target_market.country_code.clone() },
        },
        contact_information: vec![],
        synchronisation_dates: TradeItemSynchronisationDates::default(),
        global_model_info: vec![GlobalModelInformation {
            number: basic_udi_di.to_string(),
            descriptions: vec![],
        }],
        gtin: gtin.to_string(),
        additional_identification: vec![],
        referenced_trade_items: Vec::new(),
    }
}

fn build_base_unit(basic_udi: &MdrBasicUdi, udidi: &MdrUdidiData, config: &Config) -> Result<TradeItem> {
    let base_di = udidi.identifier.as_ref()
        .and_then(|id| id.di_code.as_deref())
        .unwrap_or("");
    let basic_udi_di = basic_udi.identifier.as_ref()
        .and_then(|id| id.di_code.as_deref())
        .unwrap_or("");
    let risk_class = basic_udi.risk_class.as_deref().unwrap_or("");

    // Build additional classifications (risk class + MDN codes)
    let mut classifications = Vec::new();

    // MDN codes (system 88) - sorted alphabetically
    if let Some(ref mdn) = udidi.mdn_codes {
        let mut codes: Vec<&str> = mdn.split_whitespace().collect();
        codes.sort();
        for code in codes {
            classifications.push(AdditionalClassification {
                system_code: CodeValue { value: "88".to_string() },
                values: vec![AdditionalClassificationValue { code_value: code.to_string() }],
            });
        }
    }

    // Risk class (system 76)
    if !risk_class.is_empty() {
        classifications.push(AdditionalClassification {
            system_code: CodeValue { value: "76".to_string() },
            values: vec![AdditionalClassificationValue {
                code_value: mappings::risk_class_to_gs1(risk_class).to_string(),
            }],
        });
    }

    // Contact information
    let mut contacts = Vec::new();

    // Manufacturer (EMA)
    if let Some(ref mf) = basic_udi.mf_actor_code {
        contacts.push(TradeItemContactInformation {
            contact_type: CodeValue { value: "EMA".to_string() },
            party_identification: vec![AdditionalPartyIdentification {
                type_code: "SRN".to_string(),
                value: mf.clone(),
            }],
            contact_name: None,
            addresses: vec![],
            communication_channels: vec![],
        });
    }

    // Authorised representative (EAR)
    if let Some(ref ar) = basic_udi.ar_actor_code {
        contacts.push(TradeItemContactInformation {
            contact_type: CodeValue { value: "EAR".to_string() },
            party_identification: vec![AdditionalPartyIdentification {
                type_code: "SRN".to_string(),
                value: ar.clone(),
            }],
            contact_name: None,
            addresses: vec![],
            communication_channels: vec![],
        });
    }

    // Product designer (EPD)
    if let Some(ref pd) = udidi.product_designer_actor {
        if let Some(ref org) = pd.organisation {
            let mut pd_contact = TradeItemContactInformation {
                contact_type: CodeValue { value: "EPD".to_string() },
                party_identification: vec![],
                contact_name: org.org_name.clone(),
                addresses: vec![],
                communication_channels: vec![],
            };

            if let Some(ref addr) = org.address {
                let country_numeric = addr.country.as_deref()
                    .map(mappings::country_alpha2_to_numeric)
                    .unwrap_or("");
                pd_contact.addresses.push(StructuredAddress {
                    city: addr.city.clone().unwrap_or_default(),
                    country_code: CodeValue { value: country_numeric.to_string() },
                    postal_code: addr.post_code.clone().unwrap_or_default(),
                    street: addr.street.clone().unwrap_or_default(),
                    street_number: addr.street_num.clone(),
                });
            }

            // Email and phone are now directly on the organisation struct
            let mut channels = Vec::new();
            if let Some(ref email) = org.email {
                channels.push(CommunicationChannel {
                    channel_code: CodeValue { value: "EMAIL".to_string() },
                    value: email.clone(),
                });
            }
            if let Some(ref phone) = org.phone {
                channels.push(CommunicationChannel {
                    channel_code: CodeValue { value: "TELEPHONE".to_string() },
                    value: phone.clone(),
                });
            }
            if !channels.is_empty() {
                pd_contact.communication_channels.push(TargetMarketCommunicationChannel {
                    channels,
                });
            }

            contacts.push(pd_contact);
        }
    }

    // Production identifier types - sorted
    let mut production_ids: Vec<CodeValue> = udidi.production_identifier.as_deref()
        .map(|s| s.split_whitespace()
            .map(|id| CodeValue {
                value: mappings::production_identifier_to_gs1(id).to_string(),
            })
            .collect())
        .unwrap_or_default();
    production_ids.sort_by(|a, b| {
        prod_id_sort_key(&a.value).cmp(&prod_id_sort_key(&b.value))
    });

    // Annex XVI types (now Vec<String> directly)
    let annex_xvi: Vec<CodeValue> = udidi.annex_xvi_types.iter()
        .map(|t| CodeValue { value: t.clone() })
        .collect();

    // Multi-component type
    let multi_component = basic_udi.device_kind.as_ref().map(|t| CodeValue { value: t.clone() });

    // Status (now Option<String> directly)
    let status = udidi.status.as_deref()
        .map(mappings::device_status_to_gs1)
        .unwrap_or("ON_MARKET");

    // Reusability
    let reusability = udidi.number_of_reuses.map(|n| {
        if n == 0 {
            ReusabilityInformation {
                reusability_type: CodeValue { value: "SINGLE_USE".to_string() },
                max_cycles: None,
            }
        } else {
            ReusabilityInformation {
                reusability_type: CodeValue { value: "LIMITED_REUSABLE".to_string() },
                max_cycles: Some(n),
            }
        }
    });

    // Sterility (booleans are now plain Option<bool>)
    let sterility = {
        let sterile = udidi.sterile.unwrap_or(false);
        let sterilization = udidi.sterilization.unwrap_or(false);

        let manufacturer_code = if sterile {
            config.sterilisation_method.as_deref().unwrap_or("UNSPECIFIED").to_string()
        } else {
            "NOT_STERILISED".to_string()
        };

        let prior_to_use = if sterilization {
            vec![CodeValue {
                value: config.sterilisation_method.as_deref().unwrap_or("UNSPECIFIED").to_string(),
            }]
        } else {
            vec![]
        };

        Some(SterilityInformation {
            manufacturer_sterilisation: vec![CodeValue { value: manufacturer_code }],
            prior_to_use,
        })
    };

    // Healthcare item information (booleans are now plain Option<bool>)
    let healthcare_module = {
        let human_blood = basic_udi.human_product_check
            .map(|b| if b { "TRUE" } else { "FALSE" }.to_string());
        let latex = udidi.latex
            .map(|b| if b { "TRUE" } else { "FALSE" }.to_string());
        let human_tissue = basic_udi.human_tissues_cells
            .map(|b| if b { "TRUE" } else { "FALSE" }.to_string());
        let animal_tissue = basic_udi.animal_tissues_cells
            .map(|b| serde_json::Value::Bool(b));

        // Storage handling
        let storage = transform_storage_handling(udidi);

        // Clinical sizes
        let clinical_sizes = transform_clinical_sizes(udidi);

        // Clinical warnings
        let warnings = transform_warnings(udidi);

        Some(HealthcareItemInformationModule {
            info: HealthcareItemInformation {
                human_blood_derivative: human_blood,
                contains_latex: latex,
                human_tissue,
                animal_tissue,
                storage_handling: storage,
                clinical_sizes,
                clinical_warnings: warnings,
            },
        })
    };

    // Chemical regulation (substances)
    let chem_module = transform_substances(udidi, config);

    // Trade item descriptions (now Option<Vec<LanguageSpecificName>>)
    let description_module = {
        let descriptions = transform_lang_names(&udidi.trade_names);
        let additional = transform_lang_names(&udidi.additional_description);

        if !descriptions.is_empty() || !additional.is_empty() {
            Some(TradeItemDescriptionModule {
                info: TradeItemDescriptionInformation {
                    additional_descriptions: additional,
                    descriptions,
                },
            })
        } else {
            None
        }
    };

    // Referenced file (website → IFU)
    let referenced_file_module = udidi.website.as_ref().map(|url| {
        let filename = url.rsplit('/').next().unwrap_or("document.pdf");
        let is_pdf = filename.to_lowercase().ends_with(".pdf");
        ReferencedFileDetailInformationModule {
            headers: vec![ReferencedFileHeader {
                media_source_gln: Some(config.provider.gln.clone()),
                mime_type: if is_pdf { Some("application/pdf".to_string()) } else { None },
                file_type: CodeValue { value: "IFU".to_string() },
                format_name: if is_pdf { Some("Pdf".to_string()) } else { None },
                file_name: Some(filename.to_string()),
                uri: url.clone(),
                is_primary: "FALSE".to_string(),
            }],
        }
    });

    // Regulated trade item module
    let regulated_module = Some(RegulatedTradeItemModule {
        info: vec![RegulatoryInformation {
            act: mappings::regulation_from_risk_class(risk_class).to_string(),
            agency: "EU".to_string(),
        }],
    });

    // Sales information (market info - now Vec<MarketInfo> directly)
    let sales_module = transform_market_info(udidi);

    // Global model info
    let model_desc = basic_udi.model_name.as_ref()
        .and_then(|m| m.name.as_ref())
        .map(|n| vec![LangValue { language_code: "en".to_string(), value: n.clone() }])
        .unwrap_or_default();

    // Additional identifications
    let mut additional_ids = Vec::new();
    if let Some(ref rn) = udidi.reference_number {
        additional_ids.push(AdditionalTradeItemIdentification {
            type_code: "MANUFACTURER_PART_NUMBER".to_string(),
            value: rn.clone(),
        });
    }
    if let Some(ref model) = basic_udi.model_name.as_ref().and_then(|m| m.model.clone()) {
        additional_ids.push(AdditionalTradeItemIdentification {
            type_code: "MODEL_NUMBER".to_string(),
            value: model.clone(),
        });
    }

    Ok(TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["UDI_REGISTRY".to_string()],
        chemical_regulation_module: chem_module,
        healthcare_item_module: healthcare_module,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: basic_udi.implantable
                    .map(|b| if b { "TRUE" } else { "FALSE" }.to_string()),
                device_count: udidi.base_quantity,
                direct_marking: vec![],
                measuring_function: basic_udi.measuring_function,
                is_active: basic_udi.active,
                administer_medicine: basic_udi.administering_medicine,
                is_medicinal_product: basic_udi.medicinal_product_check,
                is_reprocessed: udidi.reprocessed,
                is_reusable_surgical: basic_udi.reusable,
                production_identifier_types: production_ids,
                annex_xvi_types: annex_xvi,
                multi_component_type: multi_component,
                is_new_device: None,
                eu_status: CodeValue { value: status.to_string() },
                reusability,
                sterility,
            },
        },
        referenced_file_module,
        regulated_trade_item_module: regulated_module,
        sales_module,
        description_module,
        is_base_unit: true,
        is_despatch_unit: false,
        is_orderable_unit: false,
        unit_descriptor: CodeValue { value: "BASE_UNIT_OR_EACH".to_string() },
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
            additional_classifications: classifications,
        },
        next_lower_level: None,
        target_market: TargetMarketObj {
            country_code: CodeValue { value: config.target_market.country_code.clone() },
        },
        contact_information: contacts,
        synchronisation_dates: TradeItemSynchronisationDates::default(),
        global_model_info: vec![GlobalModelInformation {
            number: basic_udi_di.to_string(),
            descriptions: model_desc,
        }],
        gtin: base_di.to_string(),
        additional_identification: additional_ids,
        referenced_trade_items: Vec::new(),
    })
}

fn transform_lang_names(names: &Option<Vec<LanguageSpecificName>>) -> Vec<LangValue> {
    let mut result: Vec<LangValue> = names.as_ref()
        .map(|n| n.iter().filter_map(|name| {
            let lang = name.language.as_deref()?.to_lowercase();
            let val = name.text_value.as_deref()?;
            Some(LangValue {
                language_code: lang,
                value: val.to_string(),
            })
        }).collect())
        .unwrap_or_default();
    result.sort_by(|a, b| lang_sort_key(&a.language_code).cmp(&lang_sort_key(&b.language_code)));
    result
}

fn transform_lang_names_vec(names: &[LanguageSpecificName]) -> Vec<LangValue> {
    let mut result: Vec<LangValue> = names.iter().filter_map(|name| {
        let val = name.text_value.as_deref()?;
        let lang = name.language.as_deref()
            .map(|l| l.to_lowercase())
            .unwrap_or_else(|| "en".to_string());
        Some(LangValue {
            language_code: lang,
            value: val.to_string(),
        })
    }).collect();
    result.sort_by(|a, b| lang_sort_key(&a.language_code).cmp(&lang_sort_key(&b.language_code)));
    result
}

/// Sort languages in priority order: en, fr, de, it, then alphabetical
fn lang_sort_key(lang: &str) -> u8 {
    match lang {
        "en" => 0,
        "fr" => 1,
        "de" => 2,
        "it" => 3,
        _ => 4,
    }
}

fn transform_storage_handling(udidi: &MdrUdidiData) -> Vec<ClinicalStorageHandling> {
    udidi.storage_handling_conditions.iter().map(|cond| {
        let code = cond.value.as_deref().unwrap_or("");
        let gs1_code = mappings::storage_handling_to_gs1(code);
        let descriptions = transform_lang_names_vec(&cond.comments);

        ClinicalStorageHandling {
            type_code: CodeValue { value: gs1_code },
            descriptions,
        }
    }).collect()
}

fn transform_clinical_sizes(udidi: &MdrUdidiData) -> Vec<ClinicalSizeOutput> {
    udidi.clinical_sizes.iter().map(|size| {
        let size_type_eu = size.clinical_size_type.as_deref().unwrap_or("");
        let gs1_type = mappings::clinical_size_type_to_gs1(size_type_eu);
        let xsi_type = size.size_type.as_deref().unwrap_or("");

        let unit = size.value_unit.as_deref()
            .map(mappings::measurement_unit_to_gs1)
            .unwrap_or("");

        match xsi_type {
            "RangeClinicalSizeType" => {
                let min_val: f64 = size.minimum.as_deref().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let max_val: f64 = size.maximum.as_deref().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                ClinicalSizeOutput {
                    type_code: CodeValue { value: gs1_type.to_string() },
                    values: vec![MeasurementValue { unit_code: unit.to_string(), value: min_val }],
                    maximums: vec![MeasurementValue { unit_code: unit.to_string(), value: max_val }],
                    precision: CodeValue { value: "RANGE".to_string() },
                    text: None,
                }
            }
            "TextClinicalSizeType" => {
                ClinicalSizeOutput {
                    type_code: CodeValue { value: gs1_type.to_string() },
                    values: vec![],
                    maximums: vec![],
                    precision: CodeValue { value: "TEXT".to_string() },
                    text: size.text.clone(),
                }
            }
            "ValueClinicalSizeType" | _ => {
                let val: f64 = size.value.as_deref().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                ClinicalSizeOutput {
                    type_code: CodeValue { value: gs1_type.to_string() },
                    values: vec![MeasurementValue { unit_code: unit.to_string(), value: val }],
                    maximums: vec![],
                    precision: CodeValue { value: "VALUE".to_string() },
                    text: None,
                }
            }
        }
    }).collect()
}

fn transform_warnings(udidi: &MdrUdidiData) -> Vec<ClinicalWarningOutput> {
    udidi.critical_warnings.iter().map(|w| {
        let code = w.warning_value.as_deref().unwrap_or("");
        let descriptions = transform_lang_names_vec(&w.comments);

        ClinicalWarningOutput {
            agency_code: CodeValue { value: "EUDAMED".to_string() },
            warning_code: code.to_string(),
            descriptions,
        }
    }).collect()
}

fn transform_substances(udidi: &MdrUdidiData, config: &Config) -> Option<ChemicalRegulationInformationModule> {
    if udidi.substances.is_empty() {
        return None;
    }

    let mut chem_infos: Vec<ChemicalRegulationInformation> = Vec::new();

    for substance in &udidi.substances {
        let xsi_type = substance.substance_type.as_deref().unwrap_or("");
        let sub_type = substance.sub_type.as_deref().unwrap_or("");

        let (agency, regulation_name, chemical_type_code, cmr_type) = match xsi_type {
            "CMRSubstanceType" => {
                ("ECHA", "ECICS", "CMR_SUBSTANCE", Some(sub_type.to_string()))
            }
            "EndocrineSubstanceType" => {
                ("ECHA", "ECICS", "ENDOCRINE_SUBSTANCE", None)
            }
            "MedicalHumanProductSubstanceType" => {
                let gs1_type = mappings::substance_type_to_gs1(sub_type);
                ("WHO", "INN", gs1_type, None)
            }
            _ => ("WHO", "INN", sub_type, None),
        };

        // Build chemicals
        let has_names = !substance.names.is_empty();
        let has_inn = substance.inn.is_some();

        if xsi_type == "EndocrineSubstanceType" {
            // Endocrine: EC/CAS identifiers from config, combined into single entry
            let name_text = substance.names.first()
                .and_then(|n| n.text_value.as_deref())
                .unwrap_or("");

            let lookup = config.endocrine_substances.get(name_text);

            let mut chemicals = Vec::new();

            if let Some(ids) = lookup {
                let descriptions = transform_lang_names_vec(&substance.names);
                if let Some(ref ec) = ids.ec_number {
                    chemicals.push(RegulatedChemical {
                        identifier_ref: Some(ChemicalIdentifierRef {
                            agency_name: "EC".to_string(),
                            value: ec.clone(),
                        }),
                        chemical_name: None,
                        descriptions: descriptions.clone(),
                        cmr_type: None,
                        chemical_type: CodeValue { value: chemical_type_code.to_string() },
                    });
                }
                if let Some(ref cas) = ids.cas_number {
                    chemicals.push(RegulatedChemical {
                        identifier_ref: Some(ChemicalIdentifierRef {
                            agency_name: "CAS".to_string(),
                            value: cas.clone(),
                        }),
                        chemical_name: None,
                        descriptions: descriptions.clone(),
                        cmr_type: None,
                        chemical_type: CodeValue { value: chemical_type_code.to_string() },
                    });
                }
            }

            if chemicals.is_empty() {
                let descriptions = transform_lang_names_vec(&substance.names);
                chemicals.push(RegulatedChemical {
                    identifier_ref: None,
                    chemical_name: None,
                    descriptions,
                    cmr_type: None,
                    chemical_type: CodeValue { value: chemical_type_code.to_string() },
                });
            }

            // Combine EC and CAS into a single ChemicalRegulationInformation entry
            chem_infos.push(ChemicalRegulationInformation {
                agency: agency.to_string(),
                regulations: vec![ChemicalRegulation {
                    regulation_name: regulation_name.to_string(),
                    chemicals,
                }],
            });
        } else if has_names {
            let descriptions = transform_lang_names_vec(&substance.names);
            chem_infos.push(ChemicalRegulationInformation {
                agency: agency.to_string(),
                regulations: vec![ChemicalRegulation {
                    regulation_name: regulation_name.to_string(),
                    chemicals: vec![RegulatedChemical {
                        identifier_ref: None,
                        chemical_name: None,
                        descriptions,
                        cmr_type: cmr_type.map(|t| CodeValue { value: t }),
                        chemical_type: CodeValue { value: chemical_type_code.to_string() },
                    }],
                }],
            });
        } else if has_inn {
            chem_infos.push(ChemicalRegulationInformation {
                agency: agency.to_string(),
                regulations: vec![ChemicalRegulation {
                    regulation_name: regulation_name.to_string(),
                    chemicals: vec![RegulatedChemical {
                        identifier_ref: None,
                        chemical_name: substance.inn.clone(),
                        descriptions: vec![],
                        cmr_type: cmr_type.map(|t| CodeValue { value: t }),
                        chemical_type: CodeValue { value: chemical_type_code.to_string() },
                    }],
                }],
            });
        }
    }

    if chem_infos.is_empty() {
        None
    } else {
        // Sort: WHO first, then ECHA; within each agency sort by chemical type
        chem_infos.sort_by(|a, b| {
            let a_key = substance_sort_key(&a.agency, &a.regulations);
            let b_key = substance_sort_key(&b.agency, &b.regulations);
            a_key.cmp(&b_key)
        });
        Some(ChemicalRegulationInformationModule { infos: chem_infos })
    }
}

fn substance_sort_key(agency: &str, regulations: &[ChemicalRegulation]) -> (u8, u8) {
    let agency_key = match agency {
        "WHO" => 0,
        "ECHA" => 1,
        _ => 2,
    };
    let type_key = regulations.first()
        .and_then(|r| r.chemicals.first())
        .map(|c| match c.chemical_type.value.as_str() {
            "MEDICINAL_PRODUCT" => 0,
            "HUMAN_PRODUCT" => 1,
            "ENDOCRINE_SUBSTANCE" => 0,
            "CMR_SUBSTANCE" => 1,
            _ => 2,
        })
        .unwrap_or(2);
    (agency_key, type_key)
}

fn transform_market_info(udidi: &MdrUdidiData) -> Option<SalesInformationModule> {
    if udidi.market_infos.is_empty() {
        return None;
    }

    let mut conditions: Vec<TargetMarketSalesCondition> = udidi.market_infos.iter().map(|mi| {
        let is_original = mi.original_placed.unwrap_or(false);
        let condition_code = if is_original {
            "ORIGINAL_PLACED"
        } else {
            "ADDITIONAL_MARKET_AVAILABILITY"
        };

        let country = mi.country.as_deref().unwrap_or("");
        let numeric_country = mappings::country_alpha2_to_numeric(country);

        let start = mi.start_date.as_deref().unwrap_or("");
        let end = mi.end_date.as_deref();

        let start_dt = convert_date_to_datetime(start, false);
        let end_dt = end.map(|d| convert_date_to_datetime(d, true));

        TargetMarketSalesCondition {
            condition_code: CodeValue { value: condition_code.to_string() },
            countries: vec![SalesConditionCountry {
                country_code: CodeValue { value: numeric_country.to_string() },
                end_datetime: end_dt,
                start_datetime: start_dt,
            }],
        }
    }).collect();

    // Sort: ORIGINAL_PLACED first, then by country code
    conditions.sort_by(|a, b| {
        let a_orig = a.condition_code.value == "ORIGINAL_PLACED";
        let b_orig = b.condition_code.value == "ORIGINAL_PLACED";
        b_orig.cmp(&a_orig).then_with(|| {
            let a_cc = a.countries.first().map(|c| &c.country_code.value).map(|s| s.as_str()).unwrap_or("");
            let b_cc = b.countries.first().map(|c| &c.country_code.value).map(|s| s.as_str()).unwrap_or("");
            a_cc.cmp(b_cc)
        })
    });

    Some(SalesInformationModule {
        sales: SalesInformation { conditions },
    })
}

/// Convert EUDAMED date "2026-02-03+01:00" to datetime.
/// Start dates use T13:00:00+00:00, end dates use T21:00:00+00:00.
fn convert_date_to_datetime(date_str: &str, is_end_date: bool) -> String {
    let date_part = if date_str.contains('+') && !date_str.contains('T') {
        date_str.split('+').next().unwrap_or(date_str)
    } else if date_str.contains('T') {
        return date_str.to_string();
    } else {
        date_str
    };
    let time = if is_end_date { "21:00:00" } else { "13:00:00" };
    format!("{}T{}+00:00", date_part, time)
}

/// Sort production identifiers: SERIAL_NUMBER, MANUFACTURING_DATE, BATCH_NUMBER, ...
fn prod_id_sort_key(id: &str) -> u8 {
    match id {
        "SERIAL_NUMBER" => 0,
        "MANUFACTURING_DATE" => 1,
        "BATCH_NUMBER" => 2,
        "EXPIRATION_DATE" => 3,
        "SOFTWARE_IDENTIFICATION" => 4,
        _ => 5,
    }
}

fn generate_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
