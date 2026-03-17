use serde::{Deserialize, Serialize};

fn default_prefix() -> String {
    "!".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub rules: Vec<ConfigRuleConfig>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            prefix: default_prefix(),
            rules: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRuleConfig {
    pub trigger: String,
    pub tool: String,
    pub params: serde_json::Value,
}
