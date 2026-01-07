//! Heap profiling and analysis.
//!
//! Provides heap-level profiling using system APIs and optional DHAT integration.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// ============================================================================
// Heap Profiler
// ============================================================================

/// Heap profiler for analyzing memory usage patterns.
pub struct HeapProfiler {
    /// Start time for profiling session.
    start_time: Instant,
    /// Samples collected.
    samples: parking_lot::Mutex<Vec<HeapSample>>,
    /// Sample interval in ms.
    sample_interval_ms: AtomicUsize,
}

impl HeapProfiler {
    /// Create a new heap profiler.
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            samples: parking_lot::Mutex::new(Vec::new()),
            sample_interval_ms: AtomicUsize::new(100),
        }
    }

    /// Set the sample interval.
    pub fn set_sample_interval(&self, ms: usize) {
        self.sample_interval_ms.store(ms, Ordering::Relaxed);
    }

    /// Take a heap sample.
    pub fn sample(&self) -> HeapSample {
        let sample = HeapSample::capture();
        self.samples.lock().push(sample.clone());
        sample
    }

    /// Get current heap statistics.
    pub fn stats(&self) -> HeapStats {
        HeapStats::capture()
    }

    /// Get all samples.
    pub fn samples(&self) -> Vec<HeapSample> {
        self.samples.lock().clone()
    }

    /// Generate a heap report.
    pub fn report(&self) -> HeapReport {
        let samples = self.samples.lock().clone();
        let current = HeapStats::capture();

        HeapReport {
            duration: self.start_time.elapsed(),
            samples,
            current_stats: current,
        }
    }

    /// Clear all samples.
    pub fn clear(&self) {
        self.samples.lock().clear();
    }
}

impl Default for HeapProfiler {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Heap Sample
// ============================================================================

/// A point-in-time sample of heap state.
#[derive(Debug, Clone)]
pub struct HeapSample {
    /// Timestamp (ms since profiler start).
    pub timestamp_ms: u64,
    /// Resident set size (RSS).
    pub rss_bytes: usize,
    /// Virtual memory size.
    pub virtual_bytes: usize,
    /// Heap used (if available).
    pub heap_used: Option<usize>,
}

impl HeapSample {
    /// Capture a heap sample.
    pub fn capture() -> Self {
        let (rss, virtual_mem) = get_memory_usage();

        Self {
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            rss_bytes: rss,
            virtual_bytes: virtual_mem,
            heap_used: None,
        }
    }
}

// ============================================================================
// Heap Statistics
// ============================================================================

/// Heap statistics.
#[derive(Debug, Clone, Default)]
pub struct HeapStats {
    /// Bytes currently used.
    pub used_bytes: usize,
    /// Bytes currently allocated from OS.
    pub allocated_bytes: usize,
    /// Resident set size.
    pub rss_bytes: usize,
    /// Virtual memory size.
    pub virtual_bytes: usize,
    /// Peak RSS.
    pub peak_rss_bytes: usize,
    /// Number of heap segments.
    pub segments: usize,
}

impl HeapStats {
    /// Capture current heap statistics.
    pub fn capture() -> Self {
        let (rss, virtual_mem) = get_memory_usage();

        Self {
            used_bytes: rss, // Approximation
            allocated_bytes: virtual_mem,
            rss_bytes: rss,
            virtual_bytes: virtual_mem,
            peak_rss_bytes: rss, // Would need tracking for true peak
            segments: 0,
        }
    }

    /// Calculate fragmentation ratio (0.0 = no fragmentation, 1.0 = highly fragmented).
    pub fn fragmentation_ratio(&self) -> f64 {
        if self.allocated_bytes == 0 {
            return 0.0;
        }
        let unused = self.allocated_bytes.saturating_sub(self.used_bytes);
        unused as f64 / self.allocated_bytes as f64
    }

    /// Get memory efficiency (used / allocated).
    pub fn efficiency(&self) -> f64 {
        if self.allocated_bytes == 0 {
            return 1.0;
        }
        self.used_bytes as f64 / self.allocated_bytes as f64
    }
}

// ============================================================================
// Heap Report
// ============================================================================

/// Comprehensive heap report.
#[derive(Debug, Clone)]
pub struct HeapReport {
    /// Duration of profiling.
    pub duration: std::time::Duration,
    /// Collected samples.
    pub samples: Vec<HeapSample>,
    /// Current heap stats.
    pub current_stats: HeapStats,
}

impl HeapReport {
    /// Get peak RSS from samples.
    pub fn peak_rss(&self) -> usize {
        self.samples
            .iter()
            .map(|s| s.rss_bytes)
            .max()
            .unwrap_or(0)
    }

    /// Get average RSS from samples.
    pub fn avg_rss(&self) -> usize {
        if self.samples.is_empty() {
            return 0;
        }
        self.samples.iter().map(|s| s.rss_bytes).sum::<usize>() / self.samples.len()
    }

    /// Check for memory growth trend.
    pub fn has_growth_trend(&self) -> bool {
        if self.samples.len() < 3 {
            return false;
        }

        // Compare first third vs last third
        let third = self.samples.len() / 3;
        let first_avg: usize =
            self.samples[..third].iter().map(|s| s.rss_bytes).sum::<usize>() / third;
        let last_avg: usize = self.samples[self.samples.len() - third..]
            .iter()
            .map(|s| s.rss_bytes)
            .sum::<usize>()
            / third;

        // More than 20% growth indicates trend
        last_avg > first_avg + (first_avg / 5)
    }

    /// Generate summary string.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "Heap Report (duration: {:?})\n",
            self.duration
        ));
        s.push_str(&format!(
            "  Current RSS: {} bytes ({:.2} MB)\n",
            self.current_stats.rss_bytes,
            self.current_stats.rss_bytes as f64 / 1_048_576.0
        ));
        s.push_str(&format!(
            "  Peak RSS: {} bytes ({:.2} MB)\n",
            self.peak_rss(),
            self.peak_rss() as f64 / 1_048_576.0
        ));
        s.push_str(&format!(
            "  Fragmentation: {:.1}%\n",
            self.current_stats.fragmentation_ratio() * 100.0
        ));

        if self.has_growth_trend() {
            s.push_str("  ⚠️  Memory growth trend detected\n");
        }

        s
    }
}

// ============================================================================
// Platform-specific memory usage
// ============================================================================

/// Get current memory usage (RSS, Virtual).
#[cfg(target_os = "linux")]
fn get_memory_usage() -> (usize, usize) {
    use std::fs;

    // Read /proc/self/statm
    if let Ok(statm) = fs::read_to_string("/proc/self/statm") {
        let parts: Vec<&str> = statm.split_whitespace().collect();
        if parts.len() >= 2 {
            let page_size = 4096; // Usually 4K pages
            let virtual_pages: usize = parts[0].parse().unwrap_or(0);
            let rss_pages: usize = parts[1].parse().unwrap_or(0);
            return (rss_pages * page_size, virtual_pages * page_size);
        }
    }

    (0, 0)
}

#[cfg(target_os = "macos")]
fn get_memory_usage() -> (usize, usize) {
    // Use mach APIs on macOS
    // For simplicity, return 0 if memory-stats is not available
    #[cfg(feature = "profiling")]
    {
        if let Some(usage) = memory_stats::memory_stats() {
            return (usage.physical_mem, usage.virtual_mem);
        }
    }
    (0, 0)
}

#[cfg(target_os = "windows")]
fn get_memory_usage() -> (usize, usize) {
    #[cfg(feature = "profiling")]
    {
        if let Some(usage) = memory_stats::memory_stats() {
            return (usage.physical_mem, usage.virtual_mem);
        }
    }
    (0, 0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn get_memory_usage() -> (usize, usize) {
    (0, 0)
}

// ============================================================================
// DHAT Integration
// ============================================================================

/// DHAT heap profiler wrapper (requires `dhat-heap` feature).
#[cfg(feature = "dhat-heap")]
pub mod dhat_profiler {
    use dhat::{Profiler, ProfilerBuilder};

    /// Guard that stops profiling when dropped.
    pub struct DhatGuard {
        #[allow(dead_code)]
        profiler: Profiler,
    }

    /// Start DHAT heap profiling.
    pub fn start_dhat() -> DhatGuard {
        let profiler = dhat::Profiler::builder().build();
        DhatGuard { profiler }
    }

    /// Start DHAT with custom options.
    pub fn start_dhat_with_file(path: &str) -> DhatGuard {
        let profiler = dhat::Profiler::builder()
            .file_name(path)
            .build();
        DhatGuard { profiler }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heap_sample() {
        let sample = HeapSample::capture();
        assert!(sample.timestamp_ms > 0);
    }

    #[test]
    fn test_heap_stats() {
        let stats = HeapStats::capture();
        // Basic sanity check
        assert!(stats.fragmentation_ratio() >= 0.0);
        assert!(stats.efficiency() >= 0.0 && stats.efficiency() <= 1.0);
    }

    #[test]
    fn test_heap_profiler() {
        let profiler = HeapProfiler::new();

        // Take some samples
        profiler.sample();
        profiler.sample();
        profiler.sample();

        let report = profiler.report();
        assert_eq!(report.samples.len(), 3);
    }

    #[test]
    fn test_growth_detection() {
        let report = HeapReport {
            duration: std::time::Duration::from_secs(10),
            samples: vec![
                HeapSample {
                    timestamp_ms: 0,
                    rss_bytes: 1000,
                    virtual_bytes: 2000,
                    heap_used: None,
                },
                HeapSample {
                    timestamp_ms: 100,
                    rss_bytes: 1100,
                    virtual_bytes: 2100,
                    heap_used: None,
                },
                HeapSample {
                    timestamp_ms: 200,
                    rss_bytes: 1200,
                    virtual_bytes: 2200,
                    heap_used: None,
                },
                HeapSample {
                    timestamp_ms: 300,
                    rss_bytes: 2000,
                    virtual_bytes: 3000,
                    heap_used: None,
                },
                HeapSample {
                    timestamp_ms: 400,
                    rss_bytes: 2500,
                    virtual_bytes: 3500,
                    heap_used: None,
                },
                HeapSample {
                    timestamp_ms: 500,
                    rss_bytes: 3000,
                    virtual_bytes: 4000,
                    heap_used: None,
                },
            ],
            current_stats: HeapStats::default(),
        };

        assert!(report.has_growth_trend());
    }
}

