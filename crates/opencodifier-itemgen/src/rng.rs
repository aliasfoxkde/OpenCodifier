//! Seeded deterministic randomness for item generation.
//!
//! Generation must be reproducible byte-for-byte from a seed (the same
//! `--seed` and `--n` produce the identical corpus, so a training run's
//! data provenance is one number), and the workspace forbids pulling a
//! random-number dependency for this. [`Rng`] is `SplitMix64` — 15 lines,
//! well-mixed, and plenty for vocabulary and graph-shape sampling. All
//! consumers draw through this one type; nothing else in the crate
//! touches time, threads, or entropy.

/// A `SplitMix64` generator seeded from the run seed.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

/// `SplitMix64`'s golden-gamma increment.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

impl Rng {
    /// A generator seeded by `seed`. Equal seeds produce equal streams.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `0..n`. `n` must be non-zero (a zero request
    /// is a sampling bug, so it fails loudly in debug and wraps to 0 in
    /// release — callers guard).
    // The modulus bounds the result below `n`, which fits `usize`
    // wherever `n` did.
    #[allow(clippy::cast_possible_truncation)]
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0, "below(0) is a sampling bug");
        (self.next_u64() % n as u64) as usize
    }

    /// A uniform choice from `xs`. Panics only on an empty slice, which
    /// is a static vocabulary bug, not a runtime condition.
    #[must_use]
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    /// In-place Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, xs: &mut [T]) {
        for i in (1..xs.len()).rev() {
            xs.swap(i, self.below(i + 1));
        }
    }

    /// True with `percent` probability (0-100).
    pub fn percent(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn equal_seeds_produce_equal_streams() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let same = (0..8).filter(|_| a.next_u64() == b.next_u64()).count();
        assert_eq!(same, 0);
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::new(9);
        for _ in 0..1000 {
            assert!(rng.below(7) < 7);
        }
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut rng = Rng::new(5);
        let mut xs: Vec<usize> = (0..32).collect();
        rng.shuffle(&mut xs);
        let mut sorted = xs.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..32).collect::<Vec<_>>());
    }

    #[test]
    fn percent_covers_both_sides() {
        let mut rng = Rng::new(11);
        let trues = (0..1000).filter(|_| rng.percent(40)).count();
        assert!((300..500).contains(&trues), "40% coin gave {trues}/1000");
    }
}
