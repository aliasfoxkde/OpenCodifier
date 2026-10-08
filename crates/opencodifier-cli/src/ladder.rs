//! `opencodifier ladder fit-boolean`: the offline fitter of RESEARCH
//! §15.6 item 2 (never hard-0.5 a Boolean).
//!
//! Reads labeled Boolean evidence, fits the verdict boundary by grid
//! sweep (F1 on the true class, ties toward the smallest threshold), and
//! writes the [`LadderProfile`] document the engine loads with `--ladder`.
//! The fit is data, not a constant: what ships is a document the operator
//! can inspect, diff, and version, and loading it runs every validation
//! the D25 seam enforces.

use std::path::Path;

use opencodifier_core::DecisionPolicy;
use opencodifier_engine::{BooleanEvidence, fit_boolean_threshold};
use serde::Deserialize;

use crate::args::LadderSubcommand;
use crate::error::{
    CODE_INVALID_JSON, CODE_INVALID_LADDER, CODE_LADDER_EMPTY_EVIDENCE, CODE_LADDER_ID_REFUSED,
    CODE_UNWRITABLE_OUTPUT, CliError,
};
use crate::{input, output};

/// One evidence row on the wire.
#[derive(Debug, Deserialize)]
struct EvidenceRow {
    /// The model's probability for the `true` verdict.
    p_true: f64,
    /// Whether `true` was the correct verdict.
    label: bool,
}

/// Runs `ladder`.
///
/// # Errors
///
/// [`CliError::input`] when the evidence cannot be read or parsed, is
/// empty, or the profile id is refused, or the output file cannot be
/// written.
pub(crate) fn run(command: &LadderSubcommand) -> Result<(), CliError> {
    let LadderSubcommand::FitBoolean { evidence, id, out, base } = command;
    fit_boolean(evidence, id, out, base.as_deref())
}

/// Fits the boundary and writes the emitted profile.
fn fit_boolean(
    evidence_path: &Path,
    id: &str,
    out_path: &Path,
    base_path: Option<&Path>,
) -> Result<(), CliError> {
    if id.is_empty() || id == "none" {
        return Err(CliError::input(
            CODE_LADDER_ID_REFUSED,
            "the profile id is empty or \"none\"; the id decorates every cache key (D25)",
        ));
    }
    let base = match base_path {
        Some(path) => input::read_policy(path)?,
        None => DecisionPolicy::default(),
    };
    let evidence = read_evidence(evidence_path)?;
    let Some(fit) = fit_boolean_threshold(&evidence) else {
        return Err(CliError::input(
            CODE_LADDER_EMPTY_EVIDENCE,
            format!("{}: no evidence rows to fit", evidence_path.display()),
        ));
    };
    let document = opencodifier_engine::emit_boolean_profile(id, &fit, &base)
        .map_err(|error| CliError::input(CODE_INVALID_LADDER, error.to_string()))?;
    std::fs::write(out_path, document).map_err(|error| {
        CliError::input(CODE_UNWRITABLE_OUTPUT, format!("{}: {error}", out_path.display()))
    })?;
    // Four-decimal rendering — a report, not a measurement channel: the
    // profile document carries the exact fitted value.
    output::print_line(&format!(
        "ok: fitted boolean threshold {} (F1 {:.4}, argmax {:.4}, {} rows) -> {}",
        fit.threshold,
        fit.f1,
        fit.f1_at_argmax,
        fit.samples,
        out_path.display()
    ))
}

/// Reads JSONL evidence, skipping blank lines and naming the offending
/// line number. `p_true` must be a probability — a row outside `[0, 1]`
/// would silently poison the sweep.
fn read_evidence(path: &Path) -> Result<Vec<BooleanEvidence>, CliError> {
    let bytes = input::read_path(path)?;
    let text = String::from_utf8(bytes).map_err(|error| {
        CliError::input(CODE_INVALID_JSON, format!("{}: {error}", path.display()))
    })?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: EvidenceRow = serde_json::from_str(line).map_err(|error| {
            CliError::input(
                CODE_INVALID_JSON,
                format!("{} line {}: {error}", path.display(), index + 1),
            )
        })?;
        if !(0.0..=1.0).contains(&row.p_true) {
            return Err(CliError::input(
                CODE_INVALID_JSON,
                format!(
                    "{} line {}: p_true {} is not a probability",
                    path.display(),
                    index + 1,
                    row.p_true
                ),
            ));
        }
        rows.push(BooleanEvidence { p_true: row.p_true, label: row.label });
    }
    Ok(rows)
}
