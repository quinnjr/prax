//! Throughput benchmarks measuring queries-per-second for common ORM patterns.
//!
//! These benchmarks measure the sustained throughput of the ORM layer,
//! simulating real-world usage patterns.
//!
//! Run with: `cargo bench --package prax-query --bench throughput_bench`

use criterion::{
use std::hint::black_box;
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use prax_query::{
    filter::{Filter, FilterValue},
    sql::SqlBuilder,
    types::{OrderByField, Select},
    mem_optimize::{
        arena::QueryArena,
        interning::{GlobalInterner, ScopedInterner},
    },
};
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// Simple CRUD Operation Throughput
// ============================================================================

fn bench_crud_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("crud_throughput");
    group.measurement_time(Duration::from_secs(10));

    // Find by ID - the most common operation
    group.throughput(Throughput::Elements(1));
    group.bench_function("find_by_id", |b| {
        b.iter(|| {
            let filter = Filter::Equals("id".into(), FilterValue::Int(12345));
            let (sql, params) = filter.to_sql(0);
            let query = format!("SELECT * FROM users WHERE {}", sql);
            black_box((query, params))
        });
    });

    // Find many with simple filter
    group.bench_function("find_many_simple", |b| {
        b.iter(|| {
            let filter = Filter::Equals("status".into(), FilterValue::String("active".into()));
            let (sql, params) = filter.to_sql(0);
            let query = format!("SELECT * FROM users WHERE {} LIMIT 100", sql);
            black_box((query, params))
        });
    });

    // Find many with complex filter
    group.bench_function("find_many_complex", |b| {
        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::Gte("age".into(), FilterValue::Int(18)),
                Filter::Lt("age".into(), FilterValue::Int(65)),
                Filter::IsNotNull("email".into()),
            ]);
            let (sql, params) = filter.to_sql(0);
            let query = format!(
                "SELECT * FROM users WHERE {} ORDER BY created_at DESC LIMIT 50",
                sql
            );
            black_box((query, params))
        });
    });

    // Update query
    group.bench_function("update_by_id", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("UPDATE users SET status = ");
            builder.push_param(FilterValue::String("inactive".into()));
            builder.push(", updated_at = NOW() WHERE id = ");
            builder.push_param(FilterValue::Int(12345));
            black_box(builder.build())
        });
    });

    // Delete query
    group.bench_function("delete_by_id", |b| {
        b.iter(|| {
            let filter = Filter::Equals("id".into(), FilterValue::Int(12345));
            let (sql, params) = filter.to_sql(0);
            let query = format!("DELETE FROM users WHERE {}", sql);
            black_box((query, params))
        });
    });

    // Insert query
    group.bench_function("insert_single", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("INSERT INTO users (email, name, status) VALUES (");
            builder.push_param(FilterValue::String("test@example.com".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("Test User".into()));
            builder.push(", ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(") RETURNING id");
            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// Batch Operation Throughput
// ============================================================================

fn bench_batch_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_throughput");
    group.measurement_time(Duration::from_secs(10));

    for batch_size in [10, 50, 100, 500, 1000] {
        group.throughput(Throughput::Elements(batch_size as u64));

        // Batch insert
        group.bench_function(BenchmarkId::new("batch_insert", batch_size), |b| {
            b.iter(|| {
                let mut builder = SqlBuilder::postgres();
                builder.push("INSERT INTO events (user_id, event_type, created_at) VALUES ");

                for i in 0..batch_size {
                    if i > 0 {
                        builder.push(", ");
                    }
                    builder.push("(");
                    builder.push_param(FilterValue::Int((i % 100) as i64));
                    builder.push(", ");
                    builder.push_param(FilterValue::String("page_view".into()));
                    builder.push(", NOW())");
                }

                black_box(builder.build())
            });
        });

        // IN clause query
        group.bench_function(BenchmarkId::new("in_clause_query", batch_size), |b| {
            let ids: Vec<FilterValue> = (0..batch_size).map(|i| FilterValue::Int(i as i64)).collect();
            let filter = Filter::In("id".into(), ids);
            b.iter(|| {
                let (sql, params) = filter.to_sql(0);
                let query = format!("SELECT * FROM users WHERE {}", sql);
                black_box((query, params))
            });
        });

        // Batch delete
        group.bench_function(BenchmarkId::new("batch_delete", batch_size), |b| {
            let ids: Vec<FilterValue> = (0..batch_size).map(|i| FilterValue::Int(i as i64)).collect();
            let filter = Filter::In("id".into(), ids);
            b.iter(|| {
                let (sql, params) = filter.to_sql(0);
                let query = format!("DELETE FROM users WHERE {}", sql);
                black_box((query, params))
            });
        });
    }

    group.finish();
}

// ============================================================================
// Query Pattern Throughput
// ============================================================================

fn bench_query_pattern_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_patterns");
    group.measurement_time(Duration::from_secs(10));

    // Pagination query
    group.throughput(Throughput::Elements(1));
    group.bench_function("paginated_list", |b| {
        b.iter(|| {
            let filter = Filter::Equals("status".into(), FilterValue::String("active".into()));
            let (where_sql, params) = filter.to_sql(0);

            let query = format!(
                "SELECT id, email, name, created_at FROM users WHERE {} ORDER BY created_at DESC LIMIT 25 OFFSET 100",
                where_sql
            );
            black_box((query, params))
        });
    });

    // Search query with multiple conditions
    group.bench_function("search_query", |b| {
        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Contains("name".into(), FilterValue::String("john".into())),
                Filter::or(vec![
                    Filter::Contains("email".into(), FilterValue::String("@gmail".into())),
                    Filter::Contains("email".into(), FilterValue::String("@outlook".into())),
                ]),
                Filter::Equals("status".into(), FilterValue::String("active".into())),
            ]);
            let (sql, params) = filter.to_sql(0);
            let query = format!(
                "SELECT * FROM users WHERE {} ORDER BY relevance DESC LIMIT 20",
                sql
            );
            black_box((query, params))
        });
    });

    // Aggregation query
    group.bench_function("aggregation_query", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT status, COUNT(*) as count, MAX(created_at) as latest ");
            builder.push("FROM users WHERE created_at >= ");
            builder.push_param(FilterValue::String("2024-01-01".into()));
            builder.push(" GROUP BY status ORDER BY count DESC");
            black_box(builder.build())
        });
    });

    // Join query
    group.bench_function("join_query", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT u.id, u.email, p.title, p.created_at ");
            builder.push("FROM users u ");
            builder.push("INNER JOIN posts p ON p.user_id = u.id ");
            builder.push("WHERE u.status = ");
            builder.push_param(FilterValue::String("active".into()));
            builder.push(" AND p.status = ");
            builder.push_param(FilterValue::String("published".into()));
            builder.push(" ORDER BY p.created_at DESC LIMIT 50");
            black_box(builder.build())
        });
    });

    // Subquery
    group.bench_function("subquery", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT * FROM users WHERE id IN (");
            builder.push("SELECT user_id FROM orders WHERE total > ");
            builder.push_param(FilterValue::Float(100.0));
            builder.push(" AND created_at >= ");
            builder.push_param(FilterValue::String("2024-01-01".into()));
            builder.push(")");
            black_box(builder.build())
        });
    });

    group.finish();
}

// ============================================================================
// High-Throughput Scenarios
// ============================================================================

fn bench_high_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_throughput");
    group.measurement_time(Duration::from_secs(15));
    group.sample_size(200);

    // Simulate 1000 queries
    group.throughput(Throughput::Elements(1000));

    group.bench_function("1000_simple_queries", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let filter = Filter::Equals("id".into(), FilterValue::Int(i));
                let (sql, params) = filter.to_sql(0);
                black_box((sql, params));
            }
        });
    });

    group.bench_function("1000_queries_with_interning", |b| {
        let interner = GlobalInterner::get();
        let id_field = interner.intern("id");

        b.iter(|| {
            for i in 0..1000 {
                let filter = Filter::Equals(id_field.to_cow(), FilterValue::Int(i));
                let (sql, params) = filter.to_sql(0);
                black_box((sql, params));
            }
        });
    });

    group.bench_function("1000_queries_with_arena", |b| {
        let arena = QueryArena::new();

        b.iter(|| {
            for i in 0..1000 {
                let sql = arena.scope(|scope| {
                    scope.build_select("users", scope.eq("id", i as i64))
                });
                black_box(sql);
            }
        });
    });

    group.finish();
}

// ============================================================================
// Real-World Scenario Benchmarks
// ============================================================================

fn bench_realistic_scenarios(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_scenarios");
    group.measurement_time(Duration::from_secs(10));

    // E-commerce: Product search
    group.throughput(Throughput::Elements(1));
    group.bench_function("ecommerce_product_search", |b| {
        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Contains("name".into(), FilterValue::String("laptop".into())),
                Filter::Gte("price".into(), FilterValue::Float(500.0)),
                Filter::Lte("price".into(), FilterValue::Float(2000.0)),
                Filter::Equals("in_stock".into(), FilterValue::Bool(true)),
                Filter::In(
                    "category_id".into(),
                    vec![
                        FilterValue::Int(1),
                        FilterValue::Int(2),
                        FilterValue::Int(5),
                    ],
                ),
            ]);
            let (sql, params) = filter.to_sql(0);
            let query = format!(
                "SELECT * FROM products WHERE {} ORDER BY relevance DESC, price ASC LIMIT 24",
                sql
            );
            black_box((query, params))
        });
    });

    // SaaS: User dashboard data
    group.bench_function("saas_dashboard", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("WITH user_stats AS (");
            builder.push("SELECT user_id, COUNT(*) as action_count, MAX(created_at) as last_action ");
            builder.push("FROM user_actions WHERE created_at >= NOW() - INTERVAL '7 days' ");
            builder.push("AND user_id = ");
            builder.push_param(FilterValue::Int(12345));
            builder.push(" GROUP BY user_id");
            builder.push(") SELECT u.*, s.action_count, s.last_action ");
            builder.push("FROM users u LEFT JOIN user_stats s ON s.user_id = u.id ");
            builder.push("WHERE u.id = ");
            builder.push_param(FilterValue::Int(12345));
            black_box(builder.build())
        });
    });

    // Social: Feed generation
    group.bench_function("social_feed", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT p.*, u.name as author_name, u.avatar_url ");
            builder.push("FROM posts p ");
            builder.push("INNER JOIN users u ON u.id = p.user_id ");
            builder.push("WHERE p.user_id IN (SELECT following_id FROM follows WHERE follower_id = ");
            builder.push_param(FilterValue::Int(12345));
            builder.push(") AND p.status = ");
            builder.push_param(FilterValue::String("published".into()));
            builder.push(" AND p.created_at >= NOW() - INTERVAL '7 days' ");
            builder.push("ORDER BY p.created_at DESC LIMIT 50");
            black_box(builder.build())
        });
    });

    // Analytics: Event aggregation
    group.bench_function("analytics_aggregation", |b| {
        b.iter(|| {
            let mut builder = SqlBuilder::postgres();
            builder.push("SELECT ");
            builder.push("DATE_TRUNC('hour', created_at) as hour, ");
            builder.push("event_type, ");
            builder.push("COUNT(*) as event_count, ");
            builder.push("COUNT(DISTINCT user_id) as unique_users ");
            builder.push("FROM events ");
            builder.push("WHERE tenant_id = ");
            builder.push_param(FilterValue::Int(1));
            builder.push(" AND created_at >= ");
            builder.push_param(FilterValue::String("2024-01-01".into()));
            builder.push(" AND created_at < ");
            builder.push_param(FilterValue::String("2024-01-02".into()));
            builder.push(" GROUP BY DATE_TRUNC('hour', created_at), event_type ");
            builder.push("ORDER BY hour, event_count DESC");
            black_box(builder.build())
        });
    });

    // Multi-tenant: Tenant-scoped query
    group.bench_function("multitenant_query", |b| {
        b.iter(|| {
            let filter = Filter::and(vec![
                Filter::Equals("tenant_id".into(), FilterValue::Int(42)),
                Filter::Equals("status".into(), FilterValue::String("active".into())),
                Filter::Gte("created_at".into(), FilterValue::String("2024-01-01".into())),
            ]);
            let (sql, params) = filter.to_sql(0);
            let query = format!(
                "SELECT * FROM orders WHERE {} ORDER BY created_at DESC LIMIT 100",
                sql
            );
            black_box((query, params))
        });
    });

    group.finish();
}

// ============================================================================
// Memory Allocation Throughput
// ============================================================================

fn bench_allocation_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_throughput");
    group.measurement_time(Duration::from_secs(10));

    // Measure string allocation overhead
    group.throughput(Throughput::Elements(100));

    group.bench_function("100_filters_no_optimization", |b| {
        b.iter(|| {
            let filters: Vec<Filter> = (0..100)
                .map(|i| {
                    Filter::Equals(
                        format!("field_{}", i % 10).into(),
                        FilterValue::Int(i),
                    )
                })
                .collect();
            black_box(filters)
        });
    });

    group.bench_function("100_filters_with_interning", |b| {
        let interner = GlobalInterner::get();
        // Pre-intern field names
        let field_names: Vec<_> = (0..10)
            .map(|i| interner.intern(&format!("field_{}", i)))
            .collect();

        b.iter(|| {
            let filters: Vec<Filter> = (0..100)
                .map(|i| {
                    Filter::Equals(
                        field_names[i % 10].to_cow(),
                        FilterValue::Int(i as i64),
                    )
                })
                .collect();
            black_box(filters)
        });
    });

    group.bench_function("100_filters_with_arena", |b| {
        let mut arena = QueryArena::with_capacity(8192);

        b.iter(|| {
            let sql = arena.scope(|scope| {
                let mut filters = Vec::with_capacity(100);
                for i in 0..100 {
                    filters.push(scope.eq(&format!("field_{}", i % 10), i as i64));
                }
                let filter = scope.and(filters);
                scope.build_select("test", filter)
            });
            arena.reset();
            black_box(sql)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_crud_throughput,
    bench_batch_throughput,
    bench_query_pattern_throughput,
    bench_high_throughput,
    bench_realistic_scenarios,
    bench_allocation_throughput,
);

criterion_main!(benches);

