//! The Decision Registry (PLANNING.md §63, D20): decision definitions as
//! reusable artifacts.
//!
//! A [`DecisionDefinition`] is a named, versioned document carrying a
//! question template and a policy; [`instantiate`](DecisionDefinition::instantiate)
//! turns it into a canonical [`DecisionRequest`] given the per-call state
//! (and, for dynamic choice definitions, the candidate list). Decoding
//! validates every field through the same constructors a wire request
//! would hit, so a registry cannot hold a definition the engine would
//! refuse, and the definition's identity is the SHA-256 of its canonical
//! serialization — `id` and `version` are labels, content is identity
//! (D20).
//!
//! ```text
//! document ──► DecisionDefinition::decode ──► Registry
//!                  │   (validating decode)          │
//!                  ▼                                ▼
//!          content_hash()                    get(id).instantiate(state, ...)
//!                                                   │
//!                                                   ▼
//!                                          canonical DecisionRequest
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use opencodifier_core::{
    BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest,
    RequestMetadata, ScoreLevel, ScoreQuestion, State,
};

use crate::error::{SchemaError, SchemaResult};

/// Which question kind a template names (the IR's three kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefinitionKind {
    /// One candidate from a runtime-defined list.
    Choice,
    /// True/false with probability.
    Boolean,
    /// An ordered level with a full distribution.
    Score,
}

/// A validated decision definition: the artifact form of §63.
///
/// Construct only through [`DecisionDefinition::decode`]; the validated
/// pieces it stores are exactly what a wire request would carry, so
/// instantiation cannot drift from the request path.
#[derive(Debug, Clone)]
pub struct DecisionDefinition {
    /// The dotted artifact name, also the instantiated question's id.
    id: String,
    /// The artifact's label version (≥ 1). Identity is the content hash,
    /// never this field.
    version: u64,
    /// Which question kind the template names.
    kind: DefinitionKind,
    /// The question text (validated non-empty at decode).
    text: String,
    /// Ordered level labels (score definitions only).
    levels: Vec<String>,
    /// Whether candidates come from the caller at instantiation (`true`)
    /// or from the artifact (`false`, choice only).
    dynamic_candidates: bool,
    /// The artifact's own candidates (static choice definitions);
    /// already validated by [`Candidate`]'s constructor at decode.
    static_candidates: Vec<Candidate>,
    /// The definition's policy; absent on the wire, this is
    /// [`DecisionPolicy::default`].
    policy: DecisionPolicy,
    /// The canonical serialization the identity hash derives from.
    canonical: Vec<u8>,
}

/// The wire document for [`DecisionDefinition`]; conversion validates.
#[derive(Debug, Deserialize, Serialize)]
struct RawDefinition {
    /// Dotted artifact name; becomes the instantiated question's id.
    id: String,
    /// Label version, ≥ 1.
    version: u64,
    /// The question template, a nested document (`{"type", "text", ...}`).
    question: RawQuestion,
    /// Where a choice question's candidates come from. Required for
    /// choice, forbidden for boolean and score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidates: Option<RawCandidates>,
    /// The policy; absent means [`DecisionPolicy::default`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy: Option<DecisionPolicy>,
}

/// The question template on the wire; `type` tags the variant.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RawQuestion {
    /// A choice question; the `candidates` sibling supplies candidates.
    Choice {
        /// Question text scored against candidate descriptions.
        text: String,
    },
    /// A boolean question.
    Boolean {
        /// Question text.
        text: String,
    },
    /// A score question over ordered levels.
    Score {
        /// Question text.
        text: String,
        /// Ordered level labels, lowest first.
        levels: Vec<String>,
    },
}

/// The candidate source on the wire: exactly one of `dynamic` or
/// `static` (§63's `{"dynamic": true}` form, or an embedded list).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct RawCandidates {
    /// Candidates arrive at instantiation.
    #[serde(default)]
    dynamic: bool,
    /// Candidates are embedded in the artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    r#static: Option<Vec<Candidate>>,
}

impl DecisionDefinition {
    /// Decodes and validates one definition document.
    ///
    /// Every field passes through the same validating constructors a wire
    /// request would — [`Candidate`] and [`DecisionPolicy`] validate on
    /// deserialize, and the text/level rules below mirror the core
    /// questions' own checks — so decode failure is the earliest possible
    /// refusal and a decoded definition is always instantiable.
    ///
    /// # Errors
    ///
    /// [`SchemaError`] under the adapter's stable codes: unknown question
    /// kinds and malformed candidate sources are `schema.invalid_value`;
    /// anything a deserializing core type rejects carries its message.
    pub fn decode(document: &Value) -> SchemaResult<Self> {
        let raw: RawDefinition = serde_json::from_value(document.clone())
            .map_err(|error| SchemaError::invalid_value("definition", error.to_string()))?;
        if raw.id.is_empty() {
            return Err(SchemaError::invalid_value("id", "must be non-empty"));
        }
        if raw.version == 0 {
            return Err(SchemaError::invalid_value("version", "must be at least 1"));
        }
        let text = match &raw.question {
            RawQuestion::Choice { text }
            | RawQuestion::Boolean { text }
            | RawQuestion::Score { text, .. } => text,
        };
        if text.is_empty() {
            return Err(SchemaError::invalid_value("text", "must be non-empty"));
        }

        let (kind, dynamic_candidates, static_candidates, levels) = match &raw.question {
            RawQuestion::Choice { .. } => {
                let source = raw.candidates.clone().ok_or_else(|| {
                    SchemaError::invalid_value(
                        "candidates",
                        "choice definitions require a candidate source",
                    )
                })?;
                let dynamic = match (source.dynamic, source.r#static) {
                    (true, None) => (true, Vec::new()),
                    (false, Some(candidates)) if !candidates.is_empty() => (false, candidates),
                    (true, Some(_)) => {
                        return Err(SchemaError::invalid_value(
                            "candidates",
                            "declare `dynamic` or `static`, not both",
                        ));
                    }
                    (false, None) => {
                        return Err(SchemaError::invalid_value(
                            "candidates",
                            "declare `dynamic: true` or a non-empty `static` list",
                        ));
                    }
                    (false, Some(_)) => {
                        return Err(SchemaError::invalid_value(
                            "candidates",
                            "`static` candidate lists may not be empty",
                        ));
                    }
                };
                (DefinitionKind::Choice, dynamic.0, dynamic.1, Vec::new())
            }
            RawQuestion::Boolean { .. } => {
                if raw.candidates.is_some() {
                    return Err(SchemaError::invalid_value(
                        "candidates",
                        "boolean definitions take no candidates",
                    ));
                }
                (DefinitionKind::Boolean, false, Vec::new(), Vec::new())
            }
            RawQuestion::Score { levels, .. } => {
                if raw.candidates.is_some() {
                    return Err(SchemaError::invalid_value(
                        "candidates",
                        "score definitions take no candidates",
                    ));
                }
                if levels.len() < 2 {
                    return Err(SchemaError::invalid_value(
                        "levels",
                        "score definitions need at least two levels",
                    ));
                }
                for label in levels {
                    ScoreLevel::new(label).map_err(SchemaError::from)?;
                }
                (DefinitionKind::Score, false, Vec::new(), levels.clone())
            }
        };

        let policy = raw.policy.clone().unwrap_or_default();
        let canonical = serde_json::to_vec(&raw)
            .map_err(|error| SchemaError::invalid_value("definition", error.to_string()))?;
        Ok(Self {
            id: raw.id,
            version: raw.version,
            kind,
            text: text.clone(),
            levels,
            dynamic_candidates,
            static_candidates,
            policy,
            canonical,
        })
    }

    /// The dotted artifact name.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The label version.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The definition's identity: SHA-256 hex over its canonical
    /// serialization (D20). Two documents with equal hashes are the same
    /// artifact regardless of what their labels claim.
    #[must_use]
    pub fn content_hash(&self) -> String {
        let digest = Sha256::digest(&self.canonical);
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    /// Whether instantiation requires caller-supplied candidates.
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        self.dynamic_candidates
    }

    /// Builds the canonical request this definition names: `state_text`
    /// becomes the state, and for a dynamic choice definition
    /// `dynamic_candidates` supplies the candidate list (a static
    /// definition refuses extra candidates rather than merging).
    ///
    /// # Errors
    ///
    /// [`SchemaError`] when the candidate source contradicts the
    /// definition, or when the core's question/request constructors
    /// reject the combination (`schema.*` codes throughout).
    pub fn instantiate(
        &self,
        state_text: &str,
        dynamic_candidates: &[Candidate],
    ) -> SchemaResult<DecisionRequest> {
        let question = match self.kind {
            DefinitionKind::Choice => {
                let candidates = if self.dynamic_candidates {
                    if dynamic_candidates.is_empty() {
                        return Err(SchemaError::invalid_value(
                            "candidates",
                            "this definition expects dynamic candidates; none were given",
                        ));
                    }
                    dynamic_candidates.to_vec()
                } else {
                    if !dynamic_candidates.is_empty() {
                        return Err(SchemaError::invalid_value(
                            "candidates",
                            "this definition embeds its candidates; pass none",
                        ));
                    }
                    self.static_candidates.clone()
                };
                DecisionQuestion::Choice(
                    ChoiceQuestion::new(&self.id, &self.text, candidates)
                        .map_err(SchemaError::from)?,
                )
            }
            DefinitionKind::Boolean => DecisionQuestion::Boolean(
                BooleanQuestion::new(&self.id, &self.text).map_err(SchemaError::from)?,
            ),
            DefinitionKind::Score => {
                let levels = self
                    .levels
                    .iter()
                    .map(|label| ScoreLevel::new(label).map_err(SchemaError::from))
                    .collect::<SchemaResult<Vec<_>>>()?;
                DecisionQuestion::Score(
                    ScoreQuestion::new(&self.id, &self.text, levels).map_err(SchemaError::from)?,
                )
            }
        };
        DecisionRequest::new(
            State::from_text(state_text),
            vec![question],
            self.policy.clone(),
            RequestMetadata::default(),
        )
        .map_err(SchemaError::from)
    }
}

/// An index of definitions by id (D20): lookup, not discovery.
///
/// Duplicates are refused at construction so a registry is always
/// unambiguous. No filesystem, no network, no reload — loading documents
/// is the caller's job, which keeps this crate sync and local-first.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    definitions: BTreeMap<String, DecisionDefinition>,
}

impl Registry {
    /// Indexes `definitions` by id, refusing duplicates.
    ///
    /// # Errors
    ///
    /// [`SchemaError::invalid_value`] when two definitions share an id —
    /// a silent last-one-wins would make lookups depend on input order.
    pub fn new(definitions: Vec<DecisionDefinition>) -> SchemaResult<Self> {
        let mut definitions = definitions;
        let mut map: BTreeMap<String, DecisionDefinition> = BTreeMap::new();
        for definition in definitions.drain(..) {
            let inserted = map.insert(definition.id().to_owned(), definition);
            if inserted.is_some() {
                return Err(SchemaError::invalid_value(
                    "id",
                    "duplicate definition id in registry",
                ));
            }
        }
        Ok(Self { definitions: map })
    }

    /// The definition registered under `id`, if any.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&DecisionDefinition> {
        self.definitions.get(id)
    }

    /// How many definitions the registry holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use serde_json::json;

    use super::*;
    use crate::WireFormat;

    fn static_choice_document() -> Value {
        json!({
            "id": "amortyx.model_selection",
            "version": 1,
            "question": {
                "type": "choice",
                "text": "Which model should process the request?",
            },
            "candidates": {
                "static": [
                    { "id": "local-qwen", "description": "fast local coding model" },
                    { "id": "cloud-large", "description": "long-context cloud service" },
                ],
            },
            "policy": {
                "min_confidence": 0.8,
                "verify_below": 0.65,
                "abstain_below": 0.5,
                "risk": "low",
            },
        })
    }

    #[test]
    fn a_static_choice_definition_round_trips_to_a_wire_request() {
        let definition = DecisionDefinition::decode(&static_choice_document()).unwrap();
        let request = definition.instantiate("the gpu queue is saturated", &[]).unwrap();

        assert_eq!(request.questions().len(), 1);
        let DecisionQuestion::Choice(question) = &request.questions()[0] else {
            panic!("expected a choice question");
        };
        assert_eq!(question.id().as_str(), "amortyx.model_selection");
        assert_eq!(question.candidates().len(), 2);
        // The instantiated request is an ordinary wire request: the native
        // adapter accepts it, proving the registry path feeds the same IR.
        let encoded = crate::native::Native.encode_request(&request).unwrap();
        assert_eq!(encoded["questions"][0]["type"], json!("choice"));
        assert!((request.policy().min_confidence() - 0.8).abs() < 1e-12);
    }

    #[test]
    fn identity_is_content_and_labels_are_labels() {
        let document = static_choice_document();
        let definition = DecisionDefinition::decode(&document).unwrap();
        assert_eq!(definition.content_hash().len(), 64);
        assert_eq!(
            definition.content_hash(),
            DecisionDefinition::decode(&document).unwrap().content_hash(),
            "identical documents are the same artifact"
        );

        // Bumping the label version without touching content is still a
        // content change: the hash moves (D20 — labels are not identity).
        let mut bumped = document.clone();
        bumped["version"] = json!(2);
        assert_ne!(
            definition.content_hash(),
            DecisionDefinition::decode(&bumped).unwrap().content_hash()
        );

        // A different policy under the same labels moves the hash too.
        let mut repolicied = document.clone();
        repolicied["policy"]["min_confidence"] = json!(0.9);
        assert_ne!(
            definition.content_hash(),
            DecisionDefinition::decode(&repolicied).unwrap().content_hash()
        );
    }

    #[test]
    fn dynamic_choice_definitions_take_candidates_at_instantiation() {
        let document = json!({
            "id": "router.target",
            "version": 1,
            "question": { "type": "choice", "text": "Which target should run this?" },
            "candidates": { "dynamic": true },
        });
        let definition = DecisionDefinition::decode(&document).unwrap();
        assert!(definition.is_dynamic());

        let candidates =
            vec![Candidate::new("a", "first").unwrap(), Candidate::new("b", "second").unwrap()];
        let request = definition.instantiate("pick one", &candidates).unwrap();
        let DecisionQuestion::Choice(question) = &request.questions()[0] else {
            panic!("expected a choice question");
        };
        assert_eq!(question.candidates().len(), 2);

        // No candidates: refused, not an empty-candidate panic downstream.
        assert!(definition.instantiate("pick one", &[]).is_err());
    }

    #[test]
    fn static_definitions_refuse_extra_candidates() {
        let definition = DecisionDefinition::decode(&static_choice_document()).unwrap();
        let extra = vec![Candidate::new("c", "intruder").unwrap()];
        let error = definition.instantiate("state", &extra).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
    }

    #[test]
    fn decode_refuses_malformed_definitions_with_schema_codes() {
        // No candidate source on a choice definition.
        let missing = json!({
            "id": "x", "version": 1,
            "question": { "type": "choice", "text": "pick" },
        });
        assert_eq!(
            DecisionDefinition::decode(&missing).unwrap_err().code(),
            "schema.invalid_value"
        );

        // Both candidate sources at once.
        let both = json!({
            "id": "x", "version": 1,
            "question": { "type": "choice", "text": "pick" },
            "candidates": {
                "dynamic": true,
                "static": [{ "id": "a", "description": "only" }],
            },
        });
        assert!(DecisionDefinition::decode(&both).is_err());

        // Candidates on a boolean definition.
        let boolean_with_candidates = json!({
            "id": "x", "version": 1,
            "question": { "type": "boolean", "text": "ok?" },
            "candidates": { "dynamic": true },
        });
        assert!(DecisionDefinition::decode(&boolean_with_candidates).is_err());

        // A one-level score question.
        let one_level = json!({
            "id": "x", "version": 1,
            "question": { "type": "score", "text": "how bad?", "levels": ["low"] },
        });
        assert!(DecisionDefinition::decode(&one_level).is_err());

        // Unknown question kind, empty text, version 0.
        let unknown =
            json!({ "id": "x", "version": 1, "question": { "type": "poem", "text": "t" } });
        assert!(DecisionDefinition::decode(&unknown).is_err());
        let empty_text = json!({
            "id": "x", "version": 1,
            "question": { "type": "boolean", "text": "" },
        });
        assert!(DecisionDefinition::decode(&empty_text).is_err());
        let zero = json!({
            "id": "x", "version": 0,
            "question": { "type": "boolean", "text": "ok?" },
        });
        assert!(DecisionDefinition::decode(&zero).is_err());
    }

    #[test]
    fn a_registry_refuses_duplicate_ids_and_looks_up_by_id() {
        let first = DecisionDefinition::decode(&static_choice_document()).unwrap();
        let duplicate = DecisionDefinition::decode(&static_choice_document()).unwrap();
        assert!(Registry::new(vec![first.clone(), duplicate]).is_err());

        let registry = Registry::new(vec![first]).unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("amortyx.model_selection").unwrap().version(), 1);
        assert!(registry.get("other").is_none());

        assert!(Registry::default().is_empty());
    }

    #[test]
    fn a_boolean_definition_instantiates_without_candidates() {
        let document = json!({
            "id": "triage.needs_human",
            "version": 3,
            "question": { "type": "boolean", "text": "Does this need a human?" },
        });
        let definition = DecisionDefinition::decode(&document).unwrap();
        assert_eq!(definition.version(), 3);
        let request = definition.instantiate("an unusual stack trace appeared", &[]).unwrap();
        let DecisionQuestion::Boolean(question) = &request.questions()[0] else {
            panic!("expected a boolean question");
        };
        assert_eq!(question.id().as_str(), "triage.needs_human");
    }

    /// The registry recipe's request is derived, never hand-written: this
    /// test instantiates the committed definition and pins the wire request
    /// in `recipes/registry/` to the exact bytes `instantiate` produces, so
    /// the recipe cannot silently drift from the code (the same discipline
    /// every other recipe's captured output follows).
    #[test]
    fn the_registry_recipe_request_is_the_definitions_exact_instantiation() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let root = std::path::Path::new(manifest)
            .parent()
            .and_then(|path| path.parent())
            .expect("crate sits two levels under the repo root");
        let raw =
            std::fs::read_to_string(root.join("recipes/registry/model-selection.definition.json"))
                .expect("recipe definition present in the repo");
        let document: Value = serde_json::from_str(&raw).unwrap();

        let definition = DecisionDefinition::decode(&document).unwrap();
        let request = definition.instantiate("complex reasoning over a large proof", &[]).unwrap();
        let wire = crate::WireFormat::encode_request(&crate::native::Native, &request).unwrap();

        let path = root.join("recipes/registry/requests/model-selection.json");
        let Ok(committed) = std::fs::read_to_string(&path) else {
            // First capture: write the derived request so it can be
            // committed, and fail this run to force the review.
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&wire).unwrap() + "\n").unwrap();
            panic!("recipe request captured to {path:?}; commit it and re-run");
        };
        let committed: Value = serde_json::from_str(&committed).unwrap();
        assert_eq!(wire, committed, "recipe request drifted from instantiate()");
    }
}
