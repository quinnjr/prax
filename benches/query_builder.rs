#![allow(dead_code, unused, clippy::type_complexity)]
//! Benchmarks for query builder operations.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_query::filter::{Filter, FilterValue, ScalarFilter};
use prax_query::raw::Sql;
use prax_query::sql::{DatabaseType, FastSqlBuilder, QueryCapacity, templates};
use prax_query::types::{NullsOrder, OrderBy, OrderByField};

/// Benchmark filter construction.
fn bench_filter_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_construction");

    group.bench_function("equals_filter", |b| {
        b.iter(|| black_box(Filter::Equals("id".into(), FilterValue::Int(42))))
    });

    group.bench_function("in_filter_10", |b| {
        let values: Vec<FilterValue> = (0..10).map(FilterValue::Int).collect();
        b.iter(|| black_box(Filter::In("id".into(), values.clone())))
    });

    group.bench_function("in_filter_100", |b| {
        let values: Vec<FilterValue> = (0..100).map(FilterValue::Int).collect();
        b.iter(|| black_box(Filter::In("id".into(), values.clone())))
    });

    group.bench_function("and_filter_3", |b| {
        b.iter(|| {
            black_box(Filter::and([
                Filter::Equals("id".into(), FilterValue::Int(1)),
                Filter::Equals("name".into(), FilterValue::String("test".to_string())),
                Filter::Equals("active".into(), FilterValue::Bool(true)),
            ]))
        })
    });

    group.bench_function("complex_nested_filter", |b| {
        b.iter(|| {
            black_box(Filter::and([
                Filter::or([
                    Filter::Equals("status".into(), FilterValue::String("active".into())),
                    Filter::Equals("status".into(), FilterValue::String("pending".to_string())),
                ]),
                Filter::Not(Box::new(Filter::Equals(
                    "deleted".into(),
                    FilterValue::Bool(true),
                ))),
                Filter::Gt("created_at".into(), FilterValue::Int(1000)),
            ]))
        })
    });

    group.finish();
}

/// Benchmark scalar filter operations.
fn bench_scalar_filters(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalar_filters");

    group.bench_function("equals", |b| b.iter(|| black_box(ScalarFilter::Equals(42))));

    group.bench_function("not_equals", |b| {
        b.iter(|| black_box(ScalarFilter::Not(Box::new(42))))
    });

    group.bench_function("contains_string", |b| {
        b.iter(|| black_box(ScalarFilter::Contains("test".to_string())))
    });

    group.bench_function("starts_with", |b| {
        b.iter(|| black_box(ScalarFilter::StartsWith("prefix".to_string())))
    });

    group.bench_function("in_list", |b| {
        let values: Vec<i32> = (0..10).collect();
        b.iter(|| black_box(ScalarFilter::In(values.clone())))
    });

    group.finish();
}

/// Benchmark SQL builder operations.
fn bench_sql_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("sql_builder");

    group.bench_function("simple_query", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = ").bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("query_with_multiple_binds", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE ")
                .push("age > ")
                .bind(18)
                .push(" AND status = ")
                .bind("active")
                .push(" AND created_at > ")
                .bind("2024-01-01");
            black_box(sql.build())
        })
    });

    group.bench_function("query_with_10_binds", |b| {
        b.iter(|| {
            let mut sql = Sql::new("INSERT INTO users (c1,c2,c3,c4,c5,c6,c7,c8,c9,c10) VALUES (");
            for i in 0..10 {
                if i > 0 {
                    sql = sql.push(",");
                }
                sql = sql.bind(i);
            }
            sql = sql.push(")");
            black_box(sql.build())
        })
    });

    group.bench_function("complex_query", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT u.id, u.name, u.email, p.title, p.content ")
                .push("FROM users u ")
                .push("LEFT JOIN posts p ON p.user_id = u.id ")
                .push("WHERE u.active = ")
                .bind(true)
                .push(" AND u.created_at > ")
                .bind("2024-01-01")
                .push(" AND (u.role = ")
                .bind("admin")
                .push(" OR u.role = ")
                .bind("moderator")
                .push(") ")
                .push("ORDER BY u.created_at DESC ")
                .push("LIMIT ")
                .bind(100)
                .push(" OFFSET ")
                .bind(0);
            black_box(sql.build())
        })
    });

    group.finish();
}

/// Benchmark FastSqlBuilder (optimized) vs Sql (original).
fn bench_fast_sql_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("fast_sql_builder");

    // Simple query comparison
    group.bench_function("simple_query_original", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = ").bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("simple_query_fast", |b| {
        b.iter(|| {
            let mut builder = FastSqlBuilder::postgres(QueryCapacity::SimpleSelect);
            builder.push_str("SELECT * FROM users WHERE id = ");
            builder.bind(42);
            black_box(builder.build())
        })
    });

    // Complex query comparison
    group.bench_function("complex_query_original", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE active = ")
                .bind(true)
                .push(" AND age > ")
                .bind(18)
                .push(" AND status = ")
                .bind("approved")
                .push(" ORDER BY created_at LIMIT ")
                .bind(10);
            black_box(sql.build())
        })
    });

    group.bench_function("complex_query_fast", |b| {
        b.iter(|| {
            let mut builder = FastSqlBuilder::postgres(QueryCapacity::SelectWithFilters(4));
            builder
                .push_str("SELECT * FROM users WHERE active = ")
                .bind(true)
                .push_str(" AND age > ")
                .bind(18)
                .push_str(" AND status = ")
                .bind("approved")
                .push_str(" ORDER BY created_at LIMIT ")
                .bind(10);
            black_box(builder.build())
        })
    });

    // IN clause comparison
    group.bench_function("in_clause_original_10", |b| {
        let values: Vec<FilterValue> = (1..=10).map(FilterValue::Int).collect();
        b.iter(|| {
            let mut sql = Sql::new("SELECT * FROM users WHERE id IN (");
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    sql = sql.push(", ");
                }
                sql = sql.bind(v.clone());
            }
            sql = sql.push(")");
            black_box(sql.build())
        })
    });

    group.bench_function("in_clause_fast_10", |b| {
        let values: Vec<FilterValue> = (1..=10).map(FilterValue::Int).collect();
        b.iter(|| {
            let mut builder = FastSqlBuilder::postgres(QueryCapacity::Custom(64));
            builder.push_str("SELECT * FROM users WHERE id IN (");
            builder.bind_in_clause(values.clone());
            builder.push_char(')');
            black_box(builder.build())
        })
    });

    group.bench_function("in_clause_original_100", |b| {
        let values: Vec<FilterValue> = (1..=100).map(FilterValue::Int).collect();
        b.iter(|| {
            let mut sql = Sql::new("SELECT * FROM users WHERE id IN (");
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    sql = sql.push(", ");
                }
                sql = sql.bind(v.clone());
            }
            sql = sql.push(")");
            black_box(sql.build())
        })
    });

    group.bench_function("in_clause_fast_100", |b| {
        let values: Vec<FilterValue> = (1..=100).map(FilterValue::Int).collect();
        b.iter(|| {
            let mut builder = FastSqlBuilder::postgres(QueryCapacity::Custom(512));
            builder.push_str("SELECT * FROM users WHERE id IN (");
            builder.bind_in_clause(values.clone());
            builder.push_char(')');
            black_box(builder.build())
        })
    });

    // Template benchmarks
    group.bench_function("template_select_by_id", |b| {
        b.iter(|| black_box(templates::select_by_id("users", &["id", "name", "email"])))
    });

    group.bench_function("template_insert_returning", |b| {
        b.iter(|| {
            black_box(templates::insert_returning(
                "users",
                &["name", "email", "age"],
            ))
        })
    });

    group.bench_function("template_batch_placeholders_10x3", |b| {
        b.iter(|| {
            black_box(templates::batch_placeholders(
                DatabaseType::PostgreSQL,
                3,
                10,
            ))
        })
    });

    group.finish();
}

/// Benchmark order by operations.
fn bench_order_by(c: &mut Criterion) {
    let mut group = c.benchmark_group("order_by");

    group.bench_function("single_field", |b| {
        b.iter(|| {
            let order = OrderByField::desc("created_at");
            black_box(order.to_sql())
        })
    });

    group.bench_function("single_field_with_nulls", |b| {
        b.iter(|| {
            let order = OrderByField::asc("name").nulls(NullsOrder::Last);
            black_box(order.to_sql())
        })
    });

    group.bench_function("multiple_fields", |b| {
        b.iter(|| {
            let order = OrderBy::Field(OrderByField::desc("created_at"))
                .then(OrderByField::asc("name"))
                .then(OrderByField::asc("id"));
            black_box(order.to_sql())
        })
    });

    group.finish();
}

/// Benchmark filter value conversions.
fn bench_filter_values(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_values");

    group.bench_function("int_value", |b| b.iter(|| black_box(FilterValue::Int(42))));

    group.bench_function("string_value_short", |b| {
        b.iter(|| black_box(FilterValue::String("test".to_string())))
    });

    group.bench_function("string_value_long", |b| {
        let long_string = "a".repeat(1000);
        b.iter(|| black_box(FilterValue::String(long_string.clone())))
    });

    group.bench_function("json_value", |b| {
        let json = serde_json::json!({"key": "value", "nested": {"a": 1, "b": 2}});
        b.iter(|| black_box(FilterValue::Json(json.clone())))
    });

    group.bench_function("list_value_10", |b| {
        let values: Vec<FilterValue> = (0..10).map(FilterValue::Int).collect();
        b.iter(|| black_box(FilterValue::List(values.clone())))
    });

    group.finish();
}

/// Benchmark SQL builder for different database types.
fn bench_sql_builder_by_db(c: &mut Criterion) {
    let mut group = c.benchmark_group("sql_builder_by_db");

    group.bench_function("postgres_builder", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = ")
                .with_db_type(DatabaseType::PostgreSQL)
                .bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("mysql_builder", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = ")
                .with_db_type(DatabaseType::MySQL)
                .bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("sqlite_builder", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = ")
                .with_db_type(DatabaseType::SQLite)
                .bind(42);
            black_box(sql.build())
        })
    });

    group.finish();
}

/// Benchmark throughput for batch operations.
fn bench_batch_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_throughput");

    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(
            BenchmarkId::new("filter_construction", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let filters: Vec<Filter> = (0..size)
                        .map(|i| Filter::Equals("id".into(), FilterValue::Int(i as i64)))
                        .collect();
                    black_box(filters)
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("sql_bind_params", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let mut sql = Sql::new("SELECT * FROM users WHERE id IN (");
                    for i in 0..size {
                        if i > 0 {
                            sql = sql.push(",");
                        }
                        sql = sql.bind(i as i64);
                    }
                    sql = sql.push(")");
                    black_box(sql.build())
                })
            },
        );
    }

    group.finish();
}

/// Benchmark database type placeholder generation.
fn bench_placeholder_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("placeholder_generation");

    group.bench_function("postgres_placeholder", |b| {
        b.iter(|| {
            let db = DatabaseType::PostgreSQL;
            for i in 1..=100 {
                black_box(db.placeholder(i));
            }
        })
    });

    group.bench_function("mysql_placeholder", |b| {
        b.iter(|| {
            let db = DatabaseType::MySQL;
            for i in 1..=100 {
                black_box(db.placeholder(i));
            }
        })
    });

    group.bench_function("sqlite_placeholder", |b| {
        b.iter(|| {
            let db = DatabaseType::SQLite;
            for i in 1..=100 {
                black_box(db.placeholder(i));
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_filter_construction,
    bench_scalar_filters,
    bench_sql_builder,
    bench_fast_sql_builder,
    bench_order_by,
    bench_filter_values,
    bench_sql_builder_by_db,
    bench_batch_throughput,
    bench_placeholder_generation,
);

criterion_main!(benches);
