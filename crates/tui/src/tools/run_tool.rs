//! Canonical action-based wrapper for run/test/verifier tools.
//!
//! The model sees one tool: `Run` with an `action` parameter
//! (tests | verifiers). Legacy names (`run_tests`, `run_verifiers`) stay
//! registered as hidden compat aliases that force the action so saved
//! transcripts replay correctly.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::spec::{
    ApprovalRequirement, ToolCapability, ToolContext, ToolError, ToolResult, ToolSpec,
};
use super::test_runner::RunTestsTool;
use super::verifier::RunVerifiersTool;

pub struct RunTool {
    name: &'static str,
    forced_action: Option<&'static str>,
}

impl RunTool {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            forced_action: None,
        }
    }

    pub const fn alias(name: &'static str, action: &'static str) -> Self {
        Self {
            name,
            forced_action: Some(action),
        }
    }

    fn resolve_action<'a>(&self, input: &'a Value) -> &'a str {
        self.forced_action.unwrap_or_else(|| {
            input
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("tests")
        })
    }

    fn strip_action(&self, input: Value) -> Result<Value, ToolError> {
        let mut input = input;
        if let Some(obj) = input.as_object_mut() {
            obj.remove("action");
            Ok(input)
        } else {
            Err(ToolError::invalid_input("Run tool input must be an object"))
        }
    }
}

#[async_trait]
impl ToolSpec for RunTool {
    fn name(&self) -> &'static str {
        self.name
    }

    fn model_visible(&self) -> bool {
        self.name == "Run"
    }

    fn description(&self) -> &'static str {
        "Run Cargo tests or repository verifier gates. Use tests for focused Rust test runs; use verifiers for cross-language build, test, lint, and syntax gates. Set background=true for verifier suites expected to take more than a few seconds."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["tests", "verifiers"],
                    "description": "Action to perform"
                },
                "args": {
                    "type": "string",
                    "description": "Extra arguments for cargo test (action=tests)"
                },
                "all_features": {
                    "type": "boolean",
                    "description": "Include --all-features for cargo test (action=tests)"
                },
                "profile": {
                    "type": "string",
                    "enum": ["auto", "rust", "node", "python", "go"],
                    "description": "Verifier profile (action=verifiers)"
                },
                "level": {
                    "type": "string",
                    "enum": ["quick", "full"],
                    "description": "Verifier level (action=verifiers)"
                },
                "max_python_files": {
                    "type": "integer",
                    "description": "Maximum Python files for the verifier syntax gate (action=verifiers)"
                },
                "commands": {
                    "type": "array",
                    "description": "Optional explicit verifier gates (action=verifiers)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "program": { "type": "string" },
                            "args": { "type": "array", "items": { "type": "string" } },
                            "cwd": { "type": "string" }
                        },
                        "required": ["name", "program"]
                    }
                },
                "background": {
                    "type": "boolean",
                    "description": "Start verifier gates as background jobs (action=verifiers)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ExecutesCode, ToolCapability::Sandboxable]
    }

    fn approval_requirement_for(&self, _input: &Value) -> ApprovalRequirement {
        ApprovalRequirement::Required
    }

    fn is_read_only_for(&self, _input: &Value) -> bool {
        false
    }

    fn supports_parallel_for(&self, _input: &Value) -> bool {
        false
    }

    fn starts_detached_for(&self, input: &Value) -> bool {
        self.resolve_action(input) == "verifiers"
            && input.get("background").and_then(Value::as_bool) == Some(true)
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let action = self.resolve_action(&input).to_string();
        let input = self.strip_action(input)?;

        match action.as_str() {
            "tests" => RunTestsTool.execute(input, context).await,
            "verifiers" => RunVerifiersTool.execute(input, context).await,
            other => Err(ToolError::invalid_input(format!(
                "Unknown Run action: {other}"
            ))),
        }
    }
}
