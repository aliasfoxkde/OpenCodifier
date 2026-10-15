//! Shared assembly of the engine configuration and handle the CLI's
//! interfaces run on.
//!
//! `decide`, `serve`, and `mcp serve` all decide through one shape of
//! runtime — the built-in default pipeline or a `--graph` replacement,
//! plus the optional focused-extraction policy, the optional `--ladder`
//! escalation profile, and the optional `--llama` model rung — so the
//! three interfaces cannot drift apart on what a decision is.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use opencodifier_engine::{
    Classifier, EngineConfig, EngineHandle, FocusPolicy, LadderPolicy, LadderProfile,
    RelationalSolver, Rung,
};

use crate::args::LlamaRungArgs;
use crate::error::{CODE_INVALID_LADDER, CODE_MODEL_RUNG_UNAVAILABLE, CliError};
use crate::graph;

/// The engine configuration behind the CLI's deciding surfaces: the
/// `--graph` replacement or the built-in default pipeline, plus the
/// `--focus-budget` extraction policy and the `--ladder` escalation
/// profile when one was requested.
///
/// Returns the ladder alongside the config because a `--llama` model
/// rung assembles through [`EngineHandle::with_rungs`], which applies
/// the ladder itself — the same ladder, applied once, either way.
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
) -> Result<(EngineConfig, LadderPolicy), CliError> {
    let config = match graph_path {
        Some(path) => EngineConfig::new(graph::load(path)?),
        None => EngineConfig::with_default_pipeline()?,
    };
    let config = config.with_focus(focus_budget.map(FocusPolicy::new));
    let Some(ladder_path) = ladder_path else {
        return Ok((config, LadderPolicy::new("none")));
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
    Ok((config.with_ladder(ladder.clone()), ladder))
}

/// The `--llama` decision rung as parsed: where the server lives, the
/// cache-key identity of the served model, and the per-request timeout.
///
/// The fields are read only by the `llamacpp`-feature builder below; a
/// default build refuses the rung before construction matters.
#[cfg_attr(not(feature = "llamacpp"), allow(dead_code))]
#[derive(Debug, Clone)]
pub(crate) struct ModelRung {
    url: String,
    model_id: String,
    timeout: Duration,
}

/// Extracts the model-rung configuration from parsed arguments.
///
/// `--llama` and `--llama-model-id` require each other at the clap
/// layer; the defensive check here keeps the invariant local so the
/// struct cannot be constructed incoherently even by a future caller
/// that bypasses parsing.
///
/// # Errors
///
/// [`CliError::input`] with [`CODE_MODEL_RUNG_UNAVAILABLE`] when the
/// flags arrive without their required partner.
pub(crate) fn model_rung(args: &LlamaRungArgs) -> Result<Option<ModelRung>, CliError> {
    let Some(url) = args.llama.clone() else {
        return Ok(None);
    };
    let Some(model_id) = args.llama_model_id.clone() else {
        return Err(CliError::input(
            CODE_MODEL_RUNG_UNAVAILABLE,
            "--llama requires --llama-model-id: the model id is the decision cache key's \
             discriminator for the served model (D6)",
        ));
    };
    let timeout =
        Duration::from_millis(args.llama_timeout_ms.unwrap_or(llama_default_timeout_ms()));
    Ok(Some(ModelRung { url, model_id, timeout }))
}

/// The default per-request server timeout, in milliseconds, mirroring
/// [`opencodifier_model::DEFAULT_TIMEOUT`].
#[must_use]
fn llama_default_timeout_ms() -> u64 {
    opencodifier_model::DEFAULT_TIMEOUT.as_millis().try_into().unwrap_or(u64::MAX)
}

/// The engine handle behind the CLI's deciding surfaces: the lexical
/// stack as the primary rung, and — when `--llama` was given — the
/// llama.cpp decision rung as the ordered escalation tail (D26/D27).
/// The model is fired only by the gate, on a non-accepting outcome,
/// never on a question the primary rungs already decided.
///
/// # Errors
///
/// [`CliError::input`] as in [`engine_config`], plus
/// [`CODE_MODEL_RUNG_UNAVAILABLE`] when `--llama` was given to a binary
/// built without the `llamacpp` feature; [`CliError::engine`] when the
/// runtime cannot be assembled.
pub(crate) fn handle(
    graph_path: Option<&Path>,
    focus_budget: Option<usize>,
    ladder_path: Option<&Path>,
    model_rung: Option<ModelRung>,
) -> Result<EngineHandle, CliError> {
    let (config, ladder) = engine_config(graph_path, focus_budget, ladder_path)?;
    let Some(rung) = model_rung else {
        return Ok(EngineHandle::lexical(config)?);
    };
    let classifier = model_classifier(&rung)?;
    Ok(EngineHandle::with_rungs(
        config,
        Arc::new(RelationalSolver::lexical()),
        None,
        ladder,
        vec![Rung::new(classifier)],
    )?)
}

/// Builds the llama.cpp decision rung behind the `llamacpp` feature.
///
/// Assembly is offline: the loopback transport is validated for shape
/// (absolute `http://` URL) but nothing connects until a decision needs
/// the rung — a server that is down fails the requests that escalate to
/// it, honestly, not the process start.
///
/// # Errors
///
/// [`CliError::input`] with [`CODE_MODEL_RUNG_UNAVAILABLE`] on a build
/// without the feature; [`CliError::engine`] when the base URL is not
/// an absolute `http://` URL.
fn model_classifier(rung: &ModelRung) -> Result<Arc<dyn Classifier>, CliError> {
    #[cfg(feature = "llamacpp")]
    {
        let config =
            opencodifier_model::LlamaConfig::new(rung.model_id.clone()).with_timeout(rung.timeout);
        let classifier =
            opencodifier_model::LlamaDecisionClassifier::loopback(rung.url.clone(), config)?;
        Ok(Arc::new(classifier))
    }
    #[cfg(not(feature = "llamacpp"))]
    {
        let _ = rung;
        Err(CliError::input(
            CODE_MODEL_RUNG_UNAVAILABLE,
            "the --llama decision rung requires a build with `--features llamacpp` \
             (it adds the ureq dependency; the default build stays dependency-identical)",
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    fn llama_args(
        url: Option<&str>,
        model_id: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> LlamaRungArgs {
        LlamaRungArgs {
            llama: url.map(str::to_owned),
            llama_model_id: model_id.map(str::to_owned),
            llama_timeout_ms: timeout_ms,
        }
    }

    #[test]
    fn no_llama_flags_mean_no_model_rung() {
        assert!(model_rung(&llama_args(None, None, None)).unwrap().is_none());
    }

    #[test]
    fn llama_flags_extract_with_the_measured_default_timeout() {
        let rung =
            model_rung(&llama_args(Some("http://127.0.0.1:8080"), Some("pd|2b|tree-v2"), None))
                .unwrap()
                .expect("flags were set");
        assert_eq!(rung.url, "http://127.0.0.1:8080");
        assert_eq!(rung.model_id, "pd|2b|tree-v2");
        assert_eq!(rung.timeout, opencodifier_model::DEFAULT_TIMEOUT);
    }

    #[test]
    fn an_explicit_timeout_overrides_the_default() {
        let rung = model_rung(&llama_args(Some("http://127.0.0.1:8080"), Some("m"), Some(600_000)))
            .unwrap()
            .expect("flags were set");
        assert_eq!(rung.timeout, Duration::from_secs(600));
    }

    #[test]
    fn a_url_without_a_model_id_is_refused_defensively() {
        let error = model_rung(&llama_args(Some("http://127.0.0.1:8080"), None, None)).unwrap_err();
        assert_eq!(error.code(), CODE_MODEL_RUNG_UNAVAILABLE, "{error}");
    }

    #[cfg(not(feature = "llamacpp"))]
    #[test]
    fn a_default_build_refuses_the_model_rung() {
        let rung = model_rung(&llama_args(Some("http://127.0.0.1:8080"), Some("m"), None))
            .unwrap()
            .expect("flags were set");
        let error = handle(None, None, None, Some(rung)).unwrap_err();
        assert_eq!(error.code(), CODE_MODEL_RUNG_UNAVAILABLE, "{error}");
    }

    #[cfg(feature = "llamacpp")]
    #[test]
    fn the_model_rung_assembles_offline_and_rekeys_cache_identity() {
        let rung = model_rung(&llama_args(
            Some("http://127.0.0.1:8080"),
            Some("pd-fork|2b|tree-v2"),
            None,
        ))
        .unwrap()
        .expect("flags were set");
        let handle = handle(None, None, None, Some(rung)).unwrap();
        let model_id = handle.identity().model_id;
        assert!(model_id.contains("rungs-v1@"), "{model_id}");
        assert!(model_id.contains("pd-fork|2b|tree-v2"), "{model_id}");
    }

    #[cfg(feature = "llamacpp")]
    #[test]
    fn without_the_rung_the_handle_is_the_plain_lexical_posture() {
        let handle = handle(None, None, None, None).unwrap();
        assert!(!handle.identity().model_id.contains("rungs-v1"));
    }
}
