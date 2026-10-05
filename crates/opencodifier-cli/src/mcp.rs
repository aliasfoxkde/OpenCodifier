//! `opencodifier mcp`: the Model Context Protocol surface over stdio.
//!
//! The tools run on the same assembled engine as `serve` —
//! [`opencodifier_engine::EngineHandle::lexical`] over the default pipeline or a `--graph`
//! replacement, plus the same optional ladder and llama.cpp model rung —
//! so the CLI, HTTP, and MCP interfaces cannot drift apart on
//! what a decision is. The transport is the process's own stdin/stdout:
//! there is no socket, and therefore nothing to bind, refuse, or expose.

use std::path::Path;
use std::sync::Arc;

use crate::args::{LlamaRungArgs, McpArgs, McpSubcommand};
use crate::error::{CODE_MCP_SESSION_FAILED, CODE_RUNTIME_FAILED, CliError};
use crate::runtime;

/// Runs `mcp`.
///
/// # Errors
///
/// [`CliError::input`] for a rejected `--graph` document, and
/// [`CliError::engine`] when the runtime or the MCP session itself fails.
pub(crate) fn run(args: &McpArgs) -> Result<(), CliError> {
    match &args.command {
        McpSubcommand::Serve { graph, focus_budget, ladder, llama } => {
            serve(graph.as_deref(), *focus_budget, ladder.as_deref(), llama)
        }
    }
}

/// Assembles the runtime and serves the tools until the client goes away.
///
/// # Errors
///
/// [`CliError::input`] for a rejected `--graph` document, and
/// [`CliError::engine`] when the runtime cannot be assembled or the MCP
/// session fails.
fn serve(
    graph_path: Option<&Path>,
    focus_budget: Option<usize>,
    ladder_path: Option<&Path>,
    llama: &LlamaRungArgs,
) -> Result<(), CliError> {
    let handle =
        runtime::handle(graph_path, focus_budget, ladder_path, runtime::model_rung(llama)?)?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| CliError::engine(CODE_RUNTIME_FAILED, error.to_string()))?;
    runtime
        .block_on(opencodifier_mcp::serve_stdio(Arc::new(handle)))
        .map_err(|error| CliError::engine(CODE_MCP_SESSION_FAILED, error.to_string()))
}
