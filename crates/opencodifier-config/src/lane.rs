//! The lane table (#191): named, complete answers to "what stack should
//! serve this request?". A lane is a contract, not a size — it names the
//! engine configuration, the rung tail, the model backend, the ladder
//! profile, and the escalation policy, plus the tags selection filters
//! by.

use crate::resolved::{
    EscalationSection, LadderSection, LaneSection, ModelSection, RawConfig, RungsSection,
};
use std::fmt;

/// Access constraints a lane declares; first-run detection proposes
/// lanes by what the host has, selection is by constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaneTag {
    /// Fully offline, no artifacts beyond what ships with the binary.
    Offline,
    /// Runs on CPU only.
    CpuOnly,
    /// Benefits from a GPU (Vulkan or CUDA).
    Gpu,
    /// Interactive latency profile.
    Interactive,
    /// Batch throughput profile.
    Batch,
    /// Serves a continuous-integration caller.
    Ci,
}

impl LaneTag {
    /// The tag as written in the config document and CLI output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::CpuOnly => "cpu-only",
            Self::Gpu => "gpu",
            Self::Interactive => "interactive",
            Self::Batch => "batch",
            Self::Ci => "ci",
        }
    }
}

impl fmt::Display for LaneTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of the lane table: a fixed, small set — a lane that cannot be
/// measured does not ship.
#[derive(Debug, Clone)]
pub struct LanePreset {
    /// The lane name (`zero`, `guard`, `delegate`, `local-2b`, ...).
    pub name: &'static str,
    /// One-line description of the stack.
    pub description: &'static str,
    /// Declared access tags.
    pub tags: &'static [LaneTag],
    /// Accuracy figure of record: the repo's *closest measured row*
    /// (`docs/BENCHMARKS.md`), or `None` when no measured row stands in
    /// for it. Absence is explicit.
    pub accuracy_standing_in: Option<f64>,
    /// Model artifacts the lane expects; absent artifacts fail loudly
    /// with `model.missing` at use time (checked by `init`/`doctor`,
    /// not here — this crate does no filesystem probing).
    pub expected_artifacts: &'static [&'static str],
}

/// The lane table of record. Numbers in `accuracy_standing_in` are the
/// repo's own measured rows for the *closest* measured stack
/// (`docs/BENCHMARKS.md`), not measurements of the composed lane; the
/// description says which row stands in.
pub const LANE_TABLE: &[LanePreset] = &[
    LanePreset {
        name: "zero",
        description: "rules + relational + lexical; offline, deterministic, no artifacts (engine floor, 0.683)",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly, LaneTag::Interactive],
        accuracy_standing_in: Some(0.683),
        expected_artifacts: &[],
    },
    LanePreset {
        name: "guard",
        description: "zero + local embedding model (gte-modernbert int8); better semantic narrowing, no generative model — unmeasured as a composition",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly],
        accuracy_standing_in: None,
        expected_artifacts: &["gte-modernbert int8 embedding ONNX"],
    },
    LanePreset {
        name: "delegate",
        description: "zero + escalate-to-host-LLM; the cheapest lane, composes with an existing agent (0.683 engine-side)",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly, LaneTag::Interactive],
        accuracy_standing_in: Some(0.683),
        expected_artifacts: &[],
    },
    LanePreset {
        name: "local-2b",
        description: "guard + local llama.cpp 2B (CPU); self-contained accuracy (0.725 model arm)",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly, LaneTag::Interactive],
        accuracy_standing_in: Some(0.725),
        expected_artifacts: &["2B decision GGUF"],
    },
    LanePreset {
        name: "local-4b",
        description: "guard + local llama.cpp 4B Q4_K_M; self-contained accuracy, Vulkan optional (0.800 model arm)",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly, LaneTag::Gpu],
        accuracy_standing_in: Some(0.800),
        expected_artifacts: &["4B Q4_K_M GGUF"],
    },
    LanePreset {
        name: "local-9b",
        description: "guard + 9B Q3_K_S; the accuracy ceiling, the latency floor (0.817 model arm)",
        tags: &[LaneTag::Offline, LaneTag::Gpu, LaneTag::Batch],
        accuracy_standing_in: Some(0.817),
        expected_artifacts: &["9B Q3_K_S GGUF"],
    },
    LanePreset {
        name: "verify",
        description: "local-2b as decider + a second model as verifier, agreement-gated — unmeasured as a composition",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly],
        accuracy_standing_in: None,
        expected_artifacts: &["2B decision GGUF", "verifier GGUF"],
    },
    LanePreset {
        name: "ci",
        description: "zero + CI facts graph + escalation gate; task-specific, decision support only — the runtime never becomes a CI system",
        tags: &[LaneTag::Offline, LaneTag::CpuOnly, LaneTag::Ci],
        accuracy_standing_in: None,
        expected_artifacts: &[],
    },
];

/// Errors of lane lookup and expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaneError {
    /// The lane name is not in the table.
    Unknown {
        /// The rejected name.
        name: String,
    },
}

impl LaneError {
    /// The stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown { .. } => "lane.unknown",
        }
    }
}

impl fmt::Display for LaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { name } => write!(f, "{}: unknown lane `{name}`", self.code()),
        }
    }
}

impl std::error::Error for LaneError {}

/// Looks a lane up by name.
///
/// # Errors
///
/// [`LaneError::Unknown`] when the name is not in the table.
pub fn preset(name: &str) -> Result<&'static LanePreset, LaneError> {
    LANE_TABLE
        .iter()
        .find(|lane| lane.name == name)
        .ok_or_else(|| LaneError::Unknown { name: name.to_owned() })
}

/// Expands a lane into the config document it names. The expansion is
/// the lane's *contract*: `lane show` prints it, `lane use` writes it.
/// Lanes that name model artifacts record the expectation in
/// [`LanePreset::expected_artifacts`]; probing the filesystem for them
/// is `init`/`doctor`'s job, so expansion never touches the disk.
///
/// # Errors
///
/// [`LaneError::Unknown`] when the name is not in the table.
pub fn expand(name: &str) -> Result<RawConfig, LaneError> {
    let lane = preset(name)?;
    let model = match lane.name {
        "guard" => Some(ModelSection {
            backend: Some("onnx".to_owned()),
            id: Some("gte-modernbert-int8".to_owned()),
            timeout_ms: None,
            artifacts: None,
        }),
        "local-2b" | "verify" => Some(ModelSection {
            backend: Some("llamacpp".to_owned()),
            id: Some("decider-2b".to_owned()),
            timeout_ms: None,
            artifacts: None,
        }),
        "local-4b" => Some(ModelSection {
            backend: Some("llamacpp".to_owned()),
            id: Some("decider-4b".to_owned()),
            timeout_ms: None,
            artifacts: None,
        }),
        "local-9b" => Some(ModelSection {
            backend: Some("llamacpp".to_owned()),
            id: Some("decider-9b".to_owned()),
            timeout_ms: None,
            artifacts: None,
        }),
        _ => None,
    };
    let escalation = match lane.name {
        "delegate" | "ci" => {
            Some(EscalationSection { target: None, threshold: None, mode: Some("post".to_owned()) })
        }
        "verify" => Some(EscalationSection {
            target: None,
            threshold: None,
            mode: Some("verify".to_owned()),
        }),
        _ => None,
    };
    Ok(RawConfig {
        lane: Some(LaneSection { name: lane.name.to_owned() }),
        engine: None,
        rungs: rung_tail(lane.name),
        model,
        ladder: ladder_section(lane.name),
        escalation,
        http: None,
    })
}

fn ladder_section(name: &str) -> Option<LadderSection> {
    match name {
        "local-2b" | "local-4b" | "local-9b" | "verify" => {
            Some(LadderSection { profile: "ladders/default.json".into() })
        }
        _ => None,
    }
}

/// The rung tail each lane implies; `local-*` and `verify` append the
/// model rung to the deterministic tail.
fn rung_tail(lane: &str) -> Option<RungsSection> {
    let order: &[&str] = match lane {
        "zero" | "delegate" | "ci" => {
            &["rule", "cache", "metadata_filter", "lexical", "relational"]
        }
        "guard" => &["rule", "cache", "metadata_filter", "lexical", "relational", "embedding"],
        "local-2b" | "local-4b" | "local-9b" | "verify" => {
            &["rule", "cache", "metadata_filter", "lexical", "relational", "embedding", "model"]
        }
        _ => return None,
    };
    Some(RungsSection { order: order.iter().map(ToString::to_string).collect() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn every_table_row_expands() {
        for lane in LANE_TABLE {
            let config = expand(lane.name).unwrap();
            assert_eq!(config.lane.as_ref().unwrap().name, lane.name);
        }
    }

    #[test]
    fn unknown_lane_is_typed() {
        let error = expand("turbo").unwrap_err();
        assert_eq!(error.code(), "lane.unknown");
        assert!(error.to_string().contains("turbo"));
    }

    #[test]
    fn zero_lane_is_artifact_free_and_deterministic() {
        let lane = preset("zero").unwrap();
        assert_eq!(lane.expected_artifacts, &[] as &[&str]);
        assert_eq!(lane.accuracy_standing_in, Some(0.683));
        let config = expand("zero").unwrap();
        assert!(config.model.is_none());
        let rungs = config.rungs.unwrap();
        assert!(!rungs.order.iter().any(|rung| rung == "model"));
    }

    #[test]
    fn unmeasured_lanes_do_not_claim_numbers() {
        for name in ["guard", "verify"] {
            assert!(preset(name).unwrap().accuracy_standing_in.is_none());
        }
    }

    #[test]
    fn model_lanes_name_their_artifacts() {
        for name in ["local-2b", "local-4b", "local-9b"] {
            let lane = preset(name).unwrap();
            assert!(!lane.expected_artifacts.is_empty(), "{name}");
            assert!(expand(name).unwrap().model.is_some(), "{name}");
        }
    }
}
