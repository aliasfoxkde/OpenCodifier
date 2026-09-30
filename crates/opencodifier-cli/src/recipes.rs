//! The built-in recipe fleet (PLANNING.md §34): the twelve decision areas
//! as runnable graphs, shipped inside the binary.
//!
//! `recipe list` names the fleet; `recipe install <name>` writes a
//! recipe's three files — `graph.json`, `request.json`,
//! `expected-response.json` — into a directory so `serve --graph` and
//! `decide --input` can run it immediately. The files are byte-identical
//! to the committed copies under `recipes/` (the same `include_str!`
//! sources), and the expected responses were captured from the engine,
//! never written by hand (the procedure is `recipes/README.md`).
//!
//! Installation is deliberately boring: no network, no registry, no
//! discovery beyond the compiled-in table. A recipe that claims to be
//! installed must be the exact bytes this binary was built with — the
//! overwrite guard (`cli.recipe_exists`) exists so a stale local copy can
//! never be mistaken for a fresh install without `--force` saying so.

use std::path::Path;

use crate::args::RecipeSubcommand;
use crate::error::{
    CODE_RECIPE_EXISTS, CODE_UNKNOWN_RECIPE, CODE_UNWRITABLE_DESTINATION, CliError,
};
use crate::output;

/// One shippable recipe: a graph, the request that exercises it, and the
/// response the committed engine produced for that request.
struct Recipe {
    /// The name `recipe list` prints and `recipe install` accepts.
    name: &'static str,
    /// One line: which §34 decision area the recipe decides.
    description: &'static str,
    /// The graph document (`serve --graph` input).
    graph: &'static str,
    /// The request document (`decide --input` input).
    request: &'static str,
    /// The captured response for `request` on a fresh server of `graph`.
    expected: &'static str,
}

/// The fleet: the twelve decision areas of PLANNING.md §34, one recipe
/// each. `recipe list` prints exactly this table, in this order.
const FLEET: &[Recipe] = &[
    Recipe {
        name: "model-routing",
        description: "Route a request to the model that should serve it (choice over a small local fleet).",
        graph: include_str!("../../../recipes/model-routing.json"),
        request: include_str!("../../../recipes/requests/model-routing.json"),
        expected: include_str!("../../../recipes/expected/model-routing-response.json"),
    },
    Recipe {
        name: "task-classification",
        description: "Classify a task into a workflow class (choice over the full deterministic ladder).",
        graph: include_str!("../../../recipes/task-classification.json"),
        request: include_str!("../../../recipes/requests/task-classification.json"),
        expected: include_str!("../../../recipes/expected/task-classification-response.json"),
    },
    Recipe {
        name: "tool-selection",
        description: "Pick the one tool the current step should call (choice with lexical evidence).",
        graph: include_str!("../../../recipes/tool-selection.json"),
        request: include_str!("../../../recipes/requests/tool-selection.json"),
        expected: include_str!("../../../recipes/expected/tool-selection-response.json"),
    },
    Recipe {
        name: "tool-gating",
        description: "Gate whether a tool run is allowed at all (boolean, critical-risk posture).",
        graph: include_str!("../../../recipes/tool-gating.json"),
        request: include_str!("../../../recipes/requests/tool-gating.json"),
        expected: include_str!("../../../recipes/expected/tool-gating-response.json"),
    },
    Recipe {
        name: "context-pruning",
        description: "Score how much of a long context survives pruning (score, ordered levels).",
        graph: include_str!("../../../recipes/context-pruning.json"),
        request: include_str!("../../../recipes/requests/context-pruning.json"),
        expected: include_str!("../../../recipes/expected/context-pruning-response.json"),
    },
    Recipe {
        name: "cache-eligibility",
        description: "Decide whether a request may be answered from cache (boolean after the rule node).",
        graph: include_str!("../../../recipes/cache-eligibility.json"),
        request: include_str!("../../../recipes/requests/cache-eligibility.json"),
        expected: include_str!("../../../recipes/expected/cache-eligibility-response.json"),
    },
    Recipe {
        name: "skill-selection",
        description: "Choose the repository skill that should handle the task (choice, full ladder).",
        graph: include_str!("../../../recipes/skill-selection.json"),
        request: include_str!("../../../recipes/requests/skill-selection.json"),
        expected: include_str!("../../../recipes/expected/skill-selection-response.json"),
    },
    Recipe {
        name: "memory-selection",
        description: "Pick the memory scope a fact belongs in (choice over scopes).",
        graph: include_str!("../../../recipes/memory-selection.json"),
        request: include_str!("../../../recipes/requests/memory-selection.json"),
        expected: include_str!("../../../recipes/expected/memory-selection-response.json"),
    },
    Recipe {
        name: "escalation",
        description: "Escalate to a human only on overwhelming evidence (boolean, 0.95 gate).",
        graph: include_str!("../../../recipes/escalation.json"),
        request: include_str!("../../../recipes/requests/escalation.json"),
        expected: include_str!("../../../recipes/expected/escalation-response.json"),
    },
    Recipe {
        name: "verification",
        description: "Decide whether an answer needs independent verification (boolean, cache in the ladder).",
        graph: include_str!("../../../recipes/verification.json"),
        request: include_str!("../../../recipes/requests/verification.json"),
        expected: include_str!("../../../recipes/expected/verification-response.json"),
    },
    Recipe {
        name: "document-relevance",
        description: "Score a document's relevance to the query (score, retrieval ordering).",
        graph: include_str!("../../../recipes/document-relevance.json"),
        request: include_str!("../../../recipes/requests/document-relevance.json"),
        expected: include_str!("../../../recipes/expected/document-relevance-response.json"),
    },
    Recipe {
        name: "code-review-risk",
        description: "Score the risk of a change under review (score, severity ladder).",
        graph: include_str!("../../../recipes/code-review-risk.json"),
        request: include_str!("../../../recipes/requests/code-review-risk.json"),
        expected: include_str!("../../../recipes/expected/code-review-risk-response.json"),
    },
];

/// Runs `recipe`.
///
/// # Errors
///
/// [`CliError::input`] for an unknown recipe name or an installation
/// target that cannot be written.
pub(crate) fn run(command: &RecipeSubcommand) -> Result<(), CliError> {
    match command {
        RecipeSubcommand::List => list(),
        RecipeSubcommand::Install { name, dest, force } => install(name, dest.as_deref(), *force),
    }
}

/// Prints the fleet, one recipe per line.
fn list() -> Result<(), CliError> {
    for recipe in FLEET {
        output::print_line(&format!("{:<19}{}", recipe.name, recipe.description))?;
    }
    Ok(())
}

/// Writes one recipe's three files under the destination directory.
///
/// The default destination is `recipes/` — project-local, because an
/// install that lands outside the working tree is an install nobody can
/// find again. An existing directory means a previous install (or
/// something the caller wrote) is in the way, and clobbering it silently
/// would break the whole point of a captured expected response; `--force`
/// is the explicit statement that the bytes underneath may be replaced.
fn install(name: &str, dest: Option<&Path>, force: bool) -> Result<(), CliError> {
    let recipe = FLEET.iter().find(|recipe| recipe.name == name).ok_or_else(|| {
        CliError::input(
            CODE_UNKNOWN_RECIPE,
            format!("unknown recipe `{name}` — `opencodifier recipe list` names the fleet"),
        )
    })?;
    let directory = dest.map_or_else(|| std::path::PathBuf::from("recipes"), Path::to_path_buf);
    let target = directory.join(recipe.name);
    if target.exists() && !force {
        return Err(CliError::input(
            CODE_RECIPE_EXISTS,
            format!("`{}` already exists — pass --force to overwrite it", target.display()),
        ));
    }
    let files: &[(&str, &str)] = &[
        ("graph.json", recipe.graph),
        ("request.json", recipe.request),
        ("expected-response.json", recipe.expected),
    ];
    std::fs::create_dir_all(&target).map_err(|error| {
        CliError::input(CODE_UNWRITABLE_DESTINATION, format!("{}: {error}", target.display()))
    })?;
    for (file, contents) in files {
        let path = target.join(file);
        std::fs::write(&path, contents).map_err(|error| {
            CliError::input(CODE_UNWRITABLE_DESTINATION, format!("{}: {error}", path.display()))
        })?;
    }
    output::print_line(&format!(
        "ok: installed recipe `{}` into `{}` (graph.json, request.json, expected-response.json)",
        recipe.name,
        target.display()
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_engine::{DecisionGraph, EngineHandle, NodeSpec};
    use serde::Deserialize;

    /// The graph wire shape, parsed the same way `graph validate` parses
    /// it (see [`crate::graph`]).
    #[derive(Debug, Deserialize)]
    struct FleetGraph {
        version: u64,
        nodes: Vec<NodeSpec>,
    }

    /// Every fleet graph must be a DAG the engine would serve, and every
    /// request/response document must be well-formed JSON with the fields
    /// the README's comparison procedure relies on.
    #[test]
    fn every_fleet_file_is_runnable_and_well_formed() {
        assert_eq!(FLEET.len(), 12, "one recipe per §34 decision area");
        for recipe in FLEET {
            let document: FleetGraph = serde_json::from_str(recipe.graph)
                .unwrap_or_else(|error| panic!("{} graph: {error}", recipe.name));
            let graph = DecisionGraph::new(document.version, document.nodes)
                .unwrap_or_else(|error| panic!("{}: {error}", recipe.name));
            EngineHandle::validate_graph(&graph)
                .unwrap_or_else(|error| panic!("{}: {error}", recipe.name));

            let request: serde_json::Value = serde_json::from_str(recipe.request)
                .unwrap_or_else(|error| panic!("{} request: {error}", recipe.name));
            let request = request.as_object().expect("request is an object");
            assert!(request.get("state").is_some(), "{}: state", recipe.name);
            assert_eq!(
                request.get("questions").and_then(|q| q.as_array()).map(Vec::len),
                Some(1),
                "{}: one question per recipe",
                recipe.name
            );

            let expected: serde_json::Value = serde_json::from_str(recipe.expected)
                .unwrap_or_else(|error| panic!("{} expected: {error}", recipe.name));
            let expected = expected.as_object().expect("response is an object");
            assert!(
                expected.contains_key("outcome"),
                "{}: the captured response names its outcome",
                recipe.name
            );
            assert_eq!(
                expected.get("answers").and_then(|a| a.as_array()).map(Vec::len),
                Some(1),
                "{}: one captured answer",
                recipe.name
            );
        }
    }

    /// The fleet names are exactly the twelve §34 decision areas.
    #[test]
    fn the_fleet_names_the_twelve_decision_areas() {
        let names: Vec<_> = FLEET.iter().map(|recipe| recipe.name).collect();
        assert_eq!(
            names,
            [
                "model-routing",
                "task-classification",
                "tool-selection",
                "tool-gating",
                "context-pruning",
                "cache-eligibility",
                "skill-selection",
                "memory-selection",
                "escalation",
                "verification",
                "document-relevance",
                "code-review-risk",
            ]
        );
    }

    /// Uniqueness counter for scratch directories: the install tests run
    /// in parallel and must never share a path.
    static NEXT_SCRATCH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A scratch directory for install tests, removed on drop.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "opencodifier-recipes-{}-{}",
                std::process::id(),
                NEXT_SCRATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn install(&self, name: &str, force: bool) -> Result<(), CliError> {
            let dest = self.0.clone();
            super::install(name, Some(&dest), force)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Installing writes the three files, byte-identical to the compiled-in
    /// sources, and a second install refuses without `--force`.
    #[test]
    fn install_writes_the_fleet_files_and_guards_overwrites() {
        let scratch = Scratch::new();
        scratch.install("model-routing", false).unwrap();

        let target = scratch.0.join("model-routing");
        for file in ["graph.json", "request.json", "expected-response.json"] {
            assert!(target.join(file).is_file(), "{file}");
        }
        let installed = std::fs::read_to_string(target.join("graph.json")).unwrap();
        let embedded = FLEET.iter().find(|r| r.name == "model-routing").unwrap();
        assert_eq!(installed, embedded.graph, "installed bytes are the shipped bytes");

        let error = scratch.install("model-routing", false).unwrap_err();
        assert_eq!(error.code(), CODE_RECIPE_EXISTS, "{error}");
        scratch.install("model-routing", true).unwrap();
    }

    /// An unknown name is an input error naming the list command.
    #[test]
    fn installing_an_unknown_recipe_is_an_input_error() {
        let scratch = Scratch::new();
        let error = scratch.install("does-not-exist", false).unwrap_err();
        assert_eq!(error.code(), CODE_UNKNOWN_RECIPE, "{error}");
        assert!(error.to_string().contains("recipe list"), "{error}");
    }

    /// A file sitting where the recipe directory belongs stops the
    /// install: the destination is named, and the refusal is an input
    /// error, not a panic and not a partial write.
    #[test]
    fn a_file_in_the_way_refuses_the_install() {
        let scratch = Scratch::new();
        let target = scratch.0.join("model-routing");
        std::fs::write(&target, b"not a directory").unwrap();
        let error = scratch.install("model-routing", true).unwrap_err();
        assert_eq!(error.code(), CODE_UNWRITABLE_DESTINATION, "{error}");
        assert!(error.to_string().contains("model-routing"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"not a directory", "the file is untouched");
    }

    /// An unwritable file slot stops the install after the directory is
    /// made: the first blocked write is the refusal, naming that file.
    #[test]
    fn an_unwritable_file_slot_refuses_the_install() {
        let scratch = Scratch::new();
        // A directory cannot be overwritten by `fs::write`: the first
        // recipe file's slot is blocked without touching permissions,
        // which root (CI containers) would bypass.
        let slot = scratch.0.join("model-routing").join("graph.json");
        std::fs::create_dir_all(&slot).unwrap();
        let error = scratch.install("model-routing", true).unwrap_err();
        assert_eq!(error.code(), CODE_UNWRITABLE_DESTINATION, "{error}");
        assert!(error.to_string().contains("graph.json"), "{error}");
    }
}
