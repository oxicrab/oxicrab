use crate::agent::tools::{Tool, ToolResult};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn get_definitions(&self) -> Vec<Value> {
        self.tools.values().map(|t| t.to_schema()).collect()
    }

    pub fn get_tool_definitions(&self) -> Vec<crate::providers::base::ToolDefinition> {
        self.tools
            .values()
            .map(|t| {
                let schema = t.to_schema();
                crate::providers::base::ToolDefinition {
                    name: schema["function"]["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    description: schema["function"]["description"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    parameters: schema["function"]["parameters"].clone(),
                }
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, params: Value) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool '{}' not found", name))?;

        tool.execute(params).await
    }

    #[allow(dead_code)] // May be used for introspection/debugging
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    #[allow(dead_code)] // May be used for introspection/debugging
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    #[allow(dead_code)] // May be used for introspection/debugging
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
