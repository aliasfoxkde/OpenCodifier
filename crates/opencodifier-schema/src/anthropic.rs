//! `Anthropic` tool-use projection (PLANNING.md §7, §42).
//!
//! `Anthropic` exposes decisions as a *tool*: the model answers by emitting a
//! `tool_use` block whose `input` is the JSON object constrained by the
//! tool's `input_schema`. That schema is a standard JSON Schema draft, so
//! the decision mapping is the same as [`crate::openai`]'s — this module
//! delegates to it rather than duplicating it — with one deliberate
//! difference:
//!
//! > **`const` is supported here.** `Anthropic`'s `input_schema` is
//! > documented as standard JSON Schema, where `const` is a normal
//! > keyword, so `{"const": "qwen"}` normalizes to a single-candidate
//! > choice. `OpenAI` strict mode does not document `const`, so
//! > [`crate::openai`] rejects it.
//!
//! `oneOf` stays unsupported in both adapters: a union of decision shapes
//! is not a decision, it is several, and callers should model that as
//! distinct questions.
//!
//! # Wire shapes
//!
//! * Request: `{"tools":[{"name":...,"description":...,"input_schema":{...}}],
//!   "messages":[{"role":"user","content":<state text>}], "facts":{...}}`.
//!   A bare tool definition (just the tool object) is also accepted, since
//!   that is all `OpenCodifier` contributes to a vendor call.
//! * Response: the parsed tool `input` object (`{"<question id>": value}`),
//!   a `tool_use` block (`{"type":"tool_use","name":...,"input":{...}}`), or
//!   a message envelope (`{"content":[{"type":"tool_use","input":{...}}]}`).
//!
//! # Documented fidelity limitations
//!
//! Identical to [`crate::openai`]: answers carry the chosen value only, so
//! distributions are reconstructed with total mass on the reported value
//! and the outcome is [`opencodifier_core::DecisionOutcome::Verify`] —
//! the reconstruction describes the wire payload, never model certainty.
//! Policy and limits are local and are not transmitted.
//!
//! # Example
//!
//! ```
//! use opencodifier_core::{Candidate, ChoiceQuestion, DecisionQuestion, State};
//! use opencodifier_schema::{WireFormat, anthropic::Anthropic};
//!
//! let request = opencodifier_core::DecisionRequest::new(
//!     State::from_text("pick a storage engine"),
//!     vec![DecisionQuestion::Choice(
//!         ChoiceQuestion::new("engine", "Which engine?", vec![
//!             Candidate::new("sqlite", "embedded, zero ops").unwrap(),
//!             Candidate::new("postgres", "operational overhead").unwrap(),
//!         ]).unwrap(),
//!     )],
//!     opencodifier_core::DecisionPolicy::default(),
//!     opencodifier_core::RequestMetadata::default(),
//! )
//! .unwrap();
//!
//! let tool = Anthropic.encode_request(&request).unwrap();
//! assert_eq!(tool["tools"][0]["name"], serde_json::json!("opencodifier_decide"));
//! let schema = &tool["tools"][0]["input_schema"];
//! assert_eq!(schema["additionalProperties"], serde_json::json!(false));
//! assert_eq!(
//!     schema["properties"]["engine"]["enum"],
//!     serde_json::json!(["sqlite", "postgres"])
//! );
//! ```

use serde_json::{Value, json};

use opencodifier_core::{DecisionRequest, DecisionResponse, Limits, State};

use crate::codec::{WireFormat, enforce_limits};
use crate::error::{SchemaError, SchemaResult};
use crate::openai::{OpenAi, answer_for, report_for};
use crate::support::{as_object, required_object};

/// Name of the tool that carries the decision answers.
const TOOL_NAME: &str = "opencodifier_decide";

/// What the tool asks the model to do.
const TOOL_DESCRIPTION: &str =
    "Answer every decision question exactly once, using the provided state.";

/// The `Anthropic` tool-use codec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Anthropic;

impl Anthropic {
    // The codecs are stateless unit structs, but every operation takes
    // `&self` so a future registry can hand out configured variants without
    // an API break (PLANNING.md §36). The private helpers follow the same
    // shape for symmetry, hence the two allows.
    #![allow(clippy::unused_self, clippy::trivially_copy_pass_by_ref)]
    /// Builds the tool definition for one request's questions.
    pub fn tool_for(&self, request: &DecisionRequest) -> SchemaResult<Value> {
        let input_schema = OpenAi.schema_for(request)?;
        Ok(json!({
            "name": TOOL_NAME,
            "description": TOOL_DESCRIPTION,
            "input_schema": input_schema,
        }))
    }

    /// Normalizes a tool `input_schema` into questions.
    ///
    /// `const` properties are rewritten to single-value `enum` properties
    /// first (see the module docs), then the shared decision decoder runs.
    pub fn decode_input_schema(
        &self,
        schema: &Value,
        limits: &Limits,
    ) -> SchemaResult<Vec<opencodifier_core::DecisionQuestion>> {
        OpenAi.decode_schema(&rewrite_const(schema), limits)
    }
}

impl WireFormat for Anthropic {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    /// Projects a request onto the tool definition plus the user message
    /// carrying the state text.
    fn encode_request(&self, request: &DecisionRequest) -> SchemaResult<Value> {
        let tool = self.tool_for(request)?;
        let facts: serde_json::Map<String, Value> =
            request.state().facts().map(|(key, value)| (key.clone(), json!(value))).collect();
        Ok(json!({
            "tools": [tool],
            "tool_choice": {"type": "tool", "name": TOOL_NAME},
            "messages": [{"role": "user", "content": request.state().text()}],
            "facts": Value::Object(facts),
        }))
    }

    /// Normalizes a wire request body produced by
    /// [`Anthropic::encode_request`], or a bare tool definition.
    fn decode_request(&self, payload: &Value, limits: &Limits) -> SchemaResult<DecisionRequest> {
        let (schema, text, facts) = split_request(payload)?;
        let questions = self.decode_input_schema(&schema, limits)?;
        enforce_limits(text.len(), questions.len(), limits)?;
        let mut state = State::from_text(text);
        if let Some(facts) = facts {
            for (key, value) in &facts {
                let fact: opencodifier_core::FactValue = serde_json::from_value(value.clone())
                    .map_err(|error| SchemaError::invalid_value(key, error.to_string()))?;
                state = state.with_fact(key.clone(), fact);
            }
        }
        DecisionRequest::new(
            state,
            questions,
            opencodifier_core::DecisionPolicy::default(),
            opencodifier_core::RequestMetadata { request_id: None, limits: limits.clone() },
        )
        .map_err(SchemaError::from)
    }

    /// Projects a response onto the tool `input` object the model is asked
    /// to emit.
    fn encode_response(&self, response: &DecisionResponse) -> SchemaResult<Value> {
        OpenAi.encode_response(response)
    }

    /// Normalizes a tool-use answer into canonical IR answers.
    fn decode_response(
        &self,
        payload: &Value,
        request: &DecisionRequest,
        _limits: &Limits,
    ) -> SchemaResult<DecisionResponse> {
        let values = match unwrap_tool_use(payload)? {
            Some(values) => values,
            None => payload.clone(),
        };
        let values = as_object("input", &values)?;
        let mut answers = Vec::new();
        for question in request.questions() {
            let id = question.id().to_string();
            let Some(value) = values.get(&id) else {
                return Err(SchemaError::MissingField {
                    field: id,
                    context: ": every question must be answered".to_owned(),
                });
            };
            answers.push(answer_for(question, value)?);
        }
        let report = report_for(&answers);
        DecisionResponse::new(
            answers,
            opencodifier_core::DecisionOutcome::Verify,
            report,
            opencodifier_core::DecisionTrace::new(),
            opencodifier_core::DecisionMetrics::default(),
        )
        .map_err(SchemaError::from)
    }
}

/// Splits a request body into `(input_schema, state text, facts)`.
///
/// Accepts either the full body (`tools` + `messages`) or a bare tool
/// definition; the bare form carries no state text, which decodes to an
/// empty state (the caller owns the prompt).
#[allow(clippy::type_complexity)]
fn split_request(
    payload: &Value,
) -> SchemaResult<(Value, String, Option<serde_json::Map<String, Value>>)> {
    let body = as_object("payload", payload)?;
    let facts = match body.get("facts") {
        Some(Value::Null) | None => None,
        Some(value) => Some(as_object("facts", value)?.clone()),
    };
    if let Some(tools) = body.get("tools") {
        let tools =
            tools.as_array().ok_or_else(|| SchemaError::invalid_type("tools", "array", tools))?;
        let tool = tools.first().ok_or_else(|| {
            SchemaError::invalid_value("tools", "at least one tool definition is required")
        })?;
        let tool = as_object("tools[]", tool)?;
        let schema = required_object(tool, "input_schema")?;
        let text = match body.get("messages") {
            Some(messages) => state_from_messages(messages)?,
            None => String::new(),
        };
        return Ok((Value::Object(schema.clone()), text, facts));
    }
    let schema = required_object(body, "input_schema")?;
    Ok((Value::Object(schema.clone()), String::new(), facts))
}

/// Reads the state text out of a `messages` array.
fn state_from_messages(messages: &Value) -> SchemaResult<String> {
    let messages = messages
        .as_array()
        .ok_or_else(|| SchemaError::invalid_type("messages", "array", messages))?;
    let mut text = String::new();
    for message in messages {
        let message = as_object("messages[]", message)?;
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        match message.get("content") {
            Some(Value::String(content)) => text.push_str(content),
            Some(other) => {
                return Err(SchemaError::invalid_type("messages[].content", "string", other));
            }
            None => return Err(SchemaError::missing("messages[].content")),
        }
    }
    Ok(text)
}

/// Rewrites `{"const": x}` properties into `{"enum": [x]}` recursively.
///
/// `Anthropic`'s `input_schema` is standard JSON Schema, where `const` is a
/// normal keyword; the shared decision decoder models single-value
/// constraints as `enum`, so the two are folded here instead of duplicating
/// the decoder.
fn rewrite_const(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut rewritten = serde_json::Map::new();
            for (key, inner) in map {
                // `const` wins over any sibling `enum`: one value.
                if key == "const" {
                    rewritten.insert("enum".to_owned(), json!([inner.clone()]));
                } else if key == "enum" && map.contains_key("const") {
                    // Replaced by the `const` branch above.
                } else {
                    rewritten.insert(key.clone(), rewrite_const(inner));
                }
            }
            Value::Object(rewritten)
        }
        Value::Array(items) => Value::Array(items.iter().map(rewrite_const).collect()),
        other => other.clone(),
    }
}

/// Unwraps a `tool_use` block or a message envelope into the tool input.
///
/// Returns `Ok(None)` when `payload` is already the bare input object.
fn unwrap_tool_use(payload: &Value) -> SchemaResult<Option<Value>> {
    let Some(object) = payload.as_object() else { return Ok(None) };
    if object.get("type").and_then(Value::as_str) == Some("tool_use") {
        let input = required_object(object, "input")?;
        return Ok(Some(Value::Object(input.clone())));
    }
    let Some(content) = object.get("content") else { return Ok(None) };
    let content =
        content.as_array().ok_or_else(|| SchemaError::invalid_type("content", "array", content))?;
    for block in content {
        let block = as_object("content[]", block)?;
        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
            let input = required_object(block, "input")?;
            return Ok(Some(Value::Object(input.clone())));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion,
        RequestMetadata,
    };

    use serde_json::json;

    use super::*;

    /// One choice question and one boolean question, both answerable over
    /// the tool `input` object.
    fn request() -> DecisionRequest {
        DecisionRequest::new(
            State::from_text("route this request"),
            vec![
                DecisionQuestion::Choice(
                    ChoiceQuestion::new(
                        "model",
                        "Which model?",
                        vec![
                            Candidate::new("local-qwen", "fast").unwrap(),
                            Candidate::new("local-glm", "deep").unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
                DecisionQuestion::Boolean(BooleanQuestion::new("needs_tools", "Tools?").unwrap()),
            ],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    #[test]
    fn full_bodies_round_trip_through_the_tool() {
        let request = request();
        let body = Anthropic.encode_request(&request).unwrap();
        let decoded = Anthropic.decode_request(&body, &Limits::default()).unwrap();
        assert_eq!(decoded.state().text(), "route this request");
        assert_eq!(decoded.questions(), request.questions());
    }

    #[test]
    fn typed_facts_survive_the_wire() {
        let request = request();
        let mut body = Anthropic.encode_request(&request).unwrap();
        body["facts"] = json!({
            "context_tokens": {"kind": "integer", "value": 4096},
            "lane": {"kind": "text", "value": "fast"},
        });
        let decoded = Anthropic.decode_request(&body, &Limits::default()).unwrap();
        assert_eq!(
            decoded.state().fact("context_tokens").map(opencodifier_core::FactValue::as_f64),
            Some(Some(4_096.0))
        );
        assert_eq!(
            decoded.state().fact("lane").map(opencodifier_core::FactValue::as_str),
            Some(Some("fast"))
        );

        // A fact value the IR cannot represent is a typed error naming the
        // fact, not a silently dropped field.
        body["facts"] = json!({"context_tokens": {"deep": true}});
        let error = Anthropic.decode_request(&body, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("context_tokens"), "{error}");
    }

    #[test]
    fn messages_supply_the_state_text_only() {
        let request = request();
        let mut body = Anthropic.encode_request(&request).unwrap();
        // Non-user turns are skipped; user turns are concatenated in order.
        body["messages"] = json!([
            {"role": "system", "content": "be brief"},
            {"role": "user", "content": "route "},
            {"role": "user", "content": "this request"},
        ]);
        let decoded = Anthropic.decode_request(&body, &Limits::default()).unwrap();
        assert_eq!(decoded.state().text(), "route this request");

        // Structured user content is a typed error rather than prose guess.
        body["messages"] = json!([{"role": "user", "content": [{"type": "text"}]}]);
        assert_eq!(
            Anthropic.decode_request(&body, &Limits::default()).unwrap_err().code(),
            "schema.invalid_type"
        );

        // A user turn without content has no state to decode.
        body["messages"] = json!([{"role": "user"}]);
        assert_eq!(
            Anthropic.decode_request(&body, &Limits::default()).unwrap_err().code(),
            "schema.missing_field"
        );
    }

    #[test]
    fn const_properties_normalize_to_single_candidate_choices() {
        let schema = json!({
            "type": "object",
            "properties": {
                "lane": {"type": "string", "const": "fast", "title": "Which lane?"},
                // A sibling `enum` is replaced by the `const` value.
                "tier": {"type": "string", "const": "pro", "enum": ["free"], "title": "Which tier?"},
            },
            "required": ["lane", "tier"],
        });
        let questions = Anthropic.decode_input_schema(&schema, &Limits::default()).unwrap();
        assert_eq!(questions.len(), 2);
        for question in &questions {
            let DecisionQuestion::Choice(choice) = question else { unreachable!() };
            assert_eq!(choice.candidates().len(), 1, "{choice:?}");
        }
    }

    #[test]
    fn tool_use_envelopes_are_unwrapped() {
        let request = request();
        let answers = json!({"model": "local-glm", "needs_tools": true});

        // A bare `tool_use` block.
        let block =
            json!({"type": "tool_use", "id": "toolu_1", "name": TOOL_NAME, "input": answers});
        assert_eq!(
            Anthropic
                .decode_response(&block, &request, &Limits::default())
                .unwrap()
                .answers()
                .len(),
            2
        );

        // The full assistant message envelope.
        let envelope = json!({"content": [
            {"type": "text", "text": "considering the options"},
            {"type": "tool_use", "input": answers},
        ]});
        assert_eq!(
            Anthropic
                .decode_response(&envelope, &request, &Limits::default())
                .unwrap()
                .answers()
                .len(),
            2
        );

        // `content` that is not an array is a typed error.
        let not_a_list = json!({"content": "considering the options"});
        assert_eq!(
            Anthropic
                .decode_response(&not_a_list, &request, &Limits::default())
                .unwrap_err()
                .code(),
            "schema.invalid_type"
        );
    }

    #[test]
    fn every_question_must_be_answered() {
        let request = request();
        let error = Anthropic
            .decode_response(&json!({"model": "local-glm"}), &request, &Limits::default())
            .unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert!(error.to_string().contains("needs_tools"), "{error}");
    }

    #[test]
    fn a_body_without_messages_decodes_to_an_empty_state() {
        let tool = Anthropic.tool_for(&request()).unwrap();
        let body = json!({"tools": [tool]});
        let decoded = Anthropic.decode_request(&body, &Limits::default()).unwrap();
        assert_eq!(decoded.state().text(), "", "the caller owns the prompt");
        assert_eq!(decoded.questions().len(), 2);
    }

    #[test]
    fn malformed_request_bodies_report_typed_errors() {
        // An empty `tools` array carries no schema to decode.
        let error =
            Anthropic.decode_request(&json!({"tools": []}), &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("at least one tool"), "{error}");

        // A message envelope with no `tool_use` block answers nothing.
        let request = request();
        let answer = json!({"content": [{"type": "text", "text": "no tool call"}]});
        let error = Anthropic.decode_response(&answer, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert!(error.to_string().contains("model"), "{error}");
    }

    /// A bare tool definition — everything `OpenCodifier` contributes to a
    /// vendor call, without the `tools` array or any message — is a
    /// request body too, and `facts` travel with it.
    #[test]
    fn a_bare_tool_definition_decodes_with_facts() {
        let tool = Anthropic.tool_for(&request()).unwrap();
        let body = json!({
            "input_schema": tool["input_schema"],
            "facts": {"lane": {"kind": "text", "value": "fast"}},
        });
        let decoded = Anthropic.decode_request(&body, &Limits::default()).unwrap();
        assert_eq!(decoded.state().text(), "", "the caller owns the prompt");
        assert_eq!(decoded.questions(), request().questions());
        assert_eq!(
            decoded.state().fact("lane").map(opencodifier_core::FactValue::as_str),
            Some(Some("fast"))
        );
    }

    /// The response projection is the shared structured-output object, so
    /// the tool `input` a peer sent back re-encodes byte-for-byte.
    #[test]
    fn responses_project_onto_the_tool_input_object() {
        let request = request();
        let input = json!({"model": "local-qwen", "needs_tools": false});
        let response = Anthropic.decode_response(&input, &request, &Limits::default()).unwrap();
        assert_eq!(response.outcome(), opencodifier_core::DecisionOutcome::Verify);
        assert_eq!(Anthropic.encode_response(&response).unwrap(), input);

        // The same answers wrapped in a `tool_use` block decode identically.
        let block = json!({"type": "tool_use", "id": "toolu_1", "name": TOOL_NAME, "input": input});
        let wrapped = Anthropic.decode_response(&block, &request, &Limits::default()).unwrap();
        assert_eq!(wrapped.answers(), response.answers());
    }

    #[test]
    fn malformed_tool_shapes_report_typed_errors() {
        // `tools` must be an array when present.
        let error = Anthropic
            .decode_request(&json!({"tools": {"name": TOOL_NAME}}), &Limits::default())
            .unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        // `messages` must be an array of turns when present.
        let tool = Anthropic.tool_for(&request()).unwrap();
        let error = Anthropic
            .decode_request(&json!({"tools": [tool], "messages": "route this"}), &Limits::default())
            .unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        // A `tool_use` block without its `input` object carries no answer.
        let request = request();
        let block = json!({"type": "tool_use", "id": "toolu_1", "name": TOOL_NAME});
        assert_eq!(
            Anthropic.decode_response(&block, &request, &Limits::default()).unwrap_err().code(),
            "schema.missing_field"
        );

        // A payload that is not an object is not a tool input either.
        assert_eq!(
            Anthropic
                .decode_response(&json!(["model", "local-qwen"]), &request, &Limits::default())
                .unwrap_err()
                .code(),
            "schema.invalid_type"
        );
    }
}
