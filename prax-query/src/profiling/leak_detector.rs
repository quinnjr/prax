//! Memory leak detection.
//!
//! Provides utilities for identifying potential memory leaks through
//! allocation tracking, age analysis, and pattern detection.

use super::allocation::{AllocationRecord, AllocationStats, AllocationTracker};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ============================================================================
// Leak Detector
// ============================================================================

/// Memory leak detector.
pub struct LeakDetector {
    /// Threshold age for considering an allocation "old".
    old_threshold: Duration,
    /// Minimum size to track.
    min_size: usize,
}

impl LeakDetector {
    /// Create a new leak detector with default settings.
    pub fn new() -> Self {
        Self {
            old_threshold: Duration::from_secs(60),
            min_size: 64,
        }
    }

    /// Create a leak detector with custom threshold.
    pub fn with_threshold(threshold: Duration) -> Self {
        Self {
            old_threshold: threshold,
            min_size: 64,
        }
    }

    /// Set the age threshold for old allocations.
    pub fn set_old_threshold(&mut self, threshold: Duration) {
        self.old_threshold = threshold;
    }

    /// Set minimum allocation size to track.
    pub fn set_min_size(&mut self, size: usize) {
        self.min_size = size;
    }

    /// Start a leak detection session.
    pub fn start(&self) -> LeakDetectorGuard<'_> {
        super::enable_profiling();
        LeakDetectorGuard {
            detector: self,
            start_time: Instant::now(),
            initial_stats: super::GLOBAL_TRACKER.stats(),
        }
    }

    /// Analyze allocations for potential leaks.
    pub fn analyze(&self, tracker: &AllocationTracker) -> LeakReport {
        let stats = tracker.stats();
        let old_allocations = tracker.old_allocations(self.old_threshold);

        // Group by size for pattern detection
        let mut by_size: HashMap<usize, Vec<&AllocationRecord>> = HashMap::new();
        for alloc in &old_allocations {
            by_size.entry(alloc.size).or_default().push(alloc);
        }

        // Identify potential leaks
        let mut potential_leaks = Vec::new();

        // Check for many allocations of same size (common leak pattern)
        for (size, allocs) in &by_size {
            if allocs.len() >= 3 {
                potential_leaks.push(PotentialLeak {
                    pattern: LeakPattern::RepeatedSize {
                        size: *size,
                        count: allocs.len(),
                    },
                    severity: if allocs.len() > 10 {
                        LeakSeverity::High
                    } else if allocs.len() > 5 {
                        LeakSeverity::Medium
                    } else {
                        LeakSeverity::Low
                    },
                    total_bytes: size * allocs.len(),
                    oldest_age: allocs.iter().map(|a| a.age()).max().unwrap_or_default(),
                    #[cfg(debug_assertions)]
                    sample_backtrace: allocs.first().and_then(|a| a.backtrace.clone()),
                    #[cfg(not(debug_assertions))]
                    sample_backtrace: None,
                });
            }
        }

        // Check for very old allocations
        for alloc in &old_allocations {
            if alloc.age() > self.old_threshold * 5 {
                potential_leaks.push(PotentialLeak {
                    pattern: LeakPattern::VeryOld {
                        age: alloc.age(),
                        size: alloc.size,
                    },
                    severity: LeakSeverity::High,
                    total_bytes: alloc.size,
                    oldest_age: alloc.age(),
                    #[cfg(debug_assertions)]
                    sample_backtrace: alloc.backtrace.clone(),
                    #[cfg(not(debug_assertions))]
                    sample_backtrace: None,
                });
            }
        }

        // Check for growing allocation count
        if stats.net_allocations() > 100 {
            potential_leaks.push(PotentialLeak {
                pattern: LeakPattern::GrowingCount {
                    net_allocations: stats.net_allocations(),
                },
                severity: if stats.net_allocations() > 1000 {
                    LeakSeverity::High
                } else if stats.net_allocations() > 500 {
                    LeakSeverity::Medium
                } else {
                    LeakSeverity::Low
                },
                total_bytes: stats.current_bytes,
                oldest_age: Duration::default(),
                sample_backtrace: None,
            });
        }

        // Sort by severity (highest first).
        potential_leaks.sort_by_key(|leak| std::cmp::Reverse(leak.severity));

        LeakReport {
            session_duration: stats.uptime,
            total_allocations: stats.total_allocations,
            total_deallocations: stats.total_deallocations,
            current_bytes: stats.current_bytes,
            peak_bytes: stats.peak_bytes,
            old_allocations_count: old_allocations.len(),
            potential_leaks,
        }
    }

    /// Finish a leak detection session and return the report.
    pub fn finish(&self) -> LeakReport {
        self.analyze(&super::GLOBAL_TRACKER)
    }
}

impl Default for LeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Leak Detector Guard
// ============================================================================

/// RAII guard for leak detection sessions.
pub struct LeakDetectorGuard<'a> {
    detector: &'a LeakDetector,
    start_time: Instant,
    initial_stats: AllocationStats,
}

impl<'a> LeakDetectorGuard<'a> {
    /// Get the current leak report.
    pub fn current_report(&self) -> LeakReport {
        self.detector.analyze(&super::GLOBAL_TRACKER)
    }

    /// Get the delta from start.
    pub fn delta(&self) -> AllocationDelta {
        let current = super::GLOBAL_TRACKER.stats();
        AllocationDelta {
            allocations_delta: current.total_allocations as i64
                - self.initial_stats.total_allocations as i64,
            deallocations_delta: current.total_deallocations as i64
                - self.initial_stats.total_deallocations as i64,
            bytes_delta: current.current_bytes as i64 - self.initial_stats.current_bytes as i64,
            duration: self.start_time.elapsed(),
        }
    }
}

impl Drop for LeakDetectorGuard<'_> {
    fn drop(&mut self) {
        super::disable_profiling();
    }
}

// ============================================================================
// Leak Report
// ============================================================================

/// Report from leak detection analysis.
#[derive(Debug, Clone)]
pub struct LeakReport {
    /// Duration of the detection session.
    pub session_duration: Duration,
    /// Total allocations during session.
    pub total_allocations: u64,
    /// Total deallocations during session.
    pub total_deallocations: u64,
    /// Current bytes allocated.
    pub current_bytes: usize,
    /// Peak bytes allocated.
    pub peak_bytes: usize,
    /// Number of old allocations found.
    pub old_allocations_count: usize,
    /// Identified potential leaks.
    pub potential_leaks: Vec<PotentialLeak>,
}

impl LeakReport {
    /// Check if any potential leaks were detected.
    pub fn has_leaks(&self) -> bool {
        !self.potential_leaks.is_empty()
    }

    /// Check if any high-severity leaks were detected.
    pub fn has_high_severity_leaks(&self) -> bool {
        self.potential_leaks
            .iter()
            .any(|l| l.severity == LeakSeverity::High)
    }

    /// Get total bytes potentially leaked.
    pub fn total_leaked_bytes(&self) -> usize {
        self.potential_leaks.iter().map(|l| l.total_bytes).sum()
    }

    /// Generate a summary string.
    pub fn summary(&self) -> String {
        let mut s = String::new();

        if self.potential_leaks.is_empty() {
            s.push_str("  No potential leaks detected\n");
            return s;
        }

        for (i, leak) in self.potential_leaks.iter().enumerate() {
            s.push_str(&format!(
                "  {}. [{:?}] {}\n",
                i + 1,
                leak.severity,
                leak.pattern.description()
            ));
            s.push_str(&format!(
                "     Total bytes: {} ({:.2} KB)\n",
                leak.total_bytes,
                leak.total_bytes as f64 / 1024.0
            ));

            if let Some(bt) = &leak.sample_backtrace {
                s.push_str("     Sample backtrace:\n");
                // Show first few frames
                for line in bt.lines().take(10) {
                    s.push_str(&format!("       {}\n", line));
                }
            }
        }

        s
    }
}

impl std::fmt::Display for LeakReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Leak Detection Report ===")?;
        writeln!(f, "Session duration: {:?}", self.session_duration)?;
        writeln!(
            f,
            "Allocations: {} / Deallocations: {}",
            self.total_allocations, self.total_deallocations
        )?;
        writeln!(
            f,
            "Current bytes: {} / Peak bytes: {}",
            self.current_bytes, self.peak_bytes
        )?;
        writeln!(f, "Old allocations: {}", self.old_allocations_count)?;
        writeln!(f)?;

        if self.has_leaks() {
            writeln!(
                f,
                "⚠️  {} potential leak(s) detected:",
                self.potential_leaks.len()
            )?;
            write!(f, "{}", self.summary())?;
        } else {
            writeln!(f, "✅ No potential leaks detected")?;
        }

        Ok(())
    }
}

// ============================================================================
// Potential Leak
// ============================================================================

/// A potential memory leak.
#[derive(Debug, Clone)]
pub struct PotentialLeak {
    /// Pattern that indicates this leak.
    pub pattern: LeakPattern,
    /// Severity assessment.
    pub severity: LeakSeverity,
    /// Total bytes involved.
    pub total_bytes: usize,
    /// Age of oldest allocation.
    pub oldest_age: Duration,
    /// Sample backtrace (if available).
    pub sample_backtrace: Option<String>,
}

// ============================================================================
// Leak Pattern
// ============================================================================

/// Pattern indicating a potential leak.
#[derive(Debug, Clone)]
pub enum LeakPattern {
    /// Many allocations of the same size.
    RepeatedSize { size: usize, count: usize },
    /// Very old allocation that was never freed.
    VeryOld { age: Duration, size: usize },
    /// Growing count of allocations over time.
    GrowingCount { net_allocations: i64 },
    /// Large single allocation held too long.
    LargeOld { size: usize, age: Duration },
}

impl LeakPattern {
    /// Get a description of the pattern.
    pub fn description(&self) -> String {
        match self {
            LeakPattern::RepeatedSize { size, count } => {
                format!("{} allocations of {} bytes each", count, size)
            }
            LeakPattern::VeryOld { age, size } => {
                format!("{} byte allocation held for {:?}", size, age)
            }
            LeakPattern::GrowingCount { net_allocations } => {
                format!("{} net allocations (allocs - deallocs)", net_allocations)
            }
            LeakPattern::LargeOld { size, age } => {
                format!("Large {} byte allocation held for {:?}", size, age)
            }
        }
    }
}

// ============================================================================
// Leak Severity
// ============================================================================

/// Severity of a potential leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LeakSeverity {
    /// Low severity - may be intentional caching.
    Low,
    /// Medium severity - warrants investigation.
    Medium,
    /// High severity - likely a leak.
    High,
}

// ============================================================================
// Allocation Delta
// ============================================================================

/// Change in allocations over time.
#[derive(Debug, Clone)]
pub struct AllocationDelta {
    /// Change in allocation count.
    pub allocations_delta: i64,
    /// Change in deallocation count.
    pub deallocations_delta: i64,
    /// Change in bytes allocated.
    pub bytes_delta: i64,
    /// Duration of the delta period.
    pub duration: Duration,
}

impl AllocationDelta {
    /// Check if memory grew.
    pub fn memory_grew(&self) -> bool {
        self.bytes_delta > 0
    }

    /// Get allocation rate per second.
    pub fn allocation_rate(&self) -> f64 {
        if self.duration.as_secs_f64() > 0.0 {
            self.allocations_delta as f64 / self.duration.as_secs_f64()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leak_detector_new() {
        let detector = LeakDetector::new();
        assert_eq!(detector.old_threshold, Duration::from_secs(60));
        assert_eq!(detector.min_size, 64);
    }

    #[test]
    fn test_leak_pattern_description() {
        let pattern = LeakPattern::RepeatedSize {
            size: 1024,
            count: 10,
        };
        assert!(pattern.description().contains("10 allocations"));
        assert!(pattern.description().contains("1024 bytes"));

        let pattern = LeakPattern::VeryOld {
            age: Duration::from_secs(300),
            size: 2048,
        };
        assert!(pattern.description().contains("2048 byte"));
    }

    #[test]
    fn test_leak_severity_ordering() {
        assert!(LeakSeverity::High > LeakSeverity::Medium);
        assert!(LeakSeverity::Medium > LeakSeverity::Low);
    }

    #[test]
    fn test_leak_report_empty() {
        let report = LeakReport {
            session_duration: Duration::from_secs(10),
            total_allocations: 100,
            total_deallocations: 100,
            current_bytes: 0,
            peak_bytes: 1000,
            old_allocations_count: 0,
            potential_leaks: vec![],
        };

        assert!(!report.has_leaks());
        assert!(!report.has_high_severity_leaks());
        assert_eq!(report.total_leaked_bytes(), 0);
    }

    #[test]
    fn test_leak_report_with_leaks() {
        let report = LeakReport {
            session_duration: Duration::from_secs(10),
            total_allocations: 100,
            total_deallocations: 50,
            current_bytes: 5000,
            peak_bytes: 6000,
            old_allocations_count: 5,
            potential_leaks: vec![
                PotentialLeak {
                    pattern: LeakPattern::RepeatedSize {
                        size: 1024,
                        count: 5,
                    },
                    severity: LeakSeverity::Medium,
                    total_bytes: 5120,
                    oldest_age: Duration::from_secs(120),
                    sample_backtrace: None,
                },
                PotentialLeak {
                    pattern: LeakPattern::GrowingCount {
                        net_allocations: 50,
                    },
                    severity: LeakSeverity::High,
                    total_bytes: 5000,
                    oldest_age: Duration::default(),
                    sample_backtrace: None,
                },
            ],
        };

        assert!(report.has_leaks());
        assert!(report.has_high_severity_leaks());
        assert_eq!(report.total_leaked_bytes(), 10120);
    }

    #[test]
    fn test_allocation_delta() {
        let delta = AllocationDelta {
            allocations_delta: 100,
            deallocations_delta: 50,
            bytes_delta: 5000,
            duration: Duration::from_secs(10),
        };

        assert!(delta.memory_grew());
        assert_eq!(delta.allocation_rate(), 10.0);
    }
}
