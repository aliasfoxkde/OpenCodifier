//! `opencodifier feedback`: offline evidence append for the feedback loop
//! (#193).
//!
//! Writes the exact same `{p_true, label, source, digest}` JSONL row
//! `POST /v1/feedback` writes, straight to a file — no server, no
//! runtime. This is the loop's offline half: a host that labels decisions
//! in batch appends here, `serve --evidence` appends there, and
//! `ladder fit-boolean` reads both without caring which wrote what.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args;
use serde_json::json;

use crate::error::{CODE_INVALID_POLICY, CODE_UNWRITABLE_OUTPUT, CliError};

/// Arguments of `opencodifier feedback`.
#[derive(Debug, Args)]
pub struct FeedbackArgs {
    /// Evidence JSONL to append to (created when missing).
    #[arg(long, value_name = "PATH")]
    pub evidence: PathBuf,

    /// The model's probability for the `true` verdict, within [0, 1].
    #[arg(long, value_name = "P")]
    pub p_true: f64,

    /// Whether `true` was actually correct.
    #[arg(long)]
    pub label: bool,

    /// Where the label came from: `human`, `host_model`, or `outcome`.
    #[arg(long, value_name = "SOURCE")]
    pub source: String,

    /// Hex sha256 of the canonical request bytes (the join key).
    #[arg(long, value_name = "HEX")]
    pub digest: Option<String>,
}

/// Runs `feedback`, appending one row.
///
/// # Errors
///
/// [`CliError::input`] for a `p_true` outside `[0, 1]`, an unknown
/// `source`, or an unwritable evidence file.
pub(crate) fn run(args: &FeedbackArgs) -> Result<(), CliError> {
    if !(0.0..=1.0).contains(&args.p_true) || !args.p_true.is_finite() {
        return Err(CliError::input(
            CODE_INVALID_POLICY,
            format!("--p-true {} is not within [0, 1]", args.p_true),
        ));
    }
    if !matches!(args.source.as_str(), "human" | "host_model" | "outcome") {
        return Err(CliError::input(
            CODE_INVALID_POLICY,
            format!("--source `{}` is not one of: human, host_model, outcome", args.source),
        ));
    }
    let row = json!({
        "p_true": args.p_true,
        "label": args.label,
        "source": args.source,
        "digest": args.digest,
    });
    let mut file =
        std::fs::OpenOptions::new().create(true).append(true).open(&args.evidence).map_err(
            |error| {
                CliError::input(
                    CODE_UNWRITABLE_OUTPUT,
                    format!("cannot open {}: {error}", args.evidence.display()),
                )
            },
        )?;
    writeln!(file, "{row}").map_err(|error| {
        CliError::input(
            CODE_UNWRITABLE_OUTPUT,
            format!("write to {} failed: {error}", args.evidence.display()),
        )
    })?;
    file.flush().map_err(|error| {
        CliError::input(
            CODE_UNWRITABLE_OUTPUT,
            format!("flush to {} failed: {error}", args.evidence.display()),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn args(evidence: PathBuf, p_true: f64, source: &str) -> FeedbackArgs {
        FeedbackArgs {
            evidence,
            p_true,
            label: true,
            source: source.to_owned(),
            digest: Some("abc".to_owned()),
        }
    }

    #[test]
    fn appends_a_row_the_fitter_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let evidence = dir.path().join("e.jsonl");
        run(&args(evidence.clone(), 0.82, "human")).unwrap();
        run(&args(evidence.clone(), 0.31, "outcome")).unwrap();

        let rows = std::fs::read_to_string(&evidence).unwrap();
        let lines: Vec<&str> = rows.lines().collect();
        assert_eq!(lines.len(), 2);
        let row: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(row["p_true"], 0.82);
        assert_eq!(row["label"], true);
        assert_eq!(row["source"], "human");
        assert_eq!(row["digest"], "abc");
        let row: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(row["p_true"], 0.31);
        assert_eq!(row["source"], "outcome");
    }

    #[test]
    fn out_of_range_p_true_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = run(&args(dir.path().join("e.jsonl"), 1.5, "human")).unwrap_err();
        assert_eq!(error.code(), CODE_INVALID_POLICY);
    }

    #[test]
    fn non_finite_p_true_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = run(&args(dir.path().join("e.jsonl"), f64::NAN, "human")).unwrap_err();
        assert_eq!(error.code(), CODE_INVALID_POLICY);
    }

    #[test]
    fn unknown_source_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = run(&args(dir.path().join("e.jsonl"), 0.5, "vibes")).unwrap_err();
        assert_eq!(error.code(), CODE_INVALID_POLICY);
    }

    #[test]
    fn unwritable_evidence_is_refused() {
        let error =
            run(&args(std::path::PathBuf::from("/nonexistent-dir-xyz/e.jsonl"), 0.5, "human"))
                .unwrap_err();
        assert_eq!(error.code(), CODE_UNWRITABLE_OUTPUT);
    }
}
