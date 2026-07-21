//! Canonical action-based wrapper for git inspection tools.
//!
//! The model sees one tool: `Git` with an `action` parameter
//! (status | diff | log | show | blame). Legacy names (`git_status`,
//! `git_diff`, etc.) stay registered as hidden compat aliases that force the
//! action so saved transcripts replay correctly.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::git::{GitDiffTool, GitStatusTool};
use super::git_history::{GitBlameTool, GitLogTool, GitShowTool};
use super::spec::{
    ApprovalRequirement, ToolCapability, ToolContext, ToolError, ToolResult, ToolSpec,
};

pub struct GitTool {
    name: &'static str,
    forced_action: Option<&'static str>,
}

impl GitTool {
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
                .unwrap_or("status")
        })
    }

    fn strip_action(&self, input: Value) -> Result<Value, ToolError> {
        let mut input = input;
        if let Some(obj) = input.as_object_mut() {
            obj.remove("action");
            Ok(input)
        } else {
            Err(ToolError::invalid_input("Git tool input must be an object"))
        }
    }
}

#[async_trait]
impl ToolSpec for GitTool {
    fn name(&self) -> &'static str {
        self.name
    }

    fn model_visible(&self) -> bool {
        self.name == "Git"
    }

    fn description(&self) -> &'static str {
        "Inspect repository state and history with status, diff, log, show, or blame. All actions are read-only and parallel-safe."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "show", "blame"],
                    "description": "Action to perform"
                },
                "path": {
                    "type": "string",
                    "description": "Optional subdirectory or file path to scope the git command"
                },
                "cached": {
                    "type": "boolean",
                    "description": "When true, diff staged changes (action=diff)"
                },
                "unified": {
                    "type": "integer",
                    "description": "Number of context lines for diff or show output"
                },
                "max_count": {
                    "type": "integer",
                    "description": "Maximum commits to return (action=log)"
                },
                "author": {
                    "type": "string",
                    "description": "Author filter (action=log)"
                },
                "since": {
                    "type": "string",
                    "description": "Lower date bound (action=log)"
                },
                "until": {
                    "type": "string",
                    "description": "Upper date bound (action=log)"
                },
                "rev": {
                    "type": "string",
                    "description": "Revision to show (action=show) or blame against (action=blame)"
                },
                "patch": {
                    "type": "boolean",
                    "description": "Include patch hunks (action=show)"
                },
                "stat": {
                    "type": "boolean",
                    "description": "Include stat summary (action=show)"
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to include (action=blame)"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum lines to include (action=blame)"
                },
                "porcelain": {
                    "type": "boolean",
                    "description": "Emit line-porcelain output (action=blame)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly, ToolCapability::Sandboxable]
    }

    fn approval_requirement_for(&self, _input: &Value) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    fn is_read_only_for(&self, _input: &Value) -> bool {
        true
    }

    fn supports_parallel_for(&self, _input: &Value) -> bool {
        true
    }

    fn starts_detached_for(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let action = self.resolve_action(&input).to_string();
        let input = self.strip_action(input)?;

        match action.as_str() {
            "status" => GitStatusTool.execute(input, context).await,
            "diff" => GitDiffTool.execute(input, context).await,
            "log" => GitLogTool.execute(input, context).await,
            "show" => GitShowTool.execute(input, context).await,
            "blame" => GitBlameTool.execute(input, context).await,
            other => Err(ToolError::invalid_input(format!(
                "Unknown Git action: {other}"
            ))),
        }
    }
}
