use crate::api_detail::{
    ApiDeviceDetail, BasicUdiDiData, CmrSubstance, ContainedItemNode, Substance,
};
use crate::config::Config;
use crate::firstbase::*;
use crate::mappings;
use chrono::Utc;

/// Transform a full API device detail record into a firstbase TradeItem.
/// Optional `basic_udi` provides real MDR mandatory fields from the Basic UDI-DI level.
pub fn transform_detail_device(
    device: &ApiDeviceDetail,
    config: &Config,
    basic_udi: Option<&BasicUdiDiData>,
) -> TradeItem {
    let now = Utc::now();
    let now_str = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    // Use version_date for effectiveDateTime; lastChangeDateTime uses current time (avoids SYS25 on re-uploads)
    let effective_date = device
        .version_date
        .as_ref()
        .filter(|d| !d.is_empty())
        .cloned()
        .unwrap_or_else(|| now_str.clone());

    let gtin = device.gtin();

    // --- Device status ---
    let eudamed_status = device.status_code().unwrap_or_default();
    let status_code = mappings::device_status_to_gs1(&eudamed_status).to_string();

    // discontinuedDateTime: today+1 day when NO_LONGER_ON_THE_MARKET
    let discontinued = if eudamed_status == "NO_LONGER_PLACED_ON_THE_MARKET"
        || eudamed_status == "NO_LONGER_ON_THE_MARKET"
    {
        let tomorrow = now + chrono::Duration::days(1);
        Some(tomorrow.format("%Y-%m-%dT%H:%M:%S").to_string())
    } else {
        None
    };

    // --- Regulatory act (needed early for legacy detection) ---
    // Prefer legislation field (more accurate: distinguishes MDD from MDR for same risk classes)
    // Fall back to risk class inference
    let reg_act = basic_udi
        .and_then(|b| b.regulatory_act())
        .or_else(|| {
            basic_udi
                .and_then(|b| b.risk_class_code())
                .map(|rc| mappings::regulation_from_risk_class_refdata(&rc).to_string())
        })
        .unwrap_or_else(|| "MDR".to_string());
    let is_legacy = matches!(reg_act.as_str(), "MDD" | "AIMDD" | "IVDD");
    let is_ivdr = reg_act == "IVDR" || reg_act == "IVDD";
    let is_mdr = reg_act == "MDR";

    // 097.096: Since 2026-03-10, downgraded from error to warning — legacy devices publishable
    if is_legacy {
        eprintln!(
            "Info: {} is a legacy {} device (097.096 now warning only)",
            device.uuid.as_deref().unwrap_or("unknown"),
            reg_act
        );
    }

    // --- Production identifiers ---
    // 097.095: Legacy devices (MDD/AIMDD/IVDD) must NOT have production identifiers.
    // MDR/IVDR: udiPiType is mandatory in EUDAMED, so production_identifiers() is never empty.
    let raw_production_ids: Vec<String> = device.production_identifiers();
    let production_ids: Vec<CodeValue> = if is_legacy {
        Vec::new()
    } else {
        raw_production_ids
            .iter()
            .map(|id| CodeValue { value: id.clone() })
            .collect()
    };

    // 097.091: SOFTWARE_IDENTIFICATION requires specialDeviceTypeCode = SOFTWARE
    let special_device_type = if raw_production_ids
        .iter()
        .any(|id| id == "SOFTWARE_IDENTIFICATION")
    {
        Some(CodeValue {
            value: "SOFTWARE".to_string(),
        })
    } else {
        None
    };

    // --- Sterility ---
    let sterility = build_sterility(device, config);

    // --- Reusability ---
    let reusability = build_reusability(device);

    // Real SPP (MDR Art. 22(1)/(3)) vs MDR device with multi-component shape
    // (MDR Art. 22(4), "Procedure pack which is a device in itself"):
    //   - criterion="SPP"      (FLD-UDID-261) → systemOrProcedurePackTypeCode
    //   - criterion="STANDARD" (FLD-UDID-12)  → multiComponentDeviceTypeCode
    // Discriminator is `multiComponent.criterion`, not `code` — see issue #31.
    //
    // SPP is an MDR-only concept (no SPP under IVDR/legacy). 097.049 forbids
    // systemOrProcedurePackTypeCode whenever ContactType=EMA (any legislation),
    // so we gate is_system_or_pack on is_mdr — this keeps the three rules
    // (097.016 / 097.049 / 097.056) consistent.
    let is_system_or_pack = basic_udi.map(|b| b.is_spp()).unwrap_or(false) && is_mdr;

    // --- Contacts ---
    let mut contacts = build_contacts(device);

    // 097.016: SPP+MDR ⇒ ContactType MUST be EPP with SRN
    // 097.049: ContactType=EMA ⇒ systemOrProcedurePackTypeCode MUST NOT be used
    // 097.056: ContactType=EPP ⇒ regulatoryAct MUST be MDR + agency MUST be EU
    //
    // The three rules together mean ContactType is fully determined by whether
    // the device is an SPP under MDR — not by the SRN prefix. An MF-actor that
    // registered an SPP device in EUDAMED still gets EPP (with their MF-SRN);
    // a PR-actor that registered a non-SPP device still gets EMA. See issue #30,
    // #33, and Maik's clarification 2026-05-03.
    let mfr_srn_val = basic_udi
        .and_then(|b| b.manufacturer.as_ref())
        .and_then(|m| m.srn.clone())
        .unwrap_or_else(|| "XX-MF-000000000".to_string());
    let contact_type_code = if is_system_or_pack { "EPP" } else { "EMA" };
    let has_contact = contacts
        .iter()
        .any(|c| c.contact_type.value == contact_type_code);
    if !has_contact {
        let mfr_name = basic_udi
            .and_then(|b| b.manufacturer.as_ref())
            .and_then(|m| m.name.clone());
        contacts.push(TradeItemContactInformation {
            contact_type: CodeValue {
                value: contact_type_code.to_string(),
            },
            party_identification: vec![AdditionalPartyIdentification {
                type_code: "SRN".to_string(),
                value: mfr_srn_val.clone(),
            }],
            contact_name: mfr_name,
            addresses: Vec::new(),
            communication_channels: Vec::new(),
        });
    }

    // 097.054: Non-EU manufacturers need EAR contact — only if AR exists in EUDAMED
    let is_non_eu = !is_eu_srn(&mfr_srn_val);
    if is_non_eu {
        let has_ear = contacts.iter().any(|c| c.contact_type.value == "EAR");
        if !has_ear {
            if let Some(ar) = basic_udi.and_then(|b| b.authorised_representative.as_ref()) {
                if let Some(ref ar_srn) = ar.srn {
                    contacts.push(TradeItemContactInformation {
                        contact_type: CodeValue {
                            value: "EAR".to_string(),
                        },
                        party_identification: vec![AdditionalPartyIdentification {
                            type_code: "SRN".to_string(),
                            value: ar_srn.clone(),
                        }],
                        contact_name: ar.name.clone(),
                        addresses: Vec::new(),
                        communication_channels: Vec::new(),
                    });
                }
            }
        }
    }

    // --- Trade name / description ---
    let trade_names = device.trade_name_texts();
    let additional_descs = device.additional_description_texts();
    let description_module = if !trade_names.is_empty() || !additional_descs.is_empty() {
        Some(TradeItemDescriptionModule {
            info: TradeItemDescriptionInformation {
                description_short: trade_names
                    .iter()
                    .map(|(lang, text)| LangValue {
                        language_code: lang.clone(),
                        value: crate::firstbase::truncate_short_description(text),
                    })
                    .collect(),
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
    // 097.006: MANUFACTURER_PART_NUMBER is mandatory. Use reference, fallback to primary DI code.
    // GDSN limits additionalTradeItemIdentificationValue to 80 characters.
    let truncate_id = |s: String| -> String {
        if s.len() <= 80 {
            s
        } else {
            s.chars().take(80).collect()
        }
    };
    let mut additional_identification = Vec::new();
    let mfr_part = device
        .reference
        .as_ref()
        .filter(|r| r != &"-" && !r.is_empty())
        .cloned()
        .unwrap_or_else(|| device.primary_di_code());
    if !mfr_part.is_empty() {
        additional_identification.push(AdditionalTradeItemIdentification {
            type_code: "MANUFACTURER_PART_NUMBER".to_string(),
            value: truncate_id(mfr_part),
        });
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

    // --- Secondary DI → additional identification ---
    // 097.087: Only one secondary DI with type HIBC/ICCBBA/PPN/PZN allowed under MDR/IVDR.
    // Use issuing agency to determine the correct type code.
    if let Some(ref secondary) = device.secondary_di {
        if let Some(ref code) = secondary.code {
            let sec_type = secondary
                .issuing_agency
                .as_ref()
                .and_then(|a| a.code.as_ref())
                .map(|c| mappings::issuing_agency_to_type_code(c))
                .unwrap_or("GTIN_14");
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: sec_type.to_string(),
                value: code.clone(),
            });
        }
    }

    // MODEL_NUMBER from Basic UDI-DI deviceModel (FLD-UDID-20)
    if let Some(model) = basic_udi
        .and_then(|b| b.device_model.as_ref())
        .filter(|m| !m.is_empty())
    {
        additional_identification.push(AdditionalTradeItemIdentification {
            type_code: "MODEL_NUMBER".to_string(),
            value: truncate_id(model.clone()),
        });
    } else if is_legacy {
        // 097.025: Legacy devices (no globalModelInformation) need MODEL_NUMBER as fallback
        let model_number = basic_udi
            .and_then(|b| b.basic_udi.as_ref())
            .and_then(|di| di.code.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| device.primary_di_code());
        if !model_number.is_empty() {
            additional_identification.push(AdditionalTradeItemIdentification {
                type_code: "MODEL_NUMBER".to_string(),
                value: truncate_id(model_number),
            });
        }
    }

    // --- EMDN/CND nomenclature → additional classification system 88 ---
    let mut all_classifications = Vec::new();

    // Risk class from Basic UDI-DI → classification system 76 (MDR/IVDR) or 85 (MDD/AIMDD/IVDD)
    // 097.002/097.003/097.005: risk class value must match the local code list for the system
    // riskClass is mandatory in EUDAMED Basic UDI-DI — 0/100K records have null.
    // Fallback only triggers on BUDI cache miss (download.sh Step 3c ensures completeness).
    let risk_class_refdata = basic_udi.and_then(|b| b.risk_class_code());
    let risk_class_gs1 = risk_class_refdata
        .as_ref()
        .map(|rc| mappings::risk_class_refdata_to_gs1(rc).to_string())
        .unwrap_or_else(|| {
            eprintln!(
                "WARNING: No riskClass for {} — BUDI cache miss? Using EU_CLASS_I",
                device.uuid.as_deref().unwrap_or("unknown")
            );
            "EU_CLASS_I".to_string()
        });
    // 097.002: Legacy devices (MDD/AIMDD/IVDD) must use system 85, not 76
    let risk_class_system = if is_legacy {
        "85".to_string()
    } else {
        risk_class_refdata
            .as_ref()
            .map(|rc| mappings::risk_class_system_code(rc).to_string())
            .unwrap_or_else(|| "76".to_string())
    };
    all_classifications.push(AdditionalClassification {
        system_code: CodeValue {
            value: risk_class_system,
        },
        values: vec![AdditionalClassificationValue {
            code_value: risk_class_gs1.clone(),
        }],
    });

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
    // 097.078: all description fields must use consistent language codes
    let primary_lang = trade_names.first().map(|(l, _)| l.as_str()).unwrap_or("en");
    let healthcare_module =
        build_healthcare_module(device, basic_udi, is_ivdr, primary_lang, is_system_or_pack);

    // --- Chemical regulation module (substances) ---
    // 097.095: Legacy devices must not have CMR_SUBSTANCE or ENDOCRINE_SUBSTANCE
    let chemical_regulation_module = if is_legacy {
        None
    } else {
        build_chemical_regulation_module(device)
    };

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

    let regulated_trade_item_module = Some(RegulatedTradeItemModule {
        info: vec![RegulatoryInformation {
            act: reg_act.clone(),
            agency: "EU".to_string(),
        }],
    });

    // --- Sales module (market availability with ORIGINAL_PLACED distinction) ---
    // 097.021: NOT_INTENDED_FOR_EU_MARKET must not have country/sales data
    // 097.086: MDR SPP must NOT have targetMarketSalesConditions
    let sales_module = if eudamed_status == "NOT_INTENDED_FOR_EU_MARKET" || is_system_or_pack {
        None
    } else {
        build_sales_module(device, basic_udi)
    };

    // --- Direct marking DI ---
    // 097.095: Legacy devices must not have directPartMarkingIdentifier
    let direct_marking = if is_legacy {
        Vec::new()
    } else {
        build_direct_marking(device)
    };

    // --- Certification module (097.101: MDR Class III needs certificate) ---
    let certification_module = build_certification_module(basic_udi);

    // 097.101: MDR + EU_CLASS_III requires MDR_TECHNICAL_DOCUMENTATION or MDR_TYPE_EXAMINATION
    if reg_act == "MDR" && risk_class_gs1 == "EU_CLASS_III" {
        let has_required_cert = certification_module.as_ref().map_or(false, |cm| {
            cm.infos.iter().any(|ci| {
                ci.standard == "MDR_TECHNICAL_DOCUMENTATION"
                    || ci.standard == "MDR_TYPE_EXAMINATION"
            })
        });
        if !has_required_cert {
            eprintln!("Warning: {} is MDR Class III but has no MDR_TECHNICAL_DOCUMENTATION or MDR_TYPE_EXAMINATION certificate (097.101)",
                device.uuid.as_deref().unwrap_or("unknown"));
        }
    }

    // 097.105: MDD + Class IIA/IIB/III requires MDD certificate
    if reg_act == "MDD"
        && matches!(
            risk_class_gs1.as_str(),
            "EU_CLASS_IIA" | "EU_CLASS_IIB" | "EU_CLASS_III"
        )
    {
        let has_mdd_cert = certification_module.as_ref().map_or(false, |cm| {
            cm.infos.iter().any(|ci| ci.standard.starts_with("MDD_"))
        });
        if !has_mdd_cert {
            eprintln!(
                "Warning: {} is MDD {} but has no MDD certificate (097.105)",
                device.uuid.as_deref().unwrap_or("unknown"),
                risk_class_gs1
            );
        }
    }

    // --- Unit of Use DI (FLD-UDDI-135) ---
    let trade_item_information = build_unit_of_use(device);

    // --- Related devices (REPLACED/REPLACED_BY) ---
    let referenced_trade_items = build_referenced_trade_items(device);

    // --- Base quantity → device count ---
    // 097.095: Legacy devices must not have udidDeviceCount
    let device_count = if is_legacy {
        None
    } else {
        device.base_quantity
    };

    TradeItem {
        is_brand_bank_publication: false,
        target_sector: vec!["UDI_REGISTRY".to_string()],
        chemical_regulation_module,
        healthcare_item_module: healthcare_module,
        medical_device_module: MedicalDeviceTradeItemModule {
            info: MedicalDeviceInformation {
                is_implantable: if is_system_or_pack {
                    None
                } else {
                    Some(bool_str(
                        basic_udi.and_then(|b| b.implantable).unwrap_or(false),
                    ))
                },
                // 097.015: required when implantable=true and risk class=EU_CLASS_IIB
                is_exempt_from_implant_obligations: {
                    if is_system_or_pack {
                        None
                    } else {
                        let implantable = basic_udi.and_then(|b| b.implantable).unwrap_or(false);
                        if implantable && risk_class_gs1 == "EU_CLASS_IIB" {
                            Some(false)
                        } else {
                            None
                        }
                    }
                },
                device_count,
                direct_marking,
                measuring_function: if is_system_or_pack {
                    None
                } else {
                    Some(
                        basic_udi
                            .and_then(|b| b.measuring_function)
                            .unwrap_or(false),
                    )
                },
                is_active: if is_system_or_pack {
                    None
                } else {
                    Some(basic_udi.and_then(|b| b.active).unwrap_or(false))
                },
                administer_medicine: if is_system_or_pack {
                    None
                } else {
                    Some(
                        basic_udi
                            .and_then(|b| b.administering_medicine)
                            .unwrap_or(false),
                    )
                },
                is_medicinal_product: if is_system_or_pack {
                    None
                } else {
                    Some(basic_udi.and_then(|b| b.medicinal_product).unwrap_or(false))
                },
                is_reprocessed: if is_system_or_pack {
                    None
                } else {
                    device.reprocessed
                },
                is_reusable_surgical: if is_system_or_pack {
                    None
                } else {
                    Some(basic_udi.and_then(|b| b.reusable).unwrap_or(false))
                },
                production_identifier_types: production_ids,
                annex_xvi_types: Vec::new(),
                special_device_type,
                // 097.050: SPP uses SystemOrProcedurePackTypeCode, NOT MultiComponentDeviceTypeCode
                multi_component_type: if is_system_or_pack {
                    None
                } else {
                    Some(CodeValue {
                        value: basic_udi
                            .and_then(|b| b.multi_component_code())
                            .unwrap_or_else(|| "DEVICE".to_string()),
                    })
                },
                // SystemOrProcedurePackTypeCode + MedicalPurposeDescription: set for SPP devices
                system_or_procedure_pack_type: if is_system_or_pack {
                    Some(CodeValue {
                        value: basic_udi
                            .and_then(|b| b.multi_component_code())
                            .unwrap_or_else(|| "DEVICE".to_string()),
                    })
                } else {
                    None
                },
                // 097.049: SPP requires systemOrProcedurePackMedicalPurposeDescription
                // Source: BUDI medicalPurpose (NOT additionalDescription — those are separate fields)
                system_or_procedure_pack_purpose: if is_system_or_pack {
                    let purpose_texts = basic_udi
                        .map(|b| b.medical_purpose_texts())
                        .unwrap_or_default();
                    if purpose_texts.is_empty() {
                        // Fallback: use device name from BUDI
                        let name = basic_udi
                            .and_then(|b| b.device_name.as_ref())
                            .filter(|n| !n.is_empty())
                            .cloned()
                            .unwrap_or_else(|| device.primary_di_code());
                        vec![LangValue {
                            language_code: primary_lang.to_string(),
                            value: name,
                        }]
                    } else {
                        purpose_texts
                            .iter()
                            .map(|(lang, text)| LangValue {
                                language_code: lang.clone(),
                                value: text.clone(),
                            })
                            .collect()
                    }
                } else {
                    Vec::new()
                },
                // 097.047: isNewDevice mandatory for IVDR
                is_new_device: if is_ivdr {
                    Some(device.new_device.unwrap_or(false))
                } else {
                    device.new_device
                },
                // 097.046: IVDR-specific boolean fields, default to false
                is_reagent: if is_ivdr { Some(false) } else { None },
                is_instrument: if is_ivdr { Some(false) } else { None },
                is_patient_self_testing: if is_ivdr { Some(false) } else { None },
                is_near_patient_testing: if is_ivdr { Some(false) } else { None },
                is_professional_testing: if is_ivdr { Some(false) } else { None },
                is_companion_diagnostic: if is_ivdr { Some(false) } else { None },
                eu_status: CodeValue { value: status_code },
                reusability,
                sterility,
            },
        },
        certification_module,
        referenced_file_module,
        regulated_trade_item_module,
        sales_module,
        description_module,
        is_base_unit: true,
        is_despatch_unit: true, // BASE_UNIT_OR_EACH is highest level = despatch unit
        is_orderable_unit: true,
        unit_descriptor: CodeValue {
            value: "BASE_UNIT_OR_EACH".to_string(),
        },
        trade_channel_code: vec![CodeValue {
            value: "UDI_REGISTRY".to_string(),
        }],
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
            effective: effective_date,
            publication: now_str,
            discontinued,
        },
        // 097.095: Legacy devices must not have globalModelNumber
        global_model_info: if is_legacy {
            Vec::new()
        } else {
            vec![GlobalModelInformation {
                number: basic_udi
                    .and_then(|b| b.basic_udi.as_ref())
                    .and_then(|di| di.code.clone())
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| device.primary_di_code()), // fallback to primary DI
                descriptions: {
                    // 097.025: GlobalModelDescription uses deviceName (FLD-UDID-22) from Basic UDI-DI
                    // languageCode 'en' is required
                    let device_name = basic_udi
                        .and_then(|b| b.device_name.as_ref())
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| device.primary_di_code());
                    vec![LangValue {
                        language_code: "en".to_string(),
                        value: device_name,
                    }]
                },
            }]
        },
        gtin,
        additional_identification,
        referenced_trade_items,
        trade_item_information,
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

/// Check if an SRN prefix indicates an EU member state.
/// SRN format: CC-XX-NNNNNN where CC is the country code.
/// Note: EEA-only countries (IS, LI, NO) are excluded — EUDAMED treats them
/// as non-EU for EAR (authorised representative) purposes (097.054).
fn is_eu_srn(srn: &str) -> bool {
    let prefix = srn.split('-').next().unwrap_or("");
    matches!(
        prefix,
        "AT" | "BE"
            | "BG"
            | "HR"
            | "CY"
            | "CZ"
            | "DK"
            | "EE"
            | "FI"
            | "FR"
            | "DE"
            | "GR"
            | "HU"
            | "IE"
            | "IT"
            | "LV"
            | "LT"
            | "LU"
            | "MT"
            | "NL"
            | "PL"
            | "PT"
            | "RO"
            | "SK"
            | "SI"
            | "ES"
            | "SE"
    )
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
                let country_numeric = actor
                    .country_iso2_code
                    .as_ref()
                    .map(|c| mappings::country_alpha2_to_numeric(c).to_string())
                    .unwrap_or_default();
                addresses.push(StructuredAddress {
                    city,
                    country_code: CodeValue {
                        value: country_numeric,
                    },
                    postal_code: postal,
                    street,
                    street_number: if number.is_empty() {
                        None
                    } else {
                        Some(number)
                    },
                });
            }

            let mut channels = Vec::new();
            if let Some(ref phone) = actor.telephone {
                if !phone.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue {
                                value: "TELEPHONE".to_string(),
                            },
                            value: phone.clone(),
                        }],
                    });
                }
            }
            if let Some(ref email) = actor.electronic_mail {
                if !email.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue {
                                value: "EMAIL".to_string(),
                            },
                            value: email.clone(),
                        }],
                    });
                }
            }

            contacts.push(TradeItemContactInformation {
                contact_type: CodeValue {
                    value: "EPD".to_string(),
                },
                party_identification: party_ids,
                contact_name: actor.name.clone(),
                addresses,
                communication_channels: channels,
            });
        } else if let Some(ref org) = pd.oem_organisation {
            // Non-registered organisation
            let mut addresses = Vec::new();
            if let Some((street, number, postal, city)) = org.structured_address() {
                let country_numeric = org
                    .country_iso2()
                    .map(|c| mappings::country_alpha2_to_numeric(&c).to_string())
                    .unwrap_or_default();
                addresses.push(StructuredAddress {
                    city,
                    country_code: CodeValue {
                        value: country_numeric,
                    },
                    postal_code: postal,
                    street,
                    street_number: if number.is_empty() {
                        None
                    } else {
                        Some(number)
                    },
                });
            }

            let mut channels = Vec::new();
            if let Some(ref phone) = org.telephone {
                if !phone.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue {
                                value: "TELEPHONE".to_string(),
                            },
                            value: phone.clone(),
                        }],
                    });
                }
            }
            if let Some(ref email) = org.electronic_mail {
                if !email.is_empty() {
                    channels.push(TargetMarketCommunicationChannel {
                        channels: vec![CommunicationChannel {
                            channel_code: CodeValue {
                                value: "EMAIL".to_string(),
                            },
                            value: email.clone(),
                        }],
                    });
                }
            }

            contacts.push(TradeItemContactInformation {
                contact_type: CodeValue {
                    value: "EPD".to_string(),
                },
                party_identification: Vec::new(),
                contact_name: org.name.clone(),
                addresses,
                communication_channels: channels,
            });
        }
    }

    contacts
}

fn build_healthcare_module(
    device: &ApiDeviceDetail,
    basic_udi: Option<&BasicUdiDiData>,
    is_ivdr: bool,
    primary_lang: &str,
    is_system_or_pack: bool,
) -> Option<HealthcareItemInformationModule> {
    let clinical_sizes = build_clinical_sizes(device);
    let storage_handling = build_storage_handling(device, primary_lang);
    let clinical_warnings = build_clinical_warnings(device);
    let contains_latex = Some(
        device
            .latex
            .map(|b| bool_str(b))
            .unwrap_or_else(|| "FALSE".to_string()),
    );

    Some(HealthcareItemInformationModule {
        info: HealthcareItemInformation {
            // 097.046: microbial substance mandatory for IVDR/IVDD
            contains_microbial_substance: if is_ivdr { Some(false) } else { None },
            human_blood_derivative: if is_system_or_pack {
                None
            } else {
                Some(bool_str(
                    basic_udi.and_then(|b| b.human_product).unwrap_or(false),
                ))
            },
            contains_latex,
            human_tissue: if is_system_or_pack {
                None
            } else {
                Some(bool_str(
                    basic_udi.and_then(|b| b.human_tissues).unwrap_or(false),
                ))
            },
            animal_tissue: if is_system_or_pack {
                None
            } else {
                Some(basic_udi.and_then(|b| b.animal_tissues).unwrap_or(false))
            },
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

            // Skip clinical sizes with unmappable measurement units (e.g. MU999 "Other")
            if unit_code.is_empty() && cs.value.is_some() {
                return None;
            }

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

            // 097.070: DEVICE_SIZE_TEXT_SPECIFY requires clinicalSizeDescription
            let descriptions = if gs1_type == "DEVICE_SIZE_TEXT_SPECIFY" {
                let desc = cs.text.as_deref().unwrap_or("Other");
                vec![LangValue {
                    language_code: "en".to_string(),
                    value: desc.to_string(),
                }]
            } else {
                Vec::new()
            };

            Some(ClinicalSizeOutput {
                descriptions,
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

fn build_storage_handling(
    device: &ApiDeviceDetail,
    primary_lang: &str,
) -> Vec<ClinicalStorageHandling> {
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
            // 097.074 / BR-UDID-028: these SHC codes require a description
            // 097.078: fallback language must match primary language of other descriptions
            let needs_description = matches!(
                gs1_code.as_str(),
                "SHC06"
                    | "SHC07"
                    | "SHC08"
                    | "SHC09"
                    | "SHC10"
                    | "SHC13"
                    | "SHC21"
                    | "SHC22"
                    | "SHC23"
                    | "SHC25"
                    | "SHC45"
            );
            if descriptions.is_empty() && needs_description {
                descriptions.push(LangValue {
                    language_code: primary_lang.to_string(),
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
fn build_sales_module(
    device: &ApiDeviceDetail,
    basic_udi: Option<&BasicUdiDiData>,
) -> Option<SalesInformationModule> {
    // Determine which country is the "original placed" market
    let original_iso2 = device
        .placed_on_the_market
        .as_ref()
        .and_then(|c| c.iso2_code.as_ref())
        .map(|s| s.as_str());

    let mut original_countries = Vec::new();
    let mut additional_countries = Vec::new();

    let markets = device
        .market_info_link
        .as_ref()
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
                    country_code: CodeValue {
                        value: numeric.to_string(),
                    },
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

    // 097.020 fallback: if still no country, use manufacturer country from BUDI (if EU/EEA),
    // otherwise default to DE. Member State info is OOS for swissdamed.
    if original_countries.is_empty() {
        let fallback_iso2 = basic_udi
            .and_then(|b| b.manufacturer.as_ref())
            .and_then(|m| m.srn.as_ref())
            .and_then(|srn| {
                let prefix = &srn[..2.min(srn.len())];
                if mappings::is_eu_eea_country(prefix) {
                    Some(prefix.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "DE".to_string());
        let numeric = mappings::country_alpha2_to_numeric(&fallback_iso2);
        original_countries.push(SalesConditionCountry {
            country_code: CodeValue {
                value: numeric.to_string(),
            },
            start_datetime: String::new(),
            end_datetime: None,
        });
    }

    // Ensure only one country in ORIGINAL_PLACED (097.020: only one allowed)
    while original_countries.len() > 1 {
        if let Some(c) = original_countries.pop() {
            additional_countries.push(c);
        } else {
            break;
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
    let agency = di
        .issuing_agency
        .as_ref()
        .and_then(|a| a.code.as_ref())
        .map(|c| mappings::issuing_agency_to_type_code(c))
        .unwrap_or("GS1");

    // 097.118: GS1 direct marking DI must be exactly 14 digits
    if agency == "GS1" && (code.len() != 14 || !code.chars().all(|c| c.is_ascii_digit())) {
        eprintln!("Warning: {} has invalid GS1 direct marking DI '{}' (not 14 digits), skipping (097.118)",
            device.uuid.as_deref().unwrap_or("unknown"), code);
        return Vec::new();
    }

    vec![DirectPartMarking {
        agency_code: agency.to_string(),
        value: code.clone(),
    }]
}

/// Build unit of use DI as TradeItemInformation > TradeItemComponents > ComponentInformation.
/// FLD-UDDI-135: componentNumber=1, componentIdentification=GTIN, schemeAgencyCode=issuing agency.
fn build_unit_of_use(device: &ApiDeviceDetail) -> Vec<TradeItemInformation> {
    let uou = match device.unit_of_use.as_ref() {
        Some(u) => u,
        None => return Vec::new(),
    };
    let code = match uou.code.as_ref() {
        Some(c) if !c.is_empty() => c,
        _ => return Vec::new(),
    };
    let agency = uou
        .issuing_agency
        .as_ref()
        .and_then(|a| a.code.as_ref())
        .map(|c| mappings::issuing_agency_to_type_code(c))
        .unwrap_or("GS1");

    vec![TradeItemInformation {
        components: TradeItemComponents {
            total_number_of_components: 1,
            number_of_pieces_in_set: device.base_quantity,
            component_information: vec![ComponentInformation {
                component_number: 1,
                component_identification: ComponentIdentifier {
                    agency_code: agency.to_string(),
                    value: code.clone(),
                },
                component_quantity: device.base_quantity,
            }],
        },
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
        type_code: CodeValue {
            value: type_code.to_string(),
        },
        gtin,
    }]
}

/// Build chemical regulation module from substances.
/// Build certification module from Basic UDI-DI certificate list.
/// Maps MDR/IVDR certificate types to GS1 CertificationStandard codes.
fn build_certification_module(
    basic_udi: Option<&BasicUdiDiData>,
) -> Option<CertificationInformationModule> {
    let certs = basic_udi?
        .device_certificate_info_list_for_display
        .as_ref()?;
    let mut infos = Vec::new();

    for cert in certs {
        let type_code = cert.certificate_type.as_ref()?.code.as_ref()?;
        let suffix = type_code.rsplit('.').next().unwrap_or(type_code);

        // Map EUDAMED certificate types to GS1 CertificationStandard
        // DeviceCertificateInfo (manufacturer-provided) + CertificateLink (NB-provided)
        let standard = match suffix {
            "technical-documentation" => {
                if type_code.contains("mdr") {
                    "MDR_TECHNICAL_DOCUMENTATION"
                } else if type_code.contains("ivdr") {
                    "IVDR_TECHNICAL_DOCUMENTATION"
                } else {
                    continue;
                }
            }
            "type-examination" => {
                if type_code.contains("mdr") {
                    "MDR_TYPE_EXAMINATION"
                } else if type_code.contains("ivdr") {
                    "IVDR_TYPE_EXAMINATION"
                } else {
                    continue;
                }
            }
            // NB-provided MDR/IVDR certificates (CertificateLink: FLD-UDID-360)
            "quality-management-system" => {
                if type_code.contains("mdr") {
                    "MDR_QUALITY_MANAGEMENT_SYSTEM"
                } else if type_code.contains("ivdr") {
                    "IVDR_QUALITY_MANAGEMENT_SYSTEM"
                } else {
                    continue;
                }
            }
            "quality-assurance" => {
                if type_code.contains("mdr") {
                    "MDR_QUALITY_ASSURANCE"
                } else if type_code.contains("ivdr") {
                    "IVDR_QUALITY_ASSURANCE"
                } else {
                    continue;
                }
            }
            // MDD legacy certificates (097.105)
            "ii-4" => "MDD_II_4",
            "ii-excluding-4" => "MDD_II_EX_4",
            "iii" if type_code.contains("mdd") => "MDD_III",
            "iv" => "MDD_IV",
            "v" => "MDD_V",
            "vi" => "MDD_VI",
            _ => continue,
        };

        let nb = cert.notified_body.as_ref();
        let nb_number = nb.and_then(|n| n.srn.clone());
        infos.push(CertificationInformation {
            // 097.042: additionalCertificationOrganisationIdentifier with EU_NOTIFIED_BODY_NUMBER
            additional_org_ids: nb_number
                .map(|num| {
                    vec![AdditionalPartyIdentification {
                        type_code: "EU_NOTIFIED_BODY_NUMBER".to_string(),
                        value: num,
                    }]
                })
                .unwrap_or_default(),
            agency: nb.and_then(|n| n.name.clone()),
            organisation_identifier: None,
            standard: standard.to_string(),
            certifications: {
                let mut cs = Vec::new();
                // FLD-UDID-347/346: startingValidityDate, fallback to issueDate
                let start = cert
                    .starting_validity_date
                    .clone()
                    .or_else(|| cert.issue_date.clone());
                if cert.certificate_number.is_some()
                    || cert.certificate_expiry.is_some()
                    || start.is_some()
                {
                    cs.push(Certification {
                        // FLD-UDID-61/344 (097.105): CertificationValue = certificate number
                        value: cert.certificate_number.clone(),
                        // FLD-UDID-62/345: CertificationIdentification = revision number
                        identification: cert.certificate_revision.clone(),
                        // FLD-UDID-64/348: CertificationEffectiveEndDateTime = expiry date
                        effective_end: cert.certificate_expiry.clone(),
                        effective_start: start,
                    });
                }
                cs
            },
        });
    }

    if infos.is_empty() {
        None
    } else {
        Some(CertificationInformationModule { infos })
    }
}

fn build_chemical_regulation_module(
    device: &ApiDeviceDetail,
) -> Option<ChemicalRegulationInformationModule> {
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
    let cas_ref = sub
        .cas_number
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|cas| ChemicalIdentifierRef {
            agency_name: "CAS".to_string(),
            value: cas.clone(),
        });

    // EC identifier
    let ec_ref = sub
        .ec_number
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|ec| ChemicalIdentifierRef {
            agency_name: "EC".to_string(),
            value: ec.clone(),
        });

    // Use CAS if available, else EC
    let identifier_ref = cas_ref.or(ec_ref);

    // 097.081/097.080: ENDOCRINE_SUBSTANCE and CMR_SUBSTANCE always need description
    // For other types, only when no INN/CAS/EC
    let needs_description = chemical_type == "ENDOCRINE_SUBSTANCE"
        || chemical_type == "CMR_SUBSTANCE"
        || (identifier_ref.is_none() && inn.is_none());
    let descriptions = if needs_description {
        let desc = name_text
            .as_ref()
            .map(|n| n.trim().to_string())
            .or_else(|| inn.clone())
            .unwrap_or_else(|| chemical_type.to_string());
        vec![LangValue {
            language_code: "en".to_string(),
            value: desc,
        }]
    } else {
        Vec::new()
    };

    RegulatedChemical {
        identifier_ref,
        chemical_name: inn,
        descriptions,
        cmr_type: None,
        chemical_type: CodeValue {
            value: chemical_type.to_string(),
        },
    }
}

/// Build a RegulatedChemical from a CmrSubstance.
fn build_cmr_chemical(sub: &CmrSubstance) -> RegulatedChemical {
    let name_text = sub
        .name
        .as_ref()
        .and_then(|t| t.texts.as_ref())
        .and_then(|texts| texts.first())
        .and_then(|lt| lt.text.clone());

    // CAS identifier
    let cas_ref = sub
        .cas_number
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|cas| ChemicalIdentifierRef {
            agency_name: "CAS".to_string(),
            value: cas.clone(),
        });

    // EC identifier
    let ec_ref = sub
        .ec_number
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|ec| ChemicalIdentifierRef {
            agency_name: "EC".to_string(),
            value: ec.clone(),
        });

    let identifier_ref = cas_ref.or(ec_ref);

    // CMR type code from cmr_substance_type
    let cmr_type = sub
        .cmr_substance_type
        .as_ref()
        .and_then(|t| t.code.as_ref())
        .map(|c| CodeValue {
            value: mappings::cmr_type_to_gs1(c),
        });

    // 097.081/097.080: CMR_SUBSTANCE always needs description with languageCode "en"
    let descriptions = {
        let desc = name_text
            .as_ref()
            .map(|n| n.trim().to_string())
            .unwrap_or_else(|| "CMR_SUBSTANCE".to_string());
        vec![LangValue {
            language_code: "en".to_string(),
            value: desc,
        }]
    };

    RegulatedChemical {
        identifier_ref,
        chemical_name: None,
        descriptions,
        cmr_type,
        chemical_type: CodeValue {
            value: "CMR_SUBSTANCE".to_string(),
        },
    }
}

/// Extract the first text from a Substance's name field
fn extract_substance_name(sub: &Substance) -> Option<String> {
    sub.name
        .as_ref()
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
fn extract_descriptions(mlt: &Option<crate::api_detail::MultiLangText>) -> Vec<LangValue> {
    let raw: Vec<(String, String)> = mlt
        .as_ref()
        .and_then(|t| t.texts.as_ref())
        .map(|texts| {
            texts
                .iter()
                .filter_map(|lt| {
                    let text = lt.text.clone()?;
                    if text.is_empty() {
                        return None;
                    }
                    // language: null → default to "en" (same as allLanguagesApplicable)
                    let lang = lt
                        .language
                        .as_ref()
                        .and_then(|l| l.iso_code.clone())
                        .unwrap_or_else(|| "en".to_string());
                    Some((lang, text))
                })
                .collect()
        })
        .unwrap_or_default();
    // Merge duplicate languages with " / " (097.078: at most one iteration per languageCode)
    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (lang, text) in raw {
        map.entry(lang)
            .and_modify(|existing| {
                existing.push_str(" / ");
                existing.push_str(&text);
            })
            .or_insert(text);
    }
    map.into_iter()
        .map(|(lang, text)| LangValue {
            language_code: lang,
            value: text,
        })
        .collect()
}

/// Package level info extracted from containedItem hierarchy.
struct PackageLevel {
    code: String,
    quantity: u32, // quantity of items this package contains
}

/// Flatten the recursive containedItem tree into a list of package levels.
/// Returns levels from innermost to outermost.
fn flatten_package_levels(root: &ContainedItemNode) -> Vec<PackageLevel> {
    let mut levels = Vec::new();
    let mut current_children = root.contained_items.as_deref().unwrap_or(&[]);

    while !current_children.is_empty() {
        let node = &current_children[0]; // take first child at each level
        let code = node
            .item_identifier
            .as_ref()
            .and_then(|id| id.code.as_deref())
            .unwrap_or("")
            .to_string();
        let qty = node.number_of_items.unwrap_or(1);
        levels.push(PackageLevel {
            code,
            quantity: qty,
        });
        current_children = node.contained_items.as_deref().unwrap_or(&[]);
    }

    levels
}

/// Transform a device detail into a full FirstbaseDocument with packaging hierarchy.
pub fn transform_detail_document(
    device: &ApiDeviceDetail,
    config: &Config,
    basic_udi: Option<&BasicUdiDiData>,
    stem: &str,
) -> FirstbaseDocument {
    let mut base_trade_item = transform_detail_device(device, config, basic_udi);

    // Check for packaging hierarchy
    let levels = device
        .contained_item
        .as_ref()
        .map(|ci| flatten_package_levels(ci))
        .unwrap_or_default();

    if levels.is_empty() {
        // No packaging — simple document, base unit is despatch unit
        return FirstbaseDocument {
            trade_item: base_trade_item,
            children: Vec::new(),
            identifier: format!("Draft_{}", stem),
        };
    }

    // Base unit is no longer the despatch unit when packages exist
    base_trade_item.is_despatch_unit = false;

    // Extract EMA/EPP/EAR contacts for package DIs (SRN only, for CH-REP filtering)
    let pkg_contacts: Vec<TradeItemContactInformation> = base_trade_item
        .contact_information
        .iter()
        .filter(|c| {
            c.contact_type.value == "EMA"
                || c.contact_type.value == "EPP"
                || c.contact_type.value == "EAR"
        })
        .cloned()
        .collect();

    // Basic UDI-DI code for globalModelNumber on packages
    let basic_udi_code = basic_udi
        .and_then(|b| b.basic_udi.as_ref())
        .and_then(|bu| bu.code.as_deref())
        .unwrap_or("");

    let base_gtin = base_trade_item.gtin.clone();

    // Build innermost child link (base unit)
    let mut inner_link = CatalogueItemChildItemLink {
        quantity: levels[0].quantity,
        catalogue_item: CatalogueItem {
            identifier: uuid::Uuid::new_v4().to_string(),
            trade_item: base_trade_item,
            children: vec![],
        },
    };

    // Build package levels from innermost to outermost
    // levels[0] = innermost package, levels[last] = outermost package
    let total_pkg_levels = levels.len();
    for (i, level) in levels.iter().enumerate() {
        let is_outermost = i == total_pkg_levels - 1;
        let is_innermost = i == 0;

        // Descriptor logic: innermost = PACK_OR_INNER_PACK when 2+ levels, else CASE
        let descriptor = if is_innermost && total_pkg_levels >= 2 {
            "PACK_OR_INNER_PACK"
        } else {
            "CASE"
        };

        // Next lower level points to the child
        let child_gtin = if i == 0 {
            base_gtin.clone()
        } else {
            levels[i - 1].code.clone()
        };
        // levels[i].quantity = numberOfItems = how many children this package contains
        let child_qty = levels[i].quantity;

        // This package's quantity (how many fit in the NEXT outer package)
        let next_qty = if is_outermost {
            1 // outermost has no parent
        } else {
            levels[i + 1].quantity
        };

        let next_lower = NextLowerLevel {
            quantity_of_children: 1,
            total_quantity: child_qty,
            child_items: vec![ChildTradeItem {
                quantity: child_qty,
                gtin: child_gtin,
            }],
        };

        let now_str = Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();

        let pkg_trade_item = TradeItem {
            is_brand_bank_publication: false,
            target_sector: vec!["UDI_REGISTRY".to_string()],
            chemical_regulation_module: None,
            healthcare_item_module: None,
            medical_device_module: MedicalDeviceTradeItemModule {
                info: MedicalDeviceInformation {
                    eu_status: CodeValue {
                        value: "ON_MARKET".to_string(),
                    },
                    ..Default::default()
                },
            },
            certification_module: None,
            referenced_file_module: None,
            regulated_trade_item_module: {
                let pkg_reg_act = basic_udi
                    .and_then(|b| b.regulatory_act())
                    .unwrap_or_else(|| "MDR".to_string());
                Some(RegulatedTradeItemModule {
                    info: vec![RegulatoryInformation {
                        act: pkg_reg_act,
                        agency: "EU".to_string(),
                    }],
                })
            },
            sales_module: None,
            description_module: None,
            is_base_unit: false,
            is_despatch_unit: is_outermost,
            is_orderable_unit: true,
            unit_descriptor: CodeValue {
                value: descriptor.to_string(),
            },
            trade_channel_code: vec![CodeValue {
                value: "UDI_REGISTRY".to_string(),
            }],
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
            next_lower_level: Some(next_lower),
            target_market: TargetMarketObj {
                country_code: CodeValue {
                    value: config.target_market.country_code.clone(),
                },
            },
            contact_information: pkg_contacts.clone(),
            synchronisation_dates: TradeItemSynchronisationDates {
                last_change: now_str.clone(),
                effective: now_str.clone(),
                publication: now_str,
                discontinued: None,
            },
            global_model_info: vec![GlobalModelInformation {
                number: basic_udi_code.to_string(),
                descriptions: vec![],
            }],
            gtin: level.code.clone(),
            additional_identification: vec![],
            referenced_trade_items: Vec::new(),
            trade_item_information: Vec::new(),
        };

        inner_link = CatalogueItemChildItemLink {
            quantity: next_qty,
            catalogue_item: CatalogueItem {
                identifier: uuid::Uuid::new_v4().to_string(),
                trade_item: pkg_trade_item,
                children: vec![inner_link],
            },
        };
    }

    // The outermost package is the top-level trade item
    let top_catalogue = inner_link.catalogue_item;

    FirstbaseDocument {
        trade_item: top_catalogue.trade_item,
        children: top_catalogue.children,
        identifier: format!("Draft_{}", stem),
    }
}
