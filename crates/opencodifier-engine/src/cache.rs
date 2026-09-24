//! Exact-decision cache (PLANNING.md §44, §64).
//!
//! ONE normative key construction site lives here: [`CacheKeyBuilder`].
//! Nothing else in the workspace may hash a decision request, because a
//! second construction site is how stale decisions get served after a
//! policy or graph change.
//!
//! # Key layout
//!
//! ```text
//! SHA-256(
//!   len64(request_canonical_bytes) || request_canonical_bytes
//!   || len64(graph_version_le64)
//!   || len64(model_id)
//!   || len64(calibration_version_le64)
//!   || len64(engine_semver)
//! )
//! ```
//!
//! Every field is length-prefixed with a little-endian `u64` so the byte
//! stream is unambiguous — no delimiter injection, no concatenation
//! collisions between a model id and a semver string.
//!
//! `request_canonical_bytes` is the canonical JSON of the request **with
//! choice candidates sorted by id**, which is what makes the key
//! independent of candidate order (PLANNING.md §44 "candidate order
//! normalization"). Facts are already sorted by the IR (`State` stores a
//! `BTreeMap`), so no further canonicalization is needed here.
//!
//! The cache itself is a hand-written in-memory LRU with TTL. Entries hold
//! the whole [`DecisionResponse`]; a hit still re-labels the metrics and
//! trace as a cache hit, which the engine does in
//! [`crate::engine::DecisionEngine`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencodifier_core::{ChoiceQuestion, DecisionQuestion, DecisionRequest, DecisionResponse};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::clock::Clock;
use crate::error::{EngineError, EngineResult};

/// The identity artifacts a cached decision depends on (PLANNING.md §64).
///
/// Any change here invalidates every cached decision, because all four
/// fields are folded into the cache key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineIdentity {
    /// Version of the executed decision graph.
    pub graph_version: u64,
    /// Identifier of the deciding model (e.g. `builtin-lexical-v1`). A
    /// classifier swap must bump this, not just the code version.
    pub model_id: String,
    /// Version of the calibration mapping applied to raw probabilities.
    pub calibration_version: u64,
    /// Engine semver. Bumped automatically from the crate version.
    pub engine_semver: String,
}

impl EngineIdentity {
    /// Identity used until a calibration layer exists: the built-in
    /// lexical classifier, identity calibration, this crate's semver.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            graph_version: 1,
            model_id: "builtin-lexical-v1".to_owned(),
            calibration_version: 0,
            engine_semver: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

impl Default for EngineIdentity {
    fn default() -> Self {
        Self::builtin()
    }
}

/// A 256-bit exact-decision cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheKey([u8; 32]);

impl CacheKey {
    /// The raw key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hexadecimal form, as used in traces.
    #[must_use]
    pub fn as_hex(&self) -> String {
        let mut hex = String::with_capacity(64);
        for byte in &self.0 {
            hex.push(HEX[(byte >> 4) as usize]);
            hex.push(HEX[(byte & 0x0f) as usize]);
        }
        hex
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_hex())
    }
}

impl Serialize for CacheKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_hex())
    }
}

impl<'de> Deserialize<'de> for CacheKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let hex = String::deserialize(deserializer)?;
        let bytes = decode_hex(&hex).ok_or_else(|| {
            serde::de::Error::custom("cache key must be 64 lowercase hexadecimal characters")
        })?;
        Ok(Self(bytes))
    }
}

const HEX: [char; 16] =
    ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f'];

/// Decodes 64 hex characters into 32 bytes, or `None` on any mismatch.
fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        // Both digits are 0..=15, so the sum fits a byte exactly.
        out[index] = u8::try_from(high * 16 + low).ok()?;
    }
    Some(out)
}

/// The single place a decision cache key is ever built.
///
/// All methods are pure and side-effect free; the builder holds no state.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheKeyBuilder;

impl CacheKeyBuilder {
    /// Canonical form of `request`: identical semantics, candidate-order
    /// independent bytes.
    ///
    /// Only choice questions carry an order that is not already canonical:
    /// candidates arrive in caller order but must not change the key.
    /// Re-sorting cannot invalidate a validated question — candidate ids
    /// are already unique and non-empty — so the rebuild is infallible.
    /// The documented fallback keeps the original order if that invariant
    /// were ever violated rather than inventing a wrong-but-stable key.
    #[must_use]
    pub fn normalized_request(request: &DecisionRequest) -> DecisionRequest {
        let questions: Vec<DecisionQuestion> = request
            .questions()
            .iter()
            .map(|question| match question {
                DecisionQuestion::Choice(choice) => {
                    DecisionQuestion::Choice(Self::normalize_choice(choice))
                }
                other => other.clone(),
            })
            .collect();
        // Reconstructing from already-validated parts; the only failure
        // mode is a limit the original request also satisfied.
        DecisionRequest::new(
            request.state().clone(),
            questions,
            request.policy().clone(),
            request.metadata().clone(),
        )
        .unwrap_or_else(|_| request.clone())
    }

    /// Rebuilds one choice question with candidates sorted by id.
    fn normalize_choice(question: &ChoiceQuestion) -> ChoiceQuestion {
        let mut candidates = question.candidates().to_vec();
        candidates.sort_by(|left, right| left.id().cmp(right.id()));
        match ChoiceQuestion::new(question.id().as_str(), question.text(), candidates) {
            Ok(sorted) => sorted,
            // Unreachable for validated input (see `normalized_request`).
            Err(_) => question.clone(),
        }
    }

    /// Canonical bytes of `request`: JSON of [`CacheKeyBuilder::normalized_request`].
    ///
    /// This is the only function that turns a request into key material.
    pub fn canonical_bytes(request: &DecisionRequest) -> EngineResult<Vec<u8>> {
        let normalized = Self::normalized_request(request);
        serde_json::to_vec(&normalized)
            .map_err(|error| EngineError::Serialization { reason: error.to_string() })
    }

    /// Builds the exact-decision cache key for `request`.
    pub fn build(request: &DecisionRequest, identity: &EngineIdentity) -> EngineResult<CacheKey> {
        let canonical = Self::canonical_bytes(request)?;
        let mut hasher = Sha256::new();
        write_chunk(&mut hasher, &canonical);
        write_u64(&mut hasher, identity.graph_version);
        write_chunk(&mut hasher, identity.model_id.as_bytes());
        write_u64(&mut hasher, identity.calibration_version);
        write_chunk(&mut hasher, identity.engine_semver.as_bytes());
        Ok(CacheKey(hasher.finalize().into()))
    }
}

/// Appends `bytes` to the hash, length-prefixed with a little-endian `u64`.
#[allow(clippy::cast_possible_truncation)]
fn write_chunk(hasher: &mut Sha256, bytes: &[u8]) {
    write_u64(hasher, bytes.len() as u64);
    hasher.update(bytes);
}

/// Appends a little-endian `u64`, itself length-prefixed.
fn write_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(8_u64.to_le_bytes().as_slice());
    hasher.update(value.to_le_bytes().as_slice());
}

/// Cache sizing and expiry policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheConfig {
    /// Maximum number of retained responses. Insertion evicts the least
    /// recently used entry once this is reached.
    pub max_entries: usize,
    /// How long an entry stays fresh. Expired entries are dropped lazily,
    /// on access or on insertion.
    pub ttl: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { max_entries: 1024, ttl: Duration::from_secs(300) }
    }
}

/// A cached response plus its bookkeeping.
#[derive(Debug)]
struct Slot {
    response: Arc<DecisionResponse>,
    stored_at: Instant,
    last_used: u64,
}

/// In-memory exact-decision cache: bounded LRU with TTL.
///
/// Hand-written (~80 lines) rather than pulled from a cache crate: the
/// policy needed here is small, and the eviction order must be
/// deterministic — ties on recency break by cache key.
///
/// Thread-safety: a single mutex guards the map. The engine holds one lock
/// per probe and never calls back into the cache while holding it.
#[derive(Debug)]
pub struct DecisionCache {
    inner: Mutex<CacheInner>,
    config: CacheConfig,
    clock: Arc<dyn Clock>,
}

#[derive(Debug, Default)]
struct CacheInner {
    slots: HashMap<CacheKey, Slot>,
    ticks: u64,
}

impl DecisionCache {
    /// Builds a cache, validating the configuration.
    pub fn new(config: CacheConfig, clock: Arc<dyn Clock>) -> EngineResult<Self> {
        if config.max_entries == 0 {
            return Err(EngineError::CacheMisconfigured {
                reason: "max_entries must be at least 1".to_owned(),
            });
        }
        if config.ttl.is_zero() {
            return Err(EngineError::CacheMisconfigured {
                reason: "ttl must be greater than zero".to_owned(),
            });
        }
        Ok(Self { inner: Mutex::new(CacheInner::default()), config, clock })
    }

    /// The configured policy.
    #[must_use]
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    /// Number of retained entries, after dropping expired ones.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::CacheMisconfigured`] if the internal lock is
    /// unrecoverable.
    pub fn len(&self) -> EngineResult<usize> {
        let mut inner = Self::lock(&self.inner)?;
        Self::purge_expired(&mut inner, self.config.ttl, self.clock.now());
        Ok(inner.slots.len())
    }

    /// `true` when nothing is retained.
    ///
    /// # Errors
    ///
    /// See [`DecisionCache::len`].
    pub fn is_empty(&self) -> EngineResult<bool> {
        Ok(self.len()? == 0)
    }

    /// Drops every entry.
    ///
    /// # Errors
    ///
    /// See [`DecisionCache::len`].
    pub fn clear(&self) -> EngineResult<()> {
        let mut inner = Self::lock(&self.inner)?;
        inner.slots.clear();
        Ok(())
    }

    /// Looks a response up, refreshing its recency.
    ///
    /// Returns `None` for absent *and* expired keys; the caller cannot
    /// tell the difference, and does not need to.
    #[must_use]
    pub fn get(&self, key: &CacheKey) -> Option<Arc<DecisionResponse>> {
        // Reborrowed as a plain reference so the borrow checker can split
        // field accesses: a `MutexGuard` deref is a whole-value borrow.
        let mut guard = Self::lock(&self.inner).ok()?;
        let inner = &mut *guard;
        let now = self.clock.now();
        let expired = Self::is_expired(inner.slots.get(key), self.config.ttl, now);
        if expired {
            inner.slots.remove(key);
            return None;
        }
        let slot = inner.slots.get_mut(key)?;
        inner.ticks += 1;
        slot.last_used = inner.ticks;
        Some(Arc::clone(&slot.response))
    }

    /// Inserts a response, evicting expired and then least-recently-used
    /// entries as needed.
    pub fn insert(&self, key: CacheKey, response: DecisionResponse) {
        let Ok(mut guard) = Self::lock(&self.inner) else {
            // A poisoned cache degrades to "no caching" rather than failing
            // a decision that already succeeded.
            return;
        };
        let inner = &mut *guard;
        let now = self.clock.now();
        Self::purge_expired(inner, self.config.ttl, now);
        while inner.slots.len() >= self.config.max_entries {
            let Some(victim) = Self::least_recently_used(inner) else { break };
            inner.slots.remove(&victim);
        }
        inner.ticks += 1;
        let slot = Slot { response: Arc::new(response), stored_at: now, last_used: inner.ticks };
        inner.slots.insert(key, slot);
    }

    /// Locks the inner map, recovering from poisoning: cached data is
    /// derived state, and a panic in one thread must not break the next
    /// request.
    fn lock(inner: &Mutex<CacheInner>) -> EngineResult<std::sync::MutexGuard<'_, CacheInner>> {
        inner.lock().map_err(|poisoned| EngineError::CacheMisconfigured {
            reason: format!("cache lock poisoned: {poisoned}"),
        })
    }

    fn is_expired(slot: Option<&Slot>, ttl: Duration, now: Instant) -> bool {
        match slot {
            None => true,
            Some(slot) => now.duration_since(slot.stored_at) >= ttl,
        }
    }

    fn purge_expired(inner: &mut CacheInner, ttl: Duration, now: Instant) {
        inner.slots.retain(|_, slot| now.duration_since(slot.stored_at) < ttl);
    }

    /// The key of the least recently used slot; recency ties break by key
    /// so eviction is reproducible.
    fn least_recently_used(inner: &CacheInner) -> Option<CacheKey> {
        inner.slots.iter().min_by_key(|(key, slot)| (slot.last_used, *key)).map(|(key, _)| *key)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::clock::{ManualClock, SystemClock};
    use opencodifier_core::{
        BooleanQuestion, Candidate, ConfidenceReport, DecisionAnswer, DecisionMetrics,
        DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest, DecisionTrace,
        RequestMetadata, ScoreLevel, ScoreQuestion, State,
    };

    fn request(candidate_order: &[&str]) -> DecisionRequest {
        let candidates: Vec<Candidate> = candidate_order
            .iter()
            .map(|id| Candidate::new(*id, "general coding and reasoning").expect("valid"))
            .collect();
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new("model", "Which model?", candidates).expect("valid"),
        );
        DecisionRequest::new(
            State::from_text("refactor the parser"),
            vec![question],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .expect("valid")
    }

    fn response() -> DecisionResponse {
        let distribution =
            opencodifier_core::Distribution::from_pairs([("a", 0.9), ("b", 0.1)]).expect("valid");
        let report =
            ConfidenceReport::from_distribution(&distribution, 0.9, 0.0, None).expect("valid");
        DecisionResponse::new(
            vec![DecisionAnswer::Choice {
                question_id: opencodifier_core::QuestionId::new("model").expect("valid"),
                choice: opencodifier_core::CandidateId::new("a").expect("valid"),
                distribution,
                confidence: 0.9,
            }],
            DecisionOutcome::Accept,
            report,
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .expect("valid")
    }

    #[test]
    fn same_input_yields_same_key() {
        let identity = EngineIdentity::builtin();
        let first = CacheKeyBuilder::build(&request(&["a", "b"]), &identity).unwrap();
        let second = CacheKeyBuilder::build(&request(&["a", "b"]), &identity).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.as_hex().len(), 64);
    }

    #[test]
    fn candidate_reorder_yields_same_key() {
        let identity = EngineIdentity::builtin();
        let forward = CacheKeyBuilder::build(&request(&["a", "b", "c"]), &identity).unwrap();
        let backward = CacheKeyBuilder::build(&request(&["c", "b", "a"]), &identity).unwrap();
        assert_eq!(forward, backward, "candidate order must not change the key");
        assert_eq!(
            CacheKeyBuilder::normalized_request(&request(&["c", "b", "a"])),
            CacheKeyBuilder::normalized_request(&request(&["a", "b", "c"]))
        );
    }

    #[test]
    fn policy_change_changes_key() {
        let identity = EngineIdentity::builtin();
        let mut strict = request(&["a", "b"]);
        let strict_policy = opencodifier_core::DecisionPolicy::new(
            0.99,
            0.7,
            0.5,
            opencodifier_core::RiskLevel::Low,
        )
        .expect("valid");
        strict = DecisionRequest::new(
            strict.state().clone(),
            strict.questions().to_vec(),
            strict_policy,
            strict.metadata().clone(),
        )
        .expect("valid");
        assert_ne!(
            CacheKeyBuilder::build(&request(&["a", "b"]), &identity).unwrap(),
            CacheKeyBuilder::build(&strict, &identity).unwrap()
        );
    }

    #[test]
    fn identity_fields_change_key() {
        let request = request(&["a", "b"]);
        let base = CacheKeyBuilder::build(&request, &EngineIdentity::builtin()).unwrap();

        let mut graph = EngineIdentity::builtin();
        graph.graph_version += 1;
        let mut model = EngineIdentity::builtin();
        model.model_id = "other-model".into();
        let mut calibration = EngineIdentity::builtin();
        calibration.calibration_version += 1;
        let mut semver = EngineIdentity::builtin();
        semver.engine_semver = "9.9.9".into();

        for changed in [graph, model, calibration, semver] {
            assert_ne!(base, CacheKeyBuilder::build(&request, &changed).unwrap());
        }
    }

    #[test]
    fn state_and_question_text_change_key() {
        let identity = EngineIdentity::builtin();
        let base = CacheKeyBuilder::build(&request(&["a"]), &identity).unwrap();
        let other = DecisionRequest::new(
            State::from_text("summarize this research paper"),
            vec![DecisionQuestion::Choice(
                ChoiceQuestion::new(
                    "model",
                    "Which model?",
                    vec![Candidate::new("a", "general coding and reasoning").expect("valid")],
                )
                .expect("valid"),
            )],
            opencodifier_core::DecisionPolicy::default(),
            opencodifier_core::RequestMetadata::default(),
        )
        .unwrap();
        assert_ne!(base, CacheKeyBuilder::build(&other, &identity).unwrap());
    }

    #[test]
    fn canonical_bytes_are_stable_and_prefix_free() {
        let request = request(&["b", "a"]);
        let first = CacheKeyBuilder::canonical_bytes(&request).unwrap();
        let second = CacheKeyBuilder::canonical_bytes(&request).unwrap();
        assert_eq!(first, second);
        let text = String::from_utf8(first).unwrap();
        assert!(text.contains(r#""a""#));
        assert!(text.find(r#""a""#) < text.find(r#""b""#), "candidates must be sorted");
    }

    #[test]
    fn cache_key_round_trips_through_hex_json() {
        let key = CacheKeyBuilder::build(&request(&["a"]), &EngineIdentity::builtin()).unwrap();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, format!("\"{key}\""));
        let back: CacheKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, key);
        assert!(serde_json::from_str::<CacheKey>("\"zz\"").is_err());
        assert!(serde_json::from_str::<CacheKey>("\"abcd\"").is_err());
    }

    #[test]
    fn cache_rejects_zero_capacity_and_zero_ttl() {
        let clock = std::sync::Arc::new(SystemClock);
        assert!(matches!(
            DecisionCache::new(CacheConfig { max_entries: 0, ttl: Duration::from_secs(1) }, clock),
            Err(EngineError::CacheMisconfigured { .. })
        ));
        let clock = std::sync::Arc::new(SystemClock);
        assert!(matches!(
            DecisionCache::new(CacheConfig { max_entries: 1, ttl: Duration::ZERO }, clock),
            Err(EngineError::CacheMisconfigured { .. })
        ));
    }

    #[test]
    fn cache_hit_returns_stored_response() {
        let clock = std::sync::Arc::new(ManualClock::new());
        let cache = DecisionCache::new(CacheConfig::default(), clock.clone()).unwrap();
        let key = CacheKeyBuilder::build(&request(&["a"]), &EngineIdentity::builtin()).unwrap();
        assert!(cache.get(&key).is_none());
        cache.insert(key, response());
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.len().unwrap(), 1);
        assert!(!cache.is_empty().unwrap());
    }

    #[test]
    fn cache_expires_after_ttl() {
        let clock = std::sync::Arc::new(ManualClock::new());
        let cache = DecisionCache::new(
            CacheConfig { max_entries: 4, ttl: Duration::from_secs(10) },
            clock.clone(),
        )
        .unwrap();
        let key = CacheKeyBuilder::build(&request(&["a"]), &EngineIdentity::builtin()).unwrap();
        cache.insert(key, response());
        clock.advance(Duration::from_secs(9));
        assert!(cache.get(&key).is_some(), "fresh entries must survive");
        clock.advance(Duration::from_secs(2));
        assert!(cache.get(&key).is_none(), "entries must expire after the ttl");
        assert_eq!(cache.len().unwrap(), 0);
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let clock = std::sync::Arc::new(ManualClock::new());
        let cache = DecisionCache::new(
            CacheConfig { max_entries: 2, ttl: Duration::from_secs(60) },
            clock.clone(),
        )
        .unwrap();
        let identity = EngineIdentity::builtin();
        let first = CacheKeyBuilder::build(&request(&["a"]), &identity).unwrap();
        let second = CacheKeyBuilder::build(&request(&["b"]), &identity).unwrap();
        let third = CacheKeyBuilder::build(&request(&["c"]), &identity).unwrap();

        cache.insert(first, response());
        cache.insert(second, response());
        // Touch `first` so `second` becomes the least recently used.
        assert!(cache.get(&first).is_some());
        cache.insert(third, response());

        assert!(cache.get(&first).is_some(), "recently used entry must survive");
        assert!(cache.get(&third).is_some(), "newest entry must survive");
        assert!(cache.get(&second).is_none(), "least recently used entry must be evicted");
    }

    #[test]
    fn cache_clear_drops_everything() {
        let clock = std::sync::Arc::new(SystemClock);
        let cache = DecisionCache::new(CacheConfig::default(), clock.clone()).unwrap();
        cache.insert(
            CacheKeyBuilder::build(&request(&["a"]), &EngineIdentity::builtin()).unwrap(),
            response(),
        );
        cache.clear().unwrap();
        assert!(cache.is_empty().unwrap());
    }

    #[test]
    fn the_reported_policy_is_the_configured_one() {
        let clock = std::sync::Arc::new(ManualClock::new());
        let policy = CacheConfig { max_entries: 7, ttl: Duration::from_secs(123) };
        let cache = DecisionCache::new(policy, clock).unwrap();
        assert_eq!(cache.config(), &policy);
    }

    #[test]
    fn the_raw_key_encodes_to_the_hex_form() {
        let key = CacheKeyBuilder::build(&request(&["a"]), &EngineIdentity::builtin()).unwrap();
        let bytes = key.as_bytes();
        assert_eq!(bytes.len(), 32);
        let mut hex = String::with_capacity(64);
        for byte in *bytes {
            hex.push(HEX[usize::from(byte >> 4)]);
            hex.push(HEX[usize::from(byte & 0x0f)]);
        }
        assert_eq!(hex, key.as_hex());
    }

    #[test]
    fn normalization_leaves_non_choice_questions_alone() {
        // A boolean and a score question have no candidate order to
        // canonicalize, so normalization passes them through unchanged and
        // the key is unaffected.
        let identity = EngineIdentity::builtin();
        let mixed = DecisionRequest::new(
            State::from_text("refactor the parser"),
            vec![
                DecisionQuestion::Boolean(
                    BooleanQuestion::new("tools", "Are tools needed?").expect("valid"),
                ),
                DecisionQuestion::Score(
                    ScoreQuestion::new(
                        "difficulty",
                        "How hard?",
                        vec![
                            ScoreLevel::new("low").expect("valid"),
                            ScoreLevel::new("high").expect("valid"),
                        ],
                    )
                    .expect("valid"),
                ),
            ],
            opencodifier_core::DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .expect("valid");

        assert_eq!(
            CacheKeyBuilder::normalized_request(&mixed),
            CacheKeyBuilder::normalized_request(&CacheKeyBuilder::normalized_request(&mixed))
        );
        assert_eq!(
            CacheKeyBuilder::normalized_request(&mixed),
            mixed,
            "normalization is the identity for non-choice questions"
        );
        assert_eq!(
            CacheKeyBuilder::build(&mixed, &identity).unwrap(),
            CacheKeyBuilder::build(&CacheKeyBuilder::normalized_request(&mixed), &identity)
                .unwrap()
        );
    }

    #[test]
    fn an_insert_drops_entries_that_expired_before_it() {
        // Expiry is checked on the way in as well as the way out: the stale
        // entry below is removed by the insert that follows it, so the
        // capacity bound is never spent on dead entries.
        let clock = std::sync::Arc::new(ManualClock::new());
        let cache = DecisionCache::new(
            CacheConfig { max_entries: 1, ttl: Duration::from_secs(10) },
            clock.clone(),
        )
        .unwrap();
        let identity = EngineIdentity::builtin();
        let stale = CacheKeyBuilder::build(&request(&["a"]), &identity).unwrap();
        cache.insert(stale, response());

        clock.advance(Duration::from_secs(11));
        let fresh = CacheKeyBuilder::build(&request(&["b"]), &identity).unwrap();
        cache.insert(fresh, response());

        assert_eq!(cache.len().unwrap(), 1, "the stale entry must not survive the insert");
        assert!(cache.get(&stale).is_none());
        assert!(cache.get(&fresh).is_some());
    }
}
