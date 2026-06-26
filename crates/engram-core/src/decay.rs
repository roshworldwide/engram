//! Confidence decay as a first-class storage primitive (R4).
//!
//! Decay is **lazy**: it is evaluated only when a belief is read, never by a
//! background task (P7 — zero background CPU). [`DecayFunction::eval`] is
//! allocation-free and a handful of float ops, targeting < 500 ns per belief.

use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

const NANOS_PER_SEC: f32 = 1_000_000_000.0;

/// How a belief's confidence changes as time elapses since its reference time.
///
/// All curves are non-increasing in elapsed time and the result is clamped to
/// `[0, c0]` so confidence never rises above its initial value.
///
/// ```
/// use engram_core::DecayFunction;
/// let f = DecayFunction::Exponential { lambda: 0.0 };
/// assert_eq!(f.eval(0.9, 1_000_000_000), 0.9); // lambda 0 ⇒ no decay
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum DecayFunction {
    /// `c(t) = c0 · e^(−λt)`, with `t` in seconds.
    Exponential {
        /// Decay rate per second.
        lambda: f32,
    },
    /// Regularized power law `c(t) = c0 · (1 + t)^(−β)`, with `t` in seconds.
    ///
    /// The `1 +` avoids the singularity at `t = 0` of the bare `t^(−β)` form
    /// while preserving the power-law tail (Anderson-style forgetting).
    PowerLaw {
        /// Decay exponent.
        beta: f32,
    },
    /// Flat at `c0` until `drop_at` elapsed, then flat at `c_low`.
    ///
    /// `drop_at` is interpreted as an **elapsed offset** (nanoseconds since the
    /// belief's reference time), matching [`eval`](DecayFunction::eval)'s
    /// `elapsed_ns` argument rather than an absolute wall-clock time.
    Step {
        /// Elapsed time at which confidence drops.
        drop_at: Timestamp,
        /// Confidence after the drop.
        c_low: f32,
    },
    /// No decay; confidence stays at `c0` forever.
    None,
}

impl DecayFunction {
    /// Evaluate confidence given the initial confidence `c0` and the time
    /// elapsed since the belief's reference time, in nanoseconds.
    ///
    /// Negative `elapsed_ns` is treated as zero. The result is clamped to
    /// `[0, max(c0, 0)]`.
    #[must_use]
    pub fn eval(&self, c0: f32, elapsed_ns: i64) -> f32 {
        let secs = (elapsed_ns.max(0) as f32) / NANOS_PER_SEC;
        let raw = match *self {
            DecayFunction::None => c0,
            DecayFunction::Exponential { lambda } => c0 * (-lambda * secs).exp(),
            DecayFunction::PowerLaw { beta } => c0 * (1.0 + secs).powf(-beta),
            DecayFunction::Step { drop_at, c_low } => {
                if elapsed_ns < drop_at.0 {
                    c0
                } else {
                    c_low
                }
            }
        };
        // Clamp without risking a panic when c0 is negative (min must be <= max).
        raw.clamp(0.0, c0.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_NS: i64 = 86_400 * 1_000_000_000;

    #[test]
    fn none_never_decays() {
        let f = DecayFunction::None;
        assert_eq!(f.eval(0.8, 0), 0.8);
        assert_eq!(f.eval(0.8, 1000 * DAY_NS), 0.8);
    }

    #[test]
    fn exponential_is_monotonic_non_increasing() {
        let f = DecayFunction::Exponential { lambda: 1e-6 };
        let mut prev = f.eval(1.0, 0);
        for d in 1..=365 {
            let cur = f.eval(1.0, d * DAY_NS);
            assert!(cur <= prev, "day {d}: {cur} > {prev}");
            prev = cur;
        }
        assert!(prev < 1.0);
    }

    #[test]
    fn power_law_is_monotonic_and_regular_at_zero() {
        let f = DecayFunction::PowerLaw { beta: 0.5 };
        assert_eq!(f.eval(1.0, 0), 1.0); // no singularity at t = 0
        let a = f.eval(1.0, DAY_NS);
        let b = f.eval(1.0, 30 * DAY_NS);
        assert!(b < a && a < 1.0);
    }

    #[test]
    fn step_drops_at_threshold() {
        let f = DecayFunction::Step {
            drop_at: Timestamp(10 * DAY_NS),
            c_low: 0.1,
        };
        assert_eq!(f.eval(0.9, 0), 0.9);
        assert_eq!(f.eval(0.9, 9 * DAY_NS), 0.9);
        assert_eq!(f.eval(0.9, 10 * DAY_NS), 0.1);
        assert_eq!(f.eval(0.9, 100 * DAY_NS), 0.1);
    }

    #[test]
    fn result_is_clamped_and_never_exceeds_c0() {
        // c_low above c0 is capped at c0.
        let f = DecayFunction::Step {
            drop_at: Timestamp(0),
            c_low: 5.0,
        };
        assert_eq!(f.eval(0.7, DAY_NS), 0.7);
        // Negative elapsed is treated as zero.
        let e = DecayFunction::Exponential { lambda: 1.0 };
        assert_eq!(e.eval(0.5, -42), 0.5);
        // Negative c0 clamps to zero rather than panicking.
        assert_eq!(e.eval(-1.0, DAY_NS), 0.0);
    }
}
