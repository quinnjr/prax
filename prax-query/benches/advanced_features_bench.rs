//! Benchmarks for advanced query features.
//!
//! This benchmark suite covers the core advanced features implemented in prax-query:
//! - Common Table Expressions (CTEs)
//! - Window Functions
//! - JSON & Document Operations
//! - Full-Text Search
//! - Upsert & Conflict Resolution
//! - Sequences
//! - Triggers
//! - Stored Procedures
//! - Security (RLS, Roles)

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

use prax_query::cte::Cte;
use prax_query::json::JsonPath;
use prax_query::procedure::ProcedureCall;
use prax_query::search::{SearchMode, SearchQueryBuilder};
use prax_query::security::{RlsPolicyBuilder, RoleBuilder};
use prax_query::sequence::SequenceBuilder;
use prax_query::sql::DatabaseType;
use prax_query::trigger::{TriggerBuilder, TriggerEvent, TriggerTiming};
use prax_query::upsert::UpsertBuilder;
use prax_query::window::{FrameBound, WindowFn, WindowFunction, WindowSpec};
use prax_query::Point;

// ============================================================================
// CTE (Common Table Expressions) Benchmarks
// ============================================================================

fn bench_cte_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("cte_operations");

    group.bench_function("create_simple_cte", |b| {
        b.iter(|| {
            black_box(
                Cte::new("active_users")
                    .as_query("SELECT * FROM users WHERE status = 'active'"),
            )
        })
    });

    group.bench_function("create_recursive_cte", |b| {
        b.iter(|| {
            black_box(
                Cte::new("employee_tree")
                    .recursive()
                    .columns(vec!["id", "name", "manager_id", "level"])
                    .as_query(
                        "SELECT id, name, manager_id, 0 FROM employees WHERE manager_id IS NULL \
                         UNION ALL \
                         SELECT e.id, e.name, e.manager_id, et.level + 1 \
                         FROM employees e JOIN employee_tree et ON e.manager_id = et.id",
                    ),
            )
        })
    });

    group.bench_function("cte_to_sql_postgres", |b| {
        let cte = Cte::new("active_users")
            .as_query("SELECT * FROM users WHERE status = 'active'");

        b.iter(|| black_box(cte.to_sql(DatabaseType::PostgreSQL)))
    });

    group.bench_function("recursive_cte_to_sql", |b| {
        let cte = Cte::new("tree")
            .recursive()
            .columns(vec!["id", "parent_id", "depth"])
            .as_query("SELECT id, parent_id, 0 FROM nodes WHERE parent_id IS NULL UNION ALL SELECT n.id, n.parent_id, t.depth + 1 FROM nodes n JOIN tree t ON n.parent_id = t.id");

        b.iter(|| black_box(cte.to_sql(DatabaseType::PostgreSQL)))
    });

    group.finish();
}

// ============================================================================
// Window Functions Benchmarks
// ============================================================================

fn bench_window_functions(c: &mut Criterion) {
    use prax_query::types::SortOrder;

    let mut group = c.benchmark_group("window_functions");

    group.bench_function("create_row_number", |b| {
        b.iter(|| {
            black_box(WindowFunction {
                function: WindowFn::RowNumber,
                over: WindowSpec::new()
                    .partition_by(["department"])
                    .order_by("salary", SortOrder::Desc),
                alias: Some("row_num".to_string()),
            })
        })
    });

    group.bench_function("create_rank_with_partition", |b| {
        b.iter(|| {
            black_box(WindowFunction {
                function: WindowFn::Rank,
                over: WindowSpec::new()
                    .partition_by(["department", "team"])
                    .order_by("score", SortOrder::Desc),
                alias: Some("rank".to_string()),
            })
        })
    });

    group.bench_function("create_lag_function", |b| {
        b.iter(|| {
            black_box(WindowFunction {
                function: WindowFn::Lag {
                    expr: "price".to_string(),
                    offset: Some(1),
                    default: Some("0".to_string()),
                },
                over: WindowSpec::new()
                    .partition_by(["product_id"])
                    .order_by("date", SortOrder::Asc),
                alias: Some("prev_price".to_string()),
            })
        })
    });

    group.bench_function("create_sum_with_frame", |b| {
        b.iter(|| {
            black_box(WindowFunction {
                function: WindowFn::Sum("amount".to_string()),
                over: WindowSpec::new()
                    .partition_by(["account_id"])
                    .order_by("date", SortOrder::Asc)
                    .rows(FrameBound::UnboundedPreceding, Some(FrameBound::CurrentRow)),
                alias: Some("running_total".to_string()),
            })
        })
    });

    group.bench_function("window_to_sql_postgres", |b| {
        let window = WindowFunction {
            function: WindowFn::RowNumber,
            over: WindowSpec::new()
                .partition_by(["department"])
                .order_by("salary", SortOrder::Desc),
            alias: Some("row_num".to_string()),
        };

        b.iter(|| black_box(window.to_sql(DatabaseType::PostgreSQL)))
    });

    group.finish();
}

// ============================================================================
// JSON Operations Benchmarks
// ============================================================================

fn bench_json_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_operations");

    group.bench_function("create_json_path", |b| {
        b.iter(|| black_box(JsonPath::from_path("data", "$.user.profile.settings.theme")))
    });

    group.bench_function("json_path_to_sql_postgres", |b| {
        let path = JsonPath::from_path("data", "$.user.profile.avatar");
        b.iter(|| black_box(path.to_sql(DatabaseType::PostgreSQL)))
    });

    group.bench_function("json_path_to_sql_mysql", |b| {
        let path = JsonPath::from_path("config", "$.settings.theme.color");
        b.iter(|| black_box(path.to_sql(DatabaseType::MySQL)))
    });

    group.finish();
}

// ============================================================================
// Full-Text Search Benchmarks
// ============================================================================

fn bench_search_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_operations");

    group.bench_function("create_simple_search", |b| {
        b.iter(|| {
            black_box(
                SearchQueryBuilder::new("rust programming")
                    .columns(vec!["title", "content"])
                    .build(),
            )
        })
    });

    group.bench_function("create_search_with_options", |b| {
        b.iter(|| {
            black_box(
                SearchQueryBuilder::new("database optimization")
                    .columns(vec!["title", "body", "tags"])
                    .mode(SearchMode::Boolean)
                    .with_ranking()
                    .with_highlight()
                    .build(),
            )
        })
    });

    group.bench_function("search_to_sql_postgres", |b| {
        let search = SearchQueryBuilder::new("full text search")
            .columns(vec!["content"])
            .mode(SearchMode::Natural)
            .build();

        b.iter(|| black_box(search.to_sql("articles", DatabaseType::PostgreSQL)))
    });

    group.bench_function("search_to_sql_mysql", |b| {
        let search = SearchQueryBuilder::new("full text search")
            .columns(vec!["title", "body"])
            .mode(SearchMode::Boolean)
            .build();

        b.iter(|| black_box(search.to_sql("posts", DatabaseType::MySQL)))
    });

    group.finish();
}

// ============================================================================
// Upsert & Conflict Resolution Benchmarks
// ============================================================================

fn bench_upsert_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("upsert_operations");

    group.bench_function("create_simple_upsert", |b| {
        b.iter(|| {
            black_box(
                UpsertBuilder::new("users")
                    .columns(vec!["email", "name", "updated_at"])
                    .on_conflict_columns(vec!["email"])
                    .do_update(vec!["name", "updated_at"])
                    .build(),
            )
        })
    });

    group.bench_function("create_upsert_do_nothing", |b| {
        b.iter(|| {
            black_box(
                UpsertBuilder::new("log_entries")
                    .columns(vec!["id", "message", "timestamp"])
                    .on_conflict_columns(vec!["id"])
                    .do_nothing()
                    .build(),
            )
        })
    });

    group.bench_function("upsert_to_sql_postgres", |b| {
        let upsert = UpsertBuilder::new("users")
            .columns(vec!["email", "name"])
            .on_conflict_columns(vec!["email"])
            .do_update(vec!["name"])
            .build()
            .unwrap();

        b.iter(|| black_box(upsert.to_sql(DatabaseType::PostgreSQL)))
    });

    group.bench_function("upsert_to_sql_mysql", |b| {
        let upsert = UpsertBuilder::new("users")
            .columns(vec!["email", "name"])
            .on_conflict_columns(vec!["email"])
            .do_update(vec!["name"])
            .build()
            .unwrap();

        b.iter(|| black_box(upsert.to_sql(DatabaseType::MySQL)))
    });

    group.finish();
}

// ============================================================================
// Sequence Operations Benchmarks
// ============================================================================

fn bench_sequence_operations(c: &mut Criterion) {
    use prax_query::sequence;

    let mut group = c.benchmark_group("sequence_operations");

    group.bench_function("create_simple_sequence", |b| {
        b.iter(|| {
            black_box(
                SequenceBuilder::new("user_id_seq")
                    .start(1)
                    .increment(1)
                    .build(),
            )
        })
    });

    group.bench_function("create_sequence_with_options", |b| {
        b.iter(|| {
            black_box(
                SequenceBuilder::new("order_number_seq")
                    .start(1000)
                    .increment(1)
                    .min_value(1000)
                    .max_value(999999999)
                    .cache(20)
                    .cycle(false)
                    .build(),
            )
        })
    });

    group.bench_function("sequence_to_sql_postgres", |b| {
        let seq = SequenceBuilder::new("id_seq")
            .start(1)
            .increment(1)
            .cache(10)
            .build();

        b.iter(|| black_box(seq.to_create_sql(DatabaseType::PostgreSQL)))
    });

    group.bench_function("nextval_sql", |b| {
        b.iter(|| black_box(sequence::ops::nextval("order_seq", DatabaseType::PostgreSQL)))
    });

    group.bench_function("currval_sql", |b| {
        b.iter(|| black_box(sequence::ops::currval("order_seq", DatabaseType::PostgreSQL)))
    });

    group.finish();
}

// ============================================================================
// Trigger Operations Benchmarks
// ============================================================================

fn bench_trigger_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("trigger_operations");

    group.bench_function("create_simple_trigger", |b| {
        b.iter(|| {
            black_box(
                TriggerBuilder::new("update_timestamp")
                    .on_table("users")
                    .timing(TriggerTiming::Before)
                    .event(TriggerEvent::Update)
                    .execute_function("update_modified_column")
                    .build(),
            )
        })
    });

    group.bench_function("create_trigger_with_events", |b| {
        b.iter(|| {
            black_box(
                TriggerBuilder::new("audit_changes")
                    .on_table("accounts")
                    .timing(TriggerTiming::After)
                    .events(vec![TriggerEvent::Insert, TriggerEvent::Update, TriggerEvent::Delete])
                    .for_each_row()
                    .execute_function("log_balance_change")
                    .build(),
            )
        })
    });

    group.bench_function("trigger_to_sql_postgres", |b| {
        let trigger = TriggerBuilder::new("set_updated_at")
            .on_table("orders")
            .timing(TriggerTiming::Before)
            .event(TriggerEvent::Update)
            .for_each_row()
            .execute_function("trigger_set_timestamp")
            .build()
            .unwrap();

        b.iter(|| black_box(trigger.to_sql(DatabaseType::PostgreSQL)))
    });

    group.finish();
}

// ============================================================================
// Procedure/Function Benchmarks
// ============================================================================

fn bench_procedure_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("procedure_operations");

    group.bench_function("create_procedure_call", |b| {
        b.iter(|| {
            black_box(
                ProcedureCall::new("calculate_totals")
                    .param("order_id", 12345)
                    .param("include_tax", true),
            )
        })
    });

    group.bench_function("procedure_to_sql_postgres", |b| {
        let call = ProcedureCall::new("process_order")
            .param("order_id", 12345)
            .param("status", "completed");

        b.iter(|| black_box(call.to_postgres_sql()))
    });

    group.bench_function("procedure_to_sql_mysql", |b| {
        let call = ProcedureCall::new("update_inventory")
            .param("product_id", 500)
            .param("quantity", -5);

        b.iter(|| black_box(call.to_mysql_sql()))
    });

    group.finish();
}

// ============================================================================
// Security (RLS, Roles) Benchmarks
// ============================================================================

fn bench_security_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("security_operations");

    group.bench_function("create_rls_policy", |b| {
        b.iter(|| {
            black_box(
                RlsPolicyBuilder::new("tenant_isolation", "orders")
                    .using("tenant_id = current_setting('app.tenant_id')::int")
                    .build(),
            )
        })
    });

    group.bench_function("create_rls_with_check", |b| {
        b.iter(|| {
            black_box(
                RlsPolicyBuilder::new("user_data_access", "user_profiles")
                    .using("user_id = current_user_id()")
                    .with_check("user_id = current_user_id()")
                    .build(),
            )
        })
    });

    group.bench_function("create_role", |b| {
        b.iter(|| {
            black_box(
                RoleBuilder::new("app_readonly")
                    .login()
                    .build(),
            )
        })
    });

    group.finish();
}

// ============================================================================
// Extension Operations Benchmarks
// ============================================================================

fn bench_extension_operations(c: &mut Criterion) {
    use prax_query::extension;

    let mut group = c.benchmark_group("extension_operations");

    // Geospatial
    group.bench_function("create_point", |b| {
        b.iter(|| black_box(Point::new(-122.4194, 37.7749)))
    });

    group.bench_function("distance_sql_postgres", |b| {
        b.iter(|| black_box(extension::geo::distance_sql("location1", "location2", DatabaseType::PostgreSQL)))
    });

    group.bench_function("within_distance_sql", |b| {
        let p = Point::new(-122.4194, 37.7749);
        b.iter(|| black_box(extension::geo::within_distance_sql("location", &p, 10000.0, DatabaseType::PostgreSQL)))
    });

    // UUID
    group.bench_function("uuid_generate_sql_postgres", |b| {
        b.iter(|| black_box(extension::uuid::generate_v4(DatabaseType::PostgreSQL)))
    });

    // Vector
    group.bench_function("create_vector", |b| {
        b.iter(|| {
            black_box(extension::vector::Vector::new(vec![
                0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8,
            ]))
        })
    });

    group.finish();
}

// ============================================================================
// Batch Throughput Benchmarks
// ============================================================================

fn bench_batch_throughput(c: &mut Criterion) {
    use prax_query::types::SortOrder;

    let mut group = c.benchmark_group("advanced_batch_throughput");

    // Batch CTE creation
    for size in [5, 10, 20].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("create_n_ctes", size), size, |b, &n| {
            b.iter(|| {
                let ctes: Vec<_> = (0..n)
                    .map(|i| {
                        Cte::new(&format!("cte_{}", i))
                            .as_query(&format!("SELECT * FROM table_{}", i))
                    })
                    .collect();
                black_box(ctes)
            })
        });
    }

    // Batch window function creation
    for size in [5, 10, 20].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("create_n_window_functions", size),
            size,
            |b, &n| {
                b.iter(|| {
                    let windows: Vec<_> = (0..n)
                        .map(|i| WindowFunction {
                            function: WindowFn::RowNumber,
                            over: WindowSpec::new()
                                .partition_by([format!("col_{}", i)])
                                .order_by(format!("sort_{}", i), SortOrder::Asc),
                            alias: Some(format!("rn_{}", i)),
                        })
                        .collect();
                    black_box(windows)
                })
            },
        );
    }

    // Batch JSON path creation
    for size in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("create_n_json_paths", size), size, |b, &n| {
            b.iter(|| {
                let paths: Vec<_> = (0..n)
                    .map(|i| JsonPath::from_path("data", &format!("$.level1.level2.field_{}", i)))
                    .collect();
                black_box(paths)
            })
        });
    }

    group.finish();
}

// ============================================================================
// Real-World Scenario Benchmarks
// ============================================================================

fn bench_real_world_scenarios(c: &mut Criterion) {
    use prax_query::types::SortOrder;

    let mut group = c.benchmark_group("real_world_advanced");

    // Analytics query with CTE, window function
    group.bench_function("analytics_query_build", |b| {
        b.iter(|| {
            let cte = Cte::new("daily_sales")
                .as_query("SELECT date, SUM(amount) as total FROM orders GROUP BY date");

            let window = WindowFunction {
                function: WindowFn::Sum("total".to_string()),
                over: WindowSpec::new()
                    .order_by("date", SortOrder::Asc)
                    .rows(FrameBound::UnboundedPreceding, Some(FrameBound::CurrentRow)),
                alias: Some("running_total".to_string()),
            };

            black_box((cte, window))
        })
    });

    // Full-text search with options
    group.bench_function("advanced_search_build", |b| {
        b.iter(|| {
            let search = SearchQueryBuilder::new("rust async database")
                .columns(vec!["title", "body", "tags"])
                .mode(SearchMode::Boolean)
                .with_ranking()
                .with_highlight()
                .with_fuzzy()
                .build();

            black_box(search)
        })
    });

    // Upsert with conflict handling
    group.bench_function("upsert_with_conflict_handling", |b| {
        b.iter(|| {
            let upsert = UpsertBuilder::new("user_stats")
                .columns(vec!["user_id", "login_count", "last_login", "total_time"])
                .on_conflict_columns(vec!["user_id"])
                .do_update(vec!["login_count", "last_login", "total_time"])
                .where_clause("excluded.last_login > user_stats.last_login")
                .build();

            black_box(upsert)
        })
    });

    // Hierarchical query with recursive CTE
    group.bench_function("hierarchical_query_build", |b| {
        b.iter(|| {
            let cte = Cte::new("org_tree")
                .recursive()
                .columns(vec!["id", "name", "parent_id", "level", "path"])
                .as_query(
                    "SELECT id, name, parent_id, 0, ARRAY[id] FROM employees WHERE parent_id IS NULL \
                     UNION ALL \
                     SELECT e.id, e.name, e.parent_id, t.level + 1, t.path || e.id \
                     FROM employees e JOIN org_tree t ON e.parent_id = t.id",
                );

            black_box(cte)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cte_operations,
    bench_window_functions,
    bench_json_operations,
    bench_search_operations,
    bench_upsert_operations,
    bench_sequence_operations,
    bench_trigger_operations,
    bench_procedure_operations,
    bench_security_operations,
    bench_extension_operations,
    bench_batch_throughput,
    bench_real_world_scenarios,
);

criterion_main!(benches);
