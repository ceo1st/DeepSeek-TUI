//! Canonical action-based wrapper for web tools.
//!
//! The model sees one tool: `Web` with an `action` parameter
//! (search | fetch | wait). Legacy names (`web_search`, `fetch_url`,
//! `wait_for_dev_server`) stay registered as hidden compat aliases that
//! force the action so saved transcripts replay correctly.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::dev_server_readiness::WaitForDevServerTool;
use super::fetch_url::FetchUrlTool;
use super::spec::{
    ApprovalRequirement, ToolCapability, ToolContext, ToolError, ToolResult, ToolSpec,
};
use super::web_search::WebSearchTool;

pub struct WebTool {
    name: &'static str,
    forced_action: Option<&'static str>,
}

impl WebTool {
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
                .unwrap_or("search")
        })
    }

    fn strip_action(&self, input: Value) -> Result<Value, ToolError> {
        let mut input = input;
        if let Some(obj) = input.as_object_mut() {
            obj.remove("action");
            Ok(input)
        } else {
            Err(ToolError::invalid_input("Web tool input must be an object"))
        }
    }
}

#[async_trait]
impl ToolSpec for WebTool {
    fn name(&self) -> &'static str {
        self.name
    }

    fn model_visible(&self) -> bool {
        self.name == "Web"
    }

    fn description(&self) -> &'static str {
        "Search the web, fetch a known URL, or wait for a local dev server. Prefer fetch for a canonical URL and search when the source is unknown. Web actions are read-only and network-policy aware."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "fetch", "wait"],
                    "description": "Action to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (action=search)"
                },
                "q": {
                    "type": "string",
                    "description": "Search query alias (action=search)"
                },
                "search_query": {
                    "type": "array",
                    "description": "Advanced search query array (action=search)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "q": { "type": "string" },
                            "query": { "type": "string" },
                            "max_results": { "type": "integer" },
                            "recency": {
                                "oneOf": [
                                    { "type": "string", "enum": ["day", "week", "month", "year"] },
                                    { "type": "integer", "minimum": 1, "maximum": 3650 }
                                ]
                            },
                            "domains": { "type": "array", "items": { "type": "string" } },
                            "locale": { "type": "string" }
                        }
                    }
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum search results (action=search)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (action=search, fetch, or wait)"
                },
                "recency": {
                    "oneOf": [
                        { "type": "string", "enum": ["day", "week", "month", "year"] },
                        { "type": "integer", "minimum": 1, "maximum": 3650 }
                    ],
                    "description": "Requested freshness window (action=search)"
                },
                "domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Restrict search results to domains (action=search)"
                },
                "locale": {
                    "type": "string",
                    "description": "Requested result locale (action=search)"
                },
                "url": {
                    "type": "string",
                    "description": "URL to fetch (action=fetch) or healthcheck URL (action=wait)"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "raw"],
                    "description": "Post-processing for fetched response (action=fetch)"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Truncate fetched response after this many bytes (action=fetch)"
                },
                "fields": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional JSONPath projections for JSON responses (action=fetch)"
                },
                "host": {
                    "type": "string",
                    "description": "Loopback host to poll (action=wait)"
                },
                "port": {
                    "type": "integer",
                    "description": "TCP port to wait for (action=wait)"
                },
                "poll_interval_ms": {
                    "type": "integer",
                    "description": "Delay between readiness probes in milliseconds (action=wait)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly, ToolCapability::Network]
    }

    fn approval_requirement_for(&self, _input: &Value) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    fn is_read_only_for(&self, _input: &Value) -> bool {
        true
    }

    fn supports_parallel_for(&self, input: &Value) -> bool {
        self.resolve_action(input) == "search"
    }

    fn starts_detached_for(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let action = self.resolve_action(&input).to_string();
        let input = self.strip_action(input)?;

        match action.as_str() {
            "search" => WebSearchTool.execute(input, context).await,
            "fetch" => FetchUrlTool.execute(input, context).await,
            "wait" => WaitForDevServerTool.execute(input, context).await,
            other => Err(ToolError::invalid_input(format!(
                "Unknown Web action: {other}"
            ))),
        }
    }
}
