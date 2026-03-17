use serde::{Deserialize, Serialize};

fn default_prefix() -> String {
    "!".into()
}

fn default_semantic_top_k() -> usize {
    3
}

fn default_semantic_prefilter_k() -> usize {
    12
}

fn default_semantic_threshold() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub rules: Vec<ConfigRuleConfig>,
    /// Number of tools to allow in semantic filter mode.
    #[serde(default = "default_semantic_top_k")]
    pub semantic_top_k: usize,
    /// Lexical prefilter candidate count before optional embedding rerank.
    #[serde(default = "default_semantic_prefilter_k")]
    pub semantic_prefilter_k: usize,
    /// Minimum semantic score required to keep a tool candidate.
    #[serde(default = "default_semantic_threshold")]
    pub semantic_threshold: f32,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            prefix: default_prefix(),
            rules: vec![],
            semantic_top_k: default_semantic_top_k(),
            semantic_prefilter_k: default_semantic_prefilter_k(),
            semantic_threshold: default_semantic_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRuleConfig {
    pub trigger: String,
    pub tool: String,
    pub params: serde_json::Value,
}
