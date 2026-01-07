#![allow(dead_code, unused, clippy::type_complexity)]
//! Benchmarks for filter operations and SQL generation.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_query::filter::{Filter, FilterValue};
use prax_query::raw::Sql;
use prax_query::sql::DatabaseType;
use prax_query::static_filter::{and2, and5, eq, fields, gt};
use prax_query::typed_filter::{self as tf, AndN, TypedFilter};

/// Create a sample equals filter.
fn create_equals_filter() -> Filter {
    // Using static string - zero allocation with Cow::Borrowed
    Filter::Equals("id".into(), FilterValue::Int(42))
}

/// Create a sample AND filter with multiple conditions.
fn create_and_filter(count: usize) -> Filter {
    let filters: Vec<Filter> = (0..count)
        .map(|i| {
            // Using dynamic string - requires allocation with Cow::Owned
            Filter::Equals(format!("field_{}", i).into(), FilterValue::Int(i as i64))
        })
        .collect();
    Filter::and(filters)
}

/// Create an AND filter using the optimized builder with pre-sized vectors.
fn create_and_filter_builder(count: usize) -> Filter {
    let mut builder = Filter::and_builder(count);
    for i in 0..count {
        builder = builder.push(Filter::Equals(
            format!("field_{}", i).into(),
            FilterValue::Int(i as i64),
        ));
    }
    builder.build()
}

/// Create a sample OR filter with multiple conditions.
fn create_or_filter(count: usize) -> Filter {
    let filters: Vec<Filter> = (0..count)
        .map(|i| {
            // Using static string - zero allocation with Cow::Borrowed
            Filter::Equals(
                "status".into(),
                FilterValue::String(format!("status_{}", i)),
            )
        })
        .collect();
    Filter::or(filters)
}

/// Create a deeply nested filter.
fn create_nested_filter(depth: usize) -> Filter {
    if depth == 0 {
        // Using static strings - zero allocation with Cow::Borrowed
        Filter::Equals("leaf".into(), FilterValue::Bool(true))
    } else {
        Filter::and([
            Filter::or([
                create_nested_filter(depth - 1),
                Filter::Equals("check".into(), FilterValue::Int(depth as i64)),
            ]),
            Filter::Not(Box::new(Filter::Equals(
                "deleted".into(),
                FilterValue::Bool(true),
            ))),
        ])
    }
}

/// Create an AND filter using static field names (from interned strings).
/// This represents the best-case scenario with pre-defined field names.
fn create_and_filter_static(count: usize) -> Filter {
    use prax_query::fields;
    // Use a mix of common static field names
    static FIELDS: &[&str] = &[
        fields::ID,
        fields::EMAIL,
        fields::NAME,
        fields::STATUS,
        fields::CREATED_AT,
        fields::UPDATED_AT,
        fields::ACTIVE,
        fields::DELETED,
        fields::USER_ID,
        fields::TYPE,
    ];

    let filters: Vec<Filter> = (0..count)
        .map(|i| {
            // Using static strings - zero allocation with Cow::Borrowed
            Filter::Equals(FIELDS[i % FIELDS.len()].into(), FilterValue::Int(i as i64))
        })
        .collect();
    Filter::and(filters)
}

/// Create an AND filter using const generic and_n (no Vec allocation).
fn create_and_filter_const_5() -> Filter {
    use prax_query::fields;
    Filter::and_n([
        Filter::Equals(fields::ID.into(), FilterValue::Int(1)),
        Filter::Equals(fields::ACTIVE.into(), FilterValue::Bool(true)),
        Filter::Gt(fields::SCORE.into(), FilterValue::Int(100)),
        Filter::Equals(fields::STATUS.into(), FilterValue::String("active".into())),
        Filter::IsNotNull(fields::EMAIL.into()),
    ])
}

/// Create an AND filter using const generic and_n with 10 conditions.
fn create_and_filter_const_10() -> Filter {
    use prax_query::fields;
    Filter::and_n([
        Filter::Equals(fields::ID.into(), FilterValue::Int(1)),
        Filter::Equals(fields::ACTIVE.into(), FilterValue::Bool(true)),
        Filter::Gt(fields::SCORE.into(), FilterValue::Int(100)),
        Filter::Equals(fields::STATUS.into(), FilterValue::String("active".into())),
        Filter::IsNotNull(fields::EMAIL.into()),
        Filter::Lt(fields::AGE.into(), FilterValue::Int(65)),
        Filter::Equals(fields::ROLE.into(), FilterValue::String("user".into())),
        Filter::Gte(
            fields::CREATED_AT.into(),
            FilterValue::String("2024-01-01".into()),
        ),
        Filter::IsNull(fields::DELETED_AT.into()),
        Filter::NotEquals(fields::TYPE.into(), FilterValue::String("guest".into())),
    ])
}

/// Create an OR filter using const generic or_n (no Vec allocation).
fn create_or_filter_const_5() -> Filter {
    Filter::or_n([
        Filter::Equals("status".into(), FilterValue::String("pending".into())),
        Filter::Equals("status".into(), FilterValue::String("active".into())),
        Filter::Equals("status".into(), FilterValue::String("processing".into())),
        Filter::Equals("status".into(), FilterValue::String("completed".into())),
        Filter::Equals("status".into(), FilterValue::String("archived".into())),
    ])
}

/// Benchmark filter creation.
fn bench_filter_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_creation");

    group.bench_function("equals", |b| b.iter(|| black_box(create_equals_filter())));

    // Static filter: eq() function with static field name
    group.bench_function("static_eq", |b| b.iter(|| black_box(eq(fields::ID, 42))));

    // Static filter: gt() function
    group.bench_function("static_gt", |b| b.iter(|| black_box(gt(fields::AGE, 18))));

    // Optimized: and2 for exactly two filters
    group.bench_function("and2_two_filters", |b| {
        b.iter(|| {
            black_box(Filter::and2(
                Filter::Equals("id".into(), FilterValue::Int(1)),
                Filter::Equals("active".into(), FilterValue::Bool(true)),
            ))
        })
    });

    // Static filter: and2() with static eq()
    group.bench_function("static_and2", |b| {
        b.iter(|| black_box(and2(eq(fields::ACTIVE, true), gt(fields::SCORE, 100))))
    });

    // Static filter: and5() with static eq()
    group.bench_function("static_and5", |b| {
        b.iter(|| {
            black_box(and5(
                eq(fields::ID, 1),
                eq(fields::ACTIVE, true),
                gt(fields::SCORE, 100),
                eq(fields::STATUS, "active"),
                eq(fields::ROLE, "admin"),
            ))
        })
    });

    group.bench_function("and_5_conditions", |b| {
        b.iter(|| black_box(create_and_filter(5)))
    });

    group.bench_function("and_10_conditions", |b| {
        b.iter(|| black_box(create_and_filter(10)))
    });

    // Optimized: using builder with pre-sized vector
    group.bench_function("and_10_conditions_builder", |b| {
        b.iter(|| black_box(create_and_filter_builder(10)))
    });

    // Optimized: using static field names (no string allocation)
    group.bench_function("and_10_conditions_static", |b| {
        b.iter(|| black_box(create_and_filter_static(10)))
    });

    // Type-level composition: eq().and()
    group.bench_function("typed_and2", |b| {
        b.iter(|| {
            black_box(
                tf::eq("active", true)
                    .and(tf::gt("score", 100))
                    .into_filter(),
            )
        })
    });

    // Type-level composition: chained
    group.bench_function("typed_and5_chained", |b| {
        b.iter(|| {
            black_box(
                tf::eq("id", 1)
                    .and(tf::eq("active", true))
                    .and(tf::gt("score", 100))
                    .and(tf::eq("status", "active"))
                    .and(tf::eq("role", "admin"))
                    .into_filter(),
            )
        })
    });

    // Const generic: AndN<5>
    group.bench_function("typed_and5_const_generic", |b| {
        b.iter(|| {
            black_box(
                AndN::new([
                    tf::eq("id", 1).into_filter(),
                    tf::eq("active", true).into_filter(),
                    tf::gt("score", 100).into_filter(),
                    tf::eq("status", "active").into_filter(),
                    tf::eq("role", "admin").into_filter(),
                ])
                .into_filter(),
            )
        })
    });

    group.bench_function("or_5_conditions", |b| {
        b.iter(|| black_box(create_or_filter(5)))
    });

    // Const generic: and_n (no Vec allocation)
    group.bench_function("and_5_const_generic", |b| {
        b.iter(|| black_box(create_and_filter_const_5()))
    });

    group.bench_function("and_10_const_generic", |b| {
        b.iter(|| black_box(create_and_filter_const_10()))
    });

    group.bench_function("or_5_const_generic", |b| {
        b.iter(|| black_box(create_or_filter_const_5()))
    });

    // Direct and5() helper
    group.bench_function("and5_helper", |b| {
        use prax_query::fields;
        b.iter(|| {
            black_box(Filter::and5(
                Filter::Equals(fields::ID.into(), FilterValue::Int(1)),
                Filter::Equals(fields::ACTIVE.into(), FilterValue::Bool(true)),
                Filter::Gt(fields::SCORE.into(), FilterValue::Int(100)),
                Filter::Equals(fields::STATUS.into(), FilterValue::String("active".into())),
                Filter::IsNotNull(fields::EMAIL.into()),
            ))
        })
    });

    group.bench_function("nested_depth_3", |b| {
        b.iter(|| black_box(create_nested_filter(3)))
    });

    group.bench_function("nested_depth_5", |b| {
        b.iter(|| black_box(create_nested_filter(5)))
    });

    group.finish();
}

/// Benchmark IN filter with varying list sizes.
fn bench_in_filter(c: &mut Criterion) {
    use prax_query::filter::ValueList;

    let mut group = c.benchmark_group("in_filter");

    // Test slice-based IN filters
    group.bench_function("in_slice_10", |b| {
        let ids: Vec<i64> = (0..10).collect();
        b.iter(|| black_box(Filter::in_slice("id", &ids)))
    });

    group.bench_function("in_slice_32", |b| {
        let ids: Vec<i64> = (0..32).collect();
        b.iter(|| black_box(Filter::in_slice("id", &ids)))
    });

    group.bench_function("in_slice_100", |b| {
        let ids: Vec<i64> = (0..100).collect();
        b.iter(|| black_box(Filter::in_slice("id", &ids)))
    });

    // Test array-based IN filters
    group.bench_function("in_array_5", |b| {
        b.iter(|| black_box(Filter::in_array("status", ["a", "b", "c", "d", "e"])))
    });

    group.bench_function("in_array_10", |b| {
        b.iter(|| black_box(Filter::in_array("id", [1i64, 2, 3, 4, 5, 6, 7, 8, 9, 10])))
    });

    for size in [10, 50, 100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("in_list_size", size), size, |b, &size| {
            b.iter(|| {
                // Collect directly into SmallVec for optimal performance
                let values: ValueList = (0..size).map(|i| FilterValue::Int(i as i64)).collect();
                // Using static string - zero allocation with Cow::Borrowed
                black_box(Filter::In("id".into(), values))
            })
        });

        // Optimized: using in_i64 (avoids Into<FilterValue> overhead)
        group.bench_with_input(BenchmarkId::new("in_i64", size), size, |b, &size| {
            b.iter(|| black_box(Filter::in_i64("id", 0..(size as i64))))
        });

        // Optimized: using in_range (even more optimized for sequential values)
        group.bench_with_input(BenchmarkId::new("in_range", size), size, |b, &size| {
            b.iter(|| black_box(Filter::in_range("id", 0..(size as i64))))
        });

        // Optimized: using in_i64_slice (pre-allocated with exact capacity)
        let values: Vec<i64> = (0..(*size as i64)).collect();
        group.bench_with_input(
            BenchmarkId::new("in_i64_slice", size),
            &values,
            |b, values| b.iter(|| black_box(Filter::in_i64_slice("id", values))),
        );
    }

    group.finish();
}

/// Benchmark NOT_IN filter with varying list sizes.
fn bench_not_in_filter(c: &mut Criterion) {
    use prax_query::filter::ValueList;

    let mut group = c.benchmark_group("not_in_filter");

    for size in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(
            BenchmarkId::new("not_in_list_size", size),
            size,
            |b, &size| {
                b.iter(|| {
                    // Collect directly into SmallVec for optimal performance
                    let values: ValueList = (0..size).map(|i| FilterValue::Int(i as i64)).collect();
                    // Using static string - zero allocation with Cow::Borrowed
                    black_box(Filter::NotIn("id".into(), values))
                })
            },
        );
    }

    group.finish();
}

/// Benchmark database type specific placeholder generation.
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

/// Benchmark SQL builder SELECT generation.
fn bench_select_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_generation");

    group.bench_function("simple_select", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT id, name, email FROM users");
            black_box(sql.build())
        })
    });

    group.bench_function("select_with_where", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT id, name, email FROM users WHERE id = ").bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("select_with_complex_where", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT id, name, email, created_at, updated_at FROM users WHERE ")
                .push("active = ")
                .bind(true)
                .push(" AND (status = ")
                .bind("approved")
                .push(" OR status = ")
                .bind("pending")
                .push(") AND created_at > ")
                .bind("2024-01-01")
                .push(" ORDER BY created_at DESC LIMIT ")
                .bind(100)
                .push(" OFFSET ")
                .bind(0);
            black_box(sql.build())
        })
    });

    group.finish();
}

/// Benchmark INSERT query generation.
fn bench_insert_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_generation");

    for cols in [3, 5, 10, 20].iter() {
        group.bench_with_input(BenchmarkId::new("columns", cols), cols, |b, &cols| {
            b.iter(|| {
                let mut sql = Sql::new("INSERT INTO users (");

                for i in 0..cols {
                    if i > 0 {
                        sql = sql.push(", ");
                    }
                    sql = sql.push(format!("col{}", i));
                }

                sql = sql.push(") VALUES (");

                for i in 0..cols {
                    if i > 0 {
                        sql = sql.push(", ");
                    }
                    sql = sql.bind(format!("value{}", i));
                }

                sql = sql.push(")");
                black_box(sql.build())
            })
        });
    }

    group.finish();
}

/// Benchmark UPDATE query generation.
fn bench_update_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("update_generation");

    group.bench_function("simple_update", |b| {
        b.iter(|| {
            let sql = Sql::new("UPDATE users SET name = ")
                .bind("new_name")
                .push(", email = ")
                .bind("new@email.com")
                .push(" WHERE id = ")
                .bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("update_many_columns", |b| {
        b.iter(|| {
            let sql = Sql::new("UPDATE users SET ")
                .push("name = ")
                .bind("new_name")
                .push(", email = ")
                .bind("new@email.com")
                .push(", status = ")
                .bind("active")
                .push(", role = ")
                .bind("user")
                .push(", updated_at = ")
                .bind("2024-01-01")
                .push(", last_login = ")
                .bind("2024-01-01")
                .push(", settings = ")
                .bind("{}")
                .push(", metadata = ")
                .bind("{}")
                .push(", avatar_url = ")
                .bind("https://example.com")
                .push(", bio = ")
                .bind("Hello world")
                .push(" WHERE id = ")
                .bind(42);
            black_box(sql.build())
        })
    });

    group.finish();
}

/// Benchmark DELETE query generation.
fn bench_delete_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_generation");

    group.bench_function("simple_delete", |b| {
        b.iter(|| {
            let sql = Sql::new("DELETE FROM users WHERE id = ").bind(42);
            black_box(sql.build())
        })
    });

    group.bench_function("delete_with_complex_where", |b| {
        b.iter(|| {
            let sql = Sql::new("DELETE FROM users WHERE ")
                .push("active = ")
                .bind(false)
                .push(" AND created_at < ")
                .bind("2024-01-01")
                .push(" AND status IN (")
                .bind("deleted")
                .push(", ")
                .bind("banned")
                .push(")");
            black_box(sql.build())
        })
    });

    group.finish();
}

/// Benchmark filter cloning.
fn bench_filter_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_clone");

    group.bench_function("clone_simple", |b| {
        let filter = create_equals_filter();
        b.iter(|| black_box(filter.clone()))
    });

    group.bench_function("clone_and_10", |b| {
        let filter = create_and_filter(10);
        b.iter(|| black_box(filter.clone()))
    });

    group.bench_function("clone_nested_5", |b| {
        let filter = create_nested_filter(5);
        b.iter(|| black_box(filter.clone()))
    });

    group.finish();
}

/// Benchmark DirectSql trait for zero-allocation SQL generation.
fn bench_direct_sql(c: &mut Criterion) {
    use prax_query::typed_filter::{And, DirectSql, Eq, Gt, InI64, in_i64_slice};

    let mut group = c.benchmark_group("direct_sql");

    group.bench_function("eq_write_sql", |b| {
        let filter = Eq::new("id", 42i64);
        let mut buf = String::with_capacity(64);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.bench_function("and2_write_sql", |b| {
        let filter = And::new(Eq::new("id", 42i64), Gt::new("age", 18i64));
        let mut buf = String::with_capacity(128);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.bench_function("and5_write_sql", |b| {
        let filter = And::new(
            Eq::new("id", 42i64),
            And::new(
                Gt::new("age", 18i64),
                And::new(
                    Eq::new("active", true),
                    And::new(Gt::new("score", 100i64), Eq::new("status", "approved")),
                ),
            ),
        );
        let mut buf = String::with_capacity(256);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    // Benchmark pure type-level AND(5) with And5 struct - truly zero allocation
    group.bench_function("and5_direct_sql", |b| {
        use prax_query::typed_filter::And5;
        let filter = And5::new(
            Eq::new("id", 42i64),
            Gt::new("age", 18i64),
            Eq::new("active", true),
            Gt::new("score", 100i64),
            Eq::new("status", "approved"),
        );
        let mut buf = String::with_capacity(256);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    // Benchmark type-level filter construction only (no SQL generation)
    group.bench_function("and5_type_construction", |b| {
        use prax_query::typed_filter::And5;
        b.iter(|| {
            black_box(And5::new(
                Eq::new("id", 42i64),
                Gt::new("age", 18i64),
                Eq::new("active", true),
                Gt::new("score", 100i64),
                Eq::new("status", "approved"),
            ))
        })
    });

    // Benchmark chained type-level filter construction
    group.bench_function("and5_chained_construction", |b| {
        b.iter(|| {
            black_box(
                Eq::new("id", 42i64)
                    .and(Gt::new("age", 18i64))
                    .and(Eq::new("active", true))
                    .and(Gt::new("score", 100i64))
                    .and(Eq::new("status", "approved")),
            )
        })
    });

    // Benchmark IN filters with DirectSql
    group.bench_function("in_i64_10_write_sql", |b| {
        let values = [1i64, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let filter = InI64::<10>::new("id", values);
        let mut buf = String::with_capacity(64);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.bench_function("in_slice_10_write_sql", |b| {
        let values: Vec<i64> = (0..10).collect();
        let filter = in_i64_slice("id", &values);
        let mut buf = String::with_capacity(64);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.bench_function("in_slice_32_write_sql", |b| {
        let values: Vec<i64> = (0..32).collect();
        let filter = in_i64_slice("id", &values);
        let mut buf = String::with_capacity(256);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.bench_function("in_slice_100_write_sql", |b| {
        let values: Vec<i64> = (0..100).collect();
        let filter = in_i64_slice("id", &values);
        let mut buf = String::with_capacity(512);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.bench_function("in_slice_1000_write_sql", |b| {
        let values: Vec<i64> = (0..1000).collect();
        let filter = in_i64_slice("id", &values);
        let mut buf = String::with_capacity(8192);
        b.iter(|| {
            buf.clear();
            black_box(filter.write_sql(&mut buf, 1))
        })
    });

    group.finish();
}

/// Benchmark SQL template cache operations.
fn bench_sql_template_cache(c: &mut Criterion) {
    use prax_query::cache::{SqlTemplateCache, precompute_query_hash};

    let mut group = c.benchmark_group("sql_template_cache");

    // Setup cache with some templates
    let cache = SqlTemplateCache::new(1000);
    cache.register("users_by_id", "SELECT * FROM users WHERE id = $1");
    cache.register("users_all", "SELECT * FROM users");
    cache.register(
        "posts_by_author",
        "SELECT * FROM posts WHERE author_id = $1",
    );

    // Pre-compute hash for fastest path
    let template = cache.get("users_by_id").unwrap();
    let precomputed_hash = template.hash;

    group.bench_function("get_by_string_key", |b| {
        b.iter(|| black_box(cache.get("users_by_id")))
    });

    group.bench_function("get_by_hash", |b| {
        b.iter(|| black_box(cache.get_by_hash(precomputed_hash)))
    });

    group.bench_function("get_or_register_hit", |b| {
        b.iter(|| black_box(cache.get_or_register("users_by_id", || unreachable!())))
    });

    group.bench_function("precompute_hash", |b| {
        b.iter(|| black_box(precompute_query_hash("users_by_id")))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_filter_creation,
    bench_in_filter,
    bench_not_in_filter,
    bench_placeholder_generation,
    bench_select_generation,
    bench_insert_generation,
    bench_update_generation,
    bench_delete_generation,
    bench_filter_clone,
    bench_direct_sql,
    bench_sql_template_cache,
);

criterion_main!(benches);
