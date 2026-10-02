//! Shared assembly of the engine configuration the CLI's interfaces run
//! on.
//!
//! `decide`, `serve`, and `mcp serve` all decide through one shape of
//! runtime — the built-in default pipeline or a `--graph` replacement,
//! plus the optional focused-extraction policy and the optional
//! `--ladder` escalation profile — so the three interfaces cannot drift
//! apart on what a decision is.

use std::path::Path;

use opencodifier_engine::{EngineConfig, FocusPolicy, LadderProfile};

use crate::error::{CODE_INVALID_LADDER, CliError};
use crate::graph;

/// The engine configuration behind the CLI's deciding surfaces: the
/// `--graph` replacement or the built-in default pipeline, plus the
/// `--focus-budget` extraction policy and the `--ladder` escalation
/// profile when one was requested.
///
/// # Errors
///
/// [`CliError::input`] for a rejected `--graph` document, an unreadable
/// or malformed `--ladder` profile, or a profile the engine's own
/// validation refuses.
pub(crate) fn engine_config(
    graph_path: Option<&Path>,
    focus_budget: Option<usize>,
    ladder_path: Option<&Path>,
) -> Result<EngineConfig, CliError> {
    let config = match graph_path {
        Some(path) => EngineConfig::new(graph::load(path)?),
        None => EngineConfig::with_default_pipeline()?,
    };
    let config = config.with_focus(focus_budget.map(FocusPolicy::new));
    let Some(ladder_path) = ladder_path else {
        return Ok(config);
    };
    let document = std::fs::read_to_string(ladder_path).map_err(|error| {
        CliError::input(CODE_INVALID_LADDER, format!("{}: {error}", ladder_path.display()))
    })?;
    let profile: LadderProfile = serde_json::from_str(&document).map_err(|error| {
        CliError::input(CODE_INVALID_LADDER, format!("{}: {error}", ladder_path.display()))
    })?;
    let ladder = profile
        .into_ladder()
        .map_err(|error| CliError::input(CODE_INVALID_LADDER, error.to_string()))?;
    Ok(config.with_ladder(ladder))
}
