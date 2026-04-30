//! Memory snapshots for comparing state over time.
//!
//! Snapshots capture the complete memory state at a point in time,
//! allowing you to compare before/after states and identify changes.

use super::allocation::{AllocationStats, AllocationTracker, SizeHistogram};
use crate::memory::{GLOBAL_BUFFER_POOL, GLOBAL_STRING_POOL, PoolStats};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ============================================================================
// Memory Snapshot
// ============================================================================

/// A point-in-time snapshot of memory state.
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Instant for duration calculations.
    pub instant: Instant,
    /// Allocation statistics.
    pub stats: AllocationStats,
    /// Size histogram.
    pub histogram: SizeHistogram,
    /// Pool snapshot.
    pub pools: PoolSnapshot,
    /// Label for this snapshot.
    pub label: String,
}

impl MemorySnapshot {
    /// Capture a new snapshot.
    pub fn capture(tracker: &AllocationTracker) -> Self {
        Self {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            instant: Instant::now(),
            stats: tracker.stats(),
            histogram: tracker.histogram(),
            pools: PoolSnapshot::capture(),
            label: String::new(),
        }
    }

    /// Capture a labeled snapshot.
    pub fn capture_labeled(tracker: &AllocationTracker, label: impl Into<String>) -> Self {
        let mut snap = Self::capture(tracker);
        snap.label = label.into();
        snap
    }

    /// Compare this snapshot with another (self - other).
    pub fn diff(&self, other: &MemorySnapshot) -> SnapshotDiff {
        SnapshotDiff {
            time_delta: self
                .instant
                .checked_duration_since(other.instant)
                .unwrap_or_default(),
            allocations_delta: self.stats.total_allocations as i64
                - other.stats.total_allocations as i64,
            deallocations_delta: self.stats.total_deallocations as i64
                - other.stats.total_deallocations as i64,
            bytes_delta: self.stats.current_bytes as i64 - other.stats.current_bytes as i64,
            peak_delta: self.stats.peak_bytes as i64 - other.stats.peak_bytes as i64,
            string_pool_delta: self.pools.string_pool.count as i64
                - other.pools.string_pool.count as i64,
            buffer_pool_delta: self.pools.buffer_pool_available as i64
                - other.pools.buffer_pool_available as i64,
            from_label: other.label.clone(),
            to_label: self.label.clone(),
        }
    }

    /// Get total bytes currently allocated.
    pub fn current_bytes(&self) -> usize {
        self.stats.current_bytes
    }

    /// Get peak bytes allocated.
    pub fn peak_bytes(&self) -> usize {
        self.stats.peak_bytes
    }
}

// ============================================================================
// Pool Snapshot
// ============================================================================

/// Snapshot of memory pools.
#[derive(Debug, Clone, Default)]
pub struct PoolSnapshot {
    /// String pool statistics.
    pub string_pool: PoolStats,
    /// Number of available buffers in buffer pool.
    pub buffer_pool_available: usize,
}

impl PoolSnapshot {
    /// Capture current pool state.
    pub fn capture() -> Self {
        Self {
            string_pool: GLOBAL_STRING_POOL.stats(),
            buffer_pool_available: GLOBAL_BUFFER_POOL.available(),
        }
    }
}

// ============================================================================
// Snapshot Diff
// ============================================================================

/// Difference between two memory snapshots.
#[derive(Debug, Clone)]
pub struct SnapshotDiff {
    /// Time between snapshots.
    pub time_delta: Duration,
    /// Change in allocation count.
    pub allocations_delta: i64,
    /// Change in deallocation count.
    pub deallocations_delta: i64,
    /// Change in current bytes.
    pub bytes_delta: i64,
    /// Change in peak bytes.
    pub peak_delta: i64,
    /// Change in string pool size.
    pub string_pool_delta: i64,
    /// Change in buffer pool available.
    pub buffer_pool_delta: i64,
    /// Label of "from" snapshot.
    pub from_label: String,
    /// Label of "to" snapshot.
    pub to_label: String,
}

impl SnapshotDiff {
    /// Check if there are potential leaks (positive byte delta).
    pub fn has_leaks(&self) -> bool {
        self.bytes_delta > 0 && self.allocations_delta > self.deallocations_delta
    }

    /// Check if memory grew significantly (>10%).
    pub fn significant_growth(&self, from_bytes: usize) -> bool {
        if from_bytes == 0 {
            return self.bytes_delta > 0;
        }
        (self.bytes_delta as f64 / from_bytes as f64).abs() > 0.1
    }

    /// Get net allocation change.
    pub fn net_allocations(&self) -> i64 {
        self.allocations_delta - self.deallocations_delta
    }

    /// Generate a report string.
    pub fn report(&self) -> String {
        let mut s = String::new();

        // Header
        if !self.from_label.is_empty() || !self.to_label.is_empty() {
            s.push_str(&format!(
                "=== Snapshot Diff: '{}' -> '{}' ===\n",
                if self.from_label.is_empty() {
                    "start"
                } else {
                    &self.from_label
                },
                if self.to_label.is_empty() {
                    "end"
                } else {
                    &self.to_label
                },
            ));
        } else {
            s.push_str("=== Snapshot Diff ===\n");
        }

        s.push_str(&format!("Time elapsed: {:?}\n\n", self.time_delta));

        // Allocations
        s.push_str("Allocations:\n");
        s.push_str(&format!(
            "  New allocations: {:+}\n",
            self.allocations_delta
        ));
        s.push_str(&format!(
            "  New deallocations: {:+}\n",
            self.deallocations_delta
        ));
        s.push_str(&format!(
            "  Net allocations: {:+}\n",
            self.net_allocations()
        ));

        // Bytes
        s.push_str("\nMemory:\n");
        let bytes_str = if self.bytes_delta >= 0 {
            format!(
                "+{} bytes (+{:.2} KB)",
                self.bytes_delta,
                self.bytes_delta as f64 / 1024.0
            )
        } else {
            format!(
                "{} bytes ({:.2} KB)",
                self.bytes_delta,
                self.bytes_delta as f64 / 1024.0
            )
        };
        s.push_str(&format!("  Current bytes: {}\n", bytes_str));

        let peak_str = if self.peak_delta >= 0 {
            format!("+{}", self.peak_delta)
        } else {
            format!("{}", self.peak_delta)
        };
        s.push_str(&format!("  Peak bytes: {}\n", peak_str));

        // Pools
        s.push_str("\nPools:\n");
        s.push_str(&format!(
            "  String pool entries: {:+}\n",
            self.string_pool_delta
        ));
        s.push_str(&format!(
            "  Buffer pool available: {:+}\n",
            self.buffer_pool_delta
        ));

        // Assessment
        s.push_str("\nAssessment:\n");
        if self.has_leaks() {
            s.push_str("  ⚠️  Potential memory leak detected!\n");
            s.push_str(&format!(
                "     {} bytes held across {} net allocations\n",
                self.bytes_delta,
                self.net_allocations()
            ));
        } else if self.bytes_delta > 0 {
            s.push_str("  ⚡ Memory increased (may be normal caching)\n");
        } else if self.bytes_delta < 0 {
            s.push_str("  ✅ Memory decreased (cleanup working)\n");
        } else {
            s.push_str("  ✅ No memory change\n");
        }

        s
    }
}

impl std::fmt::Display for SnapshotDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.report())
    }
}

// ============================================================================
// Snapshot Series
// ============================================================================

/// A series of snapshots over time.
pub struct SnapshotSeries {
    snapshots: Vec<MemorySnapshot>,
    max_snapshots: usize,
}

impl SnapshotSeries {
    /// Create a new series with max capacity.
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: Vec::with_capacity(max_snapshots),
            max_snapshots,
        }
    }

    /// Add a snapshot to the series.
    pub fn add(&mut self, snapshot: MemorySnapshot) {
        if self.snapshots.len() >= self.max_snapshots {
            self.snapshots.remove(0);
        }
        self.snapshots.push(snapshot);
    }

    /// Get all snapshots.
    pub fn snapshots(&self) -> &[MemorySnapshot] {
        &self.snapshots
    }

    /// Get the first snapshot.
    pub fn first(&self) -> Option<&MemorySnapshot> {
        self.snapshots.first()
    }

    /// Get the last snapshot.
    pub fn last(&self) -> Option<&MemorySnapshot> {
        self.snapshots.last()
    }

    /// Get the diff between first and last snapshots.
    pub fn total_diff(&self) -> Option<SnapshotDiff> {
        match (self.first(), self.last()) {
            (Some(first), Some(last)) if !std::ptr::eq(first, last) => Some(last.diff(first)),
            _ => None,
        }
    }

    /// Check for memory growth trend.
    pub fn has_growth_trend(&self) -> bool {
        if self.snapshots.len() < 3 {
            return false;
        }

        // Check if each snapshot has more memory than the previous
        let growing = self
            .snapshots
            .windows(2)
            .filter(|w| w[1].stats.current_bytes > w[0].stats.current_bytes)
            .count();

        // More than 70% growing = trend
        growing as f64 / (self.snapshots.len() - 1) as f64 > 0.7
    }

    /// Get memory growth rate (bytes per second).
    pub fn growth_rate(&self) -> f64 {
        if let Some(diff) = self.total_diff()
            && diff.time_delta.as_secs_f64() > 0.0
        {
            return diff.bytes_delta as f64 / diff.time_delta.as_secs_f64();
        }
        0.0
    }

    /// Clear all snapshots.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
}

impl Default for SnapshotSeries {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::super::allocation::AllocationTracker;
    use super::*;

    #[test]
    fn test_memory_snapshot() {
        let tracker = AllocationTracker::new();
        let snapshot = MemorySnapshot::capture(&tracker);

        assert!(snapshot.timestamp > 0);
    }

    #[test]
    fn test_snapshot_diff() {
        let diff = SnapshotDiff {
            time_delta: Duration::from_secs(10),
            allocations_delta: 100,
            deallocations_delta: 80,
            bytes_delta: 2000,
            peak_delta: 500,
            string_pool_delta: 5,
            buffer_pool_delta: -2,
            from_label: "start".to_string(),
            to_label: "end".to_string(),
        };

        assert_eq!(diff.net_allocations(), 20);
        assert!(diff.has_leaks());
    }

    #[test]
    fn test_snapshot_diff_no_leaks() {
        let diff = SnapshotDiff {
            time_delta: Duration::from_secs(10),
            allocations_delta: 100,
            deallocations_delta: 100,
            bytes_delta: 0,
            peak_delta: 0,
            string_pool_delta: 0,
            buffer_pool_delta: 0,
            from_label: String::new(),
            to_label: String::new(),
        };

        assert!(!diff.has_leaks());
    }

    #[test]
    fn test_snapshot_series() {
        let tracker = AllocationTracker::new();
        let mut series = SnapshotSeries::new(5);

        for i in 0..3 {
            let mut snap = MemorySnapshot::capture(&tracker);
            snap.label = format!("snap_{}", i);
            series.add(snap);
        }

        assert_eq!(series.snapshots().len(), 3);
        assert_eq!(series.first().unwrap().label, "snap_0");
        assert_eq!(series.last().unwrap().label, "snap_2");
    }

    #[test]
    fn test_snapshot_series_max_capacity() {
        let tracker = AllocationTracker::new();
        let mut series = SnapshotSeries::new(3);

        for i in 0..5 {
            let mut snap = MemorySnapshot::capture(&tracker);
            snap.label = format!("snap_{}", i);
            series.add(snap);
        }

        assert_eq!(series.snapshots().len(), 3);
        assert_eq!(series.first().unwrap().label, "snap_2");
        assert_eq!(series.last().unwrap().label, "snap_4");
    }

    #[test]
    fn test_snapshot_diff_report() {
        let diff = SnapshotDiff {
            time_delta: Duration::from_secs(10),
            allocations_delta: 100,
            deallocations_delta: 80,
            bytes_delta: 2000,
            peak_delta: 500,
            string_pool_delta: 5,
            buffer_pool_delta: -2,
            from_label: "before".to_string(),
            to_label: "after".to_string(),
        };

        let report = diff.report();
        assert!(report.contains("before"));
        assert!(report.contains("after"));
        assert!(report.contains("+2000 bytes"));
        assert!(report.contains("Potential memory leak"));
    }
}
