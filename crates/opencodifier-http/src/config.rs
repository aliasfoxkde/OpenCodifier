//! Bind policy for the HTTP surface (PLANNING.md §73, SECURITY.md).
//!
//! The runtime is local-first: `opencodifier serve` binds `127.0.0.1` and
//! never an external interface unless the operator says so explicitly.
//! That posture is enforced here, at construction, rather than by
//! convention at the call site — a non-loopback bind address is a typed
//! error unless [`ServerConfig::allow_remote`] is set.

use std::net::SocketAddr;

use crate::error::HttpError;

/// Where the server listens, and how widely it is allowed to.
///
/// Loopback addresses (including `127.0.0.1:0`, the ephemeral port tests
/// and tools bind before discovering the real port) are always accepted.
/// Any other address — `0.0.0.0`, a LAN address, `::` — requires
/// `allow_remote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerConfig {
    /// The address to bind.
    pub bind: SocketAddr,
    /// Whether non-loopback bind addresses are permitted. Defaults to
    /// `false`: the server must not expose the decision runtime to the
    /// network by accident.
    pub allow_remote: bool,
}

impl ServerConfig {
    /// Validates and builds a loopback-only configuration.
    ///
    /// # Errors
    ///
    /// [`HttpError::RemoteBindForbidden`] when `bind` is not a loopback
    /// address. Use [`ServerConfig::allow_remote`] to opt into a wider
    /// bind deliberately.
    pub fn new(bind: SocketAddr) -> Result<Self, HttpError> {
        Self::with_remote(bind, false)
    }

    /// Validates and builds a configuration with an explicit remote-bind
    /// decision.
    ///
    /// # Errors
    ///
    /// [`HttpError::RemoteBindForbidden`] when `bind` is not a loopback
    /// address and `allow_remote` is `false`.
    pub fn with_remote(bind: SocketAddr, allow_remote: bool) -> Result<Self, HttpError> {
        if !bind.ip().is_loopback() && !allow_remote {
            return Err(HttpError::RemoteBindForbidden(bind));
        }
        Ok(Self { bind, allow_remote })
    }

    /// Sets whether non-loopback bind addresses are permitted, consuming
    /// and returning `self`.
    ///
    /// This deliberately does not re-validate: the remote decision is the
    /// operator's, made once, and `allow_remote` is meaningless for a
    /// loopback bind.
    #[must_use]
    pub fn allow_remote(mut self, allow_remote: bool) -> Self {
        self.allow_remote = allow_remote;
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn loopback_binds_are_always_accepted() {
        let ipv4: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        assert_eq!(ServerConfig::new(ipv4).unwrap().bind, ipv4);

        // The ephemeral port is still loopback: tests and tools bind `:0`
        // and then discover the real port.
        let ephemeral: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert!(ServerConfig::new(ephemeral).is_ok());

        let ipv6: SocketAddr = "[::1]:0".parse().unwrap();
        assert!(ServerConfig::new(ipv6).is_ok());
    }

    #[test]
    fn remote_bind_requires_the_explicit_opt_in() {
        let addr: SocketAddr = "0.0.0.0:8177".parse().unwrap();
        let error = ServerConfig::new(addr).unwrap_err();
        assert_eq!(error.code(), "http.remote_bind_forbidden");
        assert!(matches!(error, HttpError::RemoteBindForbidden(_)));

        let lan: SocketAddr = "192.168.1.10:8080".parse().unwrap();
        assert_eq!(ServerConfig::new(lan).unwrap_err().code(), "http.remote_bind_forbidden");

        // The explicit opt-in accepts it.
        let remote = ServerConfig::with_remote(addr, true).unwrap();
        assert!(remote.allow_remote);
        assert_eq!(remote.bind, addr);
    }

    #[test]
    fn builder_sets_the_remote_flag_without_revalidating() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let config = ServerConfig::new(addr).unwrap().allow_remote(true);
        assert!(config.allow_remote);
        assert_eq!(config.bind, addr);
    }
}
