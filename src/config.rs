use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    pub target_market: TargetMarket,
    pub gpc: Gpc,
    #[serde(default)]
    pub endocrine_substances: HashMap<String, EndocrineSubstanceIds>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Provider {
    pub gln: String,
    pub party_name: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TargetMarket {
    pub country_code: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Gpc {
    pub segment_code: String,
    pub class_code: String,
    pub family_code: String,
    pub category_code: String,
    pub category_name: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct EndocrineSubstanceIds {
    pub ec_number: Option<String>,
    pub cas_number: Option<String>,
}

const DEFAULT_CONFIG: &str = r#"
[provider]
gln = "7612345000480"
party_name = "EUDAMED Public Download Importing"

[target_market]
country_code = "097"

[gpc]
segment_code = "51000000"
class_code = "51150100"
family_code = "51150000"
category_code = "10005844"
category_name = "Medical Devices"

[endocrine_substances.Estradiol]
ec_number = "200-023-8"
cas_number = "50-28-2"
"#;

pub fn load_config(path: &Path) -> Result<Config> {
    let content = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        DEFAULT_CONFIG.to_string()
    };
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
