//! Time and cancellation primitives for the synchronous executor
//! (PLANNING.md §10).
//!
//! The engine is deliberately synchronous: no async runtime, no Tokio, no
//! re-entrancy. Time is therefore the only external input an execution
//! needs, and it is abstracted behind [`Clock`] so tests can advance time
//! deterministically instead of sleeping.
//!
//! # Semantics
//!
//! * [`Clock::now`] returns a `std::time::Instant` — a monotonic reading.
//!   Deadlines are immune to system-clock adjustments, cannot be shared
//!   between processes, and are never serialized: a deadline is
//!   request-scoped state, not data.
//! * [`Deadline`] is a pre-computed instant; the executor asks "has this
//!   expired *now*?" rather than sleeping until a timer fires.
//! * [`CancellationToken`] is a single `AtomicBool`. It is **checked at
//!   node boundaries only**: once a node has started, it runs to
//!   completion. A running node is *not* preemptible — a node that ignores
//!   its inputs cannot be interrupted mid-flight, only observed to have
//!   overrun afterwards. This is the price of a synchronous executor and
//!   it is why every node keeps its work bounded by construction
//!   (PLANNING.md §67).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Source of monotonic time.
///
/// Implemented by [`SystemClock`] in production and by [`ManualClock`] in
/// tests. `Send + Sync` so a single clock can be shared by every worker
/// thread of an execution.
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The current monotonic instant.
    fn now(&self) -> Instant;
}

/// The real monotonic clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that only moves when a test advances it.
///
/// `now()` is `start + offset`; [`ManualClock::advance`] moves `offset`
/// forward. Reads are serialized through a mutex, so a manual clock is
/// safe to share with worker threads.
#[derive(Debug)]
pub struct ManualClock {
    start: Instant,
    offset: Mutex<Duration>,
}

impl ManualClock {
    /// Anchors the manual clock at the current instant with zero offset.
    #[must_use]
    pub fn new() -> Self {
        Self { start: Instant::now(), offset: Mutex::new(Duration::ZERO) }
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut offset = self.lock();
        *offset += by;
    }

    /// Total time this clock claims has elapsed since construction.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        *self.lock()
    }

    /// Locks the offset, recovering cleanly from a poisoned mutex: a
    /// panicked reader cannot be allowed to wedge every later reader.
    fn lock(&self) -> std::sync::MutexGuard<'_, Duration> {
        match self.offset.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.start + self.elapsed()
    }
}

/// A request-scoped execution deadline.
///
/// Constructed once from a clock plus a timeout, then queried cheaply at
/// node boundaries.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    at: Instant,
    limit: Duration,
}

impl Deadline {
    /// Computes the deadline `timeout` from now, according to `clock`.
    #[must_use]
    pub fn after(clock: &dyn Clock, timeout: Duration) -> Self {
        Self { at: clock.now() + timeout, limit: timeout }
    }

    /// `true` once the deadline has passed.
    #[must_use]
    pub fn expired(&self, clock: &dyn Clock) -> bool {
        clock.now() >= self.at
    }

    /// Time left before the deadline; zero once it has passed.
    #[must_use]
    pub fn remaining(&self, clock: &dyn Clock) -> Duration {
        self.at.saturating_duration_since(clock.now())
    }

    /// The instant this deadline fires.
    #[must_use]
    pub fn instant(&self) -> Instant {
        self.at
    }

    /// The configured limit this deadline was built from.
    #[must_use]
    pub fn limit(&self) -> Duration {
        self.limit
    }
}

/// Cooperative cancellation flag, checked at node boundaries only.
///
/// Cancelling does not interrupt a node that is already running: it stops
/// the executor from *starting* further nodes. See the module docs for why
/// a synchronous executor accepts this.
#[derive(Debug, Default)]
pub struct CancellationToken {
    cancelled: AtomicBool,
}

impl CancellationToken {
    /// A token nobody has cancelled yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Idempotent and thread-safe.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// `true` once [`CancellationToken::cancel`] has been called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn system_clock_moves_forward() {
        let clock = SystemClock;
        let first = clock.now();
        std::thread::sleep(Duration::from_millis(1));
        assert!(clock.now() >= first);
    }

    #[test]
    fn manual_clock_only_moves_when_advanced() {
        let clock = ManualClock::new();
        let first = clock.now();
        assert_eq!(clock.elapsed(), Duration::ZERO);
        clock.advance(Duration::from_millis(25));
        assert_eq!(clock.elapsed(), Duration::from_millis(25));
        assert!(clock.now() > first);
    }

    #[test]
    fn manual_clock_is_sharable_across_threads() {
        let clock = std::sync::Arc::new(ManualClock::new());
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let clock = std::sync::Arc::clone(&clock);
                std::thread::spawn(move || clock.advance(Duration::from_millis(10)))
            })
            .collect();
        for handle in handles {
            handle.join().expect("join");
        }
        assert_eq!(clock.elapsed(), Duration::from_millis(40));
    }

    #[test]
    fn default_manual_clock_matches_a_fresh_one() {
        assert_eq!(ManualClock::default().elapsed(), ManualClock::new().elapsed());
    }

    #[test]
    fn deadline_reports_expiry_and_remaining() {
        let clock = ManualClock::new();
        let deadline = Deadline::after(&clock, Duration::from_millis(100));
        assert!(!deadline.expired(&clock));
        assert_eq!(deadline.remaining(&clock), Duration::from_millis(100));
        assert_eq!(deadline.limit(), Duration::from_millis(100));

        clock.advance(Duration::from_millis(101));
        assert!(deadline.expired(&clock));
        assert_eq!(deadline.remaining(&clock), Duration::ZERO);
    }

    #[test]
    fn a_deadline_exposes_the_instant_it_fires_at() {
        let clock = ManualClock::new();
        let deadline = Deadline::after(&clock, Duration::from_millis(250));
        assert_eq!(deadline.instant(), clock.now() + Duration::from_millis(250));
        assert_eq!(deadline.limit(), Duration::from_millis(250));
        clock.advance(Duration::from_millis(250));
        assert_eq!(deadline.instant(), clock.now(), "the deadline sits exactly at expiry");
    }

    #[test]
    fn cancellation_is_idempotent_and_visible_across_threads() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        token.cancel();
        let handle = token.is_cancelled();
        assert!(handle);
    }

    #[test]
    fn types_are_debug_and_send_sync() {
        fn assert_bounds<T: std::fmt::Debug + Send + Sync>() {}
        assert_bounds::<SystemClock>();
        assert_bounds::<ManualClock>();
        assert_bounds::<Deadline>();
        assert_bounds::<CancellationToken>();
        assert_bounds::<std::sync::Arc<dyn Clock>>();
    }
}
