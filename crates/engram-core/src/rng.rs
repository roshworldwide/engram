//! Injectable randomness.
//!
//! Like [`crate::Clock`], randomness flows through a trait so ULID generation and
//! any future randomized paths are reproducible in tests. [`SplitMix64`] is a
//! tiny, dependency-free, deterministic PRNG; [`SystemRng`] seeds one from the
//! wall clock + a process counter for non-test use. Neither is cryptographic —
//! they exist to make identifiers unique, not unguessable.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of pseudo-random 64-bit words.
pub trait Rng {
    /// The next 64-bit word.
    fn next_u64(&mut self) -> u64;

    /// The next 128-bit word, assembled from two 64-bit draws.
    fn next_u128(&mut self) -> u128 {
        (u128::from(self.next_u64()) << 64) | u128::from(self.next_u64())
    }
}

/// The SplitMix64 generator — fast, deterministic, and seedable.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Create a generator from an explicit seed (deterministic).
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }
}

impl Rng for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// A non-deterministic generator for production identifier creation. Seeds a
/// [`SplitMix64`] from the wall clock XOR a monotonically increasing process
/// counter, so two `SystemRng`s created in the same nanosecond still differ.
#[derive(Clone, Debug)]
pub struct SystemRng(SplitMix64);

impl SystemRng {
    /// Create a freshly-seeded generator.
    #[must_use]
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let bump = COUNTER.fetch_add(1, Ordering::Relaxed);
        // Mix the counter in via the golden ratio so close seeds diverge fast.
        let seed = nanos ^ bump.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        SystemRng(SplitMix64::new(seed))
    }
}

impl Default for SystemRng {
    fn default() -> Self {
        Self::new()
    }
}

impl Rng for SystemRng {
    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_deterministic_for_a_seed() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn splitmix_differs_across_seeds() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn next_u128_uses_full_width() {
        // With overwhelming probability the high 64 bits are non-zero.
        let mut r = SplitMix64::new(7);
        let v = r.next_u128();
        assert_ne!(v >> 64, 0);
    }

    #[test]
    fn system_rng_instances_diverge() {
        let mut a = SystemRng::new();
        let mut b = SystemRng::new();
        assert_ne!(a.next_u64(), b.next_u64());
    }
}
