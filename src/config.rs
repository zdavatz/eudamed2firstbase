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
    /// Gmail service-account settings for the `mailto` command.
    /// Optional — only needed when sending emails.
    #[serde(default)]
    pub gmail: Gmail,
}

/// Gmail service-account credentials used by the `mailto` command.
/// Store real values in `config.toml` (which is gitignored).
/// See `config.sample.toml` for the expected format.
#[derive(Deserialize, Debug, Clone, Default)]
pub struct Gmail {
    /// Path to the Google service account `.p12` key file.
    #[serde(default)]
    pub p12_key: String,
    /// Service account email address
    /// (e.g. `name@my-project.iam.gserviceaccount.com`).
    #[serde(default)]
    pub service_email: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Provider {
    pub gln: String,
    pub party_name: String,
    /// Default recipient GLN for `cargo run check` pushes.
    /// Can be overridden at runtime with the FIRSTBASE_PUBLISH_GLN env var.
    #[serde(default)]
    pub publish_gln: String,
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
gln         = "7612345000480"
party_name  = "EUDAMED Public Download Importing"
publish_gln = "7612345000527"

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
