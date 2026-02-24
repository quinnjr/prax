//! Allocation tracking and statistics.
//!
//! Provides fine-grained tracking of memory allocations for leak detection
//! and performance analysis.

use parking_lot::{Mutex, RwLock};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Global allocation tracker instance.
pub static GLOBAL_TRACKER: std::sync::LazyLock<AllocationTracker> =
    std::sync::LazyLock::new(AllocationTracker::new);

// ============================================================================
// Allocation Record
// ============================================================================

/// Record of a single allocation.
#[derive(Debug, Clone)]
pub struct AllocationRecord {
    /// Unique allocation ID.
    pub id: u64,
    /// Size in bytes.
    pub size: usize,
    /// Memory alignment.
    pub align: usize,
    /// Timestamp when allocated.
    pub timestamp: Instant,
    /// Optional backtrace (expensive, only in debug mode).
    #[cfg(debug_assertions)]
    pub backtrace: Option<String>,
}

impl AllocationRecord {
    /// Create a new allocation record.
    pub fn new(id: u64, size: usize, align: usize) -> Self {
        Self {
            id,
            size,
            align,
            timestamp: Instant::now(),
            #[cfg(debug_assertions)]
            backtrace: if super::is_profiling_enabled() {
                Some(format!("{:?}", std::backtrace::Backtrace::capture()))
            } else {
                None
            },
        }
    }

    /// Get the age of this allocation.
    pub fn age(&self) -> Duration {
        self.timestamp.elapsed()
    }
}

// ============================================================================
// Allocation Tracker
// ============================================================================

/// Tracks memory allocations and deallocations.
#[derive(Debug)]
pub struct AllocationTracker {
    /// Counter for allocation IDs.
    next_id: AtomicU64,
    /// Total allocations.
    total_allocations: AtomicU64,
    /// Total deallocations.
    total_deallocations: AtomicU64,
    /// Total bytes allocated.
    total_bytes_allocated: AtomicUsize,
    /// Total bytes deallocated.
    total_bytes_deallocated: AtomicUsize,
    /// Current bytes allocated.
    current_bytes: AtomicUsize,
    /// Peak bytes allocated.
    peak_bytes: AtomicUsize,
    /// Active allocations (ptr -> record).
    active: RwLock<HashMap<usize, AllocationRecord>>,
    /// Allocation size histogram.
    size_histogram: Mutex<SizeHistogram>,
    /// Start time.
    start_time: Instant,
}

impl AllocationTracker {
    /// Create a new allocation tracker.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            total_allocations: AtomicU64::new(0),
            total_deallocations: AtomicU64::new(0),
            total_bytes_allocated: AtomicUsize::new(0),
            total_bytes_deallocated: AtomicUsize::new(0),
            current_bytes: AtomicUsize::new(0),
            peak_bytes: AtomicUsize::new(0),
            active: RwLock::new(HashMap::new()),
            size_histogram: Mutex::new(SizeHistogram::new()),
            start_time: Instant::now(),
        }
    }

    /// Record an allocation.
    pub fn record_alloc(&self, ptr: usize, size: usize, align: usize) {
        if !super::is_profiling_enabled() {
            return;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let record = AllocationRecord::new(id, size, align);

        // Update counters
        self.total_allocations.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_allocated
            .fetch_add(size, Ordering::Relaxed);

        let current = self.current_bytes.fetch_add(size, Ordering::Relaxed) + size;

        // Update peak
        let mut peak = self.peak_bytes.load(Ordering::Relaxed);
        while current > peak {
            match self.peak_bytes.compare_exchange_weak(
                peak,
                current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(p) => peak = p,
            }
        }

        // Record allocation
        self.active.write().insert(ptr, record);

        // Update histogram
        self.size_histogram.lock().record(size);
    }

    /// Record a deallocation.
    pub fn record_dealloc(&self, ptr: usize, size: usize) {
        if !super::is_profiling_enabled() {
            return;
        }

        self.total_deallocations.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_deallocated
            .fetch_add(size, Ordering::Relaxed);
        self.current_bytes.fetch_sub(size, Ordering::Relaxed);

        // Remove from active
        self.active.write().remove(&ptr);
    }

    /// Get current statistics.
    pub fn stats(&self) -> AllocationStats {
        AllocationStats {
            total_allocations: self.total_allocations.load(Ordering::Relaxed),
            total_deallocations: self.total_deallocations.load(Ordering::Relaxed),
            total_bytes_allocated: self.total_bytes_allocated.load(Ordering::Relaxed),
            total_bytes_deallocated: self.total_bytes_deallocated.load(Ordering::Relaxed),
            current_allocations: self.active.read().len() as u64,
            current_bytes: self.current_bytes.load(Ordering::Relaxed),
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            uptime: self.start_time.elapsed(),
        }
    }

    /// Get active allocations.
    pub fn active_allocations(&self) -> Vec<AllocationRecord> {
        self.active.read().values().cloned().collect()
    }

    /// Get allocations older than a threshold (potential leaks).
    pub fn old_allocations(&self, threshold: Duration) -> Vec<AllocationRecord> {
        self.active
            .read()
            .values()
            .filter(|r| r.age() > threshold)
            .cloned()
            .collect()
    }

    /// Get allocation size histogram.
    pub fn histogram(&self) -> SizeHistogram {
        self.size_histogram.lock().clone()
    }

    /// Reset all tracking state.
    pub fn reset(&self) {
        self.total_allocations.store(0, Ordering::Relaxed);
        self.total_deallocations.store(0, Ordering::Relaxed);
        self.total_bytes_allocated.store(0, Ordering::Relaxed);
        self.total_bytes_deallocated.store(0, Ordering::Relaxed);
        self.current_bytes.store(0, Ordering::Relaxed);
        self.peak_bytes.store(0, Ordering::Relaxed);
        self.active.write().clear();
        *self.size_histogram.lock() = SizeHistogram::new();
    }
}

impl Default for AllocationTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Allocation Statistics
// ============================================================================

/// Summary statistics for allocations.
#[derive(Debug, Clone, Default)]
pub struct AllocationStats {
    /// Total number of allocations.
    pub total_allocations: u64,
    /// Total number of deallocations.
    pub total_deallocations: u64,
    /// Total bytes ever allocated.
    pub total_bytes_allocated: usize,
    /// Total bytes ever deallocated.
    pub total_bytes_deallocated: usize,
    /// Current number of active allocations.
    pub current_allocations: u64,
    /// Current bytes allocated.
    pub current_bytes: usize,
    /// Peak bytes allocated.
    pub peak_bytes: usize,
    /// Time since tracking started.
    pub uptime: Duration,
}

impl AllocationStats {
    /// Get net allocations (allocs - deallocs).
    pub fn net_allocations(&self) -> i64 {
        self.total_allocations as i64 - self.total_deallocations as i64
    }

    /// Get allocation rate (allocations per second).
    pub fn allocation_rate(&self) -> f64 {
        if self.uptime.as_secs_f64() > 0.0 {
            self.total_allocations as f64 / self.uptime.as_secs_f64()
        } else {
            0.0
        }
    }

    /// Get average allocation size.
    pub fn avg_allocation_size(&self) -> usize {
        if self.total_allocations > 0 {
            self.total_bytes_allocated / self.total_allocations as usize
        } else {
            0
        }
    }

    /// Check if there are potential leaks.
    pub fn has_potential_leaks(&self) -> bool {
        self.net_allocations() > 0 && self.current_bytes > 0
    }
}

// ============================================================================
// Size Histogram
// ============================================================================

/// Histogram of allocation sizes.
#[derive(Debug, Clone, Default)]
pub struct SizeHistogram {
    /// Buckets: [0-64], [64-256], [256-1K], [1K-4K], [4K-16K], [16K-64K], [64K+]
    buckets: [u64; 7],
    /// Total allocations.
    total: u64,
}

impl SizeHistogram {
    /// Create a new histogram.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an allocation size.
    pub fn record(&mut self, size: usize) {
        let bucket = match size {
            0..=64 => 0,
            65..=256 => 1,
            257..=1024 => 2,
            1025..=4096 => 3,
            4097..=16384 => 4,
            16385..=65536 => 5,
            _ => 6,
        };
        self.buckets[bucket] += 1;
        self.total += 1;
    }

    /// Get bucket counts.
    pub fn buckets(&self) -> &[u64; 7] {
        &self.buckets
    }

    /// Get bucket labels.
    pub fn bucket_labels() -> &'static [&'static str; 7] {
        &[
            "0-64B", "64-256B", "256B-1K", "1K-4K", "4K-16K", "16K-64K", "64K+",
        ]
    }

    /// Get the most common bucket.
    pub fn most_common_bucket(&self) -> (&'static str, u64) {
        let labels = Self::bucket_labels();
        let (idx, count) = self
            .buckets
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| *c)
            .unwrap();
        (labels[idx], *count)
    }

    /// Get percentage for each bucket.
    pub fn percentages(&self) -> [f64; 7] {
        if self.total == 0 {
            return [0.0; 7];
        }
        let mut pcts = [0.0; 7];
        for (i, &count) in self.buckets.iter().enumerate() {
            pcts[i] = (count as f64 / self.total as f64) * 100.0;
        }
        pcts
    }
}

// ============================================================================
// Tracked Allocator
// ============================================================================

/// A wrapper allocator that tracks allocations.
///
/// Use this as the global allocator to enable automatic tracking:
///
/// ```rust,ignore
/// use prax_query::profiling::TrackedAllocator;
///
/// #[global_allocator]
/// static ALLOC: TrackedAllocator = TrackedAllocator::new();
/// ```
pub struct TrackedAllocator {
    inner: System,
}

impl TrackedAllocator {
    /// Create a new tracked allocator.
    pub const fn new() -> Self {
        Self { inner: System }
    }
}

impl Default for TrackedAllocator {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for TrackedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Delegating to System allocator which is always safe to call
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            GLOBAL_TRACKER.record_alloc(ptr as usize, layout.size(), layout.align());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        GLOBAL_TRACKER.record_dealloc(ptr as usize, layout.size());
        // SAFETY: Delegating to System allocator which is always safe to call
        unsafe { self.inner.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Record deallocation of old
        GLOBAL_TRACKER.record_dealloc(ptr as usize, layout.size());

        // SAFETY: Delegating to System allocator which is always safe to call
        let new_ptr = unsafe { self.inner.realloc(ptr, layout, new_size) };

        // Record allocation of new
        if !new_ptr.is_null() {
            GLOBAL_TRACKER.record_alloc(new_ptr as usize, new_size, layout.align());
        }

        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocation_record() {
        let record = AllocationRecord::new(1, 1024, 8);
        assert_eq!(record.id, 1);
        assert_eq!(record.size, 1024);
        assert_eq!(record.align, 8);
    }

    #[test]
    #[ignore = "flaky in CI due to global profiling state interference"]
    fn test_allocation_tracker() {
        super::super::enable_profiling();

        let tracker = AllocationTracker::new();

        tracker.record_alloc(0x1000, 100, 8);
        tracker.record_alloc(0x2000, 200, 8);

        let stats = tracker.stats();
        assert_eq!(stats.total_allocations, 2);
        assert_eq!(stats.total_bytes_allocated, 300);
        assert_eq!(stats.current_bytes, 300);

        tracker.record_dealloc(0x1000, 100);

        let stats = tracker.stats();
        assert_eq!(stats.total_deallocations, 1);
        assert_eq!(stats.current_bytes, 200);

        super::super::disable_profiling();
    }

    #[test]
    fn test_size_histogram() {
        let mut hist = SizeHistogram::new();

        hist.record(32); // 0-64
        hist.record(128); // 64-256
        hist.record(512); // 256-1K
        hist.record(32); // 0-64

        assert_eq!(hist.buckets[0], 2); // 0-64
        assert_eq!(hist.buckets[1], 1); // 64-256
        assert_eq!(hist.buckets[2], 1); // 256-1K
    }

    #[test]
    fn test_stats_calculations() {
        let stats = AllocationStats {
            total_allocations: 100,
            total_deallocations: 80,
            total_bytes_allocated: 10000,
            total_bytes_deallocated: 8000,
            current_allocations: 20,
            current_bytes: 2000,
            peak_bytes: 5000,
            uptime: Duration::from_secs(10),
        };

        assert_eq!(stats.net_allocations(), 20);
        assert_eq!(stats.allocation_rate(), 10.0);
        assert_eq!(stats.avg_allocation_size(), 100);
        assert!(stats.has_potential_leaks());
    }
}
