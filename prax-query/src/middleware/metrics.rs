//! Metrics middleware for query performance tracking.

use super::context::{QueryContext, QueryType};
use super::types::{BoxFuture, Middleware, MiddlewareResult, Next, QueryResponse};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// Collected metrics for queries.
#[derive(Debug, Clone)]
pub struct QueryMetrics {
    /// Total number of queries executed.
    pub total_queries: u64,
    /// Number of successful queries.
    pub successful_queries: u64,
    /// Number of failed queries.
    pub failed_queries: u64,
    /// Total execution time in microseconds.
    pub total_time_us: u64,
    /// Average execution time in microseconds.
    pub avg_time_us: u64,
    /// Minimum execution time in microseconds.
    pub min_time_us: u64,
    /// Maximum execution time in microseconds.
    pub max_time_us: u64,
    /// Number of slow queries.
    pub slow_queries: u64,
    /// Number of cache hits.
    pub cache_hits: u64,
    /// Queries by type.
    pub queries_by_type: HashMap<String, u64>,
    /// Queries by model.
    pub queries_by_model: HashMap<String, u64>,
}

impl Default for QueryMetrics {
    fn default() -> Self {
        Self {
            total_queries: 0,
            successful_queries: 0,
            failed_queries: 0,
            total_time_us: 0,
            avg_time_us: 0,
            min_time_us: u64::MAX,
            max_time_us: 0,
            slow_queries: 0,
            cache_hits: 0,
            queries_by_type: HashMap::new(),
            queries_by_model: HashMap::new(),
        }
    }
}

impl QueryMetrics {
    /// Create empty metrics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate query success rate (0.0 to 1.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_queries == 0 {
            1.0
        } else {
            self.successful_queries as f64 / self.total_queries as f64
        }
    }

    /// Calculate cache hit rate (0.0 to 1.0).
    pub fn cache_hit_rate(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.cache_hits as f64 / self.total_queries as f64
        }
    }

    /// Calculate slow query rate (0.0 to 1.0).
    pub fn slow_query_rate(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.slow_queries as f64 / self.total_queries as f64
        }
    }
}

/// Interface for collecting metrics.
pub trait MetricsCollector: Send + Sync {
    /// Record a query execution.
    fn record_query(
        &self,
        query_type: QueryType,
        model: Option<&str>,
        duration_us: u64,
        success: bool,
        from_cache: bool,
    );

    /// Get current metrics.
    fn get_metrics(&self) -> QueryMetrics;

    /// Reset all metrics.
    fn reset(&self);
}

/// In-memory metrics collector.
#[derive(Debug)]
pub struct InMemoryMetricsCollector {
    total_queries: AtomicU64,
    successful_queries: AtomicU64,
    failed_queries: AtomicU64,
    total_time_us: AtomicU64,
    min_time_us: AtomicU64,
    max_time_us: AtomicU64,
    slow_queries: AtomicU64,
    cache_hits: AtomicU64,
    slow_threshold_us: u64,
    queries_by_type: RwLock<HashMap<String, u64>>,
    queries_by_model: RwLock<HashMap<String, u64>>,
}

impl InMemoryMetricsCollector {
    /// Create a new in-memory collector.
    pub fn new() -> Self {
        Self::with_slow_threshold(1_000_000) // 1 second default
    }

    /// Create with custom slow query threshold.
    pub fn with_slow_threshold(threshold_us: u64) -> Self {
        Self {
            total_queries: AtomicU64::new(0),
            successful_queries: AtomicU64::new(0),
            failed_queries: AtomicU64::new(0),
            total_time_us: AtomicU64::new(0),
            min_time_us: AtomicU64::new(u64::MAX),
            max_time_us: AtomicU64::new(0),
            slow_queries: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            slow_threshold_us: threshold_us,
            queries_by_type: RwLock::new(HashMap::new()),
            queries_by_model: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector for InMemoryMetricsCollector {
    fn record_query(
        &self,
        query_type: QueryType,
        model: Option<&str>,
        duration_us: u64,
        success: bool,
        from_cache: bool,
    ) {
        self.total_queries.fetch_add(1, Ordering::SeqCst);

        if success {
            self.successful_queries.fetch_add(1, Ordering::SeqCst);
        } else {
            self.failed_queries.fetch_add(1, Ordering::SeqCst);
        }

        self.total_time_us.fetch_add(duration_us, Ordering::SeqCst);

        // Update min (using compare-and-swap loop)
        loop {
            let current = self.min_time_us.load(Ordering::SeqCst);
            if duration_us >= current {
                break;
            }
            if self
                .min_time_us
                .compare_exchange(current, duration_us, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        // Update max
        loop {
            let current = self.max_time_us.load(Ordering::SeqCst);
            if duration_us <= current {
                break;
            }
            if self
                .max_time_us
                .compare_exchange(current, duration_us, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        if duration_us >= self.slow_threshold_us {
            self.slow_queries.fetch_add(1, Ordering::SeqCst);
        }

        if from_cache {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
        }

        // Update queries by type
        {
            let mut by_type = self.queries_by_type.write().unwrap();
            let key = format!("{:?}", query_type);
            *by_type.entry(key).or_insert(0) += 1;
        }

        // Update queries by model
        if let Some(model) = model {
            let mut by_model = self.queries_by_model.write().unwrap();
            *by_model.entry(model.to_string()).or_insert(0) += 1;
        }
    }

    fn get_metrics(&self) -> QueryMetrics {
        let total = self.total_queries.load(Ordering::SeqCst);
        let total_time = self.total_time_us.load(Ordering::SeqCst);
        let avg = total_time.checked_div(total).unwrap_or(0);
        let min = self.min_time_us.load(Ordering::SeqCst);

        QueryMetrics {
            total_queries: total,
            successful_queries: self.successful_queries.load(Ordering::SeqCst),
            failed_queries: self.failed_queries.load(Ordering::SeqCst),
            total_time_us: total_time,
            avg_time_us: avg,
            min_time_us: if min == u64::MAX { 0 } else { min },
            max_time_us: self.max_time_us.load(Ordering::SeqCst),
            slow_queries: self.slow_queries.load(Ordering::SeqCst),
            cache_hits: self.cache_hits.load(Ordering::SeqCst),
            queries_by_type: self.queries_by_type.read().unwrap().clone(),
            queries_by_model: self.queries_by_model.read().unwrap().clone(),
        }
    }

    fn reset(&self) {
        self.total_queries.store(0, Ordering::SeqCst);
        self.successful_queries.store(0, Ordering::SeqCst);
        self.failed_queries.store(0, Ordering::SeqCst);
        self.total_time_us.store(0, Ordering::SeqCst);
        self.min_time_us.store(u64::MAX, Ordering::SeqCst);
        self.max_time_us.store(0, Ordering::SeqCst);
        self.slow_queries.store(0, Ordering::SeqCst);
        self.cache_hits.store(0, Ordering::SeqCst);
        self.queries_by_type.write().unwrap().clear();
        self.queries_by_model.write().unwrap().clear();
    }
}

/// Middleware that collects query metrics.
///
/// # Example
///
/// ```rust,ignore
/// use prax_query::middleware::{MetricsMiddleware, InMemoryMetricsCollector};
///
/// let collector = Arc::new(InMemoryMetricsCollector::new());
/// let metrics = MetricsMiddleware::new(collector.clone());
///
/// // Use middleware...
///
/// // Get metrics
/// let stats = collector.get_metrics();
/// println!("Total queries: {}", stats.total_queries);
/// println!("Avg time: {}us", stats.avg_time_us);
/// ```
pub struct MetricsMiddleware {
    collector: Arc<dyn MetricsCollector>,
}

impl MetricsMiddleware {
    /// Create a new metrics middleware.
    pub fn new(collector: Arc<dyn MetricsCollector>) -> Self {
        Self { collector }
    }

    /// Create with default in-memory collector.
    pub fn in_memory() -> (Self, Arc<InMemoryMetricsCollector>) {
        let collector = Arc::new(InMemoryMetricsCollector::new());
        let middleware = Self::new(collector.clone());
        (middleware, collector)
    }

    /// Get the metrics collector.
    pub fn collector(&self) -> &Arc<dyn MetricsCollector> {
        &self.collector
    }
}

impl Middleware for MetricsMiddleware {
    fn handle<'a>(
        &'a self,
        ctx: QueryContext,
        next: Next<'a>,
    ) -> BoxFuture<'a, MiddlewareResult<QueryResponse>> {
        Box::pin(async move {
            let query_type = ctx.query_type();
            let model = ctx.metadata().model.clone();
            let start = Instant::now();

            let result = next.run(ctx).await;

            let duration_us = start.elapsed().as_micros() as u64;
            let (success, from_cache) = match &result {
                Ok(response) => (true, response.from_cache),
                Err(_) => (false, false),
            };

            self.collector.record_query(
                query_type,
                model.as_deref(),
                duration_us,
                success,
                from_cache,
            );

            result
        })
    }

    fn name(&self) -> &'static str {
        "MetricsMiddleware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_metrics_default() {
        let metrics = QueryMetrics::new();
        assert_eq!(metrics.total_queries, 0);
        assert_eq!(metrics.success_rate(), 1.0);
        assert_eq!(metrics.cache_hit_rate(), 0.0);
    }

    #[test]
    fn test_in_memory_collector() {
        let collector = InMemoryMetricsCollector::new();

        collector.record_query(QueryType::Select, Some("User"), 1000, true, false);
        collector.record_query(QueryType::Select, Some("User"), 2000, true, true);
        collector.record_query(QueryType::Insert, Some("Post"), 500, false, false);

        let metrics = collector.get_metrics();
        assert_eq!(metrics.total_queries, 3);
        assert_eq!(metrics.successful_queries, 2);
        assert_eq!(metrics.failed_queries, 1);
        assert_eq!(metrics.cache_hits, 1);
        assert_eq!(metrics.min_time_us, 500);
        assert_eq!(metrics.max_time_us, 2000);
    }

    #[test]
    fn test_collector_reset() {
        let collector = InMemoryMetricsCollector::new();
        collector.record_query(QueryType::Select, None, 1000, true, false);

        assert_eq!(collector.get_metrics().total_queries, 1);

        collector.reset();

        assert_eq!(collector.get_metrics().total_queries, 0);
    }

    #[test]
    fn test_metrics_rates() {
        let collector = InMemoryMetricsCollector::with_slow_threshold(1000);

        collector.record_query(QueryType::Select, None, 500, true, true);
        collector.record_query(QueryType::Select, None, 500, true, false);
        collector.record_query(QueryType::Select, None, 2000, true, false); // slow
        collector.record_query(QueryType::Select, None, 500, false, false);

        let metrics = collector.get_metrics();
        assert_eq!(metrics.total_queries, 4);
        assert!((metrics.success_rate() - 0.75).abs() < 0.01);
        assert!((metrics.cache_hit_rate() - 0.25).abs() < 0.01);
        assert!((metrics.slow_query_rate() - 0.25).abs() < 0.01);
    }
}
