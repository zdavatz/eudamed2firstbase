use serde::Deserialize;

/// Represents one device record from the EUDAMED public API listing endpoint
/// (GET /devices/udiDiData?page=N&pageSize=300)
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApiDevice {
    pub basic_udi: Option<String>,
    pub primary_di: Option<String>,
    pub uuid: Option<String>,
    pub ulid: Option<String>,
    pub risk_class: Option<RefCode>,
    pub trade_name: Option<String>,
    pub manufacturer_name: Option<String>,
    pub manufacturer_srn: Option<String>,
    pub device_status_type: Option<RefCode>,
    pub manufacturer_status: Option<RefCode>,
    pub latest_version: Option<bool>,
    pub version_number: Option<serde_json::Value>,
    pub reference: Option<String>,
    pub issuing_agency: Option<serde_json::Value>,
    pub container_package_count: Option<serde_json::Value>,
    pub authorised_representative_srn: Option<String>,
    pub authorised_representative_name: Option<String>,
    pub sterile: Option<serde_json::Value>,
    pub multi_component: Option<serde_json::Value>,
    pub device_criterion: Option<serde_json::Value>,
    pub device_name: Option<String>,
    pub device_model: Option<String>,
    #[serde(rename = "mfOrPrSrn")]
    pub mf_or_pr_srn: Option<String>,
    pub applicable_legislation: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct RefCode {
    pub code: Option<String>,
}

impl ApiDevice {
    /// Extract the GS1-style risk class code from the refdata code
    /// e.g. "refdata.risk-class.class-iib" → "CLASS_IIB"
    pub fn risk_class_code(&self) -> Option<String> {
        self.risk_class.as_ref()?.code.as_ref().map(|c| {
            c.rsplit('.')
                .next()
                .unwrap_or(c)
                .replace('-', "_")
                .to_uppercase()
        })
    }

    /// Extract device status code
    /// e.g. "refdata.device-model-status.on-the-market" → "ON_THE_MARKET"
    pub fn status_code(&self) -> Option<String> {
        self.device_status_type.as_ref()?.code.as_ref().map(|c| {
            c.rsplit('.')
                .next()
                .unwrap_or(c)
                .replace('-', "_")
                .to_uppercase()
        })
    }
}

/// Parse one NDJSON line into an ApiDevice
pub fn parse_api_device(json_line: &str) -> anyhow::Result<ApiDevice> {
    let device: ApiDevice = serde_json::from_str(json_line)?;
    Ok(device)
}
