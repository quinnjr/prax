//! Reconnect backoff for the listener's supervisor.

use std::time::Duration;

use prax_query::middleware::backoff;

use super::PgListenerOptions;

/// Delay before reconnect attempt `attempt` (0-based):
/// `min(initial * 2^attempt, max)` scaled by `jitter`, which is clamped to
/// `[0.5, 1.0]` (NaN counts as 1.0). Never panics, whatever the inputs.
fn backoff_delay(attempt: u32, initial: Duration, max: Duration, jitter: f64) -> Duration {
    let capped = backoff::capped_exponential(initial, 2.0, attempt, max);
    let jitter = if jitter.is_nan() {
        1.0
    } else {
        jitter.clamp(0.5, 1.0)
    };
    backoff::scale(capped, jitter)
}

/// Reconnect backoff state kept across sessions, so that a server which
/// accepts connections and drops them again at once is retried with growing
/// delays rather than every `backoff_initial`.
#[derive(Debug, Default)]
pub(super) struct ReconnectBackoff {
    /// Attempts made since the last session that stayed up for a whole
    /// `ping_interval`.
    attempt: u32,
}

impl ReconnectBackoff {
    /// Attempts counted so far (the exponent of the next delay).
    pub(super) fn step(&self) -> u32 {
        self.attempt
    }

    /// The delay before the next attempt ([`backoff_delay`] for the current
    /// attempt count); counts the attempt.
    pub(super) fn next_delay(&mut self, options: &PgListenerOptions, jitter: f64) -> Duration {
        let delay = backoff_delay(
            self.attempt,
            options.backoff_initial,
            options.backoff_max,
            jitter,
        );
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Record the loss of a session that had been up for `uptime`. Only a
    /// session that survived one `ping_interval` resets the delay to
    /// `backoff_initial`; a shorter one keeps the count, so the next delay
    /// continues to grow.
    pub(super) fn session_lost(&mut self, uptime: Duration, ping_interval: Duration) {
        if uptime >= ping_interval {
            self.attempt = 0;
        }
    }
}

/// A jitter factor in `[0.5, 1.0)` for [`backoff_delay`].
pub(super) fn jitter() -> f64 {
    0.5 + backoff::unit_jitter() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_doubles_from_initial_up_to_max() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        assert_eq!(
            backoff_delay(0, initial, max, 1.0),
            Duration::from_millis(100)
        );
        assert_eq!(
            backoff_delay(3, initial, max, 1.0),
            Duration::from_millis(800)
        );
        assert_eq!(backoff_delay(20, initial, max, 1.0), max);
    }

    #[test]
    fn backoff_delay_scales_by_jitter() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        assert_eq!(
            backoff_delay(3, initial, max, 0.5),
            Duration::from_millis(400)
        );
    }

    #[test]
    fn backoff_delay_saturates_and_clamps_jitter() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        // `2^attempt` and `initial * 2^attempt` overflow: the cap applies.
        assert_eq!(backoff_delay(u32::MAX, initial, max, 1.0), max);
        assert_eq!(
            backoff_delay(70, Duration::from_secs(1), Duration::MAX, 1.0),
            Duration::MAX
        );
        // Jitter outside [0.5, 1.0] (or NaN) never shortens the delay below
        // half or stretches it past the computed value.
        assert_eq!(
            backoff_delay(0, initial, max, 0.0),
            Duration::from_millis(50)
        );
        assert_eq!(backoff_delay(0, initial, max, 7.0), initial);
        assert_eq!(backoff_delay(0, initial, max, f64::NAN), initial);
    }

    #[test]
    fn backoff_resets_only_after_a_session_survives_a_ping_interval() {
        let options = PgListenerOptions::default();
        let ping = options.ping_interval;
        let mut backoff = ReconnectBackoff::default();
        assert_eq!(
            backoff.next_delay(&options, 1.0),
            Duration::from_millis(100)
        );
        assert_eq!(
            backoff.next_delay(&options, 1.0),
            Duration::from_millis(200)
        );
        // The reconnected session died before its first ping: a flapping
        // server keeps getting longer delays.
        backoff.session_lost(ping - Duration::from_millis(1), ping);
        assert_eq!(
            backoff.next_delay(&options, 1.0),
            Duration::from_millis(400)
        );
        // A session that stayed up for a whole ping interval starts over.
        backoff.session_lost(ping, ping);
        assert_eq!(
            backoff.next_delay(&options, 1.0),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn jitter_is_in_range_and_varies() {
        let samples: Vec<f64> = (0..256).map(|_| jitter()).collect();
        assert!(
            samples.iter().all(|j| (0.5..1.0).contains(j)),
            "{samples:?}"
        );
        assert!(samples.iter().any(|j| *j != samples[0]), "{samples:?}");
    }
}
