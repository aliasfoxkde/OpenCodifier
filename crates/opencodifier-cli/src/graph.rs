//! `opencodifier graph validate`: the DAG contract applied to a document.
//!
//! A graph that arrives from anywhere — a file, a request body, a
//! pipeline definition — is validated before it is trusted, and the stable
//! `graph.*` code is what gets reported, never the prose.

use std::path::Path;

use opencodifier_engine::{DecisionGraph, EngineHandle, NodeSpec};
use serde::Deserialize;

use crate::args::GraphSubcommand;
use crate::error::{CODE_INVALID_GRAPH_JSON, CliError};
use crate::{input, output};

/// The graph wire shape: a version plus a node array.
///
/// Deserialized by hand rather than straight into [`DecisionGraph`] because
/// the engine's `try_from` shim flattens its typed error into a serde
/// message, which would lose the stable `graph.*` code this CLI has to
/// report. [`DecisionGraph::new`] is the same validation path, and
/// [`EngineHandle::validate_graph`] re-checks the graph it is handed, so
/// validation is never a weaker local copy.
#[derive(Debug, Deserialize)]
struct GraphDocument {
    /// Graph version, folded into every decision cache key.
    version: u64,
    /// The declared nodes.
    nodes: Vec<NodeSpec>,
}

/// Runs `graph`.
///
/// # Errors
///
/// [`CliError::input`] for an unreadable document, a document without the
/// graph wire shape, or a graph the DAG contract rejects.
pub(crate) fn run(command: &GraphSubcommand) -> Result<(), CliError> {
    match command {
        GraphSubcommand::Validate { path } => validate(path),
    }
}

/// Validates one document and prints a one-line verdict.
fn validate(path: &Path) -> Result<(), CliError> {
    let graph = load(path)?;
    EngineHandle::validate_graph(&graph)
        .map_err(|error| CliError::input(error.code(), error.to_string()))?;
    let waves = graph.waves().map_or(0, |waves| waves.len());
    output::print_line(&format!(
        "ok: graph `{}` valid (version {}, {} nodes, {} waves)",
        path.display(),
        graph.version(),
        graph.nodes().len(),
        waves
    ))
}

/// Loads and validates a graph document: the one path shared by `graph
/// validate` and `serve --graph`.
///
/// # Errors
///
/// [`CliError::input`] with the engine's own `graph.*` code when the
/// document is rejected.
pub(crate) fn load(path: &Path) -> Result<DecisionGraph, CliError> {
    let document: GraphDocument =
        serde_json::from_value(input::read_json(path)?).map_err(|error| {
            CliError::input(CODE_INVALID_GRAPH_JSON, format!("{}: {error}", path.display()))
        })?;
    DecisionGraph::new(document.version, document.nodes)
        .map_err(|error| CliError::input(error.code(), format!("{}: {error}", path.display())))
}
