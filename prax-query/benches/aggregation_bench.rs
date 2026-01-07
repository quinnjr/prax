//! Benchmarks for filter and SQL generation performance.
//!
//! Note: Aggregate operations require a QueryEngine which isn't available
//! in benchmarks, so we focus on filter and SQL building performance.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_query::{
    filter::{Filter, FilterValue},
    sql::SqlBuilder,
};

// ============================================================================
// Complex Filter Benchmarks (Aggregation-style queries)
// ============================================================================

fn bench_aggregation_style_filters(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_style_filters");

    group.bench_function("simple_count_filter", |b| {
        b.iter(|| {
            black_box(Filter::Equals(
                "status".into(),
                FilterValue::String("completed".into()),
            ))
        })
    });

    group.bench_function("count_with_date_filter", |b| {
        b.iter(|| {
            black_box(Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("completed".into())),
                Filter::Gte(
                    "created_at".into(),
                    FilterValue::String("2024-01-01".into()),
                ),
            ]))
        })
    });

    group.bench_function("sum_filter_with_conditions", |b| {
        b.iter(|| {
            black_box(Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("completed".into())),
                Filter::Gt("total_amount".into(), FilterValue::Int(100)),
                Filter::Lt(
                    "created_at".into(),
                    FilterValue::String("2024-01-01".into()),
                ),
            ]))
        })
    });

    group.bench_function("group_by_style_filter", |b| {
        b.iter(|| {
            black_box(Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("completed".into())),
                Filter::In(
                    "category_id".into(),
                    vec![
                        FilterValue::Int(1),
                        FilterValue::Int(2),
                        FilterValue::Int(3),
                    ],
                ),
            ]))
        })
    });

    group.finish();
}

// ============================================================================
// Aggregation SQL Generation Benchmarks
// ============================================================================

fn bench_aggregation_sql_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("aggregation_sql_generation");

    group.bench_function("count_all_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT COUNT(*) FROM orders");
            black_box(builder.build())
        })
    });

    group.bench_function("count_with_filter_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT COUNT(*) FROM orders WHERE status = ");
            builder.push_param(FilterValue::String("completed".into()));
            black_box(builder.build())
        })
    });

    group.bench_function("sum_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT SUM(total_amount) FROM orders WHERE status = ");
            builder.push_param(FilterValue::String("completed".into()));
            black_box(builder.build())
        })
    });

    group.bench_function("multiple_aggregates_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push(
                "SELECT COUNT(*), SUM(total_amount), AVG(item_count) FROM orders WHERE status = ",
            );
            builder.push_param(FilterValue::String("completed".into()));
            black_box(builder.build())
        })
    });

    group.bench_function("group_by_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push(
                "SELECT category_id, COUNT(*), SUM(total_amount) FROM orders WHERE status = ",
            );
            builder.push_param(FilterValue::String("completed".into()));
            builder.push(" GROUP BY category_id");
            black_box(builder.build())
        })
    });

    group.bench_function("group_by_with_having_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT customer_id, COUNT(*) as cnt FROM orders GROUP BY customer_id HAVING COUNT(*) > ");
            builder.push_param(FilterValue::Int(5));
            black_box(builder.build())
        })
    });

    group.bench_function("complex_aggregation_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT category, COUNT(*) as count, SUM(total_amount) as total, AVG(item_count) as avg_items FROM orders WHERE status = ");
            builder.push_param(FilterValue::String("completed".into()));
            builder.push(" AND created_at >= ");
            builder.push_param(FilterValue::String("2024-01-01".into()));
            builder.push(" GROUP BY category HAVING SUM(total_amount) > ");
            builder.push_param(FilterValue::Int(1000));
            builder.push(" ORDER BY total DESC LIMIT ");
            builder.push_param(FilterValue::Int(10));
            black_box(builder.build())
        })
    });

    group.finish();
}

// ============================================================================
// Real-World Aggregation SQL Benchmarks
// ============================================================================

fn bench_real_world_aggregation_sql(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_world_aggregation_sql");

    // E-commerce dashboard aggregates
    group.bench_function("ecommerce_order_stats_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT COUNT(*) as total_orders, SUM(total_amount) as revenue, AVG(total_amount) as avg_order FROM orders WHERE status = ");
            builder.push_param(FilterValue::String("completed".into()));
            builder.push(" AND created_at >= ");
            builder.push_param(FilterValue::String("2024-01-01".into()));
            black_box(builder.build())
        })
    });

    // Sales by category
    group.bench_function("sales_by_category_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT category_id, COUNT(*) as item_count, SUM(quantity) as total_qty, SUM(line_total) as total_sales FROM order_items GROUP BY category_id HAVING COUNT(*) > ");
            builder.push_param(FilterValue::Int(10));
            builder.push(" ORDER BY total_sales DESC LIMIT ");
            builder.push_param(FilterValue::Int(10));
            black_box(builder.build())
        })
    });

    // User engagement metrics
    group.bench_function("user_engagement_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT user_id, COUNT(*) as event_count, MIN(created_at) as first_event, MAX(created_at) as last_event FROM user_events WHERE event_type IN (");
            builder.push_param(FilterValue::String("page_view".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("click".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("purchase".into()));
            builder.push(") GROUP BY user_id HAVING COUNT(*) > ");
            builder.push_param(FilterValue::Int(5));
            black_box(builder.build())
        })
    });

    // Inventory analysis
    group.bench_function("inventory_analysis_sql", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT warehouse_id, COUNT(*) as product_count, SUM(stock_quantity) as total_stock, AVG(unit_cost) as avg_cost, MIN(stock_quantity) as min_stock, MAX(stock_quantity) as max_stock FROM products WHERE is_active = ");
            builder.push_param(FilterValue::Bool(true));
            builder.push(" GROUP BY warehouse_id");
            black_box(builder.build())
        })
    });

    group.finish();
}

// ============================================================================
// Batch Aggregation Benchmarks
// ============================================================================

fn bench_batch_aggregation(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_aggregation");

    for count in [5, 10, 20].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(
            BenchmarkId::new("build_n_aggregate_columns", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let mut builder = SqlBuilder::postgres();
                    builder.push("SELECT ");
                    for i in 0..count {
                        if i > 0 {
                            builder.push(", ");
                        }
                        match i % 5 {
                            0 => builder.push("COUNT(*)"),
                            1 => builder.push(format!("SUM(field_{})", i)),
                            2 => builder.push(format!("AVG(field_{})", i)),
                            3 => builder.push(format!("MIN(field_{})", i)),
                            _ => builder.push(format!("MAX(field_{})", i)),
                        };
                    }
                    builder.push(" FROM analytics");
                    black_box(builder.build())
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("build_n_group_by_columns", count),
            count,
            |b, &count| {
                b.iter(|| {
                    let mut builder = SqlBuilder::postgres();
                    builder.push("SELECT ");
                    for i in 0..count {
                        if i > 0 {
                            builder.push(", ");
                        }
                        builder.push(format!("col_{}", i));
                    }
                    builder.push(", COUNT(*) FROM data GROUP BY ");
                    for i in 0..count {
                        if i > 0 {
                            builder.push(", ");
                        }
                        builder.push(format!("col_{}", i));
                    }
                    black_box(builder.build())
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_aggregation_style_filters,
    bench_aggregation_sql_generation,
    bench_real_world_aggregation_sql,
    bench_batch_aggregation,
);

criterion_main!(benches);
