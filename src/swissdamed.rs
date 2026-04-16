//! Swissdamed M2M API output model and mapper.
//! Maps EUDAMED JSON (ApiDeviceDetail + BasicUdiDiData) → Swissdamed JSON.
//! Almost 1:1 field mapping — no GDSN translation needed.

use crate::api_detail::{ApiDeviceDetail, BasicUdiDiData};
use serde::Serialize;

// --- Output DTOs matching Swissdamed OpenAPI spec ---

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MdrDto {
    pub correlation_id: String,
    pub basic_udi: MdrBasicUdiDto,
    pub udi_di: MdrUdiDiDto,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SppDto {
    pub correlation_id: String,
    pub basic_udi: SppBasicUdiDto,
    pub udi_di: SppUdiDiDto,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IvdrDto {
    pub correlation_id: String,
    pub basic_udi: IvdrBasicUdiDto,
    pub udi_di: IvdrUdiDiDto,
}

// --- UDI-DI DTOs ---

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MdrUdiDiDto {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trade_names: Vec<LangText>,
    pub reference_number: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub additional_description: Vec<LangText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub sterile: bool,
    pub sterilization: bool,
    pub nomenclature_codes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub storage_handling_conditions: Vec<StorageHandlingConditionDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub critical_warnings: Vec<CriticalWarningDto>,
    pub identifier: DiCodeDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_identifier: Option<DiCodeDto>,
    pub production_identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageUdiDiDto>,
    pub base_quantity: u32,
    pub number_of_reuses: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_marking_identifier: Option<DiCodeDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_use_identifier: Option<DiCodeDto>,
    pub latex: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clinical_sizes: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub medical_human_product_substances: Vec<serde_json::Value>,
    pub reprocessed: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cmr_substances: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endocrine_substances: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annex_xvi_applicable: Option<bool>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SppUdiDiDto {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trade_names: Vec<LangText>,
    pub reference_number: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub additional_description: Vec<LangText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub sterile: bool,
    pub sterilization: bool,
    pub nomenclature_codes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub storage_handling_conditions: Vec<StorageHandlingConditionDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub critical_warnings: Vec<CriticalWarningDto>,
    pub identifier: DiCodeDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_identifier: Option<DiCodeDto>,
    pub production_identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageUdiDiDto>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IvdrUdiDiDto {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trade_names: Vec<LangText>,
    pub reference_number: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub additional_description: Vec<LangText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub sterile: bool,
    pub sterilization: bool,
    pub nomenclature_codes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub storage_handling_conditions: Vec<StorageHandlingConditionDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub critical_warnings: Vec<CriticalWarningDto>,
    pub identifier: DiCodeDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_identifier: Option<DiCodeDto>,
    pub production_identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageUdiDiDto>,
    pub base_quantity: u32,
    pub number_of_reuses: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_marking_identifier: Option<DiCodeDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_use_identifier: Option<DiCodeDto>,
    pub latex: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clinical_sizes: Vec<serde_json::Value>,
    pub reprocessed: bool,
}

// --- Basic UDI-DI DTOs ---

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MdrBasicUdiDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub animal_tissues_cells: bool,
    pub human_tissues_cells: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_device_type: Option<String>,
    #[serde(rename = "type")]
    pub device_type: String,
    pub active: bool,
    pub administering_medicine: bool,
    pub human_product_check: bool,
    pub implantable: bool,
    pub measuring_function: bool,
    pub medicinal_product_check: bool,
    pub reusable: bool,
    pub identifier: DiCodeDto,
    pub risk_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_iib_implantable_exceptions: Option<bool>,
    pub mf_actor_code: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SppBasicUdiDto {
    // Field order matters — Swissdamed validates against XSD element ordering
    pub identifier: DiCodeDto,
    pub risk_class: String,
    #[serde(rename = "type")]
    pub device_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub medicinal_purpose: Vec<LangText>,
    pub pr_actor_code: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IvdrBasicUdiDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub animal_tissues_cells: bool,
    pub human_tissues_cells: bool,
    #[serde(rename = "type")]
    pub device_type: String,
    pub active: bool,
    pub measuring_function: bool,
    pub reusable: bool,
    pub identifier: DiCodeDto,
    pub risk_class: String,
    pub mf_actor_code: String,
}

// --- Shared DTOs ---

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiCodeDto {
    pub di_code: String,
    pub issuing_entity_code: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PackageUdiDiDto {
    pub identifier: DiCodeDto,
    pub child_of: DiCodeDto,
    pub number_of_items: u32,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LangText {
    pub language: String,
    pub text_value: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorageHandlingConditionDto {
    pub storage_handling_condition_value: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<LangText>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CriticalWarningDto {
    pub warning_value: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<LangText>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PostMarketStatusDto {
    pub market_status: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub udi_dis: Vec<UdiDiIdentifierDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub package_udi_dis: Vec<DiCodeDto>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UdiDiIdentifierDto {
    pub di_code: String,
    pub issuing_entity_code: String,
    pub basic_udi_di_code: String,
    pub basic_udi_issuing_entity_code: String,
}

// --- Mapper: EUDAMED → Swissdamed ---

/// Extract issuing entity code from EUDAMED refdata code
fn extract_issuing_entity(code: &str) -> String {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        "gs1" | "GS1" => "GS1".to_string(),
        "hibc" | "HIBC" => "HIBC".to_string(),
        "iccbba" | "ICCBBA" => "ICCBBA".to_string(),
        "ifa" | "IFA" => "IFA".to_string(),
        other => other.to_uppercase(),
    }
}

/// Extract risk class code for Swissdamed (e.g. "refdata.risk-class.class-iia" → "CLASS_IIA")
fn extract_risk_class(code: &str) -> String {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    suffix.to_uppercase().replace('-', "_")
}

/// Extract multi-component type for Swissdamed
fn extract_spp_type(code: &str) -> String {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        "system" => "SYSTEM".to_string(),
        "procedure-pack" => "PROCEDURE_PACK".to_string(),
        "spp-procedure-pack" => "SPP_PROCEDURE_PACK".to_string(),
        other => other.to_uppercase().replace('-', "_"),
    }
}

/// Map language texts from EUDAMED MultiLangText
fn map_lang_texts(texts: &[(String, String)]) -> Vec<LangText> {
    texts
        .iter()
        .map(|(lang, text)| LangText {
            language: lang.to_uppercase(),
            text_value: text.clone(),
        })
        .collect()
}

/// Map nomenclature codes from EUDAMED CND codes
fn extract_nomenclature_codes(device: &ApiDeviceDetail) -> Vec<String> {
    device
        .cnd_nomenclatures
        .as_ref()
        .map(|codes| codes.iter().filter_map(|c| c.code.clone()).collect())
        .unwrap_or_default()
}

/// Map production identifiers from udiPiType
fn map_production_identifiers(device: &ApiDeviceDetail) -> Vec<String> {
    device.production_identifiers()
}

/// Map primary DI identifier
fn map_identifier(device: &ApiDeviceDetail) -> DiCodeDto {
    let di = device.primary_di.as_ref();
    DiCodeDto {
        di_code: di.and_then(|d| d.code.clone()).unwrap_or_default(),
        issuing_entity_code: di
            .and_then(|d| d.issuing_agency.as_ref())
            .and_then(|a| a.code.as_ref())
            .map(|c| extract_issuing_entity(c))
            .unwrap_or_else(|| "GS1".to_string()),
    }
}

/// Map secondary DI identifier
fn map_secondary_identifier(device: &ApiDeviceDetail) -> Option<DiCodeDto> {
    let sec = device.secondary_di.as_ref()?;
    let code = sec.code.clone()?;
    if code.is_empty() {
        return None;
    }
    Some(DiCodeDto {
        di_code: code,
        issuing_entity_code: sec
            .issuing_agency
            .as_ref()
            .and_then(|a| a.code.as_ref())
            .map(|c| extract_issuing_entity(c))
            .unwrap_or_else(|| "GS1".to_string()),
    })
}

/// Map storage handling conditions
fn map_storage_handling(device: &ApiDeviceDetail) -> Vec<StorageHandlingConditionDto> {
    device
        .storage_handling_conditions
        .as_ref()
        .map(|conditions| {
            conditions
                .iter()
                .filter_map(|shc| {
                    let type_code = shc.type_code.as_ref()?;
                    let suffix = type_code.rsplit('.').next().unwrap_or(type_code);
                    let descriptions = shc
                        .description
                        .as_ref()
                        .and_then(|d| d.texts.as_ref())
                        .map(|texts| {
                            texts
                                .iter()
                                .filter_map(|t| {
                                    let text = t.text.clone()?;
                                    let lang = t
                                        .language
                                        .as_ref()
                                        .and_then(|l| l.iso_code.clone())
                                        .unwrap_or_else(|| "en".to_string())
                                        .to_uppercase();
                                    Some(LangText {
                                        language: lang,
                                        text_value: text,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(StorageHandlingConditionDto {
                        storage_handling_condition_value: suffix.to_uppercase(),
                        comments: descriptions,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Map critical warnings
fn map_critical_warnings(device: &ApiDeviceDetail) -> Vec<CriticalWarningDto> {
    device
        .critical_warnings
        .as_ref()
        .map(|warnings| {
            warnings
                .iter()
                .filter_map(|w| {
                    let type_code = w.type_code.as_ref()?;
                    let suffix = type_code.rsplit('.').next().unwrap_or(type_code);
                    let descriptions = w
                        .description
                        .as_ref()
                        .and_then(|d| d.texts.as_ref())
                        .map(|texts| {
                            texts
                                .iter()
                                .filter_map(|t| {
                                    let text = t.text.clone()?;
                                    let lang = t
                                        .language
                                        .as_ref()
                                        .and_then(|l| l.iso_code.clone())
                                        .unwrap_or_else(|| "en".to_string())
                                        .to_uppercase();
                                    Some(LangText {
                                        language: lang,
                                        text_value: text,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(CriticalWarningDto {
                        warning_value: suffix.to_uppercase(),
                        comments: descriptions,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Map EUDAMED device + BUDI to Swissdamed MDR DTO
pub fn to_mdr_dto(device: &ApiDeviceDetail, basic_udi: &BasicUdiDiData) -> MdrDto {
    let uuid = device.uuid.clone().unwrap_or_default();

    MdrDto {
        correlation_id: uuid,
        basic_udi: MdrBasicUdiDto {
            device_name: basic_udi.device_name.clone(),
            model_name: basic_udi.device_model.clone(),
            animal_tissues_cells: basic_udi.animal_tissues.unwrap_or(false),
            human_tissues_cells: basic_udi.human_tissues.unwrap_or(false),
            special_device_type: None,
            device_type: basic_udi
                .multi_component
                .as_ref()
                .and_then(|mc| mc.code.as_ref())
                .map(|c| extract_spp_type(c))
                .unwrap_or_else(|| "DEVICE".to_string()),
            active: basic_udi.active.unwrap_or(false),
            administering_medicine: basic_udi.administering_medicine.unwrap_or(false),
            human_product_check: basic_udi.human_product.unwrap_or(false),
            implantable: basic_udi.implantable.unwrap_or(false),
            measuring_function: basic_udi.measuring_function.unwrap_or(false),
            medicinal_product_check: basic_udi.medicinal_product.unwrap_or(false),
            reusable: basic_udi.reusable.unwrap_or(false),
            identifier: DiCodeDto {
                di_code: basic_udi
                    .basic_udi
                    .as_ref()
                    .and_then(|b| b.code.clone())
                    .unwrap_or_default(),
                issuing_entity_code: basic_udi
                    .basic_udi
                    .as_ref()
                    .and_then(|b| b.issuing_agency.as_ref())
                    .and_then(|a| a.code.as_ref())
                    .map(|c| extract_issuing_entity(c))
                    .unwrap_or_else(|| "GS1".to_string()),
            },
            risk_class: basic_udi
                .risk_class_code()
                .map(|c| extract_risk_class(&c))
                .unwrap_or_else(|| "CLASS_I".to_string()),
            class_iib_implantable_exceptions: None,
            mf_actor_code: basic_udi
                .manufacturer
                .as_ref()
                .and_then(|m| m.srn.clone())
                .unwrap_or_default(),
        },
        udi_di: MdrUdiDiDto {
            trade_names: map_lang_texts(&device.trade_name_texts()),
            reference_number: device.reference.clone().unwrap_or_default(),
            additional_description: map_lang_texts(&device.additional_description_texts()),
            website: device.additional_information_url.clone(),
            sterile: device.sterile.unwrap_or(false),
            sterilization: device.sterilization.unwrap_or(false),
            nomenclature_codes: extract_nomenclature_codes(device),
            storage_handling_conditions: map_storage_handling(device),
            critical_warnings: map_critical_warnings(device),
            identifier: map_identifier(device),
            secondary_identifier: map_secondary_identifier(device),
            production_identifiers: map_production_identifiers(device),
            packages: Vec::new(), // TODO: map from containedItem
            base_quantity: device.base_quantity.unwrap_or(1),
            number_of_reuses: device.max_number_of_reuses.map(|n| n as i32).unwrap_or(-1),
            direct_marking_identifier: None, // TODO
            unit_of_use_identifier: None,    // TODO
            latex: device.latex.unwrap_or(false),
            clinical_sizes: Vec::new(), // TODO: pass through raw JSON
            medical_human_product_substances: Vec::new(), // TODO
            reprocessed: device.reprocessed.unwrap_or(false),
            cmr_substances: Vec::new(),       // TODO
            endocrine_substances: Vec::new(), // TODO
            annex_xvi_applicable: device.annex_xvi_applicable,
        },
    }
}

/// Map EUDAMED device + BUDI to Swissdamed SPP DTO
pub fn to_spp_dto(device: &ApiDeviceDetail, basic_udi: &BasicUdiDiData) -> SppDto {
    let uuid = device.uuid.clone().unwrap_or_default();

    SppDto {
        correlation_id: uuid,
        basic_udi: SppBasicUdiDto {
            device_name: basic_udi.device_name.clone(),
            model_name: basic_udi.device_model.clone(),
            identifier: DiCodeDto {
                di_code: basic_udi
                    .basic_udi
                    .as_ref()
                    .and_then(|b| b.code.clone())
                    .unwrap_or_default(),
                issuing_entity_code: basic_udi
                    .basic_udi
                    .as_ref()
                    .and_then(|b| b.issuing_agency.as_ref())
                    .and_then(|a| a.code.as_ref())
                    .map(|c| extract_issuing_entity(c))
                    .unwrap_or_else(|| "GS1".to_string()),
            },
            risk_class: basic_udi
                .risk_class_code()
                .map(|c| extract_risk_class(&c))
                .unwrap_or_else(|| "CLASS_I".to_string()),
            device_type: basic_udi
                .multi_component
                .as_ref()
                .and_then(|mc| mc.code.as_ref())
                .map(|c| extract_spp_type(c))
                .unwrap_or_else(|| "PROCEDURE_PACK".to_string()),
            medicinal_purpose: {
                let texts = map_lang_texts(&basic_udi.medical_purpose_texts());
                if texts.is_empty() {
                    // XSD requires at least one medicinalPurpose entry for SPP
                    vec![LangText {
                        language: "EN".to_string(),
                        text_value: basic_udi.device_name.clone().unwrap_or_default(),
                    }]
                } else {
                    texts
                }
            },
            pr_actor_code: basic_udi
                .manufacturer
                .as_ref()
                .and_then(|m| m.srn.clone())
                .unwrap_or_default(),
        },
        udi_di: SppUdiDiDto {
            trade_names: map_lang_texts(&device.trade_name_texts()),
            reference_number: device.reference.clone().unwrap_or_default(),
            additional_description: map_lang_texts(&device.additional_description_texts()),
            website: device.additional_information_url.clone(),
            sterile: device.sterile.unwrap_or(false),
            sterilization: device.sterilization.unwrap_or(false),
            nomenclature_codes: extract_nomenclature_codes(device),
            storage_handling_conditions: map_storage_handling(device),
            critical_warnings: map_critical_warnings(device),
            identifier: map_identifier(device),
            secondary_identifier: map_secondary_identifier(device),
            production_identifiers: map_production_identifiers(device),
            packages: Vec::new(), // TODO: map from containedItem
        },
    }
}

/// Determine which Swissdamed endpoint to use based on legislation
pub fn legislation_endpoint(basic_udi: &BasicUdiDiData) -> &'static str {
    let is_spp = basic_udi
        .multi_component
        .as_ref()
        .and_then(|mc| mc.code.as_ref())
        .map(|c| {
            let suffix = c.rsplit('.').next().unwrap_or(c);
            matches!(suffix, "system" | "procedure-pack" | "spp-procedure-pack")
        })
        .unwrap_or(false);

    if is_spp {
        return "/v1/m2m/udi/data/spp";
    }

    let act = basic_udi.regulatory_act().unwrap_or_default();
    match act.as_str() {
        "MDR" => "/v1/m2m/udi/data/mdr",
        "IVDR" => "/v1/m2m/udi/data/ivdr",
        "MDD" => "/v1/m2m/udi/data/mdd",
        "AIMDD" => "/v1/m2m/udi/data/aimdd",
        "IVDD" => "/v1/m2m/udi/data/ivdd",
        _ => "/v1/m2m/udi/data/mdr", // default
    }
}
