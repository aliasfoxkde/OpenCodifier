//! `OpenAI` structured-outputs projection (PLANNING.md §8, §42).
//!
//! `OpenAI` strict mode constrains the JSON Schema subset a model can be
//! forced to emit. The verified rules this adapter honors:
//!
//! * every property named in `properties` must also appear in `required`;
//! * every object — including the root — declares
//!   `additionalProperties: false`;
//! * the root schema must be an object, never a bare `anyOf`;
//! * supported constructs: `type`, string `enum`, `anyOf` (where safely
//!   reducible), numeric `minimum`/`maximum`, `description`, `title`;
//! * `oneOf` and `const` are **undocumented** for strict mode and are
//!   rejected as `schema.unsupported_construct` rather than sent on the
//!   assumption they work.
//!
//! # Decision mappings
//!
//! | IR                        | Strict-mode schema                                        |
//! |---------------------------|-----------------------------------------------------------|
//! | [`ChoiceQuestion`]        | `{"type":"string","enum":[<candidate ids>]}`              |
//! | [`BooleanQuestion`]       | `{"type":"boolean"}`                                      |
//! | [`ScoreQuestion`]         | `{"type":"integer","minimum":0,"maximum":<n-1>}`          |
//!
//! Ordered level labels (and candidate descriptions) have no native slot in
//! a strict-mode enum/integer schema, so they travel in the property's
//! `description` behind an `opencodifier:` marker as JSON. `description` is
//! in the supported subset (PLANNING.md §8), so this stays inside the
//! documented surface; the marker format is private to this crate and
//! parsed by the same module that writes it.
//!
//! # Documented fidelity limitations
//!
//! * **No distributions.** `OpenAI` returns the chosen value only. The
//!   adapter reconstructs a distribution with all mass on the reported
//!   value, so `top_probability`/`margin`/`entropy` describe the *wire
//!   payload* (a strict schema admits exactly one value), never model
//!   certainty.
//! * **Outcome is `Verify`.** Decoded vendor answers are never accepted
//!   outright: they enter the engine as `DecisionOutcome::Verify` so
//!   calibration and the verifier stay in charge (PLANNING.md Rule 8).
//! * **No policy or metadata.** Confidence gates and limits are local
//!   concerns no vendor format carries; [`WireFormat::decode_request`]
//!   therefore attaches [`opencodifier_core::DecisionPolicy::default`]
//!   and the caller-supplied [`Limits`].
//! * Candidate descriptions and level labels survive only through the
//!   description markers described above; a schema written by hand without
//!   them decodes to candidates with empty descriptions and is rejected for
//!   scores (a score without its ordered labels is not decidable).
//!
//! # Example
//!
//! ```
//! use opencodifier_core::{Candidate, ChoiceQuestion, DecisionQuestion, ScoreLevel,
//!     ScoreQuestion, State};
//! use opencodifier_schema::{WireFormat, openai::OpenAi};
//!
//! let request = opencodifier_core::DecisionRequest::new(
//!     State::from_text("route this request"),
//!     vec![DecisionQuestion::Choice(
//!         ChoiceQuestion::new("model", "Which model?", vec![
//!             Candidate::new("local-qwen", "fast").unwrap(),
//!             Candidate::new("local-glm", "deep").unwrap(),
//!         ]).unwrap(),
//!     )],
//!     opencodifier_core::DecisionPolicy::default(),
//!     opencodifier_core::RequestMetadata::default(),
//! )
//! .unwrap();
//!
//! let wire = OpenAi.encode_request(&request).unwrap();
//! assert_eq!(wire["format"]["strict"], serde_json::json!(true));
//! let schema = &wire["format"]["schema"];
//! assert_eq!(schema["additionalProperties"], serde_json::json!(false));
//! assert_eq!(
//!     schema["properties"]["model"]["enum"],
//!     serde_json::json!(["local-qwen", "local-glm"])
//! );
//! ```

use serde_json::{Value, json};

use opencodifier_core::{
    BooleanQuestion, Candidate, CandidateId, ChoiceQuestion, ConfidenceReport, DecisionAnswer,
    DecisionMetrics, DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest,
    DecisionResponse, DecisionTrace, Limits, QuestionId, ScoreLevel, ScoreQuestion, State,
};

use crate::codec::{WireFormat, enforce_limits, enforce_unique_questions};
use crate::error::{SchemaError, SchemaResult};
use crate::support::{
    as_object, as_string, certain_distribution, optional_string, required_array, required_object,
    required_string, score_level, unit_interval,
};

/// Whether two branches of an `anyOf` describe the same decision.
///
/// Prose (`title`) is allowed to differ between branches; the candidate and
/// level sets are not, because they are the decision itself.
fn same_decision(left: &DecisionQuestion, right: &DecisionQuestion) -> bool {
    match (left, right) {
        (DecisionQuestion::Boolean(_), DecisionQuestion::Boolean(_)) => true,
        (DecisionQuestion::Choice(left), DecisionQuestion::Choice(right)) => {
            left.id() == right.id()
                && left
                    .candidates()
                    .iter()
                    .map(Candidate::id)
                    .eq(right.candidates().iter().map(Candidate::id))
        }
        (DecisionQuestion::Score(left), DecisionQuestion::Score(right)) => {
            left.id() == right.id()
                && left
                    .levels()
                    .iter()
                    .map(ScoreLevel::label)
                    .eq(right.levels().iter().map(ScoreLevel::label))
        }
        _ => false,
    }
}

/// Marker opening the machine-readable level list inside a property
/// description.
const LEVELS_MARKER: &str = "opencodifier:levels=";

/// Marker opening the machine-readable candidate description list inside a
/// property description.
const DESCRIPTIONS_MARKER: &str = "opencodifier:descriptions=";

/// Name of the strict-mode schema emitted for a decision request.
const SCHEMA_NAME: &str = "opencodifier_decisions";

/// The `OpenAI` structured-outputs codec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAi;

impl OpenAi {
    // The codecs are stateless unit structs, but every operation takes
    // `&self` so a future registry can hand out configured variants without
    // an API break (PLANNING.md §36). The private helpers follow the same
    // shape for symmetry, hence the two allows.
    #![allow(clippy::unused_self, clippy::trivially_copy_pass_by_ref)]
    /// Builds the strict-mode JSON Schema for one request's questions.
    ///
    /// The root is always an object with `additionalProperties: false` and
    /// every property in `required`, as strict mode demands.
    pub fn schema_for(&self, request: &DecisionRequest) -> SchemaResult<Value> {
        enforce_unique_questions(request)?;
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for question in request.questions() {
            let id = question.id().to_string();
            required.push(json!(id));
            properties.insert(id, self.property_for(question)?);
        }
        Ok(json!({
            "type": "object",
            "title": SCHEMA_NAME,
            "description": "One field per decision question, answered in a single response.",
            "properties": Value::Object(properties),
            "required": required,
            "additionalProperties": false,
        }))
    }

    /// Projects one question onto a strict-mode property schema.
    #[allow(clippy::too_many_lines)]
    fn property_for(&self, question: &DecisionQuestion) -> SchemaResult<Value> {
        match question {
            DecisionQuestion::Choice(choice) => {
                let ids: Vec<String> = choice
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.id().to_string())
                    .collect();
                let descriptions: Vec<String> = choice
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.description().to_owned())
                    .collect();
                Ok(json!({
                    "type": "string",
                    "title": choice.text(),
                    "description": format!(
                        "{DESCRIPTIONS_MARKER}{}",
                        json_string(&descriptions)?
                    ),
                    "enum": ids,
                }))
            }
            DecisionQuestion::Boolean(boolean) => Ok(json!({
                "type": "boolean",
                "title": boolean.text(),
                "description": "Answer true or false.",
            })),
            DecisionQuestion::Score(score) => {
                let labels: Vec<String> =
                    score.levels().iter().map(|level| level.label().to_owned()).collect();
                let top = labels.len() - 1;
                Ok(json!({
                    "type": "integer",
                    "title": score.text(),
                    "description": format!("{LEVELS_MARKER}{}", json_string(&labels)?),
                    "minimum": 0,
                    "maximum": top,
                }))
            }
            // The IR is `#[non_exhaustive]`: a question type added later
            // must surface as a typed error, never as a wrong schema.
            _ => Err(SchemaError::unsupported("question type")),
        }
    }

    /// Rebuilds questions from a strict-mode schema.
    ///
    /// Properties are read in schema order; unknown properties are ignored
    /// (forward compatibility), unsupported ones are rejected.
    pub fn decode_schema(
        &self,
        schema: &Value,
        limits: &Limits,
    ) -> SchemaResult<Vec<DecisionQuestion>> {
        self.decode_schema_map(as_object("schema", schema)?, limits)
    }

    /// [`OpenAi::decode_schema`] over an already-borrowed schema object.
    fn decode_schema_map(
        &self,
        root: &serde_json::Map<String, Value>,
        limits: &Limits,
    ) -> SchemaResult<Vec<DecisionQuestion>> {
        if root.contains_key("oneOf") {
            return Err(SchemaError::unsupported("oneOf"));
        }
        if root.contains_key("anyOf") {
            return Err(SchemaError::UnsupportedConstruct {
                construct: "anyOf",
                context: " at the schema root: strict mode requires an object root".to_owned(),
            });
        }
        let properties = required_object(root, "properties")?;
        let required = required_array(root, "required")?;
        if required.len() != properties.len() {
            return Err(SchemaError::InvalidValue {
                field: "required".to_owned(),
                reason: format!(
                    "strict mode requires every property to be required: {} of {} are",
                    required.len(),
                    properties.len()
                ),
            });
        }
        let mut questions = Vec::new();
        for (name, property) in properties {
            questions.push(self.property_to_question(name, property, limits)?);
        }
        if questions.is_empty() {
            return Err(SchemaError::EmptyQuestions);
        }
        if questions.len() > limits.max_questions {
            return Err(SchemaError::limit("max_questions", questions.len(), limits.max_questions));
        }
        Ok(questions)
    }

    /// Converts one property schema into a question.
    ///
    /// Every property a vendor may send that this crate cannot model as a
    /// decision is reported as a typed error, never dropped: a silently
    /// ignored field would change the meaning of the answers that follow.
    fn property_to_question(
        &self,
        name: &str,
        property: &Value,
        limits: &Limits,
    ) -> SchemaResult<DecisionQuestion> {
        let property = as_object(name, property)?;
        if property.contains_key("oneOf") {
            return Err(SchemaError::unsupported("oneOf"));
        }
        if property.contains_key("const") {
            return Err(SchemaError::unsupported("const"));
        }
        if let Some(branches) = property.get("anyOf") {
            let branches = branches
                .as_array()
                .ok_or_else(|| SchemaError::invalid_type("anyOf", "array", branches))?;
            return self.reduce_any_of(name, branches, limits);
        }
        let kind = property.get("type").ok_or_else(|| SchemaError::missing("type"))?;
        let kind = as_string("type", kind)?;
        match kind {
            "string" => {
                let Some(ids) = property.get("enum") else {
                    return Err(SchemaError::UnsupportedGenerationField { field: name.to_owned() });
                };
                let ids = ids
                    .as_array()
                    .ok_or_else(|| SchemaError::invalid_type("enum", "array", ids))?;
                self.choice_from_enum(name, property, ids, limits)
            }
            "boolean" => {
                let text = property_title(property).unwrap_or_else(|| name.to_owned());
                BooleanQuestion::new(name, text)
                    .map_err(SchemaError::from)
                    .map(DecisionQuestion::Boolean)
            }
            "integer" => {
                let labels = self.level_labels(property)?;
                let text = property_title(property).unwrap_or_else(|| name.to_owned());
                let levels: Vec<ScoreLevel> = labels
                    .iter()
                    .map(|label| score_level("levels", label.clone()))
                    .collect::<SchemaResult<_>>()?;
                ScoreQuestion::new(name, text, levels)
                    .map_err(SchemaError::from)
                    .map(DecisionQuestion::Score)
            }
            // A free-form number is generation, not a decision: PLANNING.md
            // §8 routes bounded numerics through integer minimum/maximum.
            "number" => Err(SchemaError::UnsupportedConstruct {
                construct: "number",
                context: ": use an integer with minimum/maximum for score questions".to_owned(),
            }),
            other => Err(SchemaError::InvalidType {
                field: name.to_owned(),
                expected: "string|boolean|integer",
                got: other.to_owned(),
            }),
        }
    }

    /// Reduces an `anyOf` property.
    ///
    /// Supported only where the union is safely reducible: every branch is
    /// either `null` or reduces to the same decision — same kind, and the
    /// same candidates or ordered levels for choice and score. Only the
    /// first branch is kept, so a union that merely restates a question
    /// (a nullable boolean written twice with different prose) normalizes
    /// instead of failing. Anything else (two different kinds, disjoint
    /// candidate sets, nested unions) is rejected — a union `OpenCodifier`
    /// cannot decide must not be silently collapsed.
    fn reduce_any_of(
        &self,
        name: &str,
        branches: &[Value],
        limits: &Limits,
    ) -> SchemaResult<DecisionQuestion> {
        let mut reduced: Option<DecisionQuestion> = None;
        for branch_value in branches {
            let branch = as_object(name, branch_value)?;
            if branch.get("type").and_then(Value::as_str) == Some("null") {
                continue;
            }
            let question = self.property_to_question(name, branch_value, limits)?;
            match &reduced {
                None => reduced = Some(question),
                Some(existing) if same_decision(&question, existing) => {}
                Some(_) => {
                    return Err(SchemaError::UnsupportedConstruct {
                        construct: "anyOf",
                        context: ": branches do not reduce to one decision kind".to_owned(),
                    });
                }
            }
        }
        reduced.ok_or_else(|| SchemaError::UnsupportedConstruct {
            construct: "anyOf",
            context: ": no decidable branch".to_owned(),
        })
    }

    /// Builds a choice question from a string enum.
    fn choice_from_enum(
        &self,
        name: &str,
        property: &serde_json::Map<String, Value>,
        ids: &[Value],
        limits: &Limits,
    ) -> SchemaResult<DecisionQuestion> {
        if ids.is_empty() {
            return Err(SchemaError::invalid_value(name, "enum must not be empty"));
        }
        if ids.len() > limits.max_candidates {
            return Err(SchemaError::limit("max_candidates", ids.len(), limits.max_candidates));
        }
        let text = property_title(property).unwrap_or_else(|| name.to_owned());
        let mut candidates = Vec::with_capacity(ids.len());
        for (index, value) in ids.iter().enumerate() {
            let id = as_string(&format!("{name}.enum[{index}]"), value)?;
            candidates.push(Candidate::from(CandidateId::new(id).map_err(SchemaError::from)?));
        }
        let descriptions = self.candidate_descriptions(property)?;
        for (candidate, description) in candidates.iter_mut().zip(descriptions) {
            *candidate = Candidate::new(candidate.id().to_string(), description)
                .map_err(|error| SchemaError::invalid_value(name, error.to_string()))?;
        }
        ChoiceQuestion::new(name, text, candidates)
            .map(DecisionQuestion::Choice)
            .map_err(SchemaError::from)
    }

    /// Reads the `opencodifier:descriptions=` marker, if present.
    fn candidate_descriptions(
        &self,
        property: &serde_json::Map<String, Value>,
    ) -> SchemaResult<Vec<String>> {
        match optional_string(property, "description")? {
            Some(description) if description.starts_with(DESCRIPTIONS_MARKER) => {
                serde_json::from_str::<Vec<String>>(&description[DESCRIPTIONS_MARKER.len()..])
                    .map_err(|error| SchemaError::invalid_value("description", error.to_string()))
            }
            // No marker: the schema was written by hand. Candidate
            // descriptions are then simply absent, not invented.
            _ => Ok(Vec::new()),
        }
    }

    /// Reads the `opencodifier:levels=` marker; required for scores, since
    /// an ordered score without its labels cannot be named.
    fn level_labels(&self, property: &serde_json::Map<String, Value>) -> SchemaResult<Vec<String>> {
        let description = optional_string(property, "description")?
            .filter(|text| text.starts_with(LEVELS_MARKER))
            .ok_or_else(|| SchemaError::MissingField {
                field: "description".to_owned(),
                context: format!(
                    ": integer properties must carry a `{LEVELS_MARKER}<json array>` \
                         description naming the ordered score levels"
                ),
            })?;
        let labels: Vec<String> = serde_json::from_str(&description[LEVELS_MARKER.len()..])
            .map_err(|error| SchemaError::invalid_value("description", error.to_string()))?;
        if labels.len() < 2 {
            return Err(SchemaError::invalid_value(
                "description",
                format!("a score needs at least two levels, got {}", labels.len()),
            ));
        }
        Ok(labels)
    }
}

impl WireFormat for OpenAi {
    fn name(&self) -> &'static str {
        "openai"
    }

    /// Projects a request onto the wire body `OpenCodifier` sends alongside a
    /// structured-outputs call: the strict `format` object (embed it as
    /// `response_format` in Chat Completions or `text.format` in the
    /// Responses API), the state text, and the typed facts.
    fn encode_request(&self, request: &DecisionRequest) -> SchemaResult<Value> {
        let schema = self.schema_for(request)?;
        let facts: serde_json::Map<String, Value> =
            request.state().facts().map(|(key, value)| (key.clone(), json!(value))).collect();
        Ok(json!({
            "format": {
                "type": "json_schema",
                "name": SCHEMA_NAME,
                "strict": true,
                "schema": schema,
            },
            "input": request.state().text(),
            "facts": Value::Object(facts),
        }))
    }

    /// Normalizes a wire request body produced by [`OpenAi::encode_request`].
    ///
    /// Unknown fields are ignored; the decoded request carries the default
    /// policy and the caller's limits (see the module docs).
    fn decode_request(&self, payload: &Value, limits: &Limits) -> SchemaResult<DecisionRequest> {
        let body = as_object("payload", payload)?;
        let text = required_string(body, "input")?;
        let mut state = State::from_text(text);
        if let Some(facts) = body.get("facts") {
            let facts = as_object("facts", facts)?;
            for (key, value) in facts {
                let fact: opencodifier_core::FactValue = serde_json::from_value(value.clone())
                    .map_err(|error| SchemaError::invalid_value(key, error.to_string()))?;
                state = state.with_fact(key.clone(), fact);
            }
        }
        let format = required_object(body, "format")?;
        let schema = required_object(format, "schema")?;
        let text = required_string(body, "input")?;
        let questions = self.decode_schema_map(schema, limits)?;
        enforce_limits(text.len(), questions.len(), limits)?;
        DecisionRequest::new(state, questions, DecisionPolicy::default(), limits_metadata(limits))
            .map_err(SchemaError::from)
    }

    /// Projects a response onto the parsed structured-output object: one
    /// field per question holding the chosen value.
    fn encode_response(&self, response: &DecisionResponse) -> SchemaResult<Value> {
        let mut output = serde_json::Map::new();
        for answer in response.answers() {
            let (id, value) = match answer {
                DecisionAnswer::Choice { question_id, choice, .. } => {
                    (question_id.to_string(), json!(choice.as_str()))
                }
                DecisionAnswer::Boolean { question_id, value, .. } => {
                    (question_id.to_string(), json!(value))
                }
                DecisionAnswer::Score { question_id, expected, .. } => {
                    // The wire form is the integer level index the schema
                    // asked for, not the label. It must serialize as a JSON
                    // integer: `2.0` is a number, and the decoder (rightly)
                    // rejects it.
                    // `expected` is a level index by construction, so the
                    // truncation below is exact; the level count is far
                    // inside `i64`.
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_possible_wrap,
                        clippy::useless_conversion
                    )]
                    let index = i64::from(expected.round() as i64);
                    (question_id.to_string(), Value::Number(serde_json::Number::from(index)))
                }
                _ => return Err(SchemaError::unsupported("answer type")),
            };
            output.insert(id, value);
        }
        Ok(Value::Object(output))
    }

    /// Normalizes a vendor answer into canonical IR answers.
    ///
    /// Accepts either the parsed structured-output object itself
    /// (`{"<question id>": <value>}`) or a Responses-API envelope
    /// (`{"output":[{"content":[{"type":"output_text","text":"..."}]}]}`),
    /// whose embedded JSON text is unwrapped first.
    fn decode_response(
        &self,
        payload: &Value,
        request: &DecisionRequest,
        _limits: &Limits,
    ) -> SchemaResult<DecisionResponse> {
        let values = match unwrap_envelope(payload)? {
            Some(values) => values,
            None => payload.clone(),
        };
        let values = as_object("output", &values)?;
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
        // Vendor answers are never accepted outright (module docs): they
        // arrive as Verify so the engine's confidence gate runs.
        DecisionResponse::new(
            answers,
            DecisionOutcome::Verify,
            report,
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .map_err(SchemaError::from)
    }
}

/// Wraps caller limits into request metadata.
fn limits_metadata(limits: &Limits) -> opencodifier_core::RequestMetadata {
    opencodifier_core::RequestMetadata { request_id: None, limits: limits.clone() }
}

/// The question text, taken from `title` (a supported strict-mode keyword).
fn property_title(property: &serde_json::Map<String, Value>) -> Option<String> {
    property.get("title").and_then(Value::as_str).map(str::to_owned).filter(|text| !text.is_empty())
}

/// Serializes a list of strings into the description marker payload.
fn json_string(values: &[String]) -> SchemaResult<String> {
    serde_json::to_string(values).map_err(|error| SchemaError::Json(error.to_string()))
}

/// Unwraps a Responses-API envelope into the parsed structured output.
///
/// Returns `Ok(None)` when `payload` is not an envelope at all, so the
/// caller can treat it as the bare output object.
fn unwrap_envelope(payload: &Value) -> SchemaResult<Option<Value>> {
    let Some(object) = payload.as_object() else { return Ok(None) };
    let Some(output) = object.get("output") else { return Ok(None) };
    let Some(output) = output.as_array() else {
        return Err(SchemaError::invalid_type("output", "array", output));
    };
    let mut text: Option<String> = None;
    for item in output {
        let item = as_object("output[]", item)?;
        let Some(content) = item.get("content") else { continue };
        let content = content
            .as_array()
            .ok_or_else(|| SchemaError::invalid_type("content", "array", content))?;
        for block in content {
            let block = as_object("content[]", block)?;
            if block.get("type").and_then(Value::as_str) == Some("output_text") {
                text = Some(required_string(block, "text")?);
            }
        }
    }
    let Some(text) = text else { return Ok(None) };
    serde_json::from_str::<Value>(&text)
        .map(Some)
        .map_err(|error| SchemaError::invalid_value("output_text", error.to_string()))
}

/// Converts one wire value into the canonical answer for `question`.
///
/// Shared with [`crate::anthropic`], whose tool `input` values are the
/// same chosen-value-only shape.
pub(crate) fn answer_for(
    question: &DecisionQuestion,
    value: &Value,
) -> SchemaResult<DecisionAnswer> {
    let id = question.id().to_string();
    match question {
        DecisionQuestion::Choice(choice) => {
            let reported = as_string(&id, value)?;
            let candidate = choice
                .candidates()
                .iter()
                .find(|candidate| candidate.id().as_str() == reported)
                .ok_or_else(|| {
                    SchemaError::invalid_value(
                        &id,
                        format!("`{reported}` is not one of the offered candidates"),
                    )
                })?;
            let choice_id = CandidateId::new(candidate.id().as_str()).map_err(SchemaError::from)?;
            let distribution = certain_distribution(candidate.id().as_str())?;
            Ok(DecisionAnswer::Choice {
                question_id: QuestionId::new(&id).map_err(SchemaError::from)?,
                choice: choice_id,
                distribution,
                confidence: 1.0,
            })
        }
        DecisionQuestion::Boolean(_) => {
            let reported =
                value.as_bool().ok_or_else(|| SchemaError::invalid_type(&id, "boolean", value))?;
            let probability = unit_interval(&id, 1.0)?;
            Ok(DecisionAnswer::Boolean {
                question_id: QuestionId::new(&id).map_err(SchemaError::from)?,
                value: reported,
                probability,
                confidence: probability,
            })
        }
        DecisionQuestion::Score(score) => {
            let raw =
                value.as_i64().ok_or_else(|| SchemaError::invalid_type(&id, "integer", value))?;
            if raw < 0 {
                return Err(SchemaError::invalid_value(
                    &id,
                    format!("level index {raw} is negative"),
                ));
            }
            // `raw` is non-negative here, and any value too wide for
            // `usize` is past the declared level count anyway.
            let index = usize::try_from(raw).unwrap_or(usize::MAX);
            if index >= score.levels().len() {
                return Err(SchemaError::invalid_value(
                    &id,
                    format!(
                        "level index {index} is past the {} declared levels",
                        score.levels().len()
                    ),
                ));
            }
            let label = score.levels()[index].label().to_owned();
            let distribution = certain_distribution(&label)?;
            #[allow(clippy::cast_precision_loss)]
            let expected = index as f64;
            Ok(DecisionAnswer::Score {
                question_id: QuestionId::new(&id).map_err(SchemaError::from)?,
                expected,
                level: label,
                distribution,
                confidence: 1.0,
            })
        }
        _ => Err(SchemaError::unsupported("question type")),
    }
}

/// Builds the confidence report for reconstructed answers.
///
/// Shared with [`crate::anthropic`].
///
/// Every reconstructed answer carries total mass on one value, so the
/// statistical fields are exact properties of the *wire payload*: one
/// admitted value, zero entropy, maximal margin. `calibrated_confidence`
/// is left at that same value because no calibration data arrived over the
/// wire; the `Verify` outcome is what keeps callers from treating it as an
/// accepted decision.
pub(crate) fn report_for(answers: &[DecisionAnswer]) -> ConfidenceReport {
    let mut top_probability = 1.0;
    for answer in answers {
        let probability = match answer {
            DecisionAnswer::Choice { distribution, .. }
            | DecisionAnswer::Score { distribution, .. } => distribution.top().probability,
            DecisionAnswer::Boolean { probability, .. } => *probability,
            // An answer type added after this crate was built is treated
            // as maximally uncertain: the cascade must not accept it.
            _ => 0.0,
        };
        if probability < top_probability {
            top_probability = probability;
        }
    }
    ConfidenceReport {
        top_probability,
        margin: 1.0,
        entropy: 0.0,
        calibrated_confidence: top_probability,
        ood_score: 0.0,
        verifier_agreement: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    fn choice() -> DecisionQuestion {
        DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model?",
                vec![
                    Candidate::new("local-qwen", "General coding").unwrap(),
                    Candidate::new("local-glm", "Deep reasoning").unwrap(),
                ],
            )
            .unwrap(),
        )
    }

    fn score() -> DecisionQuestion {
        DecisionQuestion::Score(
            ScoreQuestion::new(
                "difficulty",
                "How difficult?",
                ["trivial", "hard", "expert"]
                    .iter()
                    .map(|label| ScoreLevel::new(*label).unwrap())
                    .collect(),
            )
            .unwrap(),
        )
    }

    fn boolean() -> DecisionQuestion {
        DecisionQuestion::Boolean(BooleanQuestion::new("needs_tools", "Needs tools?").unwrap())
    }

    fn build_request(questions: Vec<DecisionQuestion>) -> DecisionRequest {
        DecisionRequest::new(
            State::from_text("route this request")
                .with_fact("context_tokens", opencodifier_core::FactValue::Integer(4_096)),
            questions,
            DecisionPolicy::default(),
            opencodifier_core::RequestMetadata::default(),
        )
        .unwrap()
    }

    fn tight_limits() -> Limits {
        Limits { max_candidates: 2, max_questions: 2, ..Limits::default() }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(OpenAi.name(), "openai");
    }

    #[test]
    fn schema_is_strict_mode_compliant() {
        let schema = OpenAi.schema_for(&build_request(vec![choice(), score(), boolean()])).unwrap();
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["additionalProperties"], json!(false));
        let properties = schema["properties"].as_object().unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(Value::as_str)
            .map(Option::unwrap)
            .collect();
        assert_eq!(required.len(), properties.len());
        for name in required {
            assert!(properties.contains_key(name), "{name} required but not a property");
        }
        // every property declares additionalProperties-free semantics via
        // the root object only; no undocumented keywords are emitted.
        let body = schema.to_string();
        assert!(!body.contains("oneOf"));
        assert!(!body.contains("\"const\""));
    }

    #[test]
    fn questions_survive_the_schema_projection() {
        let original = vec![choice(), score(), boolean()];
        let schema = OpenAi.schema_for(&build_request(original.clone())).unwrap();
        let decoded = OpenAi.decode_schema(&schema, &Limits::default()).unwrap();
        // Properties are a JSON object, so question order comes back in
        // sorted-key order; the questions themselves are unchanged.
        let mut expected = original.clone();
        expected.sort_by_key(|question| question.id().as_str().to_owned());
        assert_eq!(decoded, expected);
    }

    #[test]
    fn request_round_trips_with_documented_losses() {
        let original = build_request(vec![choice(), boolean()]);
        let wire = OpenAi.encode_request(&original).unwrap();
        assert_eq!(wire["format"]["type"], json!("json_schema"));
        assert_eq!(wire["format"]["strict"], json!(true));
        assert_eq!(wire["format"]["name"], json!("opencodifier_decisions"));
        let decoded = OpenAi.decode_request(&wire, &Limits::default()).unwrap();
        assert_eq!(decoded.state().text(), original.state().text());
        assert_eq!(decoded.questions(), original.questions());
        assert_eq!(
            decoded.state().fact("context_tokens").map(opencodifier_core::FactValue::as_f64),
            Some(Some(4_096.0))
        );
        // Documented loss: policy is local, never transmitted.
        assert_eq!(decoded.policy(), &DecisionPolicy::default());
        assert_eq!(decoded.metadata().limits, Limits::default());
    }

    #[test]
    fn request_payload_ignores_unknown_fields() {
        let mut wire = OpenAi.encode_request(&build_request(vec![boolean()])).unwrap();
        let body = wire.as_object_mut().unwrap();
        body.insert("temperature".to_owned(), json!(0.7));
        body.insert("vendor_future_thing".to_owned(), json!({"x": 1}));
        assert!(OpenAi.decode_request(&wire, &Limits::default()).is_ok());
    }

    /// A body without a `facts` object decodes to a state with no facts —
    /// the common vendor shape.
    #[test]
    fn request_without_facts_decodes_to_a_fact_free_state() {
        let schema = OpenAi.schema_for(&build_request(vec![boolean()])).unwrap();
        let wire = json!({
            "format": {
                "type": "json_schema",
                "name": "opencodifier_decisions",
                "strict": true,
                "schema": schema,
            },
            "input": "route this request",
        });
        let decoded = OpenAi.decode_request(&wire, &Limits::default()).unwrap();
        assert_eq!(decoded.state().text(), "route this request");
        assert_eq!(decoded.state().facts().count(), 0);
    }

    #[test]
    fn hand_written_schema_without_markers_loses_descriptions_only() {
        let schema = json!({
            "type": "object",
            "properties": {
                "model": {
                    "type": "string",
                    "enum": ["a", "b"],
                    "title": "Which model?"
                }
            },
            "required": ["model"],
            "additionalProperties": false,
        });
        let questions = OpenAi.decode_schema(&schema, &Limits::default()).unwrap();
        let DecisionQuestion::Choice(choice) = &questions[0] else { unreachable!() };
        assert_eq!(choice.text(), "Which model?");
        assert!(choice.candidates()[0].description().is_empty());
        assert_eq!(choice.candidates()[1].id().as_str(), "b");
    }

    #[test]
    fn malformed_markers_are_rejected() {
        let base = json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false,
        });
        let mut bad_levels = base.clone();
        bad_levels["properties"]["difficulty"] = json!({
            "type": "integer", "minimum": 0, "maximum": 1,
            "description": "opencodifier:levels=not json",
        });
        bad_levels["required"] = json!(["difficulty"]);
        let error = OpenAi.decode_schema(&bad_levels, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let mut short_levels = base;
        short_levels["properties"]["difficulty"] = json!({
            "type": "integer", "minimum": 0, "maximum": 0,
            "description": format!("{LEVELS_MARKER}[\"only\"]"),
        });
        short_levels["required"] = json!(["difficulty"]);
        let error = OpenAi.decode_schema(&short_levels, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let mut no_levels = json!({
            "type": "object",
            "properties": {"difficulty": {"type": "integer", "minimum": 0, "maximum": 3}},
            "required": ["difficulty"],
            "additionalProperties": false,
        });
        let error = OpenAi.decode_schema(&no_levels, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert!(error.to_string().contains(LEVELS_MARKER), "{error}");
        no_levels["properties"]["difficulty"]["description"] =
            json!(format!("{DESCRIPTIONS_MARKER}[\"a\"]"));
        let error = OpenAi.decode_schema(&no_levels, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
    }

    #[test]
    fn undocumented_constructs_are_rejected() {
        let limits = Limits::default();
        let root_one_of = json!({"oneOf": [], "properties": {}, "required": []});
        assert_eq!(
            OpenAi.decode_schema(&root_one_of, &limits).unwrap_err().code(),
            "schema.unsupported_construct"
        );

        let root_any_of = json!({"anyOf": [{"type": "object"}], "properties": {}, "required": []});
        let error = OpenAi.decode_schema(&root_any_of, &limits).unwrap_err();
        assert_eq!(error.code(), "schema.unsupported_construct");
        assert!(error.to_string().contains("object root"), "{error}");

        let property_one_of = json!({
            "type": "object",
            "properties": {"model": {"oneOf": []}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&property_one_of, &limits).unwrap_err().code(),
            "schema.unsupported_construct"
        );

        let property_const = json!({
            "type": "object",
            "properties": {"model": {"const": "qwen"}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&property_const, &limits).unwrap_err().code(),
            "schema.unsupported_construct"
        );

        let number_property = json!({
            "type": "object",
            "properties": {"difficulty": {"type": "number", "minimum": 0, "maximum": 4}},
            "required": ["difficulty"],
        });
        let error = OpenAi.decode_schema(&number_property, &limits).unwrap_err();
        assert_eq!(error.code(), "schema.unsupported_construct");
        assert!(error.to_string().contains("integer"), "{error}");
    }

    #[test]
    fn free_form_generation_fields_are_never_decisions() {
        let schema = json!({
            "type": "object",
            "properties": {"explanation": {"type": "string", "description": "prose"}},
            "required": ["explanation"],
        });
        let error = OpenAi.decode_schema(&schema, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.unsupported_generation_field");
        assert!(error.to_string().contains("explanation"), "{error}");
    }

    #[test]
    fn any_of_reduces_only_when_safe() {
        let limits = Limits::default();
        let nullable_boolean = json!({
            "type": "object",
            "properties": {"needs_tools": {"anyOf": [
                {"type": "null"},
                {"type": "boolean", "title": "Needs tools?"},
                {"type": "boolean"},
            ]}},
            "required": ["needs_tools"],
        });
        let questions = OpenAi.decode_schema(&nullable_boolean, &limits).unwrap();
        assert_eq!(questions, vec![boolean()]);

        let mixed = json!({
            "type": "object",
            "properties": {"field": {"anyOf": [
                {"type": "boolean"},
                {"type": "string", "enum": ["a"]},
            ]}},
            "required": ["field"],
        });
        assert_eq!(
            OpenAi.decode_schema(&mixed, &limits).unwrap_err().code(),
            "schema.unsupported_construct"
        );

        // Branches that restate a choice with different prose describe one
        // decision: same kind, same candidate sequence.
        let repeated_choice = json!({
            "type": "object",
            "properties": {"model": {"anyOf": [
                {"type": "string", "enum": ["a", "b"], "title": "One wording"},
                {"type": "string", "enum": ["a", "b"], "title": "Another wording"},
            ]}},
            "required": ["model"],
        });
        let questions = OpenAi.decode_schema(&repeated_choice, &limits).unwrap();
        assert!(matches!(&questions[0], DecisionQuestion::Choice(_)), "{questions:?}");

        // The same holds for a score question's ordered level sequence.
        let levels = format!("{LEVELS_MARKER}{}", json!(["low", "medium", "high"]));
        let repeated_score = json!({
            "type": "object",
            "properties": {"difficulty": {"anyOf": [
                {"type": "integer", "minimum": 0, "maximum": 2, "description": levels, "title": "one"},
                {"type": "integer", "minimum": 0, "maximum": 2, "description": levels, "title": "two"},
            ]}},
            "required": ["difficulty"],
        });
        let questions = OpenAi.decode_schema(&repeated_score, &limits).unwrap();
        assert!(matches!(&questions[0], DecisionQuestion::Score(_)), "{questions:?}");

        // Reordering the candidates is a different decision, not prose.
        let reordered = json!({
            "type": "object",
            "properties": {"model": {"anyOf": [
                {"type": "string", "enum": ["a", "b"]},
                {"type": "string", "enum": ["b", "a"]},
            ]}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&reordered, &limits).unwrap_err().code(),
            "schema.unsupported_construct"
        );

        let undecidable = json!({
            "type": "object",
            "properties": {"field": {"anyOf": [{"type": "null"}]}},
            "required": ["field"],
        });
        assert_eq!(
            OpenAi.decode_schema(&undecidable, &limits).unwrap_err().code(),
            "schema.unsupported_construct"
        );

        let not_an_array = json!({
            "type": "object",
            "properties": {"field": {"anyOf": {"type": "boolean"}}},
            "required": ["field"],
        });
        assert_eq!(
            OpenAi.decode_schema(&not_an_array, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );
    }

    #[test]
    fn malformed_schemas_report_typed_errors() {
        let limits = Limits::default();
        let missing_type =
            json!({"type": "object", "properties": {"x": {"enum": ["a"]}}, "required": ["x"]});
        assert_eq!(
            OpenAi.decode_schema(&missing_type, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let unknown_type =
            json!({"type": "object", "properties": {"x": {"type": "array"}}, "required": ["x"]});
        assert_eq!(
            OpenAi.decode_schema(&unknown_type, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );

        let not_object = json!([]);
        assert_eq!(
            OpenAi.decode_schema(&not_object, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );

        let partial_required = json!({
            "type": "object",
            "properties": {"a": {"type": "boolean"}, "b": {"type": "boolean"}},
            "required": ["a"],
        });
        let error = OpenAi.decode_schema(&partial_required, &limits).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("strict mode"), "{error}");

        let no_properties = json!({"type": "object", "required": []});
        assert_eq!(
            OpenAi.decode_schema(&no_properties, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let empty_properties = json!({"type": "object", "properties": {}, "required": []});
        assert_eq!(
            OpenAi.decode_schema(&empty_properties, &limits).unwrap_err().code(),
            "schema.empty_questions"
        );

        let empty_enum = json!({
            "type": "object",
            "properties": {"model": {"type": "string", "enum": []}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&empty_enum, &limits).unwrap_err().code(),
            "schema.invalid_value"
        );

        let enum_not_array = json!({
            "type": "object",
            "properties": {"model": {"type": "string", "enum": "a"}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&enum_not_array, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );

        let oversized = json!({
            "type": "object",
            "properties": {"model": {"type": "string", "enum": ["a", "b", "c"]}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&oversized, &tight_limits()).unwrap_err().code(),
            "schema.limit_exceeded"
        );

        let too_many_questions = json!({
            "type": "object",
            "properties": {
                "a": {"type": "boolean"},
                "b": {"type": "boolean"},
                "c": {"type": "boolean"},
            },
            "required": ["a", "b", "c"],
        });
        assert_eq!(
            OpenAi.decode_schema(&too_many_questions, &tight_limits()).unwrap_err().code(),
            "schema.limit_exceeded"
        );

        let enum_with_bad_id = json!({
            "type": "object",
            "properties": {"model": {"type": "string", "enum": ["has space"]}},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&enum_with_bad_id, &limits).unwrap_err().code(),
            "schema.invalid_value"
        );

        let property_not_object = json!({
            "type": "object",
            "properties": {"model": "enum"},
            "required": ["model"],
        });
        assert_eq!(
            OpenAi.decode_schema(&property_not_object, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );
    }

    fn answered(values: Value) -> Value {
        values
    }

    /// Finds the choice answer and its reconstructed fields.
    fn choice_answer(
        answers: &[DecisionAnswer],
    ) -> Option<(&CandidateId, &opencodifier_core::Distribution, f64)> {
        answers.iter().find_map(|answer| match answer {
            DecisionAnswer::Choice { choice, distribution, confidence, .. } => {
                Some((choice, distribution, *confidence))
            }
            _ => None,
        })
    }

    /// Finds the boolean answer and its probability.
    fn boolean_answer(answers: &[DecisionAnswer]) -> Option<(bool, f64)> {
        answers.iter().find_map(|answer| match answer {
            DecisionAnswer::Boolean { value, probability, .. } => Some((*value, *probability)),
            _ => None,
        })
    }

    /// Finds the score answer and its expected value, level, distribution.
    fn score_answer(
        answers: &[DecisionAnswer],
    ) -> Option<(f64, &str, &opencodifier_core::Distribution)> {
        answers.iter().find_map(|answer| match answer {
            DecisionAnswer::Score { expected, level, distribution, .. } => {
                Some((*expected, level.as_str(), distribution))
            }
            _ => None,
        })
    }

    /// The accessors report absence instead of panicking, which is what
    /// lets a test pin down which answer kinds a response contains.
    #[test]
    fn answer_accessors_report_absence() {
        let request = build_request(vec![boolean()]);
        let decoded = OpenAi
            .decode_response(&json!({"needs_tools": true}), &request, &Limits::default())
            .unwrap();
        let answers = decoded.answers();
        assert!(boolean_answer(answers).is_some());
        assert!(choice_answer(answers).is_none());
        assert!(score_answer(answers).is_none());

        let request = build_request(vec![choice()]);
        let decoded = OpenAi
            .decode_response(&json!({"model": "local-qwen"}), &request, &Limits::default())
            .unwrap();
        assert!(boolean_answer(decoded.answers()).is_none());
    }

    #[test]
    fn responses_decode_per_documented_reconstruction() {
        let request = build_request(vec![choice(), boolean(), score()]);
        let answers = answered(json!({
            "model": "local-glm",
            "needs_tools": true,
            "difficulty": 2,
        }));
        let response = OpenAi.decode_response(&answers, &request, &Limits::default()).unwrap();
        assert_eq!(
            response.outcome(),
            DecisionOutcome::Verify,
            "vendor answers are never accepted outright"
        );

        let answers = response.answers();
        assert_eq!(answers.len(), 3);

        let (choice, distribution, confidence) =
            choice_answer(answers).expect("expected a choice answer");
        assert_eq!(choice.as_str(), "local-glm");
        assert_eq!(distribution.entries().len(), 1);
        assert!((distribution.top().probability - 1.0).abs() < 1e-9);
        assert!((confidence - 1.0).abs() < 1e-9);

        let (value, probability) = boolean_answer(answers).expect("expected a boolean answer");
        assert!(value);
        assert!((probability - 1.0).abs() < 1e-9);

        let (expected, level, distribution) =
            score_answer(answers).expect("expected a score answer");
        assert!((expected - 2.0).abs() < 1e-9);
        assert_eq!(level, "expert");
        assert_eq!(distribution.top().key, "expert");

        // The report describes the wire payload, not model certainty.
        assert!((response.confidence().entropy - 0.0).abs() < 1e-9);
        assert!((response.confidence().margin - 1.0).abs() < 1e-9);
        assert!((response.confidence().top_probability - 1.0).abs() < 1e-9);
    }

    /// The report is the *weakest* answer, so a less certain boolean drags
    /// `top_probability` down below the certain ones around it.
    #[test]
    fn the_report_reports_the_weakest_answer() {
        let distribution = certain_distribution("local-qwen").unwrap();
        let answers = vec![
            DecisionAnswer::Choice {
                question_id: QuestionId::new("model").unwrap(),
                choice: CandidateId::new("local-qwen").unwrap(),
                distribution: distribution.clone(),
                confidence: 1.0,
            },
            DecisionAnswer::Boolean {
                question_id: QuestionId::new("needs_tools").unwrap(),
                value: true,
                probability: 0.4,
                confidence: 0.4,
            },
        ];
        let report = report_for(&answers);
        assert!((report.top_probability - 0.4).abs() < 1e-9, "{report:?}");
        assert!((report.margin - 1.0).abs() < 1e-9);
        assert!((report.calibrated_confidence - 0.4).abs() < 1e-9);
    }

    #[test]
    fn responses_survive_encode_decode() {
        let request = build_request(vec![choice(), boolean(), score()]);
        let answers = json!({"model": "local-qwen", "needs_tools": false, "difficulty": 0});
        let decoded = OpenAi.decode_response(&answers, &request, &Limits::default()).unwrap();
        let re_encoded = OpenAi.encode_response(&decoded).unwrap();
        assert_eq!(re_encoded, answers);
    }

    #[test]
    fn responses_api_envelopes_are_unwrapped() {
        let request = build_request(vec![boolean()]);
        let envelope = json!({
            "id": "resp_1",
            "output": [
                {"type": "message", "role": "assistant", "content": [
                    {"type": "output_text", "text": "{\"needs_tools\": true}"}
                ]}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 3},
        });
        let response = OpenAi.decode_response(&envelope, &request, &Limits::default()).unwrap();
        assert_eq!(response.answers().len(), 1);

        let malformed = json!({
            "output": [{"content": [{"type": "output_text", "text": "{oops"}]}],
        });
        let error = OpenAi.decode_response(&malformed, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let not_json_text = json!({"output": "nope"});
        let error =
            OpenAi.decode_response(&not_json_text, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let no_text = json!({"output": [{"content": [{"type": "output_text"}]}]});
        let error = OpenAi.decode_response(&no_text, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");

        let no_text_block =
            json!({"output": [{"content": [{"type": "refusal", "refusal": "no"}]}]});
        let error =
            OpenAi.decode_response(&no_text_block, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");

        let content_not_array = json!({"output": [{"content": "text"}]});
        let error =
            OpenAi.decode_response(&content_not_array, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let output_not_array = json!({"output": {}});
        let error =
            OpenAi.decode_response(&output_not_array, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");
    }

    #[test]
    fn incomplete_or_lying_answers_are_rejected() {
        let request = build_request(vec![choice(), boolean()]);

        let missing = json!({"model": "local-qwen"});
        let error = OpenAi.decode_response(&missing, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert!(error.to_string().contains("needs_tools"), "{error}");

        let unknown_candidate = json!({"model": "gpt-9", "needs_tools": true});
        let error =
            OpenAi.decode_response(&unknown_candidate, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let wrong_boolean_type = json!({"model": "local-qwen", "needs_tools": "yes"});
        let error =
            OpenAi.decode_response(&wrong_boolean_type, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let out_of_range_score = json!({"model": "local-qwen", "needs_tools": true});
        let score_request = build_request(vec![score()]);
        let error = OpenAi
            .decode_response(&out_of_range_score, &score_request, &Limits::default())
            .unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");

        let past_the_end = json!({"difficulty": 9});
        let error =
            OpenAi.decode_response(&past_the_end, &score_request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let negative = json!({"difficulty": -1});
        let error =
            OpenAi.decode_response(&negative, &score_request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let fractional = json!({"difficulty": 1.5});
        let error =
            OpenAi.decode_response(&fractional, &score_request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let not_an_object = json!(["model"]);
        let error =
            OpenAi.decode_response(&not_an_object, &request, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");
    }

    #[test]
    fn score_expected_values_round_to_the_wire_index() {
        let response = DecisionResponse::new(
            vec![DecisionAnswer::Score {
                question_id: QuestionId::new("difficulty").unwrap(),
                expected: 1.8,
                level: "hard".into(),
                distribution: opencodifier_core::Distribution::from_pairs([
                    ("trivial", 0.2),
                    ("hard", 0.8),
                ])
                .unwrap(),
                confidence: 0.8,
            }],
            DecisionOutcome::Accept,
            ConfidenceReport {
                top_probability: 0.8,
                margin: 0.6,
                entropy: 0.72,
                calibrated_confidence: 0.8,
                ood_score: 0.0,
                verifier_agreement: None,
            },
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .unwrap();
        let wire = OpenAi.encode_response(&response).unwrap();
        assert_eq!(wire["difficulty"], json!(2), "the wire form is an integer index");
    }

    /// A `descriptions` marker that is present but not the JSON array it
    /// promises is a typed error: half-written metadata must not silently
    /// become "no candidate descriptions".
    #[test]
    fn malformed_descriptions_marker_is_rejected() {
        let schema = json!({
            "type": "object",
            "properties": {"model": {
                "type": "string",
                "enum": ["a", "b"],
                "description": format!("{DESCRIPTIONS_MARKER}[\"a\","),
            }},
            "required": ["model"],
            "additionalProperties": false,
        });
        let error = OpenAi.decode_schema(&schema, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("description"), "{error}");
    }

    /// Every `anyOf` branch is a schema object; a branch that is not is a
    /// malformed schema, not an ignorable one.
    #[test]
    fn any_of_branches_must_be_schema_objects() {
        let schema = json!({
            "type": "object",
            "properties": {"needs_tools": {"anyOf": ["boolean"]}},
            "required": ["needs_tools"],
        });
        let error = OpenAi.decode_schema(&schema, &Limits::default()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");
        assert!(error.to_string().contains("needs_tools"), "{error}");
    }

    #[test]
    fn markers_round_trip_through_json_escaping() {
        // A candidate description and a level label containing characters
        // that must survive JSON escaping.
        let nasty = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which?",
                vec![
                    Candidate::new("a", "line\nbreak \"quoted\" \\slash").unwrap(),
                    Candidate::new("b", "emoji 🦀 |pipe").unwrap(),
                ],
            )
            .unwrap(),
        );
        let request = build_request(vec![nasty]);
        let wire = OpenAi.encode_request(&request).unwrap();
        let decoded = OpenAi.decode_request(&wire, &Limits::default()).unwrap();
        assert_eq!(decoded.questions(), request.questions());
    }
}
