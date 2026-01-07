//! Memory profiling benchmarks.
//!
//! These benchmarks measure memory-related performance characteristics:
//! - Allocation overhead
//! - Memory pool efficiency
//! - Leak detection overhead
//! - Snapshot comparison performance

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use prax_query::filter::{Filter, FilterValue};
use prax_query::memory::{BufferPool, StringPool, GLOBAL_BUFFER_POOL, GLOBAL_STRING_POOL};
use prax_query::mem_optimize::{GlobalInterner, QueryArena, ScopedInterner};
use prax_query::profiling::{
    AllocationTracker, HeapProfiler, LeakDetector, MemoryProfiler, MemorySnapshot,
};
use std::time::Duration;

// ============================================================================
// Allocation Tracking Benchmarks
// ============================================================================

fn bench_allocation_tracking(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_tracking");

    // Measure overhead of tracking allocations
    group.bench_function("track_single_alloc", |b| {
        prax_query::profiling::enable_profiling();
        let tracker = AllocationTracker::new();

        b.iter(|| {
            tracker.record_alloc(black_box(0x1000), black_box(1024), black_box(8));
            tracker.record_dealloc(black_box(0x1000), black_box(1024));
        });

        prax_query::profiling::disable_profiling();
    });

    // Measure stats collection overhead
    group.bench_function("collect_stats", |b| {
        prax_query::profiling::enable_profiling();
        let tracker = AllocationTracker::new();

        // Pre-populate with some allocations
        for i in 0..100 {
            tracker.record_alloc(i * 0x1000, 1024, 8);
        }

        b.iter(|| {
            black_box(tracker.stats())
        });

        prax_query::profiling::disable_profiling();
    });

    // Measure histogram performance
    group.bench_function("size_histogram_record", |b| {
        let mut histogram = prax_query::profiling::allocation::SizeHistogram::new();

        b.iter(|| {
            histogram.record(black_box(1024));
        });
    });

    group.finish();
}

// ============================================================================
// Memory Snapshot Benchmarks
// ============================================================================

fn bench_memory_snapshots(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_snapshots");

    group.bench_function("capture_snapshot", |b| {
        let tracker = AllocationTracker::new();

        b.iter(|| {
            black_box(MemorySnapshot::capture(&tracker))
        });
    });

    group.bench_function("snapshot_diff", |b| {
        let tracker = AllocationTracker::new();
        let before = MemorySnapshot::capture(&tracker);

        // Simulate some allocations
        prax_query::profiling::enable_profiling();
        for i in 0..100 {
            tracker.record_alloc(i * 0x1000, 1024, 8);
        }
        prax_query::profiling::disable_profiling();

        let after = MemorySnapshot::capture(&tracker);

        b.iter(|| {
            black_box(after.diff(&before))
        });
    });

    group.bench_function("generate_diff_report", |b| {
        let tracker = AllocationTracker::new();
        let before = MemorySnapshot::capture(&tracker);
        let after = MemorySnapshot::capture(&tracker);
        let diff = after.diff(&before);

        b.iter(|| {
            black_box(diff.report())
        });
    });

    group.finish();
}

// ============================================================================
// Leak Detection Benchmarks
// ============================================================================

fn bench_leak_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("leak_detection");

    group.bench_function("analyze_empty", |b| {
        let detector = LeakDetector::new();
        let tracker = AllocationTracker::new();

        b.iter(|| {
            black_box(detector.analyze(&tracker))
        });
    });

    group.bench_function("analyze_with_allocations", |b| {
        prax_query::profiling::enable_profiling();
        let detector = LeakDetector::with_threshold(Duration::from_millis(1));
        let tracker = AllocationTracker::new();

        // Add some "old" allocations
        for i in 0..50 {
            tracker.record_alloc(i * 0x1000, 1024, 8);
        }
        // Wait a tiny bit so they're "old"
        std::thread::sleep(Duration::from_millis(2));

        b.iter(|| {
            black_box(detector.analyze(&tracker))
        });

        prax_query::profiling::disable_profiling();
    });

    group.bench_function("leak_report_generation", |b| {
        let report = prax_query::profiling::LeakReport {
            session_duration: Duration::from_secs(60),
            total_allocations: 1000,
            total_deallocations: 950,
            current_bytes: 50000,
            peak_bytes: 100000,
            old_allocations_count: 50,
            potential_leaks: vec![
                prax_query::profiling::PotentialLeak {
                    pattern: prax_query::profiling::leak_detector::LeakPattern::RepeatedSize {
                        size: 1024,
                        count: 10,
                    },
                    severity: prax_query::profiling::LeakSeverity::Medium,
                    total_bytes: 10240,
                    oldest_age: Duration::from_secs(120),
                    sample_backtrace: None,
                },
            ],
        };

        b.iter(|| {
            black_box(report.summary())
        });
    });

    group.finish();
}

// ============================================================================
// Heap Profiler Benchmarks
// ============================================================================

fn bench_heap_profiling(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap_profiling");

    group.bench_function("heap_sample", |b| {
        let profiler = HeapProfiler::new();

        b.iter(|| {
            black_box(profiler.sample())
        });
    });

    group.bench_function("heap_stats", |b| {
        let profiler = HeapProfiler::new();

        b.iter(|| {
            black_box(profiler.stats())
        });
    });

    group.bench_function("heap_report", |b| {
        let profiler = HeapProfiler::new();

        // Take some samples
        for _ in 0..10 {
            profiler.sample();
        }

        b.iter(|| {
            black_box(profiler.report())
        });
    });

    group.finish();
}

// ============================================================================
// Memory Pool Benchmarks
// ============================================================================

fn bench_memory_pools(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pools");

    // String pool benchmarks
    group.bench_function("string_pool_intern_new", |b| {
        let pool = StringPool::new();
        let mut counter = 0u64;

        b.iter(|| {
            counter += 1;
            let s = format!("field_{}", counter);
            black_box(pool.intern(&s))
        });
    });

    group.bench_function("string_pool_intern_existing", |b| {
        let pool = StringPool::new();
        pool.intern("test_field");

        b.iter(|| {
            black_box(pool.intern("test_field"))
        });
    });

    group.bench_function("global_string_pool", |b| {
        GLOBAL_STRING_POOL.intern("benchmark_field");

        b.iter(|| {
            black_box(GLOBAL_STRING_POOL.intern("benchmark_field"))
        });
    });

    // Buffer pool benchmarks
    group.bench_function("buffer_pool_get_return", |b| {
        b.iter(|| {
            let buf = GLOBAL_BUFFER_POOL.get();
            // Use the buffer
            black_box(&buf);
            // Buffer returned on drop
        });
    });

    group.bench_function("string_pool_stats", |b| {
        b.iter(|| {
            black_box(GLOBAL_STRING_POOL.stats())
        });
    });

    group.finish();
}

// ============================================================================
// Memory-Efficient Filter Building
// ============================================================================

fn bench_memory_efficient_filters(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_efficient_filters");

    // Compare standard vs interned filter building
    group.bench_function("standard_filter_chain", |b| {
        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Equals("user_id".into(), FilterValue::Int(1)),
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::Gt("created_at".into(), FilterValue::String("2024-01-01".into())),
            ]);
            black_box(filter.to_sql(0))
        });
    });

    group.bench_function("interned_filter_chain", |b| {
        let interner = GlobalInterner::get_instance();
        let user_id = interner.intern("user_id");
        let status = interner.intern("status");
        let created_at = interner.intern("created_at");

        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Equals(user_id.as_ref().into(), FilterValue::Int(1)),
                Filter::Equals(status.as_ref().into(), FilterValue::String("active".into())),
                Filter::Gt(created_at.as_ref().into(), FilterValue::String("2024-01-01".into())),
            ]);
            black_box(filter.to_sql(0))
        });
    });

    group.bench_function("arena_filter_chain", |b| {
        let arena = QueryArena::new();

        b.iter(|| {
            let filter = arena.scope(|s| {
                s.and(vec![
                    s.eq("user_id", 1),
                    s.eq("status", "active"),
                    s.gt("created_at", "2024-01-01"),
                ])
            });
            black_box(filter)
        });
    });

    group.finish();
}

// ============================================================================
// Memory Profiler Full Workflow
// ============================================================================

fn bench_profiler_workflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("profiler_workflow");

    group.bench_function("full_memory_report", |b| {
        let profiler = MemoryProfiler::new();

        b.iter(|| {
            black_box(profiler.report())
        });
    });

    group.bench_function("with_profiling_wrapper", |b| {
        b.iter(|| {
            let (result, _report) = prax_query::profiling::with_profiling(|| {
                // Simulate some work
                let v: Vec<i32> = (0..100).collect();
                v.len()
            });
            black_box(result)
        });
    });

    // Measure overhead of profiling
    group.bench_function("overhead_profiling_disabled", |b| {
        prax_query::profiling::disable_profiling();

        b.iter(|| {
            // Create some filters (profiling disabled)
            for i in 0..10 {
                let filter = Filter::Equals("id".into(), FilterValue::Int(i));
                black_box(filter.to_sql(0));
            }
        });
    });

    group.bench_function("overhead_profiling_enabled", |b| {
        prax_query::profiling::enable_profiling();

        b.iter(|| {
            // Create some filters (profiling enabled)
            for i in 0..10 {
                let filter = Filter::Equals("id".into(), FilterValue::Int(i));
                black_box(filter.to_sql(0));
            }
        });

        prax_query::profiling::disable_profiling();
    });

    group.finish();
}

// ============================================================================
// Throughput: Allocations per Second
// ============================================================================

fn bench_allocation_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_throughput");

    for count in [100, 500, 1000, 5000] {
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("vec_allocations", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let mut vecs = Vec::with_capacity(count);
                    for i in 0..count {
                        vecs.push(vec![i; 100]);
                    }
                    black_box(vecs)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("string_allocations", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    let mut strings = Vec::with_capacity(count);
                    for i in 0..count {
                        strings.push(format!("field_{}", i));
                    }
                    black_box(strings)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("interned_strings", count),
            &count,
            |b, &count| {
                let interner = ScopedInterner::new();

                b.iter(|| {
                    let mut interned = Vec::with_capacity(count);
                    for i in 0..count {
                        interned.push(interner.intern(&format!("field_{}", i % 50)));
                    }
                    black_box(interned)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_allocation_tracking,
    bench_memory_snapshots,
    bench_leak_detection,
    bench_heap_profiling,
    bench_memory_pools,
    bench_memory_efficient_filters,
    bench_profiler_workflow,
    bench_allocation_throughput,
);

criterion_main!(benches);

