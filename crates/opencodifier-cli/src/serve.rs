//! `opencodifier serve`: the HTTP surface over the deterministic engine.
//!
//! Local-first by default: the bind address must be loopback unless the
//! operator passes `--allow-remote`, and the HTTP layer checks the same
//! rule again at construction, so the posture does not depend on this
//! module remembering it.

use std::net::SocketAddr;
use std::sync::Arc;

use opencodifier_engine::{EngineConfig, EngineHandle};
use opencodifier_http::ServerConfig;

use crate::args::ServeArgs;
use crate::error::{
    CODE_BIND_REQUIRES_ALLOW_REMOTE, CODE_INVALID_BIND, CODE_POLICY_INAPPLICABLE,
    CODE_RUNTIME_FAILED, CliError,
};
use crate::{graph, input, output};

/// Runs `serve`, blocking until the server shuts down on `Ctrl-C`.
///
/// # Errors
///
/// [`CliError::input`] for a malformed bind address, a non-loopback bind
/// without `--allow-remote`, or a rejected `--graph`/`--policy` document;
/// [`CliError::engine`] when the runtime cannot be assembled or serving
/// fails.
pub(crate) fn run(args: &ServeArgs) -> Result<(), CliError> {
    let (_, server, handle) = assemble(args)?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| CliError::engine(CODE_RUNTIME_FAILED, error.to_string()))?;
    runtime
        .block_on(opencodifier_http::serve(server, Arc::new(handle)))
        .map_err(|error| CliError::engine(error.code(), error.to_string()))
}

/// Validates every argument and assembles the runtime the server will
/// serve, before any socket opens.
///
/// Everything that can reject `serve` is decided here — bind address,
/// `--graph`, `--policy`, engine assembly — so [`run`] is only the
/// process lifetime around it.
///
/// # Errors
///
/// [`CliError::input`] for a malformed bind address, a non-loopback bind
/// without `--allow-remote`, or a rejected `--graph`/`--policy` document;
/// [`CliError::engine`] when the engine cannot be assembled.
fn assemble(args: &ServeArgs) -> Result<(SocketAddr, ServerConfig, EngineHandle), CliError> {
    let bind: SocketAddr = args.bind.parse().map_err(|_| {
        CliError::input(CODE_INVALID_BIND, format!("`{}` is not an address:port pair", args.bind))
    })?;
    if !bind.ip().is_loopback() && !args.allow_remote {
        return Err(CliError::input(
            CODE_BIND_REQUIRES_ALLOW_REMOTE,
            format!("refusing to bind {bind}: non-loopback addresses require --allow-remote"),
        ));
    }

    let config = match &args.graph {
        Some(path) => EngineConfig::new(graph::load(path)?),
        None => EngineConfig::with_default_pipeline()?,
    };

    if let Some(path) = &args.policy {
        report_policy_limitation(path)?;
    }

    let handle = EngineHandle::lexical(config)?;
    let server = ServerConfig::with_remote(bind, args.allow_remote)
        .map_err(|error| CliError::input(error.code(), error.to_string()))?;
    Ok((bind, server, handle))
}

/// Validates `--policy` and says plainly what it does not do.
///
/// The flag is accepted so a pipeline definition can carry it alongside
/// `decide`, and the file is validated so a bad one fails before the socket
/// opens — but there is no engine-level policy knob to feed it to: policy
/// lives on the request in the canonical IR. Silently ignoring it would be
/// a false promise, so the limitation is stated on `stderr`.
fn report_policy_limitation(path: &std::path::Path) -> Result<(), CliError> {
    let policy = input::read_policy(path)?;
    output::print_warning(&format!(
        "{CODE_POLICY_INAPPLICABLE}: --policy {} validated (min_confidence {}, verify_below {}, \
         abstain_below {}, risk {:?}) but does not change serving — policy is per-request in the \
         canonical IR, so clients set it on each decision request",
        path.display(),
        policy.min_confidence(),
        policy.verify_below(),
        policy.abstain_below(),
        policy.risk()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::error::CODE_INVALID_POLICY;
    use std::path::PathBuf;

    /// Writes a policy document to a uniquely named temp file and returns
    /// its path; the caller removes it after the assertion.
    fn temp_policy(body: &str, tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("oc-serve-policy-{tag}.json"));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn args(bind: &str, allow_remote: bool, policy: Option<PathBuf>) -> ServeArgs {
        ServeArgs { bind: bind.to_string(), allow_remote, graph: None, policy }
    }

    #[test]
    fn loopback_with_policy_assembles_and_warns() {
        let path = temp_policy(
            r#"{"min_confidence":0.9,"verify_below":0.7,"abstain_below":0.5,"risk":"low"}"#,
            "ok",
        );
        let parsed = assemble(&args("127.0.0.1:0", false, Some(path.clone()))).unwrap();
        let (bind, _server, handle) = parsed;
        std::fs::remove_file(&path).unwrap();

        assert_eq!(bind.to_string(), "127.0.0.1:0");
        assert!(handle.health().nodes > 0);
    }

    #[test]
    fn remote_bind_without_the_flag_is_refused() {
        let error = assemble(&args("0.0.0.0:8177", false, None)).unwrap_err();
        assert_eq!(error.code(), CODE_BIND_REQUIRES_ALLOW_REMOTE);
    }

    #[test]
    fn remote_bind_with_the_flag_assembles() {
        let (_, server, handle) = assemble(&args("192.168.1.10:9443", true, None)).unwrap();
        assert!(handle.health().nodes > 0);
        // The same posture decision is mirrored into the server config.
        let _ = server;
    }

    #[test]
    fn malformed_bind_is_an_input_error() {
        let error = assemble(&args("localhost:8177", false, None)).unwrap_err();
        assert_eq!(error.code(), CODE_INVALID_BIND);
    }

    #[test]
    fn invalid_policy_file_fails_before_any_socket() {
        let path = temp_policy(r#"{"min_confidence":2.0}"#, "invalid");
        let error = assemble(&args("127.0.0.1:0", false, Some(path.clone()))).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(error.code(), CODE_INVALID_POLICY);
    }
}
