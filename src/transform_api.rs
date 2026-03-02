use crate::api_json::ApiDevice;
use crate::config::Config;
use crate::firstbase::*;
use crate::mappings;
use chrono::Local;

/// Transform an API device listing record into a firstbase TradeItem.
/// This is a "best-effort" mapping from the flat listing data - the listing
/// has limited fields compared to the full DTX XML / detail endpoint.
pub fn transform_api_device(device: &ApiDevice, config: &Config) -> TradeItem {
    let now = Local::now();
    let now_str = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    let gtin = device.primary_di.clone().unwrap_or_default();
    let basic_udi = device.basic_udi.clone().unwrap_or_default();

    // Risk class → AdditionalTradeItemClassification (system 76)
    let mut additional_classifications = Vec::new();
    if let Some(rc) = device.risk_class_code() {
        let gs1_risk = mappings::risk_class_to_gs1(&rc);
        additional_classifications.push(AdditionalClassification {
            system_code: CodeValue {
                value: "76".to_string(),
            },
            values: vec![AdditionalClassificationValue {
                code_value: gs1_risk.to_string(),
            }],
        });
    }

    // Device status
    let status_code = device
        .status_code()
        .map(|s| mappings::device_status_to_gs1(&s).to_string())
        .unwrap_or_default();

    // Manufacturer contact info
    let mut contacts = Vec::new();
    if let Some(ref mf_srn) = device.manufacturer_srn {
        contacts.push(TradeItemContactInformation {
            contact_type: CodeValue {
                value: "EMA".to_string(),
            },
            party_identification: vec![AdditionalPartyIdentification {
                type_code: "SRN".to_string(),
                value: mf_srn.clone(),
            }],
            contact_name: device.manufacturer_name.clone(),
            addresses: Vec::new(),
            communication_channels: Vec::new(),
        });
    }

    // Authorised representative contact info
    if let Some(ref ar_srn) = device.authorised_representative_srn {
        contacts.push(TradeItemContactInformation {
            contact_type: CodeValue {
                value: "EAR".to_string(),
            },
            party_identification: vec![AdditionalPartyIdentification {
                type_code: "SRN".to_string(),
                value: ar_srn.clone(),
            }],
            contact_name: device.authorised_representative_name.clone(),
            addresses: Vec::new(),
            communication_channels: Vec::new(),
        });
    }

    // Trade name → description
    let description_module = device.trade_name.as_ref().map(|tn| {
        TradeItemDescriptionModule {
            info: TradeItemDescriptionInformation {
                additional_descriptions: Vec::new(),
                descriptions: vec![LangValue {
                    language_code: "en".to_string(),
                    value: tn.clone(),
                }],
            },
        }
    });

    // Reference → additional trade item identification
    let mut additional_identification = Vec::new();
    if let Some(ref reference) = device.reference {
        if reference != "-" && !reference.is_empty() {
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: "MANUFACTURER_PART_NUMBER".to_string(),
                value: reference.clone(),
            });
        }
    }

    // Sterile field - in the listing it's sometimes a number (0.0/1.0) or null
    let sterile_bool = match &device.sterile {
        Some(serde_json::Value::Bool(b)) => Some(*b),
        Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f != 0.0),
        _ => None,
    };

    let sterility = sterile_bool.map(|s| {
        if s {
            SterilityInformation {
                manufacturer_sterilisation: vec![CodeValue {
                    value: config
                        .sterilisation_method
                        .clone()
                        .unwrap_or_else(|| "UNSPECIFIED".to_string()),
                }],
                prior_to_use: Vec::new(),
            }
        } else {
            SterilityInformation {
                manufacturer_sterilisation: vec![CodeValue {
                    value: "NOT_STERILISED".to_string(),
                }],
                prior_to_use: Vec::new(),
            }
        }
    });

    TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["HEALTHCARE".to_string(), "UDI_REGISTRY".to_string()],
        chemical_regulation_module: None,
        healthcare_item_module: None,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: None,
                device_count: None,
                direct_marking: Vec::new(),
                measuring_function: None,
                is_active: None,
                administer_medicine: None,
                is_medicinal_product: None,
                is_reprocessed: None,
                is_reusable_surgical: None,
                production_identifier_types: Vec::new(),
                annex_xvi_types: Vec::new(),
                multi_component_type: None,
                is_new_device: None,
                eu_status: CodeValue {
                    value: status_code,
                },
                reusability: None,
                sterility,
            },
        },
        referenced_file_module: None,
        regulated_trade_item_module: None,
        sales_module: None,
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
            additional_classifications,
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
            number: basic_udi,
            descriptions: Vec::new(),
        }],
        gtin,
        additional_identification,
        referenced_trade_items: Vec::new(),
    }
}
