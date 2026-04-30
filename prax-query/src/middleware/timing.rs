//! Timing middleware for measuring query execution time.

use super::context::QueryContext;
use super::types::{BoxFuture, Middleware, MiddlewareResult, Next, QueryResponse};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Result of timing a query.
#[derive(Debug, Clone)]
pub struct TimingResult {
    /// Execution time in nanoseconds.
    pub duration_ns: u64,
    /// Execution time in microseconds.
    pub duration_us: u64,
    /// Execution time in milliseconds.
    pub duration_ms: u64,
}

impl TimingResult {
    /// Create from a duration.
    pub fn from_nanos(ns: u64) -> Self {
        Self {
            duration_ns: ns,
            duration_us: ns / 1000,
            duration_ms: ns / 1_000_000,
        }
    }
}

/// Middleware that measures query execution time.
///
/// This is a lightweight middleware that only adds timing information
/// to the response. For more comprehensive metrics, use `MetricsMiddleware`.
pub struct TimingMiddleware {
    /// Total execution time in nanoseconds.
    total_time_ns: AtomicU64,
    /// Number of queries timed.
    query_count: AtomicU64,
}

impl TimingMiddleware {
    /// Create a new timing middleware.
    pub fn new() -> Self {
        Self {
            total_time_ns: AtomicU64::new(0),
            query_count: AtomicU64::new(0),
        }
    }

    /// Get the total execution time in nanoseconds.
    pub fn total_time_ns(&self) -> u64 {
        self.total_time_ns.load(Ordering::Relaxed)
    }

    /// Get the total execution time in microseconds.
    pub fn total_time_us(&self) -> u64 {
        self.total_time_ns() / 1000
    }

    /// Get the total execution time in milliseconds.
    pub fn total_time_ms(&self) -> u64 {
        self.total_time_ns() / 1_000_000
    }

    /// Get the number of queries timed.
    pub fn query_count(&self) -> u64 {
        self.query_count.load(Ordering::Relaxed)
    }

    /// Get the average execution time in nanoseconds.
    pub fn avg_time_ns(&self) -> u64 {
        self.total_time_ns()
            .checked_div(self.query_count())
            .unwrap_or(0)
    }

    /// Get the average execution time in microseconds.
    pub fn avg_time_us(&self) -> u64 {
        self.avg_time_ns() / 1000
    }

    /// Reset timing statistics.
    pub fn reset(&self) {
        self.total_time_ns.store(0, Ordering::SeqCst);
        self.query_count.store(0, Ordering::SeqCst);
    }
}

impl Default for TimingMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware for TimingMiddleware {
    fn handle<'a>(
        &'a self,
        ctx: QueryContext,
        next: Next<'a>,
    ) -> BoxFuture<'a, MiddlewareResult<QueryResponse>> {
        Box::pin(async move {
            let start = Instant::now();

            let result = next.run(ctx).await;

            let elapsed = start.elapsed();
            let elapsed_ns = elapsed.as_nanos() as u64;
            let elapsed_us = elapsed.as_micros() as u64;

            self.total_time_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
            self.query_count.fetch_add(1, Ordering::Relaxed);

            // Update the response with execution time
            result.map(|mut response| {
                response.execution_time_us = elapsed_us;
                response
            })
        })
    }

    fn name(&self) -> &'static str {
        "TimingMiddleware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timing_result() {
        let result = TimingResult::from_nanos(1_500_000);
        assert_eq!(result.duration_ns, 1_500_000);
        assert_eq!(result.duration_us, 1500);
        assert_eq!(result.duration_ms, 1);
    }

    #[test]
    fn test_timing_middleware_initial_state() {
        let middleware = TimingMiddleware::new();
        assert_eq!(middleware.total_time_ns(), 0);
        assert_eq!(middleware.query_count(), 0);
        assert_eq!(middleware.avg_time_ns(), 0);
    }

    #[test]
    fn test_timing_middleware_reset() {
        let middleware = TimingMiddleware::new();
        middleware.total_time_ns.store(1000, Ordering::SeqCst);
        middleware.query_count.store(2, Ordering::SeqCst);

        middleware.reset();

        assert_eq!(middleware.total_time_ns(), 0);
        assert_eq!(middleware.query_count(), 0);
    }
}
