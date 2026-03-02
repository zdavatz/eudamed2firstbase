use crate::config::Config;
use crate::eudamed_json::EudamedDevice;
use crate::firstbase::*;
use crate::mappings;
use chrono::Local;

/// Transform an EUDAMED JSON device record into a firstbase TradeItem.
pub fn transform_eudamed_device(device: &EudamedDevice, config: &Config) -> TradeItem {
    let now = Local::now();
    let now_str = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    let basic_udi = device.basic_udi_code();

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

    // Manufacturer contact info
    let mut contacts = Vec::new();
    if let Some(ref mfr) = device.manufacturer {
        if let Some(ref srn) = mfr.srn {
            let mut addresses = Vec::new();
            if let Some(ref addr) = mfr.geographical_address {
                if !addr.is_empty() {
                    addresses.push(StructuredAddress {
                        city: String::new(),
                        country_code: CodeValue {
                            value: mfr.country_iso2_code.clone().unwrap_or_default(),
                        },
                        postal_code: String::new(),
                        street: addr.clone(),
                        street_number: None,
                    });
                }
            }

            let mut comm_channels = Vec::new();
            if let Some(ref email) = mfr.electronic_mail {
                if !email.is_empty() {
                    comm_channels.push(CommunicationChannel {
                        channel_code: CodeValue {
                            value: "EMAIL".to_string(),
                        },
                        value: email.clone(),
                    });
                }
            }
            if let Some(ref phone) = mfr.telephone {
                if !phone.is_empty() {
                    comm_channels.push(CommunicationChannel {
                        channel_code: CodeValue {
                            value: "TELEPHONE".to_string(),
                        },
                        value: phone.clone(),
                    });
                }
            }

            let communication_channels = if comm_channels.is_empty() {
                Vec::new()
            } else {
                vec![TargetMarketCommunicationChannel {
                    channels: comm_channels,
                }]
            };

            contacts.push(TradeItemContactInformation {
                contact_type: CodeValue {
                    value: "EMA".to_string(),
                },
                party_identification: vec![AdditionalPartyIdentification {
                    type_code: "SRN".to_string(),
                    value: srn.clone(),
                }],
                contact_name: mfr.name.clone(),
                addresses,
                communication_channels,
            });
        }
    }

    // Authorised representative contact info
    if let Some(ref ar) = device.authorised_representative {
        if let Some(ref srn) = ar.srn {
            let mut addresses = Vec::new();
            if let Some(ref addr) = ar.address {
                if !addr.is_empty() {
                    addresses.push(StructuredAddress {
                        city: String::new(),
                        country_code: CodeValue {
                            value: String::new(),
                        },
                        postal_code: String::new(),
                        street: addr.clone(),
                        street_number: None,
                    });
                }
            }

            let mut comm_channels = Vec::new();
            if let Some(ref email) = ar.email {
                if !email.is_empty() {
                    comm_channels.push(CommunicationChannel {
                        channel_code: CodeValue {
                            value: "EMAIL".to_string(),
                        },
                        value: email.clone(),
                    });
                }
            }
            if let Some(ref phone) = ar.telephone {
                if !phone.is_empty() {
                    comm_channels.push(CommunicationChannel {
                        channel_code: CodeValue {
                            value: "TELEPHONE".to_string(),
                        },
                        value: phone.clone(),
                    });
                }
            }

            let communication_channels = if comm_channels.is_empty() {
                Vec::new()
            } else {
                vec![TargetMarketCommunicationChannel {
                    channels: comm_channels,
                }]
            };

            contacts.push(TradeItemContactInformation {
                contact_type: CodeValue {
                    value: "EAR".to_string(),
                },
                party_identification: vec![AdditionalPartyIdentification {
                    type_code: "SRN".to_string(),
                    value: srn.clone(),
                }],
                contact_name: ar.name.clone(),
                addresses,
                communication_channels,
            });
        }
    }

    // Description from deviceName
    let description_module = device.device_name.as_ref().map(|name| {
        TradeItemDescriptionModule {
            info: TradeItemDescriptionInformation {
                descriptions: vec![LangValue {
                    language_code: "en".to_string(),
                    value: name.clone(),
                }],
                additional_descriptions: Vec::new(),
            },
        }
    });

    // Sterility
    let sterility = device.sterile.map(|s| {
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

    // Reusability
    let reusability = if device.reusable == Some(false) {
        Some(ReusabilityInformation {
            reusability_type: CodeValue {
                value: "SINGLE_USE".to_string(),
            },
            max_cycles: None,
        })
    } else if device.reusable == Some(true) {
        Some(ReusabilityInformation {
            reusability_type: CodeValue {
                value: "LIMITED_REUSABLE".to_string(),
            },
            max_cycles: None,
        })
    } else {
        None
    };

    TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["HEALTHCARE".to_string()],
        chemical_regulation_module: None,
        healthcare_item_module: None,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: device.implantable.map(|b| if b { "TRUE".to_string() } else { "FALSE".to_string() }),
                device_count: None,
                direct_marking: Vec::new(),
                measuring_function: device.measuring_function,
                is_active: None,
                administer_medicine: device.administering_medicine,
                is_medicinal_product: device.medicinal_product,
                is_reprocessed: None,
                is_reusable_surgical: None,
                production_identifier_types: Vec::new(),
                annex_xvi_types: Vec::new(),
                multi_component_type: None,
                eu_status: CodeValue {
                    value: String::new(),
                },
                reusability,
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
        gtin: String::new(), // No GTIN in EUDAMED JSON device-level records
        additional_identification: Vec::new(),
    }
}
