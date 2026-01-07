//! Benchmarks for SQLite operations.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_query::filter::FilterValue;
use prax_sqlite::{DatabasePath, SqliteConfig, SqliteEngine, SqlitePool};

/// Counter for unique email addresses.
static EMAIL_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Create a test database with sample data using a temp file.
async fn setup_test_db() -> (SqliteEngine, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let config = SqliteConfig {
        path: DatabasePath::File(db_path),
        foreign_keys: true,
        wal_mode: false, // Disable WAL for simpler benchmarks
        busy_timeout_ms: Some(5000),
        cache_size: Some(-2000),
        synchronous: prax_sqlite::SynchronousMode::Normal,
        journal_mode: prax_sqlite::JournalMode::Delete,
    };

    let pool = SqlitePool::new(config).await.unwrap();

    // Create test table
    let conn = pool.get().await.unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            email TEXT UNIQUE NOT NULL,
            age INTEGER,
            active INTEGER DEFAULT 1,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX idx_users_email ON users(email);
        CREATE INDEX idx_users_active ON users(active);
        "#,
    )
    .await
    .unwrap();

    (SqliteEngine::new(pool), temp_dir)
}

/// Insert sample users into the database.
async fn insert_sample_users(engine: &SqliteEngine, count: usize) {
    for i in 0..count {
        let mut data = HashMap::new();
        data.insert(
            "name".to_string(),
            FilterValue::String(format!("User {}", i)),
        );
        data.insert(
            "email".to_string(),
            FilterValue::String(format!("user{}@example.com", i)),
        );
        data.insert("age".to_string(), FilterValue::Int((20 + (i % 50)) as i64));
        data.insert("active".to_string(), FilterValue::Bool(i % 2 == 0));

        engine.execute_insert("users", &data).await.unwrap();
    }
}

/// Benchmark pool connection acquisition.
fn bench_pool_connection(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("pool_get_connection", |b| {
        let (_engine, _temp_dir) = rt.block_on(setup_test_db());
        let pool = _engine.pool().clone();

        b.to_async(&rt).iter(|| async {
            let conn = pool.get().await.unwrap();
            black_box(conn)
        });
    });
}

/// Benchmark simple insert operations.
fn bench_insert(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("single_insert", |b| {
        let (engine, _temp_dir) = rt.block_on(setup_test_db());

        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let counter = EMAIL_COUNTER.fetch_add(1, Ordering::SeqCst);
                let mut data = HashMap::new();
                data.insert(
                    "name".to_string(),
                    FilterValue::String(format!("Test {}", counter)),
                );
                data.insert(
                    "email".to_string(),
                    FilterValue::String(format!("bench{}@example.com", counter)),
                );
                data.insert("age".to_string(), FilterValue::Int(25));
                black_box(engine.execute_insert("users", &data).await.unwrap())
            }
        });
    });
}

/// Benchmark query operations.
fn bench_query(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("query");

    // Setup with 100 users
    let (engine, _temp_dir) = rt.block_on(async {
        let (engine, temp_dir) = setup_test_db().await;
        insert_sample_users(&engine, 100).await;
        (engine, temp_dir)
    });

    group.bench_function("query_all", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let results = engine
                    .query_many("users", &[], &HashMap::new(), &[], None, None)
                    .await
                    .unwrap();
                black_box(results)
            }
        });
    });

    group.bench_function("query_with_limit_10", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let results = engine
                    .query_many("users", &[], &HashMap::new(), &[], Some(10), None)
                    .await
                    .unwrap();
                black_box(results)
            }
        });
    });

    group.bench_function("query_with_filter", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let mut filters = HashMap::new();
                filters.insert("active".to_string(), FilterValue::Bool(true));
                let results = engine
                    .query_many("users", &[], &filters, &[], None, None)
                    .await
                    .unwrap();
                black_box(results)
            }
        });
    });

    group.bench_function("query_one", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let mut filters = HashMap::new();
                filters.insert("id".to_string(), FilterValue::Int(50));
                let result = engine.query_one("users", &[], &filters).await.unwrap();
                black_box(result)
            }
        });
    });

    group.finish();
}

/// Benchmark raw SQL operations.
fn bench_raw_sql(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("raw_sql");

    // Setup with 100 users
    let (engine, _temp_dir) = rt.block_on(async {
        let (engine, temp_dir) = setup_test_db().await;
        insert_sample_users(&engine, 100).await;
        (engine, temp_dir)
    });

    group.bench_function("raw_sql_select_all", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let results = engine
                    .raw_sql_query("SELECT * FROM users", &[])
                    .await
                    .unwrap();
                black_box(results)
            }
        });
    });

    group.bench_function("raw_sql_select_with_param", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let results = engine
                    .raw_sql_query(
                        "SELECT * FROM users WHERE active = ?",
                        &[FilterValue::Bool(true)],
                    )
                    .await
                    .unwrap();
                black_box(results)
            }
        });
    });

    group.bench_function("raw_sql_count", |b| {
        let engine = engine.clone();
        b.to_async(&rt).iter(|| {
            let engine = engine.clone();
            async move {
                let result: i64 = engine
                    .raw_sql_scalar("SELECT COUNT(*) FROM users", &[])
                    .await
                    .unwrap();
                black_box(result)
            }
        });
    });

    group.finish();
}

/// Benchmark count operations.
fn bench_count(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("count");

    // Setup with varying user counts
    for user_count in [10, 100, 1000].iter() {
        let (engine, _temp_dir) = rt.block_on(async {
            let (engine, temp_dir) = setup_test_db().await;
            insert_sample_users(&engine, *user_count).await;
            (engine, temp_dir)
        });

        group.throughput(Throughput::Elements(*user_count as u64));

        group.bench_with_input(
            BenchmarkId::new("count_all", user_count),
            &engine,
            |b, engine| {
                let engine = engine.clone();
                b.to_async(&rt).iter(|| {
                    let engine = engine.clone();
                    async move {
                        let count = engine.count("users", &HashMap::new()).await.unwrap();
                        black_box(count)
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("count_filtered", user_count),
            &engine,
            |b, engine| {
                let engine = engine.clone();
                b.to_async(&rt).iter(|| {
                    let engine = engine.clone();
                    async move {
                        let mut filters = HashMap::new();
                        filters.insert("active".to_string(), FilterValue::Bool(true));
                        let count = engine.count("users", &filters).await.unwrap();
                        black_box(count)
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_pool_connection,
    bench_insert,
    bench_query,
    bench_raw_sql,
    bench_count,
);

criterion_main!(benches);
