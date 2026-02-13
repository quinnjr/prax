//! Cache statistics and metrics.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Thread-safe cache metrics collector.
pub struct CacheMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
    writes: AtomicU64,
    deletes: AtomicU64,
    errors: AtomicU64,
    total_hit_time_ns: AtomicU64,
    total_miss_time_ns: AtomicU64,
    total_write_time_ns: AtomicU64,
    created_at: Instant,
}

impl Default for CacheMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheMetrics {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            writes: AtomicU64::new(0),
            deletes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            total_hit_time_ns: AtomicU64::new(0),
            total_miss_time_ns: AtomicU64::new(0),
            total_write_time_ns: AtomicU64::new(0),
            created_at: Instant::now(),
        }
    }

    /// Record a cache hit.
    #[inline]
    pub fn record_hit(&self, duration: Duration) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        self.total_hit_time_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record a cache miss.
    #[inline]
    pub fn record_miss(&self, duration: Duration) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        self.total_miss_time_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record a write operation.
    #[inline]
    pub fn record_write(&self, duration: Duration) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.total_write_time_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record a delete operation.
    #[inline]
    pub fn record_delete(&self) {
        self.deletes.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an error.
    #[inline]
    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of the current stats.
    pub fn snapshot(&self) -> CacheStats {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let writes = self.writes.load(Ordering::Relaxed);
        let deletes = self.deletes.load(Ordering::Relaxed);
        let errors = self.errors.load(Ordering::Relaxed);
        let total_hit_time_ns = self.total_hit_time_ns.load(Ordering::Relaxed);
        let total_miss_time_ns = self.total_miss_time_ns.load(Ordering::Relaxed);
        let total_write_time_ns = self.total_write_time_ns.load(Ordering::Relaxed);

        CacheStats {
            hits,
            misses,
            writes,
            deletes,
            errors,
            hit_rate: if hits + misses > 0 {
                hits as f64 / (hits + misses) as f64
            } else {
                0.0
            },
            avg_hit_time: if hits > 0 {
                Duration::from_nanos(total_hit_time_ns / hits)
            } else {
                Duration::ZERO
            },
            avg_miss_time: if misses > 0 {
                Duration::from_nanos(total_miss_time_ns / misses)
            } else {
                Duration::ZERO
            },
            avg_write_time: if writes > 0 {
                Duration::from_nanos(total_write_time_ns / writes)
            } else {
                Duration::ZERO
            },
            uptime: self.created_at.elapsed(),
            entries: 0, // Filled by backend
            memory_bytes: None,
        }
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.writes.store(0, Ordering::Relaxed);
        self.deletes.store(0, Ordering::Relaxed);
        self.errors.store(0, Ordering::Relaxed);
        self.total_hit_time_ns.store(0, Ordering::Relaxed);
        self.total_miss_time_ns.store(0, Ordering::Relaxed);
        self.total_write_time_ns.store(0, Ordering::Relaxed);
    }
}

/// A snapshot of cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of write operations.
    pub writes: u64,
    /// Number of delete operations.
    pub deletes: u64,
    /// Number of errors.
    pub errors: u64,
    /// Hit rate (0.0 - 1.0).
    pub hit_rate: f64,
    /// Average time for cache hits.
    pub avg_hit_time: Duration,
    /// Average time for cache misses.
    pub avg_miss_time: Duration,
    /// Average time for writes.
    pub avg_write_time: Duration,
    /// Time since cache was created.
    pub uptime: Duration,
    /// Number of entries in cache.
    pub entries: usize,
    /// Memory usage in bytes (if available).
    pub memory_bytes: Option<usize>,
}

impl Default for CacheStats {
    fn default() -> Self {
        Self {
            hits: 0,
            misses: 0,
            writes: 0,
            deletes: 0,
            errors: 0,
            hit_rate: 0.0,
            avg_hit_time: Duration::ZERO,
            avg_miss_time: Duration::ZERO,
            avg_write_time: Duration::ZERO,
            uptime: Duration::ZERO,
            entries: 0,
            memory_bytes: None,
        }
    }
}

impl CacheStats {
    /// Total number of operations.
    pub fn total_ops(&self) -> u64 {
        self.hits + self.misses + self.writes + self.deletes
    }

    /// Requests per second.
    pub fn ops_per_second(&self) -> f64 {
        if self.uptime.as_secs_f64() > 0.0 {
            self.total_ops() as f64 / self.uptime.as_secs_f64()
        } else {
            0.0
        }
    }

    /// Format as a human-readable string.
    pub fn summary(&self) -> String {
        format!(
            "Cache Stats: {} hits, {} misses ({:.1}% hit rate), {} entries, uptime {:?}",
            self.hits,
            self.misses,
            self.hit_rate * 100.0,
            self.entries,
            self.uptime
        )
    }
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_recording() {
        let metrics = CacheMetrics::new();

        metrics.record_hit(Duration::from_micros(100));
        metrics.record_hit(Duration::from_micros(200));
        metrics.record_miss(Duration::from_micros(500));

        let stats = metrics.snapshot();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_rate - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_stats_ops_per_second() {
        let stats = CacheStats {
            hits: 1000,
            misses: 100,
            writes: 50,
            deletes: 10,
            uptime: Duration::from_secs(10),
            ..Default::default()
        };

        assert_eq!(stats.total_ops(), 1160);
        assert!((stats.ops_per_second() - 116.0).abs() < 0.1);
    }
}
