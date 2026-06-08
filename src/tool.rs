//! Tool trait and registry (design §2.3).

use std::collections::HashMap;

use crate::error::ToolError;

/// Tool declaration fed to the model via [`crate::provider::CompletionRequest`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema object; provider impls map to their API format (e.g. OpenAI `parameters`).
    pub parameters: serde_json::Value,
}

/// Executable tool boundary. Failures return [`ToolError`] for loop backfill, not propagation.
pub trait Tool {
    fn schema(&self) -> ToolSchema;
    fn execute(&self, args: serde_json::Value) -> Result<String, ToolError>;
}

/// Registry of tools keyed by [`ToolSchema::name`].
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Registers a tool under `tool.schema().name`. Duplicate names overwrite the previous entry.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.schema().name;
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Collects schemas for all registered tools. Order is not stable.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubTool {
        name: &'static str,
    }

    impl Tool for StubTool {
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.into(),
                description: "stub".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } }
                }),
            }
        }

        fn execute(&self, args: serde_json::Value) -> Result<String, ToolError> {
            match args.get("value").and_then(|v| v.as_str()) {
                Some("ok") => Ok("done".into()),
                Some("fail") => Err(ToolError::ExecutionFailed("boom".into())),
                _ => Err(ToolError::InvalidArgs("missing or invalid value".into())),
            }
        }
    }

    #[test]
    fn register_and_get() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool { name: "alpha" }));
        assert!(registry.get("alpha").is_some());
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn schemas_collects_all_tools() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool { name: "a" }));
        registry.register(Box::new(StubTool { name: "b" }));
        let names: Vec<_> = registry.schemas().into_iter().map(|s| s.name).collect();
        assert_eq!(registry.len(), 2);
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
    }

    #[test]
    fn execute_success_and_errors() {
        let tool = StubTool { name: "stub" };
        let ok = tool.execute(serde_json::json!({ "value": "ok" })).expect("ok");
        assert_eq!(ok, "done");

        let bad = tool.execute(serde_json::json!({})).unwrap_err();
        assert_eq!(bad.to_string(), "invalid arguments: missing or invalid value");

        let fail = tool
            .execute(serde_json::json!({ "value": "fail" }))
            .unwrap_err();
        assert_eq!(fail.to_string(), "tool execution failed: boom");
    }

    #[test]
    fn duplicate_name_overwrites() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(StubTool { name: "same" }));
        registry.register(Box::new(StubTool { name: "same" }));
        assert_eq!(registry.len(), 1);
        assert!(registry.get("same").is_some());
    }
}
