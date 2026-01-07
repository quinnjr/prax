//! DuckDB operations benchmarks.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use prax_duckdb::{DuckDbConfig, DuckDbEngine, DuckDbPool};
use prax_query::filter::FilterValue;
use std::collections::HashMap;
use tokio::runtime::Runtime;

fn create_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn setup_engine() -> DuckDbEngine {
    let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
    let engine = DuckDbEngine::new(pool);

    engine
        .raw_sql_batch(
            r#"
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                name VARCHAR NOT NULL,
                email VARCHAR NOT NULL,
                age INTEGER,
                active BOOLEAN DEFAULT true,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                user_id INTEGER NOT NULL,
                total DECIMAL(10,2) NOT NULL,
                status VARCHAR DEFAULT 'pending',
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            -- Insert test data
            INSERT INTO users (id, name, email, age, active)
            SELECT
                i,
                'User ' || i,
                'user' || i || '@example.com',
                20 + (i % 50),
                i % 2 = 0
            FROM generate_series(1, 10000) AS t(i);

            INSERT INTO orders (id, user_id, total, status)
            SELECT
                i,
                (i % 10000) + 1,
                (random() * 1000)::DECIMAL(10,2),
                CASE i % 4
                    WHEN 0 THEN 'completed'
                    WHEN 1 THEN 'pending'
                    WHEN 2 THEN 'shipped'
                    ELSE 'cancelled'
                END
            FROM generate_series(1, 50000) AS t(i);
        "#,
        )
        .await
        .unwrap();

    engine
}

fn bench_simple_select(c: &mut Criterion) {
    let rt = create_runtime();
    let engine = rt.block_on(setup_engine());

    c.bench_function("duckdb_select_by_id", |b| {
        b.to_async(&rt).iter(|| async {
            let mut filters = HashMap::new();
            filters.insert("id".to_string(), FilterValue::Int(500));

            black_box(
                engine
                    .query_one("users", &[], &filters)
                    .await
                    .unwrap()
            )
        })
    });
}

fn bench_aggregation(c: &mut Criterion) {
    let rt = create_runtime();
    let engine = rt.block_on(setup_engine());

    let mut group = c.benchmark_group("duckdb_aggregation");

    group.bench_function("count_all", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(engine.count("users", &HashMap::new()).await.unwrap())
        })
    });

    group.bench_function("sum_with_groupby", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                engine
                    .execute_raw(
                        "SELECT status, COUNT(*), SUM(total) FROM orders GROUP BY status",
                        &[],
                    )
                    .await
                    .unwrap()
            )
        })
    });

    group.bench_function("avg_with_filter", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                engine
                    .execute_raw(
                        "SELECT AVG(age) FROM users WHERE active = true",
                        &[],
                    )
                    .await
                    .unwrap()
            )
        })
    });

    group.finish();
}

fn bench_window_functions(c: &mut Criterion) {
    let rt = create_runtime();
    let engine = rt.block_on(setup_engine());

    let mut group = c.benchmark_group("duckdb_window_functions");

    group.bench_function("row_number", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                engine
                    .execute_raw(
                        r#"
                        SELECT
                            id,
                            total,
                            ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY total DESC) as rank
                        FROM orders
                        LIMIT 1000
                        "#,
                        &[],
                    )
                    .await
                    .unwrap()
            )
        })
    });

    group.bench_function("running_total", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                engine
                    .execute_raw(
                        r#"
                        SELECT
                            id,
                            total,
                            SUM(total) OVER (
                                PARTITION BY user_id
                                ORDER BY id
                                ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                            ) as running_total
                        FROM orders
                        WHERE user_id <= 100
                        "#,
                        &[],
                    )
                    .await
                    .unwrap()
            )
        })
    });

    group.finish();
}

fn bench_joins(c: &mut Criterion) {
    let rt = create_runtime();
    let engine = rt.block_on(setup_engine());

    let mut group = c.benchmark_group("duckdb_joins");

    group.bench_function("inner_join", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                engine
                    .execute_raw(
                        r#"
                        SELECT u.name, o.total, o.status
                        FROM users u
                        INNER JOIN orders o ON u.id = o.user_id
                        WHERE u.id <= 100
                        "#,
                        &[],
                    )
                    .await
                    .unwrap()
            )
        })
    });

    group.bench_function("left_join_with_agg", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(
                engine
                    .execute_raw(
                        r#"
                        SELECT u.name, COUNT(o.id) as order_count, SUM(o.total) as total_spent
                        FROM users u
                        LEFT JOIN orders o ON u.id = o.user_id
                        WHERE u.id <= 1000
                        GROUP BY u.id, u.name
                        "#,
                        &[],
                    )
                    .await
                    .unwrap()
            )
        })
    });

    group.finish();
}

fn bench_throughput(c: &mut Criterion) {
    let rt = create_runtime();
    let engine = rt.block_on(setup_engine());

    let mut group = c.benchmark_group("duckdb_throughput");
    group.throughput(Throughput::Elements(1000));

    group.bench_function("1000_simple_queries", |b| {
        b.to_async(&rt).iter(|| async {
            for i in 1..=1000 {
                let mut filters = HashMap::new();
                filters.insert("id".to_string(), FilterValue::Int(i));
                black_box(engine.query_optional("users", &[], &filters).await.unwrap());
            }
        })
    });

    group.finish();
}

fn bench_bulk_insert(c: &mut Criterion) {
    let rt = create_runtime();

    c.bench_function("duckdb_bulk_insert_1000", |b| {
        b.to_async(&rt).iter(|| async {
            let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
            let engine = DuckDbEngine::new(pool);

            engine
                .raw_sql_batch("CREATE TABLE bench_insert (id INTEGER, value VARCHAR);")
                .await
                .unwrap();

            // Use DuckDB's efficient bulk insert
            engine
                .raw_sql_batch(
                    r#"
                    INSERT INTO bench_insert
                    SELECT i, 'value_' || i
                    FROM generate_series(1, 1000) AS t(i);
                    "#,
                )
                .await
                .unwrap();

            black_box(engine.count("bench_insert", &HashMap::new()).await.unwrap())
        })
    });
}

criterion_group!(
    benches,
    bench_simple_select,
    bench_aggregation,
    bench_window_functions,
    bench_joins,
    bench_throughput,
    bench_bulk_insert,
);

criterion_main!(benches);

