//! Benchmarks for ScyllaDB operations.
//!
//! Note: These benchmarks require a running ScyllaDB instance.
//! Skip by default; run with: cargo bench --package prax-scylladb --features bench-scylla

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::hint::black_box;

/// Benchmark configuration parsing
fn bench_config_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("scylladb_config");

    let urls = [
        "scylla://localhost:9042/keyspace",
        "scylla://user:pass@localhost:9042/keyspace",
        "scylla://node1:9042,node2:9042,node3:9042/keyspace",
        "scylla://localhost/ks?timeout=30&pool_size=16&consistency=LOCAL_QUORUM",
    ];

    for url in urls.iter() {
        group.bench_with_input(
            BenchmarkId::new("from_url", url.len()),
            url,
            |b, url| {
                b.iter(|| {
                    let config = prax_scylladb::ScyllaConfig::from_url(black_box(url));
                    black_box(config)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark config builder
fn bench_config_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("scylladb_builder");

    group.bench_function("simple_config", |b| {
        b.iter(|| {
            let config = prax_scylladb::ScyllaConfig::builder()
                .known_nodes(["localhost:9042"])
                .default_keyspace("test")
                .build();
            black_box(config)
        })
    });

    group.bench_function("full_config", |b| {
        b.iter(|| {
            let config = prax_scylladb::ScyllaConfig::builder()
                .known_nodes(["node1:9042", "node2:9042", "node3:9042"])
                .default_keyspace("production")
                .username("admin")
                .password("secret")
                .connection_timeout_secs(10)
                .request_timeout_secs(30)
                .pool_size(8)
                .local_datacenter("dc1")
                .ssl_enabled(true)
                .compression("lz4")
                .consistency(prax_scylladb::config::ConsistencyLevel::LocalQuorum)
                .build();
            black_box(config)
        })
    });

    group.finish();
}

/// Benchmark type conversions
fn bench_type_conversions(c: &mut Criterion) {
    use prax_scylladb::types::ToCqlValue;
    use prax_query::filter::FilterValue;

    let mut group = c.benchmark_group("scylladb_types");

    group.bench_function("int_to_cql", |b| {
        let value = FilterValue::Int(42);
        b.iter(|| {
            let cql = black_box(&value).to_cql();
            black_box(cql)
        })
    });

    group.bench_function("string_to_cql", |b| {
        let value = FilterValue::String("hello world".into());
        b.iter(|| {
            let cql = black_box(&value).to_cql();
            black_box(cql)
        })
    });

    group.bench_function("array_to_cql", |b| {
        let value = FilterValue::Array(vec![
            FilterValue::Int(1),
            FilterValue::Int(2),
            FilterValue::Int(3),
            FilterValue::Int(4),
            FilterValue::Int(5),
        ]);
        b.iter(|| {
            let cql = black_box(&value).to_cql();
            black_box(cql)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_config_parsing,
    bench_config_builder,
    bench_type_conversions,
);

criterion_main!(benches);

