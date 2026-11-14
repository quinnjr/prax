//! Retry middleware for automatic query retry on transient failures.

use super::backoff;
use super::context::QueryContext;
use super::types::{BoxFuture, Middleware, MiddlewareResult, Next, QueryResponse};
use crate::QueryError;
use std::time::Duration;

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Initial delay between retries.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Multiplier for exponential backoff.
    pub backoff_multiplier: f64,
    /// Whether to add jitter to delays.
    pub jitter: bool,
    /// Predicate to determine if error is retryable.
    pub retry_on: RetryPredicate,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            jitter: true,
            retry_on: RetryPredicate::Default,
        }
    }
}

impl RetryConfig {
    /// Create a new retry config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum retries.
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Set initial delay.
    pub fn initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set maximum delay.
    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Set backoff multiplier.
    pub fn backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Enable or disable jitter.
    pub fn jitter(mut self, enabled: bool) -> Self {
        self.jitter = enabled;
        self
    }

    /// Set retry predicate.
    pub fn retry_on(mut self, predicate: RetryPredicate) -> Self {
        self.retry_on = predicate;
        self
    }

    /// Calculate delay for a given attempt (0-based):
    /// `min(initial_delay * backoff_multiplier^attempt, max_delay)`, plus up
    /// to 25% on top when [`jitter`](Self::jitter) is enabled. Never panics,
    /// whatever the attempt or the configured values.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let delay = backoff::capped_exponential(
            self.initial_delay,
            self.backoff_multiplier,
            attempt,
            self.max_delay,
        );
        if self.jitter {
            // Add up to 25% jitter.
            delay.saturating_add(backoff::scale(delay, 0.25 * backoff::unit_jitter()))
        } else {
            delay
        }
    }
}

/// Predicate for determining if an error should trigger a retry.
#[derive(Debug, Clone)]
pub enum RetryPredicate {
    /// Default: retry on connection and timeout errors.
    Default,
    /// Retry on any error.
    Always,
    /// Never retry.
    Never,
    /// Retry only on connection errors.
    ConnectionOnly,
    /// Retry only on timeout errors.
    TimeoutOnly,
    /// Custom list of error types to retry.
    Custom(Vec<RetryableError>),
}

impl RetryPredicate {
    /// Check if an error should be retried.
    pub fn should_retry(&self, error: &QueryError) -> bool {
        match self {
            Self::Default => error.is_connection_error() || error.is_timeout(),
            Self::Always => true,
            Self::Never => false,
            Self::ConnectionOnly => error.is_connection_error(),
            Self::TimeoutOnly => error.is_timeout(),
            Self::Custom(errors) => errors.iter().any(|e| e.matches(error)),
        }
    }
}

/// Types of errors that can be configured for retry.
#[derive(Debug, Clone, Copy)]
pub enum RetryableError {
    /// Connection errors.
    Connection,
    /// Timeout errors.
    Timeout,
    /// Database errors.
    Database,
    /// Transaction errors.
    Transaction,
}

impl RetryableError {
    /// Check if this error type matches the given error.
    pub fn matches(&self, error: &QueryError) -> bool {
        match self {
            Self::Connection => error.is_connection_error(),
            Self::Timeout => error.is_timeout(),
            Self::Database => matches!(
                error.code,
                crate::error::ErrorCode::SqlSyntax
                    | crate::error::ErrorCode::InvalidParameter
                    | crate::error::ErrorCode::QueryTooComplex
            ),
            Self::Transaction => matches!(
                error.code,
                crate::error::ErrorCode::TransactionFailed
                    | crate::error::ErrorCode::Deadlock
                    | crate::error::ErrorCode::SerializationFailure
                    | crate::error::ErrorCode::TransactionClosed
            ),
        }
    }
}

/// Middleware that automatically retries failed queries.
///
/// # Example
///
/// ```rust,ignore
/// use prax_query::middleware::{RetryMiddleware, RetryConfig};
/// use std::time::Duration;
///
/// let retry = RetryMiddleware::new(
///     RetryConfig::new()
///         .max_retries(5)
///         .initial_delay(Duration::from_millis(50))
///         .backoff_multiplier(2.0)
/// );
/// ```
pub struct RetryMiddleware {
    config: RetryConfig,
}

impl RetryMiddleware {
    /// Create a new retry middleware with the given config.
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration.
    pub fn default_config() -> Self {
        Self::new(RetryConfig::default())
    }

    /// Get the retry configuration.
    pub fn config(&self) -> &RetryConfig {
        &self.config
    }
}

impl Default for RetryMiddleware {
    fn default() -> Self {
        Self::default_config()
    }
}

impl Middleware for RetryMiddleware {
    fn handle<'a>(
        &'a self,
        ctx: QueryContext,
        next: Next<'a>,
    ) -> BoxFuture<'a, MiddlewareResult<QueryResponse>> {
        Box::pin(async move {
            // For now, just pass through - actual retry logic would need
            // to be able to re-execute the query which requires different design
            // This is a placeholder that shows the structure

            // In a real implementation, we would:
            // 1. Execute the query
            // 2. If it fails with a retryable error, wait and retry
            // 3. Track retry attempts
            // 4. Eventually return success or final failure

            next.run(ctx).await
        })
    }

    fn name(&self) -> &'static str {
        "RetryMiddleware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert!(config.jitter);
    }

    #[test]
    fn test_retry_config_builder() {
        let config = RetryConfig::new()
            .max_retries(5)
            .initial_delay(Duration::from_millis(50))
            .jitter(false);

        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay, Duration::from_millis(50));
        assert!(!config.jitter);
    }

    #[test]
    fn test_delay_calculation() {
        let config = RetryConfig::new()
            .initial_delay(Duration::from_millis(100))
            .backoff_multiplier(2.0)
            .jitter(false);

        assert_eq!(config.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(400));
    }

    #[test]
    fn test_delay_max_cap() {
        let config = RetryConfig::new()
            .initial_delay(Duration::from_secs(1))
            .max_delay(Duration::from_secs(5))
            .backoff_multiplier(10.0)
            .jitter(false);

        // Should be capped at 5 seconds
        assert_eq!(config.delay_for_attempt(2), Duration::from_secs(5));
    }

    #[test]
    fn test_delay_never_panics() {
        let config = RetryConfig::new()
            .initial_delay(Duration::from_secs(1))
            .max_delay(Duration::MAX)
            .backoff_multiplier(2.0);
        // `attempt as i32` used to wrap here, and `initial * 2^70` overflows.
        assert_eq!(config.delay_for_attempt(u32::MAX), Duration::MAX);
        assert_eq!(config.delay_for_attempt(70), Duration::MAX);
        let config = config.jitter(false).backoff_multiplier(f64::NAN);
        assert_eq!(config.delay_for_attempt(3), Duration::MAX);
    }

    #[test]
    fn test_delay_jitter_adds_at_most_a_quarter() {
        let config = RetryConfig::new()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(1))
            .backoff_multiplier(2.0)
            .jitter(true);
        for _ in 0..64 {
            let delay = config.delay_for_attempt(1);
            assert!(
                (Duration::from_millis(200)..=Duration::from_millis(250)).contains(&delay),
                "{delay:?}"
            );
            // Jitter goes on top of the cap.
            let capped = config.delay_for_attempt(10);
            assert!(
                (Duration::from_secs(1)..=Duration::from_millis(1250)).contains(&capped),
                "{capped:?}"
            );
        }
    }

    #[test]
    fn test_retry_predicate_default() {
        let predicate = RetryPredicate::Default;

        assert!(predicate.should_retry(&QueryError::connection("test")));
        assert!(predicate.should_retry(&QueryError::timeout(1000)));
        assert!(!predicate.should_retry(&QueryError::not_found("User")));
    }

    #[test]
    fn test_retry_predicate_custom() {
        let predicate =
            RetryPredicate::Custom(vec![RetryableError::Connection, RetryableError::Database]);

        assert!(predicate.should_retry(&QueryError::connection("test")));
        assert!(predicate.should_retry(&QueryError::sql_syntax("error", "SELECT")));
        assert!(!predicate.should_retry(&QueryError::timeout(1000)));
    }
}
