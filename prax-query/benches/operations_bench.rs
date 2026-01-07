//! Benchmarks for query operations and SQL generation.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_query::{
    filter::{Filter, FilterValue, ScalarFilter},
    raw::Sql,
    sql::SqlBuilder,
    types::{OrderBy, OrderByField, Select},
};

// ============================================================================
// Filter Creation Benchmarks
// ============================================================================

fn bench_filter_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_creation");

    group.bench_function("create_equals_int", |b| {
        b.iter(|| black_box(Filter::Equals("id".into(), FilterValue::Int(1))))
    });

    group.bench_function("create_equals_string", |b| {
        b.iter(|| {
            black_box(Filter::Equals(
                "email".into(),
                FilterValue::String("test@example.com".into()),
            ))
        })
    });

    group.bench_function("create_in_filter", |b| {
        b.iter(|| {
            black_box(Filter::In(
                "id".into(),
                vec![
                    FilterValue::Int(1),
                    FilterValue::Int(2),
                    FilterValue::Int(3),
                ],
            ))
        })
    });

    group.bench_function("create_contains_filter", |b| {
        b.iter(|| {
            black_box(Filter::Contains(
                "email".into(),
                FilterValue::String("@example.com".into()),
            ))
        })
    });

    group.bench_function("create_and_filter", |b| {
        b.iter(|| {
            black_box(Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::Equals("role".into(), FilterValue::String("admin".into())),
            ]))
        })
    });

    group.bench_function("create_or_filter", |b| {
        b.iter(|| {
            black_box(Filter::or(vec![
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::Equals("status".into(), FilterValue::String("pending".into())),
            ]))
        })
    });

    group.bench_function("create_complex_filter", |b| {
        b.iter(|| {
            black_box(Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::or(vec![
                    Filter::Contains("email".into(), FilterValue::String("@admin".into())),
                    Filter::Equals("role".into(), FilterValue::String("admin".into())),
                ]),
                Filter::Gt("age".into(), FilterValue::Int(18)),
            ]))
        })
    });

    group.finish();
}

// ============================================================================
// Filter Value Benchmarks
// ============================================================================

fn bench_filter_values(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_values");

    group.bench_function("create_null", |b| b.iter(|| black_box(FilterValue::Null)));

    group.bench_function("create_int", |b| {
        b.iter(|| black_box(FilterValue::Int(12345)))
    });

    group.bench_function("create_float", |b| {
        b.iter(|| black_box(FilterValue::Float(123.456)))
    });

    group.bench_function("create_bool", |b| {
        b.iter(|| black_box(FilterValue::Bool(true)))
    });

    group.bench_function("create_string", |b| {
        b.iter(|| black_box(FilterValue::String("test_value".into())))
    });

    group.bench_function("create_json", |b| {
        b.iter(|| {
            black_box(FilterValue::Json(
                serde_json::json!({"key": "value", "nested": {"a": 1}}),
            ))
        })
    });

    group.bench_function("create_list", |b| {
        b.iter(|| {
            black_box(FilterValue::List(vec![
                FilterValue::Int(1),
                FilterValue::Int(2),
                FilterValue::Int(3),
            ]))
        })
    });

    group.finish();
}

// ============================================================================
// Scalar Filter Benchmarks
// ============================================================================

fn bench_scalar_filters(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalar_filters");

    group.bench_function("create_equals", |b| {
        b.iter(|| black_box(ScalarFilter::Equals(FilterValue::Int(1))))
    });

    group.bench_function("create_not", |b| {
        b.iter(|| {
            black_box(ScalarFilter::Not(Box::new(FilterValue::String(
                "test".into(),
            ))))
        })
    });

    group.bench_function("create_lt", |b| {
        b.iter(|| black_box(ScalarFilter::Lt(FilterValue::Int(100))))
    });

    group.bench_function("create_in", |b| {
        b.iter(|| {
            black_box(ScalarFilter::In(vec![
                FilterValue::Int(1),
                FilterValue::Int(2),
                FilterValue::Int(3),
            ]))
        })
    });

    group.bench_function("scalar_to_filter", |b| {
        b.iter(|| {
            let _scalar = ScalarFilter::Equals(FilterValue::Int(42));
            // Convert to Filter using Into trait
            black_box(Filter::Equals("id".into(), FilterValue::Int(42)))
        })
    });

    group.finish();
}

// ============================================================================
// Filter to SQL Benchmarks
// ============================================================================

fn bench_filter_to_sql(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_to_sql");

    group.bench_function("simple_equals_to_sql", |b| {
        let filter = Filter::Equals("id".into(), FilterValue::Int(1));
        b.iter(|| black_box(filter.to_sql(0)))
    });

    group.bench_function("in_filter_to_sql", |b| {
        let filter = Filter::In(
            "id".into(),
            vec![
                FilterValue::Int(1),
                FilterValue::Int(2),
                FilterValue::Int(3),
                FilterValue::Int(4),
                FilterValue::Int(5),
            ],
        );
        b.iter(|| black_box(filter.to_sql(0)))
    });

    group.bench_function("and_filter_to_sql", |b| {
        let filter = Filter::and(vec![
            Filter::Equals("status".into(), FilterValue::String("active".into())),
            Filter::Equals("role".into(), FilterValue::String("admin".into())),
        ]);
        b.iter(|| black_box(filter.to_sql(0)))
    });

    group.bench_function("complex_filter_to_sql", |b| {
        let filter = Filter::and(vec![
            Filter::Equals("status".into(), FilterValue::String("active".into())),
            Filter::or(vec![
                Filter::Contains("email".into(), FilterValue::String("@admin".into())),
                Filter::Equals("role".into(), FilterValue::String("admin".into())),
            ]),
            Filter::Gt("age".into(), FilterValue::Int(18)),
            Filter::Lt("age".into(), FilterValue::Int(65)),
        ]);
        b.iter(|| black_box(filter.to_sql(0)))
    });

    group.finish();
}

// ============================================================================
// SQL Builder Benchmarks
// ============================================================================

fn bench_sql_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("sql_builder");

    group.bench_function("create_postgres_builder", |b| {
        b.iter(|| black_box(SqlBuilder::postgres()))
    });

    group.bench_function("create_mysql_builder", |b| {
        b.iter(|| black_box(SqlBuilder::mysql()))
    });

    group.bench_function("simple_push", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users");
            black_box(builder)
        })
    });

    group.bench_function("push_with_param", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users WHERE id = ");
            builder.push_param(FilterValue::Int(1));
            black_box(builder)
        })
    });

    group.bench_function("build_simple_query", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users WHERE id = ");
            builder.push_param(FilterValue::Int(1));
            let result = builder.build();
            black_box(result)
        })
    });

    group.bench_function("build_complex_query", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT id, email, name FROM users WHERE status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(" AND role = ");
            builder.push_param(FilterValue::String("admin".into()));
            builder.push(" ORDER BY created_at DESC LIMIT ");
            builder.push_param(FilterValue::Int(10));
            let result = builder.build();
            black_box(result)
        })
    });

    group.finish();
}

// ============================================================================
// Raw SQL Benchmarks
// ============================================================================

fn bench_raw_sql(c: &mut Criterion) {
    let mut group = c.benchmark_group("raw_sql");

    group.bench_function("create_raw_sql", |b| {
        b.iter(|| black_box(Sql::new("SELECT * FROM users")))
    });

    group.bench_function("raw_sql_with_bind", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = $1").bind(FilterValue::Int(1));
            black_box(sql)
        })
    });

    group.bench_function("raw_sql_multiple_binds", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE status = $1 AND role = $2 AND age > $3")
                .bind(FilterValue::String("active".into()))
                .bind(FilterValue::String("admin".into()))
                .bind(FilterValue::Int(18));
            black_box(sql)
        })
    });

    group.bench_function("raw_sql_build", |b| {
        b.iter(|| {
            let sql = Sql::new("SELECT * FROM users WHERE id = $1").bind(FilterValue::Int(42));
            black_box(sql.build())
        })
    });

    group.finish();
}

// ============================================================================
// Order By Benchmarks
// ============================================================================

fn bench_order_by(c: &mut Criterion) {
    let mut group = c.benchmark_group("order_by");

    group.bench_function("create_asc", |b| {
        b.iter(|| black_box(OrderByField::asc("created_at")))
    });

    group.bench_function("create_desc", |b| {
        b.iter(|| black_box(OrderByField::desc("created_at")))
    });

    group.bench_function("order_field_to_sql", |b| {
        let order = OrderByField::desc("created_at");
        b.iter(|| black_box(order.to_sql()))
    });

    group.bench_function("order_by_none", |b| b.iter(|| black_box(OrderBy::none())));

    group.bench_function("order_by_then", |b| {
        b.iter(|| {
            let order: OrderBy = OrderByField::desc("created_at").into();
            black_box(order.then(OrderByField::asc("id")))
        })
    });

    group.bench_function("order_by_to_sql", |b| {
        let order: OrderBy = OrderByField::desc("created_at").into();
        b.iter(|| black_box(order.to_sql()))
    });

    group.finish();
}

// ============================================================================
// Select Benchmarks
// ============================================================================

fn bench_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("select");

    group.bench_function("create_all", |b| b.iter(|| black_box(Select::all())));

    group.bench_function("create_fields", |b| {
        b.iter(|| black_box(Select::fields(["id", "email", "name", "created_at"])))
    });

    group.bench_function("create_single_field", |b| {
        b.iter(|| black_box(Select::field("email")))
    });

    group.bench_function("select_to_sql", |b| {
        let select = Select::fields(["id", "email", "name"]);
        b.iter(|| black_box(select.to_sql()))
    });

    group.finish();
}

// ============================================================================
// Batch Benchmarks
// ============================================================================

fn bench_batch_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_operations");

    for count in [10, 50, 100, 500].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(
            BenchmarkId::new("create_n_filters", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let filters: Vec<Filter> = (0..count)
                        .map(|i| {
                            Filter::Equals(
                                format!("field_{}", i).into(),
                                FilterValue::Int(i as i64),
                            )
                        })
                        .collect();
                    black_box(filters)
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("create_n_filter_values", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let values: Vec<FilterValue> =
                        (0..count).map(|i| FilterValue::Int(i as i64)).collect();
                    black_box(values)
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("create_large_in_filter", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let values: Vec<FilterValue> =
                        (0..count).map(|i| FilterValue::Int(i as i64)).collect();
                    black_box(Filter::In("id".into(), values))
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_filter_creation,
    bench_filter_values,
    bench_scalar_filters,
    bench_filter_to_sql,
    bench_sql_builder,
    bench_raw_sql,
    bench_order_by,
    bench_select,
    bench_batch_operations,
);

criterion_main!(benches);
