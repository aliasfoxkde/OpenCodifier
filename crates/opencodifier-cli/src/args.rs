//! The clap command surface: `decide`, `graph validate`, `serve`,
//! `models verify`, and `recipe list`/`install`.
//!
//! Parsing is the only thing this module does; every subcommand hands off
//! to a module of its own, and every module drives the runtime through
//! [`opencodifier_engine::EngineHandle`]. Exit-code semantics are
//! documented in the binary's `--help` ([`LONG_ABOUT`]) and in
//! [`crate::error`].

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use opencodifier_schema::{
    BoxedWireFormat, anthropic::Anthropic, jev::Jev, native::Native, openai::OpenAi,
};

use crate::error::CliError;

/// The long `--help` text, which carries the exit-code contract.
const LONG_ABOUT: &str = "\
The OpenCodifier decision runtime's command line.

Local-first and deterministic-first: every subcommand is synchronous,
fully offline, and decides through the built-in zero-ML stack — the
relational solver (exact proofs over extracted facts) over the lexical
classifier. The base binary is useful with no model files anywhere on
the machine. Responses are
always printed in the canonical native format, the IR's own JSON projection.

EXIT CODES
  0  success — a decision was produced. An abstention also exits 0 when
     --abstain-is-success is passed.
  1  input error — unreadable input, malformed JSON, a schema decode
     failure, a rejected graph or model manifest, bad arguments, or a
     non-loopback `serve` bind without --allow-remote.
  2  policy-gate escalation — the runtime answered, but not decisively
     (abstain, escalate, verify, no_valid_candidate) and
     --abstain-is-success was not passed. The response is still printed.
  3  internal error — the decision runtime itself failed.

Diagnostics are written to stderr as `opencodifier: <code> <message>`, where
<code> is stable and machine-readable (`schema.*`, `graph.*`, `engine.*`,
`ir.*`, `model.*`, `cli.*`). Input is treated as hostile: no payload can
alter policy, thresholds, graph structure, or paths.
";

/// The `opencodifier` command line interface.
#[derive(Debug, clap::Parser)]
#[command(
    name = "opencodifier",
    version,
    about = "Decide, validate, serve, and verify — the OpenCodifier decision runtime",
    long_about = LONG_ABOUT
)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Everything the binary can do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Decide one request and print the canonical response.
    Decide(DecideArgs),
    /// Decision graph operations.
    Graph(GraphCommand),
    /// Serve the runtime over HTTP on the loopback interface.
    Serve(ServeArgs),
    /// Model Context Protocol operations.
    Mcp(McpArgs),
    /// Model artifact operations.
    Models(ModelsCommand),
    /// Built-in recipe fleet operations.
    Recipe(RecipeCommand),
}

impl Command {
    /// Runs the selected subcommand.
    ///
    /// # Errors
    ///
    /// Whatever the subcommand reports; the [`CliError`] carries the exit
    /// code.
    pub fn run(self) -> Result<(), CliError> {
        match self {
            Self::Decide(args) => crate::decide::run(&args),
            Self::Graph(graph) => crate::graph::run(&graph.command),
            Self::Serve(args) => crate::serve::run(&args),
            Self::Mcp(args) => crate::mcp::run(&args),
            Self::Models(models) => crate::models::run(&models.command),
            Self::Recipe(recipe) => crate::recipes::run(&recipe.command),
        }
    }
}

/// Arguments of `opencodifier decide`.
#[derive(Debug, Args)]
pub struct DecideArgs {
    /// Request payload: a file path, or `-` to read stdin.
    #[arg(short = 'i', long, default_value = "-", value_name = "PATH")]
    pub input: String,

    /// Wire format the payload is written in. The response is always
    /// printed in the canonical native format.
    #[arg(long, value_enum, default_value_t = WireFormatName::Native)]
    pub format: WireFormatName,

    /// JSON `DecisionPolicy` overriding the request's own policy.
    #[arg(long, value_name = "PATH")]
    pub policy: Option<PathBuf>,

    /// Print the execution report as a second JSON document, after the
    /// response.
    #[arg(long)]
    pub trace: bool,

    /// Decide each question on a focused view of at most this many
    /// estimated tokens when the state exceeds it, escalating to the full
    /// state when the focused decision is weak. Without the flag the full
    /// state is always decided on.
    #[arg(long, value_name = "TOKENS")]
    pub focus_budget: Option<usize>,

    /// Ladder profile JSON (`ladders/README.md` in the repository):
    /// per-node-kind confidence gates and per-rung calibration artifacts
    /// the engine consults at its confidence gate instead of the
    /// request's own policy. An empty default ladder runs
    /// byte-identically to no ladder.
    #[arg(long, value_name = "PATH")]
    pub ladder: Option<PathBuf>,

    /// Exit 0 when the outcome is not decisive, instead of 2.
    #[arg(long)]
    pub abstain_is_success: bool,
}

/// `opencodifier graph …`
#[derive(Debug, Args)]
pub struct GraphCommand {
    /// The graph operation to run.
    #[command(subcommand)]
    pub command: GraphSubcommand,
}

/// Graph operations.
#[derive(Debug, Subcommand)]
pub enum GraphSubcommand {
    /// Validate a graph document against the DAG contract and print a
    /// one-line verdict.
    Validate {
        /// Path of the graph JSON document.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
}

/// Arguments of `opencodifier serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address and port to bind. Anything but a loopback address also
    /// requires `--allow-remote`.
    #[arg(long, default_value = "127.0.0.1:8177", value_name = "ADDR")]
    pub bind: String,

    /// Deliberately allow a non-loopback bind address.
    #[arg(long)]
    pub allow_remote: bool,

    /// Graph JSON document replacing the built-in default pipeline.
    #[arg(long, value_name = "PATH")]
    pub graph: Option<PathBuf>,

    /// Decide each question on a focused view of at most this many
    /// estimated tokens when the state exceeds it, escalating to the full
    /// state when the focused decision is weak. Without the flag the full
    /// state is always decided on.
    #[arg(long, value_name = "TOKENS")]
    pub focus_budget: Option<usize>,

    /// Ladder profile JSON (`ladders/README.md` in the repository):
    /// per-node-kind confidence gates and per-rung calibration artifacts
    /// the engine consults at its confidence gate instead of each
    /// request's own policy. A non-empty profile decorates the cache
    /// identity with `|ladder-v1@<id>`.
    #[arg(long, value_name = "PATH")]
    pub ladder: Option<PathBuf>,

    /// JSON `DecisionPolicy`. Accepted and validated for symmetry with
    /// `decide`, but policy is per-request in the canonical IR, so no
    /// engine-level override exists: this flag does not change how serving
    /// decides, and the CLI says so on stderr.
    #[arg(long, value_name = "PATH")]
    pub policy: Option<PathBuf>,
}

/// Arguments of `opencodifier mcp`.
#[derive(Debug, Args)]
pub struct McpArgs {
    /// The MCP operation to run.
    #[command(subcommand)]
    pub command: McpSubcommand,
}

/// MCP operations.
#[derive(Debug, Subcommand)]
pub enum McpSubcommand {
    /// Serve the decision tools on stdin/stdout — the only transport the
    /// local-first posture allows.
    Serve {
        /// Graph JSON document replacing the built-in default pipeline.
        #[arg(long, value_name = "PATH")]
        graph: Option<PathBuf>,

        /// Decide each question on a focused view of at most this many
        /// estimated tokens when the state exceeds it, escalating to the
        /// full state when the focused decision is weak. Without the flag
        /// the full state is always decided on.
        #[arg(long, value_name = "TOKENS")]
        focus_budget: Option<usize>,

        /// Ladder profile JSON (`ladders/README.md` in the repository):
        /// per-node-kind confidence gates and per-rung calibration
        /// artifacts the engine consults at its confidence gate instead
        /// of the request's own policy.
        #[arg(long, value_name = "PATH")]
        ladder: Option<PathBuf>,
    },
}

/// `opencodifier models …`
#[derive(Debug, Args)]
pub struct ModelsCommand {
    /// The model operation to run.
    #[command(subcommand)]
    pub command: ModelsSubcommand,
}

/// Model operations.
#[derive(Debug, Subcommand)]
pub enum ModelsSubcommand {
    /// Verify that a model artifact's SHA-256 digest matches its manifest.
    Verify {
        /// Path of the manifest JSON document.
        #[arg(long, value_name = "PATH")]
        manifest: PathBuf,

        /// Path of the artifact. Defaults to the artifact the manifest is
        /// named after: `decision.onnx.manifest.json` verifies
        /// `decision.onnx`.
        #[arg(long, value_name = "PATH")]
        artifact: Option<PathBuf>,
    },
}

/// `opencodifier recipe …`
#[derive(Debug, Args)]
pub struct RecipeCommand {
    /// The recipe operation to run.
    #[command(subcommand)]
    pub command: RecipeSubcommand,
}

/// Recipe operations over the built-in fleet (PLANNING.md §34).
#[derive(Debug, Subcommand)]
pub enum RecipeSubcommand {
    /// List the recipes this binary ships.
    List,
    /// Write a built-in recipe's graph, request, and captured expected
    /// response into a directory of its own, ready for `serve --graph`.
    Install {
        /// Name of the recipe to install (`recipe list` prints the names).
        #[arg(value_name = "NAME")]
        name: String,

        /// Directory to install into. Defaults to `./recipes` —
        /// project-local, next to the working tree.
        #[arg(long, value_name = "DIR")]
        dest: Option<PathBuf>,

        /// Overwrite an existing installation of this recipe.
        #[arg(long)]
        force: bool,
    },
}

/// The wire format a request payload is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WireFormatName {
    /// The canonical IR's own JSON projection (the default).
    #[value(name = "native")]
    Native,
    /// `OpenAI` structured outputs.
    #[value(name = "openai")]
    OpenAi,
    /// Anthropic tool schemas.
    #[value(name = "anthropic")]
    Anthropic,
    /// Jev / System One requests.
    #[value(name = "jev")]
    Jev,
}

impl WireFormatName {
    /// The adapter that normalizes this format into the canonical IR.
    #[must_use]
    pub fn wire_format(self) -> BoxedWireFormat {
        match self {
            Self::Native => Box::new(Native),
            Self::OpenAi => Box::new(OpenAi),
            Self::Anthropic => Box::new(Anthropic),
            Self::Jev => Box::new(Jev),
        }
    }
}
