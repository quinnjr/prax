//! Database-specific integration benchmarks.
//!
//! These benchmarks test SQL generation and query building for each supported database.
//! They don't require actual database connections - they measure the ORM layer performance.
//!
//! Run with: `cargo bench --package prax-query --bench database_bench`
//!
//! For benchmarks with real database connections, see the integration tests.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use prax_query::{
    filter::{Filter, FilterValue},
    sql::{DatabaseType, SqlBuilder},
    types::{OrderByField, Select},
};
use std::borrow::Cow;

// ============================================================================
// Multi-Database SQL Generation Benchmarks
// ============================================================================

fn bench_sql_generation_by_database(c: &mut Criterion) {
    let mut group = c.benchmark_group("sql_generation_by_db");

    let databases = [
        ("postgres", DatabaseType::PostgreSQL),
        ("mysql", DatabaseType::MySQL),
        ("sqlite", DatabaseType::SQLite),
        ("mssql", DatabaseType::MSSQL),
    ];

    // Simple SELECT
    for (name, _db_type) in &databases {
        group.bench_function(BenchmarkId::new("simple_select", name), |b| {
            b.iter(|| {
                let filter = Filter::Equals("id".into(), FilterValue::Int(1));
                let (sql, params) = filter.to_sql(0);
                black_box((sql, params))
            });
        });
    }

    // SELECT with multiple conditions
    for (name, _db_type) in &databases {
        group.bench_function(BenchmarkId::new("multi_condition", name), |b| {
            b.iter(|| {
                let filter = Filter::and(vec![
                    Filter::Equals("status".into(), FilterValue::String("active".into())),
                    Filter::Gt("age".into(), FilterValue::Int(18)),
                    Filter::IsNotNull("email".into()),
                ]);
                let (sql, params) = filter.to_sql(0);
                black_box((sql, params))
            });
        });
    }

    // IN clause with varying sizes
    for size in [5, 10, 50, 100] {
        for (name, _db_type) in &databases {
            group.bench_function(BenchmarkId::new(format!("in_clause_{}", size), name), |b| {
                let values: Vec<FilterValue> = (0..size).map(|i| FilterValue::Int(i)).collect();
                let filter = Filter::In("id".into(), values);
                b.iter(|| {
                    let (sql, params) = filter.to_sql(0);
                    black_box((sql, params))
                });
            });
        }
    }

    group.finish();
}

// ============================================================================
// PostgreSQL-Specific Benchmarks
// ============================================================================

fn bench_postgres_specific(c: &mut Criterion) {
    let mut group = c.benchmark_group("postgres_specific");

    // JSONB operations
    group.bench_function("jsonb_contains", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users WHERE metadata @> ");
            builder.push_param(FilterValue::Json(
                serde_json::json!({"role": "admin"}),
            ));
            black_box(builder.build())
        });
    });

    // Array operations
    group.bench_function("array_contains", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM posts WHERE tags && ARRAY[");
            builder.push_param(FilterValue::String("rust".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("programming".into()));
            builder.push("]::varchar[]");
            black_box(builder.build())
        });
    });

    // Full-text search
    group.bench_function("full_text_search", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM articles WHERE to_tsvector('english', content) @@ plainto_tsquery('english', ");
            builder.push_param(FilterValue::String("rust programming".into()));
            builder.push(")");
            black_box(builder.build())
        });
    });

    // RETURNING clause
    group.bench_function("insert_returning", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("INSERT INTO users (email, name) VALUES (");
            builder.push_param(FilterValue::String("test@example.com".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("Test User".into()));
            builder.push(") RETURNING id, created_at");
            black_box(builder.build())
        });
    });

    // CTE (Common Table Expression)
    group.bench_function("cte_query", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("WITH active_users AS (SELECT * FROM users WHERE status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(") SELECT au.*, COUNT(p.id) as post_count FROM active_users au ");
            builder.push("LEFT JOIN posts p ON p.user_id = au.id GROUP BY au.id");
            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// MySQL-Specific Benchmarks
// ============================================================================

fn bench_mysql_specific(c: &mut Criterion) {
    let mut group = c.benchmark_group("mysql_specific");

    // JSON operations
    group.bench_function("json_extract", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::mysql();
            builder.push("SELECT * FROM users WHERE JSON_EXTRACT(metadata, '$.role') = ");
            builder.push_param(FilterValue::String("admin".into()));
            black_box(builder.build())
        });
    });

    // Full-text search (MySQL style)
    group.bench_function("fulltext_match", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::mysql();
            builder.push("SELECT * FROM articles WHERE MATCH(title, content) AGAINST (");
            builder.push_param(FilterValue::String("rust programming".into()));
            builder.push(" IN NATURAL LANGUAGE MODE)");
            black_box(builder.build())
        });
    });

    // INSERT ... ON DUPLICATE KEY UPDATE
    group.bench_function("upsert", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::mysql();
            builder.push("INSERT INTO users (email, name, updated_at) VALUES (");
            builder.push_param(FilterValue::String("test@example.com".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("Test User".into()));
            builder.push(", NOW()) ON DUPLICATE KEY UPDATE name = VALUES(name), updated_at = NOW()");
            black_box(builder.build())
        });
    });

    // GROUP_CONCAT
    group.bench_function("group_concat", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::mysql();
            builder.push("SELECT user_id, GROUP_CONCAT(tag ORDER BY tag SEPARATOR ', ') as tags ");
            builder.push("FROM user_tags WHERE user_id IN (");
            for i in 0..5 {
                if i > 0 {
                    builder.push(", ");
                }
                builder.push_param(FilterValue::Int(i));
            }
            builder.push(") GROUP BY user_id");
            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// SQLite-Specific Benchmarks
// ============================================================================

fn bench_sqlite_specific(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite_specific");

    // JSON operations (SQLite 3.38+)
    group.bench_function("json_extract", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::sqlite();
            builder.push("SELECT * FROM users WHERE json_extract(metadata, '$.role') = ");
            builder.push_param(FilterValue::String("admin".into()));
            black_box(builder.build())
        });
    });

    // FTS5 full-text search
    group.bench_function("fts5_search", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::sqlite();
            builder.push("SELECT * FROM articles_fts WHERE articles_fts MATCH ");
            builder.push_param(FilterValue::String("rust programming".into()));
            black_box(builder.build())
        });
    });

    // INSERT OR REPLACE
    group.bench_function("upsert", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::sqlite();
            builder.push("INSERT OR REPLACE INTO users (email, name, updated_at) VALUES (");
            builder.push_param(FilterValue::String("test@example.com".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("Test User".into()));
            builder.push(", datetime('now'))");
            black_box(builder.build())
        });
    });

    // Window functions
    group.bench_function("window_function", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::sqlite();
            builder.push("SELECT *, ROW_NUMBER() OVER (PARTITION BY category ORDER BY created_at DESC) as rn ");
            builder.push("FROM posts WHERE status = ");
            builder.push_param(FilterValue::String("published".into()));
            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// MSSQL-Style SQL Benchmarks (using generic builder)
// ============================================================================

fn bench_mssql_style(c: &mut Criterion) {
    let mut group = c.benchmark_group("mssql_style");

    // TOP clause (instead of LIMIT) - using postgres builder with MSSQL syntax
    group.bench_function("top_clause", |b| {
        b.iter(|| {
            // MSSQL uses numbered params like postgres
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT TOP 10 * FROM users WHERE status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(" ORDER BY created_at DESC");
            black_box(builder.build())
        });
    });

    // OFFSET FETCH (SQL Server 2012+)
    group.bench_function("offset_fetch", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users WHERE status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(" ORDER BY created_at DESC OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY");
            black_box(builder.build())
        });
    });

    // MERGE (upsert)
    group.bench_function("merge_upsert", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("MERGE INTO users AS target USING (SELECT ");
            builder.push_param(FilterValue::String("test@example.com".into()));
            builder.push(" AS email, ");
            builder.push_param(FilterValue::String("Test User".into()));
            builder.push(" AS name) AS source ON target.email = source.email ");
            builder.push("WHEN MATCHED THEN UPDATE SET name = source.name ");
            builder.push("WHEN NOT MATCHED THEN INSERT (email, name) VALUES (source.email, source.name);");
            black_box(builder.build())
        });
    });

    // STRING_AGG (SQL Server 2017+)
    group.bench_function("string_agg", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT user_id, STRING_AGG(tag, ', ') WITHIN GROUP (ORDER BY tag) as tags ");
            builder.push("FROM user_tags WHERE user_id IN (");
            for i in 0..5 {
                if i > 0 {
                    builder.push(", ");
                }
                builder.push_param(FilterValue::Int(i));
            }
            builder.push(") GROUP BY user_id");
            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// Complex Query Pattern Benchmarks
// ============================================================================

fn bench_complex_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_queries");

    // Typical "find users with filters" query
    group.bench_function("user_search_query", |b| {
        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::or(vec![
                    Filter::Contains("email".into(), FilterValue::String("@company.com".into())),
                    Filter::Equals("role".into(), FilterValue::String("admin".into())),
                ]),
                Filter::Gte("created_at".into(), FilterValue::String("2024-01-01".into())),
                Filter::IsNotNull("verified_at".into()),
            ]);

            let (sql, params) = filter.to_sql(0);

            // Build full query
            let mut query = String::with_capacity(256);
            query.push_str("SELECT id, email, name, role, created_at FROM users WHERE ");
            query.push_str(&sql);
            query.push_str(" ORDER BY created_at DESC LIMIT 20 OFFSET 0");

            black_box((query, params))
        });
    });

    // Dashboard stats query
    group.bench_function("dashboard_aggregation", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT ");
            builder.push("COUNT(*) FILTER (WHERE status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(") as active_count, ");
            builder.push("COUNT(*) FILTER (WHERE status = ");
            builder.push_param(FilterValue::String("pending".into()));
            builder.push(") as pending_count, ");
            builder.push("COUNT(*) FILTER (WHERE created_at >= CURRENT_DATE - INTERVAL '7 days') as new_this_week ");
            builder.push("FROM users");
            black_box(builder.build())
        });
    });

    // Multi-table join query
    group.bench_function("multi_join_query", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT u.id, u.email, COUNT(p.id) as post_count, MAX(p.created_at) as last_post ");
            builder.push("FROM users u ");
            builder.push("LEFT JOIN posts p ON p.user_id = u.id AND p.status = ");
            builder.push_param(FilterValue::String("published".into()));
            builder.push(" LEFT JOIN user_roles ur ON ur.user_id = u.id ");
            builder.push("WHERE u.status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(" AND ur.role IN (");
            builder.push_param(FilterValue::String("admin".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("moderator".into()));
            builder.push(") GROUP BY u.id, u.email HAVING COUNT(p.id) > ");
            builder.push_param(FilterValue::Int(5));
            builder.push(" ORDER BY post_count DESC LIMIT ");
            builder.push_param(FilterValue::Int(50));
            black_box(builder.build())
        });
    });

    // Bulk insert preparation
    group.bench_function("bulk_insert_100_rows", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("INSERT INTO events (user_id, event_type, payload, created_at) VALUES ");

            for i in 0..100 {
                if i > 0 {
                    builder.push(", ");
                }
                builder.push("(");
                builder.push_param(FilterValue::Int(i % 10));
                builder.push(", ");
                builder.push_param(FilterValue::String("page_view".into()));
                builder.push(", ");
                builder.push_param(FilterValue::Json(serde_json::json!({"page": "/home"})));
                builder.push(", NOW())");
            }

            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// Parameter Placeholder Benchmarks
// ============================================================================

fn bench_parameter_styles(c: &mut Criterion) {
    let mut group = c.benchmark_group("parameter_styles");

    let param_counts = [5, 10, 25, 50, 100];

    for count in param_counts {
        group.throughput(Throughput::Elements(count as u64));

        // PostgreSQL style: $1, $2, ...
        group.bench_function(BenchmarkId::new("postgres_style", count), |b| {
            b.iter(|| {
                let mut builder = SqlBuilder::postgres();
                builder.push("SELECT * FROM users WHERE id IN (");
                for i in 0..count {
                    if i > 0 {
                        builder.push(", ");
                    }
                    builder.push_param(FilterValue::Int(i as i64));
                }
                builder.push(")");
                black_box(builder.build())
            });
        });

        // MySQL style: ?, ?, ...
        group.bench_function(BenchmarkId::new("mysql_style", count), |b| {
            b.iter(|| {
                let mut builder = SqlBuilder::mysql();
                builder.push("SELECT * FROM users WHERE id IN (");
                for i in 0..count {
                    if i > 0 {
                        builder.push(", ");
                    }
                    builder.push_param(FilterValue::Int(i as i64));
                }
                builder.push(")");
                black_box(builder.build())
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_sql_generation_by_database,
    bench_postgres_specific,
    bench_mysql_specific,
    bench_sqlite_specific,
    bench_mssql_style,
    bench_complex_queries,
    bench_parameter_styles,
);

criterion_main!(benches);

