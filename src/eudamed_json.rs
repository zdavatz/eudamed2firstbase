use serde::Deserialize;

/// Represents one device record from the EUDAMED JSON export files.
/// These files contain device-level data with inline manufacturer and
/// authorised representative information.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct EudamedDevice {
    pub uuid: Option<String>,
    pub ulid: Option<String>,
    pub manufacturer: Option<Manufacturer>,
    pub authorised_representative: Option<AuthorisedRepresentative>,
    pub basic_udi: Option<BasicUdi>,
    pub risk_class: Option<RefCode>,
    pub legislation: Option<RefCode>,
    pub device_name: Option<String>,
    pub device_model: Option<String>,
    pub device_criterion: Option<String>,
    pub container_type: Option<String>,

    // Boolean flags
    pub active: Option<bool>,
    pub sterile: Option<bool>,
    pub reusable: Option<bool>,
    pub implantable: Option<bool>,
    pub measuring_function: Option<bool>,
    pub administering_medicine: Option<bool>,
    pub medicinal_product: Option<bool>,
    pub human_tissues: Option<bool>,
    pub human_product: Option<bool>,
    pub animal_tissues: Option<bool>,
    pub microbial_substances: Option<serde_json::Value>,
    pub sutures: Option<serde_json::Value>,

    // Version info
    pub version_date: Option<String>,
    pub version_state: Option<RefCode>,
    pub version_number: Option<serde_json::Value>,
    pub latest_version: Option<bool>,

    // Other fields
    pub device_model_applicable: Option<bool>,
    pub special_device_type: Option<serde_json::Value>,
    pub special_device_type_applicable: Option<bool>,
    pub clinical_investigation_applicable: Option<bool>,
    pub type_examination_applicable: Option<serde_json::Value>,
    pub legacy_device_udi_di_applicable: Option<serde_json::Value>,
    pub nb_decision: Option<serde_json::Value>,
    pub companion_diagnostics: Option<serde_json::Value>,
    pub reagent: Option<serde_json::Value>,
    pub instrument: Option<serde_json::Value>,
    pub professional_testing: Option<serde_json::Value>,
    pub kit: Option<serde_json::Value>,
    pub device: Option<bool>,
    pub multi_component: Option<serde_json::Value>,
    pub self_testing: Option<serde_json::Value>,
    pub near_patient_testing: Option<serde_json::Value>,
    pub medical_purpose: Option<serde_json::Value>,
    pub basic_udi_type: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Manufacturer {
    pub uuid: Option<String>,
    pub srn: Option<String>,
    pub name: Option<String>,
    pub country_iso2_code: Option<String>,
    pub country_name: Option<String>,
    pub geographical_address: Option<String>,
    pub electronic_mail: Option<String>,
    pub telephone: Option<String>,
    pub actor_type: Option<serde_json::Value>,
    pub status: Option<serde_json::Value>,
    pub names: Option<serde_json::Value>,
    pub abbreviated_names: Option<serde_json::Value>,
    pub version_number: Option<serde_json::Value>,
    pub version_state: Option<serde_json::Value>,
    pub latest_version: Option<bool>,
    pub last_update_date: Option<String>,
    pub country_type: Option<String>,
    pub status_from_date: Option<serde_json::Value>,
    pub actor_validated: Option<serde_json::Value>,
    pub ulid: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AuthorisedRepresentative {
    pub authorised_representative_uuid: Option<String>,
    pub srn: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
    pub country_name: Option<String>,
    pub email: Option<String>,
    pub telephone: Option<String>,
    pub non_eu_manufacturer_uuid: Option<String>,
    pub authorised_representative_ulid: Option<String>,
    pub start_date: Option<serde_json::Value>,
    pub end_date: Option<serde_json::Value>,
    pub termination_date: Option<serde_json::Value>,
    pub mandate_status: Option<serde_json::Value>,
    pub actor_status: Option<serde_json::Value>,
    pub actor_status_from_date: Option<serde_json::Value>,
    pub version_number: Option<serde_json::Value>,
    pub version_state: Option<serde_json::Value>,
    pub latest_version: Option<bool>,
    pub last_update_date: Option<String>,
    pub ulid: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct BasicUdi {
    pub uuid: Option<String>,
    pub code: Option<String>,
    pub issuing_agency: Option<RefCode>,
    #[serde(rename = "type")]
    pub udi_type: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct RefCode {
    pub code: Option<String>,
}

impl EudamedDevice {
    /// Extract risk class code: "refdata.risk-class.class-iia" → "CLASS_IIA"
    pub fn risk_class_code(&self) -> Option<String> {
        let code = self.risk_class.as_ref()?.code.as_ref()?;
        Some(
            code.rsplit('.')
                .next()
                .unwrap_or(code)
                .replace('-', "_")
                .to_uppercase(),
        )
    }

    /// Extract basic UDI code
    pub fn basic_udi_code(&self) -> String {
        self.basic_udi
            .as_ref()
            .and_then(|bu| bu.code.clone())
            .unwrap_or_default()
    }
}

/// Parse a EUDAMED JSON file into an EudamedDevice
pub fn parse_eudamed_json(json_str: &str) -> anyhow::Result<EudamedDevice> {
    let device: EudamedDevice = serde_json::from_str(json_str)?;
    Ok(device)
}
