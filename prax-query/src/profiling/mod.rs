//! Memory profiling and leak detection utilities.
//!
//! This module provides comprehensive memory profiling tools for detecting
//! memory leaks, tracking allocations, and analyzing memory usage patterns.
//!
//! # Features
//!
//! - **Allocation Tracking**: Track every allocation and deallocation
//! - **Leak Detection**: Identify memory that wasn't freed
//! - **Memory Snapshots**: Capture and compare memory state
//! - **Heap Profiling**: Integration with DHAT for detailed heap analysis
//! - **Pool Monitoring**: Track string/buffer pool usage
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use prax_query::profiling::{MemoryProfiler, AllocationTracker};
//!
//! // Start profiling
//! let profiler = MemoryProfiler::new();
//!
//! // Take initial snapshot
//! let before = profiler.snapshot();
//!
//! // ... do some work ...
//!
//! // Take final snapshot
//! let after = profiler.snapshot();
//!
//! // Analyze difference
//! let diff = after.diff(&before);
//! if diff.has_leaks() {
//!     eprintln!("Potential memory leak detected!");
//!     eprintln!("{}", diff.report());
//! }
//! ```
//!
//! # Enabling Profiling
//!
//! Add the `profiling` feature to enable runtime profiling:
//! ```toml
//! [dependencies]
//! prax-query = { version = "0.3", features = ["profiling"] }
//! ```
//!
//! For DHAT heap profiling (slower but more detailed):
//! ```toml
//! [dependencies]
//! prax-query = { version = "0.3", features = ["dhat-heap"] }
//! ```

pub mod allocation;
pub mod heap;
pub mod leak_detector;
pub mod snapshot;

pub use allocation::{
    AllocationRecord, AllocationStats, AllocationTracker, GLOBAL_TRACKER, TrackedAllocator,
};
pub use heap::{HeapProfiler, HeapReport, HeapStats};
pub use leak_detector::{LeakDetector, LeakReport, LeakSeverity, PotentialLeak};
pub use snapshot::{MemorySnapshot, PoolSnapshot, SnapshotDiff};

use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag to enable/disable profiling at runtime.
static PROFILING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable memory profiling globally.
pub fn enable_profiling() {
    PROFILING_ENABLED.store(true, Ordering::SeqCst);
    tracing::info!("Memory profiling enabled");
}

/// Disable memory profiling globally.
pub fn disable_profiling() {
    PROFILING_ENABLED.store(false, Ordering::SeqCst);
    tracing::info!("Memory profiling disabled");
}

/// Check if profiling is enabled.
#[inline]
pub fn is_profiling_enabled() -> bool {
    PROFILING_ENABLED.load(Ordering::Relaxed)
}

/// Run a closure with profiling enabled, returning the result and a memory report.
pub fn with_profiling<F, R>(f: F) -> (R, LeakReport)
where
    F: FnOnce() -> R,
{
    let detector = LeakDetector::new();
    let _guard = detector.start();

    let result = f();

    let report = detector.finish();
    (result, report)
}

/// Async version of `with_profiling`.
pub async fn with_profiling_async<F, Fut, R>(f: F) -> (R, LeakReport)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    let detector = LeakDetector::new();
    let _guard = detector.start();

    let result = f().await;

    let report = detector.finish();
    (result, report)
}

/// Memory profiler combining all profiling capabilities.
pub struct MemoryProfiler {
    tracker: AllocationTracker,
    heap_profiler: HeapProfiler,
    leak_detector: LeakDetector,
}

impl MemoryProfiler {
    /// Create a new memory profiler.
    pub fn new() -> Self {
        Self {
            tracker: AllocationTracker::new(),
            heap_profiler: HeapProfiler::new(),
            leak_detector: LeakDetector::new(),
        }
    }

    /// Take a memory snapshot.
    pub fn snapshot(&self) -> MemorySnapshot {
        MemorySnapshot::capture(&self.tracker)
    }

    /// Get current allocation statistics.
    pub fn stats(&self) -> AllocationStats {
        self.tracker.stats()
    }

    /// Get heap statistics.
    pub fn heap_stats(&self) -> HeapStats {
        self.heap_profiler.stats()
    }

    /// Run leak detection and generate a report.
    pub fn detect_leaks(&self) -> LeakReport {
        self.leak_detector.analyze(&self.tracker)
    }

    /// Generate a comprehensive memory report.
    pub fn report(&self) -> MemoryReport {
        MemoryReport {
            allocation_stats: self.stats(),
            heap_stats: self.heap_stats(),
            leak_report: self.detect_leaks(),
            pool_stats: self.pool_stats(),
        }
    }

    /// Get pool statistics (string pool, buffer pool, etc.).
    pub fn pool_stats(&self) -> PoolStats {
        use crate::memory::{GLOBAL_BUFFER_POOL, GLOBAL_STRING_POOL};

        PoolStats {
            string_pool: GLOBAL_STRING_POOL.stats(),
            buffer_pool_available: GLOBAL_BUFFER_POOL.available(),
        }
    }

    /// Reset all tracking state.
    pub fn reset(&self) {
        self.tracker.reset();
    }
}

impl Default for MemoryProfiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Comprehensive memory report.
#[derive(Debug)]
pub struct MemoryReport {
    /// Allocation statistics.
    pub allocation_stats: AllocationStats,
    /// Heap statistics.
    pub heap_stats: HeapStats,
    /// Leak detection report.
    pub leak_report: LeakReport,
    /// Pool statistics.
    pub pool_stats: PoolStats,
}

impl MemoryReport {
    /// Check if the report indicates potential issues.
    pub fn has_issues(&self) -> bool {
        self.leak_report.has_leaks()
            || self.allocation_stats.net_allocations() > 1000
            || self.heap_stats.fragmentation_ratio() > 0.3
    }

    /// Generate a human-readable summary.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str("=== Memory Profile Report ===\n\n");

        s.push_str("Allocations:\n");
        s.push_str(&format!(
            "  Total: {} ({} bytes)\n",
            self.allocation_stats.total_allocations, self.allocation_stats.total_bytes_allocated
        ));
        s.push_str(&format!(
            "  Current: {} ({} bytes)\n",
            self.allocation_stats.current_allocations, self.allocation_stats.current_bytes
        ));
        s.push_str(&format!(
            "  Peak: {} bytes\n",
            self.allocation_stats.peak_bytes
        ));

        s.push_str("\nHeap:\n");
        s.push_str(&format!("  Used: {} bytes\n", self.heap_stats.used_bytes));
        s.push_str(&format!("  RSS: {} bytes\n", self.heap_stats.rss_bytes));
        s.push_str(&format!(
            "  Fragmentation: {:.1}%\n",
            self.heap_stats.fragmentation_ratio() * 100.0
        ));

        s.push_str("\nPools:\n");
        s.push_str(&format!(
            "  String pool: {} strings ({} bytes)\n",
            self.pool_stats.string_pool.count, self.pool_stats.string_pool.total_bytes
        ));
        s.push_str(&format!(
            "  Buffer pool: {} available\n",
            self.pool_stats.buffer_pool_available
        ));

        if self.leak_report.has_leaks() {
            s.push_str("\n⚠️  POTENTIAL LEAKS DETECTED:\n");
            s.push_str(&self.leak_report.summary());
        } else {
            s.push_str("\n✅ No memory leaks detected\n");
        }

        s
    }
}

impl std::fmt::Display for MemoryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary())
    }
}

/// Pool statistics.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// String pool statistics.
    pub string_pool: crate::memory::PoolStats,
    /// Number of available buffers in buffer pool.
    pub buffer_pool_available: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profiling_toggle() {
        disable_profiling();
        assert!(!is_profiling_enabled());

        enable_profiling();
        assert!(is_profiling_enabled());

        disable_profiling();
        assert!(!is_profiling_enabled());
    }

    #[test]
    fn test_memory_profiler() {
        let profiler = MemoryProfiler::new();

        // Take snapshot
        let snapshot = profiler.snapshot();
        assert!(snapshot.timestamp > 0);

        // Get stats
        let stats = profiler.stats();
        assert!(stats.total_allocations >= 0);

        // Generate report
        let report = profiler.report();
        assert!(!report.summary().is_empty());
    }

    #[test]
    fn test_with_profiling() {
        let (result, report) = with_profiling(|| {
            // Allocate some memory
            let v: Vec<i32> = (0..100).collect();
            v.len()
        });

        assert_eq!(result, 100);
        // Report should be valid
        assert!(report.potential_leaks.is_empty() || !report.potential_leaks.is_empty());
    }
}
