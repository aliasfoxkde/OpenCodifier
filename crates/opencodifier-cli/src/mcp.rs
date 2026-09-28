//! `opencodifier mcp`: the Model Context Protocol surface over stdio.
//!
//! The tools run on the same assembled engine as `serve` —
//! [`EngineHandle::lexical`] over the default pipeline or a `--graph`
//! replacement — so the CLI, HTTP, and MCP interfaces cannot drift apart on
//! what a decision is. The transport is the process's own stdin/stdout:
//! there is no socket, and therefore nothing to bind, refuse, or expose.

use std::path::Path;
use std::sync::Arc;

use opencodifier_engine::{EngineConfig, EngineHandle};

use crate::args::{McpArgs, McpSubcommand};
use crate::error::{CODE_MCP_SESSION_FAILED, CODE_RUNTIME_FAILED, CliError};
use crate::graph;

/// Runs `mcp`.
///
/// # Errors
///
/// [`CliError::input`] for a rejected `--graph` document, and
/// [`CliError::engine`] when the runtime or the MCP session itself fails.
pub(crate) fn run(args: &McpArgs) -> Result<(), CliError> {
    match &args.command {
        McpSubcommand::Serve { graph } => serve(graph.as_deref()),
    }
}

/// Assembles the runtime and serves the tools until the client goes away.
///
/// # Errors
///
/// [`CliError::input`] for a rejected `--graph` document, and
/// [`CliError::engine`] when the runtime cannot be assembled or the MCP
/// session fails.
fn serve(graph_path: Option<&Path>) -> Result<(), CliError> {
    let config = match graph_path {
        Some(path) => EngineConfig::new(graph::load(path)?),
        None => EngineConfig::with_default_pipeline()?,
    };
    let handle = EngineHandle::lexical(config)?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| CliError::engine(CODE_RUNTIME_FAILED, error.to_string()))?;
    runtime
        .block_on(opencodifier_mcp::serve_stdio(Arc::new(handle)))
        .map_err(|error| CliError::engine(CODE_MCP_SESSION_FAILED, error.to_string()))
}
