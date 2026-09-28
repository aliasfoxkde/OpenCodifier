//! Shared assembly of the engine configuration the CLI's interfaces run
//! on.
//!
//! `decide`, `serve`, and `mcp serve` all decide through one shape of
//! runtime — the built-in default pipeline or a `--graph` replacement,
//! plus the optional focused-extraction policy — so the three interfaces
//! cannot drift apart on what a decision is.

use std::path::Path;

use opencodifier_engine::{EngineConfig, FocusPolicy};

use crate::error::CliError;
use crate::graph;

/// The engine configuration behind the CLI's deciding surfaces: the
/// `--graph` replacement or the built-in default pipeline, plus the
/// `--focus-budget` extraction policy when one was requested.
///
/// # Errors
///
/// [`CliError::input`] for a rejected `--graph` document.
pub(crate) fn engine_config(
    graph_path: Option<&Path>,
    focus_budget: Option<usize>,
) -> Result<EngineConfig, CliError> {
    let config = match graph_path {
        Some(path) => EngineConfig::new(graph::load(path)?),
        None => EngineConfig::with_default_pipeline()?,
    };
    Ok(config.with_focus(focus_budget.map(FocusPolicy::new)))
}
