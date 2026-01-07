//! Benchmarks for async optimizations.
//!
//! Run with: `cargo bench --package prax-query --bench async_bench`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use prax_query::async_optimize::{
    concurrent::{ConcurrencyConfig, ConcurrentExecutor, execute_batch},
    introspect::{ConcurrentIntrospector, IntrospectionConfig, TableMetadata},
    pipeline::{BulkInsertPipeline, PipelineConfig, QueryPipeline, SimulatedExecutor},
};
use prax_query::filter::FilterValue;
use prax_query::sql::DatabaseType;
use std::time::Duration;
use tokio::runtime::Runtime;

fn create_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}

// ============================================================================
// Concurrent Executor Benchmarks
// ============================================================================

fn bench_concurrent_executor(c: &mut Criterion) {
    let rt = create_runtime();
    let mut group = c.benchmark_group("concurrent_executor");

    for num_tasks in [10, 50, 100, 500].iter() {
        group.throughput(Throughput::Elements(*num_tasks as u64));

        // Benchmark different concurrency limits
        for max_concurrency in [4, 8, 16].iter() {
            group.bench_with_input(
                BenchmarkId::new(
                    format!("tasks_{}_concurrency_{}", num_tasks, max_concurrency),
                    num_tasks,
                ),
                num_tasks,
                |b, &num_tasks| {
                    b.to_async(&rt).iter(|| async {
                        let config = ConcurrencyConfig::default()
                            .with_max_concurrency(*max_concurrency)
                            .without_timeout();

                        let executor = ConcurrentExecutor::new(config);

                        let tasks: Vec<_> = (0..num_tasks)
                            .map(|i| move || async move {
                                // Simulate minimal work
                                tokio::task::yield_now().await;
                                Ok::<_, String>(i * 2)
                            })
                            .collect();

                        let (results, _stats) = executor.execute_all(tasks).await;
                        black_box(results);
                    });
                },
            );
        }
    }

    group.finish();
}

fn bench_concurrent_with_latency(c: &mut Criterion) {
    let rt = create_runtime();
    let mut group = c.benchmark_group("concurrent_with_latency");
    group.sample_size(20);

    for latency_ms in [1, 5, 10].iter() {
        for num_tasks in [10, 50].iter() {
            group.bench_with_input(
                BenchmarkId::new(
                    format!("latency_{}ms_tasks_{}", latency_ms, num_tasks),
                    latency_ms,
                ),
                latency_ms,
                |b, &latency_ms| {
                    b.to_async(&rt).iter(|| async {
                        let config = ConcurrencyConfig::default()
                            .with_max_concurrency(16);

                        let executor = ConcurrentExecutor::new(config);

                        let tasks: Vec<_> = (0..*num_tasks)
                            .map(|i| {
                                let latency = Duration::from_millis(latency_ms as u64);
                                move || async move {
                                    tokio::time::sleep(latency).await;
                                    Ok::<_, String>(i)
                                }
                            })
                            .collect();

                        let (results, _) = executor.execute_all(tasks).await;
                        black_box(results);
                    });
                },
            );
        }
    }

    group.finish();
}

// ============================================================================
// Execute Batch Benchmarks
// ============================================================================

fn bench_execute_batch(c: &mut Criterion) {
    let rt = create_runtime();
    let mut group = c.benchmark_group("execute_batch");

    for num_items in [100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*num_items as u64));

        group.bench_with_input(
            BenchmarkId::new("items", num_items),
            num_items,
            |b, &num_items| {
                b.to_async(&rt).iter(|| async {
                    let items: Vec<i32> = (0..num_items).collect();

                    let results = execute_batch(
                        items,
                        8,
                        |item: i32| async move {
                            Ok::<_, String>(item * 2)
                        },
                    )
                    .await;

                    black_box(results);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Introspection Benchmarks
// ============================================================================

fn bench_concurrent_introspection(c: &mut Criterion) {
    let rt = create_runtime();
    let mut group = c.benchmark_group("concurrent_introspection");
    group.sample_size(20);

    for num_tables in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*num_tables as u64));

        // Sequential vs concurrent comparison
        group.bench_with_input(
            BenchmarkId::new("sequential", num_tables),
            num_tables,
            |b, &num_tables| {
                b.to_async(&rt).iter(|| async {
                    let table_names: Vec<String> =
                        (0..num_tables).map(|i| format!("table_{}", i)).collect();

                    let mut results = Vec::with_capacity(num_tables);
                    for name in table_names {
                        // Simulate introspection delay
                        tokio::time::sleep(Duration::from_micros(100)).await;
                        results.push(TableMetadata::new(name));
                    }

                    black_box(results);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("concurrent", num_tables),
            num_tables,
            |b, &num_tables| {
                b.to_async(&rt).iter(|| async {
                    let config = IntrospectionConfig::default().with_max_concurrency(8);
                    let introspector = ConcurrentIntrospector::new(config);

                    let table_names: Vec<String> =
                        (0..num_tables).map(|i| format!("table_{}", i)).collect();

                    let result = introspector
                        .introspect_tables(table_names, |name| async move {
                            // Simulate introspection delay
                            tokio::time::sleep(Duration::from_micros(100)).await;
                            Ok(TableMetadata::new(name))
                        })
                        .await;

                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Pipeline Benchmarks
// ============================================================================

fn bench_pipeline_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_construction");

    for num_queries in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*num_queries as u64));

        group.bench_with_input(
            BenchmarkId::new("build_pipeline", num_queries),
            num_queries,
            |b, &num_queries| {
                b.iter(|| {
                    let mut pipeline = QueryPipeline::new(PipelineConfig::default())
                        .for_database(DatabaseType::PostgreSQL);

                    for i in 0..num_queries {
                        pipeline = pipeline.add_insert(
                            "INSERT INTO users (name, age) VALUES ($1, $2)",
                            vec![
                                FilterValue::String(format!("User{}", i).into()),
                                FilterValue::Int(i as i64),
                            ],
                        );
                    }

                    black_box(pipeline);
                });
            },
        );
    }

    group.finish();
}

fn bench_bulk_insert_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_insert_pipeline");

    for num_rows in [100, 1000, 10000].iter() {
        group.throughput(Throughput::Elements(*num_rows as u64));

        group.bench_with_input(
            BenchmarkId::new("build_and_generate", num_rows),
            num_rows,
            |b, &num_rows| {
                b.iter(|| {
                    let mut pipeline = BulkInsertPipeline::new(
                        "users",
                        vec!["name".into(), "email".into(), "age".into()],
                    )
                    .with_batch_size(1000);

                    for i in 0..num_rows {
                        pipeline.add_row(vec![
                            FilterValue::String(format!("User{}", i).into()),
                            FilterValue::String(format!("user{}@example.com", i).into()),
                            FilterValue::Int((i % 100) as i64),
                        ]);
                    }

                    let statements = pipeline.to_insert_statements();
                    black_box(statements);
                });
            },
        );
    }

    group.finish();
}

fn bench_pipeline_execution_simulated(c: &mut Criterion) {
    let rt = create_runtime();
    let mut group = c.benchmark_group("pipeline_execution_simulated");
    group.sample_size(20);

    for num_queries in [50, 100, 500].iter() {
        group.throughput(Throughput::Elements(*num_queries as u64));

        // Compare sequential vs pipelined execution
        group.bench_with_input(
            BenchmarkId::new("sequential", num_queries),
            num_queries,
            |b, &num_queries| {
                b.to_async(&rt).iter(|| async {
                    let latency = Duration::from_micros(100);
                    let mut results = Vec::with_capacity(num_queries);

                    for i in 0..num_queries {
                        // Simulate full round-trip for each query
                        tokio::time::sleep(latency).await;
                        results.push(i);
                    }

                    black_box(results);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("pipelined", num_queries),
            num_queries,
            |b, &num_queries| {
                b.to_async(&rt).iter(|| async {
                    let executor = SimulatedExecutor::new(Duration::from_micros(100), 0.0);

                    let mut pipeline = QueryPipeline::new(PipelineConfig::default());

                    for i in 0..num_queries {
                        pipeline = pipeline.add_insert(
                            format!("INSERT INTO t VALUES ({})", i),
                            vec![],
                        );
                    }

                    let result = executor.execute(&pipeline).await;
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Combined Workload Benchmarks
// ============================================================================

fn bench_realistic_workload(c: &mut Criterion) {
    let rt = create_runtime();
    let mut group = c.benchmark_group("realistic_workload");
    group.sample_size(10);

    // Simulate a realistic introspection + migration scenario
    group.bench_function("introspect_30_tables", |b| {
        b.to_async(&rt).iter(|| async {
            let config = IntrospectionConfig::default().with_max_concurrency(8);
            let introspector = ConcurrentIntrospector::new(config);

            let table_names: Vec<String> = (0..30).map(|i| format!("table_{}", i)).collect();

            let result = introspector
                .introspect_tables(table_names, |name| async move {
                    // Simulate 5ms database query
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    Ok(TableMetadata::new(name))
                })
                .await;

            black_box(result);
        });
    });

    group.bench_function("bulk_insert_1000_rows", |b| {
        b.to_async(&rt).iter(|| async {
            let executor = SimulatedExecutor::new(Duration::from_millis(1), 0.0);

            let mut pipeline = BulkInsertPipeline::new(
                "users",
                vec!["name".into(), "email".into()],
            )
            .with_batch_size(100);

            for i in 0..1000 {
                pipeline.add_row(vec![
                    FilterValue::String(format!("User{}", i).into()),
                    FilterValue::String(format!("user{}@example.com", i).into()),
                ]);
            }

            // Convert to pipeline and execute
            let query_pipeline = pipeline.to_pipeline();
            let result = executor.execute(&query_pipeline).await;
            black_box(result);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_concurrent_executor,
    bench_concurrent_with_latency,
    bench_execute_batch,
    bench_concurrent_introspection,
    bench_pipeline_construction,
    bench_bulk_insert_pipeline,
    bench_pipeline_execution_simulated,
    bench_realistic_workload,
);

criterion_main!(benches);

