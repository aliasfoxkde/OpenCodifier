//! Tool-call results and the error envelope (PLANNING.md §31, §36).
//!
//! The MCP surface reports failures in the same shape every other ingress
//! reports them — `{"error":{"code":"...","message":"..."}}` — under the
//! same stable codes: `schema.*` from wire normalization, `graph.*` /
//! `engine.*` from the engine, and exactly one code of its own
//! (`mcp.batch_too_large`), because only the batch bound is MCP-specific.
//! Callers branch on codes, never on prose.
//!
//! Failure mode discipline follows the MCP spec's own guidance: a request
//! that reached the tool but failed there (a refused payload, an
//! abstaining question is *not* one of these — abstention is a successful
//! outcome) is a tool-level error, `Ok(CallToolResult::error(...))`, never
//! a protocol-level `Err(ErrorData)`.

use opencodifier_engine::EngineError;
use opencodifier_schema::SchemaError;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

/// Everything a tool can refuse, as a stable code plus a typed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Stable machine-readable code (`schema.*`, `graph.*`, `engine.*`,
    /// `mcp.*`).
    pub code: String,
    /// Human-readable, typed message. Never carries internal state, never
    /// echoes the offending payload back.
    pub message: String,
}

impl Failure {
    /// Builds a failure from an explicit code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    /// A payload the wire adapter refused — the adapter owns the code.
    pub fn schema(error: &SchemaError) -> Self {
        Self::new(error.code(), error.to_string())
    }

    /// An engine fault or refusal — the engine owns the code.
    pub fn engine(error: &EngineError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

/// Wraps a tool's outcome as a `CallToolResult`.
///
/// Success and failure both carry the payload as structured content (so an
/// MCP client gets typed JSON, not a string to re-parse) and as the text
/// content (so a client that only reads text still sees the document).
pub fn respond(result: Result<Value, Failure>) -> CallToolResult {
    let (payload, is_error) = match result {
        Ok(value) => (value, false),
        Err(failure) => {
            let code = failure.code;
            let message = failure.message;
            (serde_json::json!({ "error": { "code": code, "message": message } }), true)
        }
    };
    let mut call = CallToolResult::success(vec![ContentBlock::text(payload.to_string())]);
    call.structured_content = Some(payload);
    call.is_error = Some(is_error);
    call
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use opencodifier_core::CoreError;

    #[test]
    fn success_carries_the_payload_as_structured_content() {
        let call = respond(Ok(serde_json::json!({ "valid": true, "nodes": 4 })));
        assert_eq!(call.is_error, Some(false));
        assert_eq!(call.structured_content.as_ref().unwrap()["nodes"], 4);
        let text = call.content.first().unwrap().as_text().unwrap();
        assert_eq!(text.text, r#"{"nodes":4,"valid":true}"#);
    }

    #[test]
    fn failures_use_the_shared_error_envelope() {
        let call = respond(Err(Failure::new("mcp.batch_too_large", "too many requests")));
        assert_eq!(call.is_error, Some(true));
        let structured = call.structured_content.unwrap();
        assert_eq!(structured["error"]["code"], "mcp.batch_too_large");
        assert_eq!(structured["error"]["message"], "too many requests");
    }

    #[test]
    fn schema_failures_keep_the_adapters_code() {
        let error = SchemaError::Core(CoreError::EmptyQuestions);
        let call = respond(Err(Failure::schema(&error)));
        let structured = call.structured_content.unwrap();
        assert_eq!(structured["error"]["code"], "schema.empty_questions");
    }

    #[test]
    fn engine_failures_keep_the_engines_code() {
        let error = EngineError::Cancelled;
        let call = respond(Err(Failure::engine(&error)));
        let structured = call.structured_content.unwrap();
        assert_eq!(structured["error"]["code"], "engine.cancelled");
    }
}
