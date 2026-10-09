//! Overflow-safe exponential backoff shared by Prax's retry loops:
//! [`RetryConfig::delay_for_attempt`](super::RetryConfig::delay_for_attempt)
//! and the reconnect loop of `prax_postgres::PgListener`.
//!
//! Not a stable API: public only so driver crates can share it.

use std::hash::BuildHasher;
use std::time::Duration;

/// `min(initial * multiplier^attempt, max)`.
///
/// Never panics, whatever the inputs: a product that overflows (or a NaN
/// one) is capped at `max`, and a negative one counts as zero. Products
/// that are whole numbers of nanoseconds below 2^53 (for example any
/// millisecond delay doubled a few dozen times) are exact.
pub fn capped_exponential(
    initial: Duration,
    multiplier: f64,
    attempt: u32,
    max: Duration,
) -> Duration {
    let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
    let nanos = initial.as_nanos() as f64 * multiplier.powi(exponent);
    if nanos.is_nan() || nanos >= max.as_nanos() as f64 {
        return max;
    }
    if nanos <= 0.0 {
        return Duration::ZERO;
    }
    // Below `max` (up to rounding of the comparison above), so at most
    // `Duration::MAX` in nanoseconds; the casts saturate rather than wrap.
    let nanos = nanos as u128;
    let Ok(secs) = u64::try_from(nanos / 1_000_000_000) else {
        return max;
    };
    let subsec = (nanos % 1_000_000_000) as u32;
    Duration::new(secs, subsec).min(max)
}

/// `delay` scaled by `factor`, which is clamped to `[0, 1]` (NaN counts as
/// 1), so the result never exceeds `delay`. Never panics.
pub fn scale(delay: Duration, factor: f64) -> Duration {
    let factor = if factor.is_nan() {
        1.0
    } else {
        factor.clamp(0.0, 1.0)
    };
    // Not `mul_f64`: rounding can push a delay near `Duration::MAX` past it,
    // which panics there.
    Duration::try_from_secs_f64(delay.as_secs_f64() * factor).map_or(delay, |d| d.min(delay))
}

/// A random fraction in `[0, 1)` for jitter, taken from std's randomly keyed
/// hasher (every `RandomState` gets fresh keys), so no `rand` dependency is
/// needed. Not suitable for anything security-related.
pub fn unit_jitter() -> f64 {
    let bits = std::collections::hash_map::RandomState::new().hash_one(());
    // The top 53 bits as a fraction in [0, 1).
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_exponential_grows_by_multiplier_up_to_max() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        assert_eq!(capped_exponential(initial, 2.0, 0, max), initial);
        assert_eq!(
            capped_exponential(initial, 2.0, 3, max),
            Duration::from_millis(800)
        );
        assert_eq!(
            capped_exponential(initial, 1.5, 2, max),
            Duration::from_millis(225)
        );
        assert_eq!(capped_exponential(initial, 2.0, 20, max), max);
    }

    #[test]
    fn capped_exponential_never_panics() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        // Overflowing exponents and products are capped.
        assert_eq!(capped_exponential(initial, 2.0, u32::MAX, max), max);
        assert_eq!(
            capped_exponential(Duration::from_secs(1), 2.0, 70, Duration::MAX),
            Duration::MAX
        );
        assert_eq!(
            capped_exponential(Duration::MAX, 1.0, 0, Duration::MAX),
            Duration::MAX
        );
        assert_eq!(
            capped_exponential(Duration::MAX, 2.0, 1, Duration::MAX),
            Duration::MAX
        );
        // Degenerate multipliers.
        assert_eq!(capped_exponential(initial, f64::NAN, 1, max), max);
        assert_eq!(capped_exponential(initial, f64::INFINITY, 1, max), max);
        assert_eq!(capped_exponential(initial, -2.0, 1, max), Duration::ZERO);
        assert_eq!(capped_exponential(initial, 0.0, 1, max), Duration::ZERO);
        // `multiplier^0` is 1 whatever the multiplier (NaN included).
        assert_eq!(capped_exponential(initial, f64::NAN, 0, max), initial);
    }

    #[test]
    fn scale_clamps_the_factor() {
        let delay = Duration::from_millis(800);
        assert_eq!(scale(delay, 0.5), Duration::from_millis(400));
        assert_eq!(scale(delay, 1.0), delay);
        assert_eq!(scale(delay, 7.0), delay);
        assert_eq!(scale(delay, f64::NAN), delay);
        assert_eq!(scale(delay, -1.0), Duration::ZERO);
        assert_eq!(scale(Duration::MAX, 1.0), Duration::MAX);
    }

    #[test]
    fn unit_jitter_is_in_range_and_varies() {
        let samples: Vec<f64> = (0..256).map(|_| unit_jitter()).collect();
        assert!(
            samples.iter().all(|j| (0.0..1.0).contains(j)),
            "{samples:?}"
        );
        assert!(samples.iter().any(|j| *j != samples[0]), "{samples:?}");
    }
}
