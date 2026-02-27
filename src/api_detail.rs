use serde::Deserialize;

/// Full device detail from GET /devices/udiDiData/{uuid}?languageIso2Code=en
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiDeviceDetail {
    pub uuid: Option<String>,
    pub ulid: Option<String>,
    pub primary_di: Option<DiIdentifier>,
    pub secondary_di: Option<DiIdentifier>,
    pub reference: Option<String>,
    pub base_quantity: Option<u32>,
    pub trade_name: Option<MultiLangText>,
    pub additional_description: Option<MultiLangText>,
    pub additional_information_url: Option<String>,

    // Booleans / flags
    pub sterile: Option<bool>,
    pub sterilization: Option<bool>,
    pub latex: Option<bool>,
    pub reprocessed: Option<bool>,
    pub single_use: Option<bool>,
    pub max_number_of_reuses: Option<u32>,
    pub max_number_of_reuses_applicable: Option<bool>,
    pub direct_marking: Option<serde_json::Value>,
    pub direct_marking_same_as_udi_di: Option<bool>,
    pub direct_marking_di: Option<DiIdentifier>,
    pub unit_of_use: Option<serde_json::Value>,

    // Production identifiers
    pub udi_pi_type: Option<UdiPiType>,

    // Clinical sizes
    pub clinical_size_applicable: Option<bool>,
    pub clinical_sizes: Option<Vec<ClinicalSize>>,

    // Storage and warnings
    pub storage_applicable: Option<bool>,
    pub storage_handling_conditions: Option<Vec<StorageHandlingCondition>>,
    pub critical_warnings_applicable: Option<bool>,
    pub critical_warnings: Option<Vec<CriticalWarning>>,

    // Market info
    pub market_info_link: Option<MarketInfoLink>,
    pub placed_on_the_market: Option<Country>,

    // Device status
    pub device_status: Option<DeviceStatus>,

    // Nomenclature codes (CND/EMDN)
    pub cnd_nomenclatures: Option<Vec<CndNomenclature>>,

    // Substances
    pub medicinal_product_substances: Option<serde_json::Value>,
    pub human_product_substances: Option<serde_json::Value>,
    pub cmr_substances: Option<Vec<serde_json::Value>>,
    pub cmr_substance: Option<serde_json::Value>,
    pub endocrine_disrupting_substances: Option<serde_json::Value>,
    pub endocrine_disruptor: Option<serde_json::Value>,

    // Annex XVI
    pub annex_xvi_applicable: Option<bool>,

    // Product designer
    pub product_designer: Option<serde_json::Value>,

    // OEM
    pub oem_applicable: Option<bool>,

    // Component DIs (multi-component devices)
    pub component_dis: Option<Vec<serde_json::Value>>,

    // Version info
    pub version_number: Option<serde_json::Value>,
    pub latest_version: Option<bool>,
    pub version_date: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiIdentifier {
    pub uuid: Option<String>,
    pub code: Option<String>,
    pub issuing_agency: Option<RefCode>,
    #[serde(rename = "type")]
    pub di_type: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct RefCode {
    pub code: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MultiLangText {
    pub texts: Option<Vec<LangText>>,
    pub text_by_default_language: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LangText {
    pub language: Option<Language>,
    pub text: Option<String>,
    pub all_languages_applicable: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub iso_code: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UdiPiType {
    pub batch_number: Option<bool>,
    pub serialization_number: Option<bool>,
    pub manufacturing_date: Option<bool>,
    pub expiration_date: Option<bool>,
    pub software_identification: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClinicalSize {
    pub text: Option<String>,
    pub value: Option<f64>,
    pub minimum_value: Option<f64>,
    pub maximum_value: Option<f64>,
    #[serde(rename = "type")]
    pub size_type: Option<RefCode>,
    pub precision: Option<RefCode>,
    pub metric_of_measurement: Option<RefCode>,
    pub clinical_size_type_description: Option<serde_json::Value>,
    pub measuring_unit_description: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorageHandlingCondition {
    pub type_code: Option<String>,
    pub mandatory: Option<bool>,
    pub description: Option<MultiLangText>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CriticalWarning {
    pub type_code: Option<String>,
    pub mandatory: Option<bool>,
    pub description: Option<MultiLangText>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketInfoLink {
    pub ms_where_available: Option<Vec<MarketAvailability>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketAvailability {
    pub country: Option<Country>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Country {
    pub name: Option<String>,
    pub iso2_code: Option<String>,
    #[serde(rename = "type")]
    pub country_type: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    #[serde(rename = "type")]
    pub status_type: Option<RefCode>,
    pub status_date: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CndNomenclature {
    pub code: Option<String>,
    pub description: Option<MultiLangText>,
}

impl ApiDeviceDetail {
    /// Extract the refdata suffix and normalize to uppercase with underscores
    fn extract_refdata_code(code: &str) -> String {
        code.rsplit('.')
            .next()
            .unwrap_or(code)
            .replace('-', "_")
            .to_uppercase()
    }

    /// Extract status code e.g. "refdata.device-model-status.on-the-market" → "ON_THE_MARKET"
    pub fn status_code(&self) -> Option<String> {
        let ds = self.device_status.as_ref()?;
        let st = ds.status_type.as_ref()?;
        let code = st.code.as_ref()?;
        Some(Self::extract_refdata_code(code))
    }

    /// Get the primary DI code (GTIN)
    pub fn gtin(&self) -> String {
        self.primary_di
            .as_ref()
            .and_then(|di| di.code.clone())
            .unwrap_or_default()
    }

    /// Get trade name texts as (language_code, text) pairs
    pub fn trade_name_texts(&self) -> Vec<(String, String)> {
        extract_lang_texts(self.trade_name.as_ref())
    }

    /// Get additional description texts
    pub fn additional_description_texts(&self) -> Vec<(String, String)> {
        extract_lang_texts(self.additional_description.as_ref())
    }

    /// Get production identifier type codes for UDI PI
    pub fn production_identifiers(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(ref pi) = self.udi_pi_type {
            if pi.batch_number == Some(true) {
                ids.push("BATCH_NUMBER".to_string());
            }
            if pi.serialization_number == Some(true) {
                ids.push("SERIAL_NUMBER".to_string());
            }
            if pi.manufacturing_date == Some(true) {
                ids.push("MANUFACTURING_DATE".to_string());
            }
            if pi.expiration_date == Some(true) {
                ids.push("EXPIRATION_DATE".to_string());
            }
            if pi.software_identification == Some(true) {
                ids.push("SOFTWARE_IDENTIFICATION".to_string());
            }
        }
        ids
    }
}

fn extract_lang_texts(mlt: Option<&MultiLangText>) -> Vec<(String, String)> {
    mlt.and_then(|t| t.texts.as_ref())
        .map(|texts| {
            texts
                .iter()
                .filter_map(|lt| {
                    let lang = lt.language.as_ref()?.iso_code.clone()?;
                    let text = lt.text.clone()?;
                    if text.is_empty() {
                        return None;
                    }
                    Some((lang, text))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse one NDJSON line into an ApiDeviceDetail
pub fn parse_api_detail(json_line: &str) -> anyhow::Result<ApiDeviceDetail> {
    let detail: ApiDeviceDetail = serde_json::from_str(json_line)?;
    Ok(detail)
}
