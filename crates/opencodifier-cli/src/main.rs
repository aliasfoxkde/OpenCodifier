//! `opencodifier` — the decision runtime's command line.
//!
//! The binary is a thin shell: argument parsing lives in [`args`], each
//! subcommand in a module of its own, and every operation runs through
//! [`opencodifier_engine::EngineHandle`], the same facade the HTTP and MCP
//! surfaces use, so no interface can drift into its own pipeline.
//!
//! # Exit codes
//!
//! The contract scripts and CI rely on, also printed by `--help`:
//!
//! * `0` — success, including an abstention when `--abstain-is-success` is
//!   passed.
//! * `1` — input error: unreadable input, malformed JSON, a schema decode
//!   failure, a rejected graph or model manifest, bad arguments, or a
//!   non-loopback `serve` bind without `--allow-remote`.
//! * `2` — policy-gate escalation: the runtime answered, but not decisively
//!   (`abstain`, `escalate`, `verify`, `no_valid_candidate`) and
//!   `--abstain-is-success` was not passed. The response is still printed.
//! * `3` — internal error: the decision runtime itself failed.
//!
//! [`args`]: crate::args

mod args;
mod decide;
mod error;
mod graph;
mod input;
mod models;
mod output;
mod report;
mod serve;

use std::process::ExitCode;

use clap::Parser as _;

use crate::args::Cli;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return error::report_clap_error(&error),
    };
    match cli.command.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => error.report(),
    }
}
