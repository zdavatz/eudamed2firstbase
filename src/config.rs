use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    pub target_market: TargetMarket,
    pub gpc: Gpc,
    pub sterilisation_method: Option<String>,
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

pub fn load_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
