//! Error type for constructing and validating decision IR values.

/// Everything that can go wrong while building canonical IR values.
///
/// All variants are validation errors: the IR types refuse to exist in an
/// invalid state, so engine code can rely on their invariants (non-empty
/// candidate sets, normalized distributions, ordered policies) without
/// re-checking them.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CoreError {
    /// A required text field was empty (question text, level label, ...).
    #[error("required field `{field}` must not be empty")]
    EmptyField {
        /// Name of the offending field.
        field: &'static str,
    },

    /// A choice question was constructed with no candidates.
    #[error("choice question `{question}` must have at least one candidate")]
    EmptyCandidates {
        /// Id of the offending question.
        question: String,
    },

    /// The same candidate id appeared twice in one question.
    #[error("duplicate candidate id `{id}`")]
    DuplicateCandidate {
        /// The repeated candidate id.
        id: String,
    },

    /// A score question was constructed with fewer than two levels.
    #[error("score question `{question}` needs at least two ordered levels, got {count}")]
    TooFewLevels {
        /// Id of the offending question.
        question: String,
        /// Number of levels supplied.
        count: usize,
    },

    /// The same score level label appeared twice in one question.
    #[error("duplicate score level `{label}`")]
    DuplicateLevel {
        /// The repeated level label.
        label: String,
    },

    /// A decision policy violated its ordering or range constraints.
    #[error("invalid decision policy: {reason}")]
    InvalidPolicy {
        /// Human-readable explanation of the violated constraint.
        reason: String,
    },

    /// An identifier contained characters outside the allowed set
    /// (`a-z A-Z 0-9 _ - .`), was empty, or exceeded the length limit.
    #[error("invalid identifier `{value}`: {reason}")]
    InvalidId {
        /// The rejected identifier.
        value: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A probability or confidence value was outside `[0.0, 1.0]` or not
    /// finite (NaN / infinity are never valid probabilities).
    #[error("probability/confidence out of range: {value}")]
    InvalidProbability {
        /// The rejected value.
        value: f64,
    },

    /// A distribution's probabilities did not sum to 1 within tolerance.
    #[error("distribution sums to {sum}, expected 1.0 (tolerance {tolerance})")]
    DistributionNotNormalized {
        /// The actual sum.
        sum: f64,
        /// The allowed deviation from 1.0.
        tolerance: f64,
    },

    /// A request carried no questions; deciding nothing is not a decision.
    #[error("request must contain at least one question")]
    EmptyQuestions,
}

impl CoreError {
    /// Convenience constructor for [`CoreError::InvalidPolicy`].
    pub fn policy(reason: impl Into<String>) -> Self {
        Self::InvalidPolicy { reason: reason.into() }
    }

    /// Stable machine-readable code for this error, suitable for mapping
    /// onto HTTP statuses and MCP error payloads.
    ///
    /// Codes are part of the public contract: new codes may appear, but
    /// existing strings never change meaning.
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyField { .. } => "ir.empty_field",
            Self::EmptyCandidates { .. } => "ir.empty_candidates",
            Self::DuplicateCandidate { .. } => "ir.duplicate_candidate",
            Self::TooFewLevels { .. } => "ir.too_few_levels",
            Self::DuplicateLevel { .. } => "ir.duplicate_level",
            Self::InvalidPolicy { .. } => "ir.invalid_policy",
            Self::InvalidId { .. } => "ir.invalid_id",
            Self::InvalidProbability { .. } => "ir.invalid_probability",
            Self::DistributionNotNormalized { .. } => "ir.distribution_not_normalized",
            Self::EmptyQuestions => "ir.empty_questions",
        }
    }
}

/// Alias used throughout the crate for fallible constructors.
pub type CoreResult<T> = Result<T, CoreError>;

/// Tolerance used when checking that a distribution sums to 1.
///
/// Float probabilities are stored as `f64`; a handful of additions per
/// decision keeps accumulated error far below this bound on every
/// platform IEEE-754 target we support.
pub(crate) const SUM_TOLERANCE: f64 = 1e-6;

/// Returns `true` when `value` is a finite number in `[0.0, 1.0]`.
pub(crate) fn is_unit_interval(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
