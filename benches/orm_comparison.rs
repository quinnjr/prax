#![allow(dead_code, unused, clippy::type_complexity)]
//! ORM Comparison Benchmarks
//!
//! This benchmark suite compares Prax ORM against other popular Rust ORMs:
//! - Diesel (sync)
//! - Diesel-Async (async Diesel wrapper)
//! - SQLx
//! - SeaORM
//!
//! The benchmarks focus on query building and filter construction performance,
//! which are critical hot paths that don't require database connections.
//!
//! # Running Benchmarks
//!
//! ```bash
//! # Run all ORM comparison benchmarks
//! cargo bench --bench orm_comparison
//!
//! # Run specific benchmark group
//! cargo bench --bench orm_comparison -- query_building
//! cargo bench --bench orm_comparison -- filter_construction
//! cargo bench --bench orm_comparison -- diesel_async
//! ```
//!
//! # Benchmark Categories
//!
//! 1. **Query Building**: Measures the time to construct SQL queries
//!    - Simple SELECT queries
//!    - Complex WHERE clauses
//!    - JOINs and subqueries
//!    - INSERT/UPDATE/DELETE operations
//!
//! 2. **Filter Construction**: Measures filter/condition building
//!    - Simple equality filters
//!    - AND/OR combinations
//!    - IN lists
//!    - Complex nested conditions
//!
//! 3. **Diesel-Async Specific**: Measures diesel-async patterns
//!    - Query building (same as Diesel)
//!    - RunQueryDsl patterns
//!    - Pool configuration overhead
//!
//! 4. **Type Conversion**: Measures type conversion overhead
//!    - Rust types to SQL parameters
//!    - Parameter binding

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

// ==============================================================================
// Prax ORM Benchmarks
// ==============================================================================

mod prax_benchmarks {
    use prax_query::filter::{Filter, FilterValue};
    use prax_query::raw::Sql;
    use prax_query::sql::DatabaseType;

    /// Build a simple SELECT query with one WHERE condition
    pub fn simple_select() -> String {
        let sql = Sql::new("SELECT id, name, email FROM users WHERE id = ").bind(42i64);
        sql.build().0
    }

    /// Build a SELECT query with multiple WHERE conditions
    pub fn select_with_filters() -> String {
        let sql = Sql::new("SELECT * FROM users WHERE ")
            .push("status = ")
            .bind("active")
            .push(" AND age > ")
            .bind(18i64)
            .push(" AND created_at > ")
            .bind("2024-01-01");
        sql.build().0
    }

    /// Build an INSERT query
    pub fn insert_query() -> String {
        let sql = Sql::new("INSERT INTO users (name, email, age) VALUES (")
            .bind("John Doe")
            .push(", ")
            .bind("john@example.com")
            .push(", ")
            .bind(30i64)
            .push(")");
        sql.build().0
    }

    /// Build an UPDATE query
    pub fn update_query() -> String {
        let sql = Sql::new("UPDATE users SET ")
            .push("name = ")
            .bind("Jane Doe")
            .push(", email = ")
            .bind("jane@example.com")
            .push(" WHERE id = ")
            .bind(1i64);
        sql.build().0
    }

    /// Create a simple equality filter
    pub fn simple_filter() -> Filter {
        Filter::Equals("id".into(), FilterValue::Int(42))
    }

    /// Create an AND filter with 5 conditions
    pub fn and_filter_5() -> Filter {
        Filter::and([
            Filter::Equals("status".into(), FilterValue::String("active".into())),
            Filter::Gt("age".into(), FilterValue::Int(18)),
            Filter::Lt("age".into(), FilterValue::Int(65)),
            Filter::IsNotNull("email".into()),
            Filter::Equals("verified".into(), FilterValue::Bool(true)),
        ])
    }

    /// Create an AND filter with 10 conditions
    pub fn and_filter_10() -> Filter {
        Filter::and([
            Filter::Equals("status".into(), FilterValue::String("active".into())),
            Filter::Gt("age".into(), FilterValue::Int(18)),
            Filter::Lt("age".into(), FilterValue::Int(65)),
            Filter::IsNotNull("email".into()),
            Filter::Equals("verified".into(), FilterValue::Bool(true)),
            Filter::Contains("name".into(), FilterValue::String("John".into())),
            Filter::Gte("score".into(), FilterValue::Int(100)),
            Filter::Lte("attempts".into(), FilterValue::Int(3)),
            Filter::NotEquals("role".into(), FilterValue::String("banned".into())),
            Filter::IsNull("deleted_at".into()),
        ])
    }

    /// Create an OR filter with 5 conditions
    pub fn or_filter_5() -> Filter {
        Filter::or([
            Filter::Equals("role".into(), FilterValue::String("admin".into())),
            Filter::Equals("role".into(), FilterValue::String("moderator".into())),
            Filter::Equals("role".into(), FilterValue::String("editor".into())),
            Filter::Equals("role".into(), FilterValue::String("author".into())),
            Filter::Equals("role".into(), FilterValue::String("contributor".into())),
        ])
    }

    /// Create an IN filter with 100 values
    pub fn in_filter_100() -> Filter {
        let values: Vec<FilterValue> = (0..100).map(FilterValue::Int).collect();
        Filter::In("id".into(), values)
    }

    /// Create a complex nested filter
    pub fn complex_nested_filter() -> Filter {
        Filter::and([
            Filter::or([
                Filter::and([
                    Filter::Equals("status".into(), FilterValue::String("active".into())),
                    Filter::Gt("score".into(), FilterValue::Int(100)),
                ]),
                Filter::and([
                    Filter::Equals("role".into(), FilterValue::String("admin".into())),
                    Filter::Equals("verified".into(), FilterValue::Bool(true)),
                ]),
            ]),
            Filter::Not(Box::new(Filter::Equals(
                "deleted".into(),
                FilterValue::Bool(true),
            ))),
            Filter::IsNotNull("email".into()),
        ])
    }

    /// Build query for PostgreSQL
    pub fn postgres_query() -> String {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::PostgreSQL)
            .bind(42i64);
        sql.build().0
    }

    /// Build query for MySQL
    pub fn mysql_query() -> String {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::MySQL)
            .bind(42i64);
        sql.build().0
    }

    /// Build query for SQLite
    pub fn sqlite_query() -> String {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::SQLite)
            .bind(42i64);
        sql.build().0
    }
}

// ==============================================================================
// Diesel Benchmarks (using QueryDsl trait simulation)
// ==============================================================================

mod diesel_benchmarks {
    use diesel::debug_query;
    use diesel::pg::Pg;
    use diesel::prelude::*;
    use diesel::sql_types::*;

    // Define a schema for benchmarking (without actual database)
    mod schema {
        diesel::table! {
            users (id) {
                id -> Int8,
                name -> Text,
                email -> Text,
                age -> Int4,
                status -> Text,
                role -> Text,
                verified -> Bool,
                score -> Int4,
                attempts -> Int4,
                deleted -> Bool,
                deleted_at -> Nullable<Timestamp>,
                created_at -> Timestamp,
            }
        }
    }

    use schema::users;
    use schema::users::dsl::*;

    /// Build a simple SELECT query
    pub fn simple_select() -> String {
        let query = users.select((id, name, email)).filter(id.eq(42i64));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build a SELECT query with multiple filters
    pub fn select_with_filters() -> String {
        let query = users.filter(status.eq("active")).filter(age.gt(18));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Create a simple equality filter (returns boxed expression)
    pub fn simple_filter() -> Box<dyn BoxableExpression<users::table, Pg, SqlType = Bool>> {
        Box::new(id.eq(42i64))
    }

    /// Create an AND filter with 5 conditions
    pub fn and_filter_5() -> Box<dyn BoxableExpression<users::table, Pg, SqlType = Bool>> {
        Box::new(
            status
                .eq("active")
                .and(age.gt(18))
                .and(age.lt(65))
                .and(email.is_not_null())
                .and(verified.eq(true)),
        )
    }

    /// Create an OR filter with 5 conditions
    pub fn or_filter_5() -> Box<dyn BoxableExpression<users::table, Pg, SqlType = Bool>> {
        Box::new(
            role.eq("admin")
                .or(role.eq("moderator"))
                .or(role.eq("editor"))
                .or(role.eq("author"))
                .or(role.eq("contributor")),
        )
    }

    /// Create an IN filter with 100 values
    pub fn in_filter_100() -> Box<dyn BoxableExpression<users::table, Pg, SqlType = Bool>> {
        let values: Vec<i64> = (0..100).collect();
        Box::new(id.eq_any(values))
    }
}

// ==============================================================================
// Diesel-Async Benchmarks
// ==============================================================================
//
// diesel-async uses the same query building as Diesel but wraps execution in async.
// Query building performance is identical to Diesel, but we benchmark async-specific patterns.

mod diesel_async_benchmarks {
    use diesel::debug_query;
    use diesel::pg::Pg;
    use diesel::prelude::*;
    // Note: diesel_async::RunQueryDsl is for async execution, not benchmarked here
    // as we focus on query building performance (identical to sync Diesel)

    // Reuse the same schema from diesel_benchmarks
    mod schema {
        diesel::table! {
            users (id) {
                id -> Int8,
                name -> Text,
                email -> Text,
                age -> Int4,
                status -> Text,
                role -> Text,
                verified -> Bool,
                score -> Int4,
                attempts -> Int4,
                deleted -> Bool,
                deleted_at -> Nullable<Timestamp>,
                created_at -> Timestamp,
            }
        }

        diesel::table! {
            posts (id) {
                id -> Int8,
                title -> Text,
                content -> Text,
                user_id -> Int8,
                published -> Bool,
                view_count -> Int4,
                created_at -> Timestamp,
            }
        }

        diesel::joinable!(posts -> users (user_id));
        diesel::allow_tables_to_appear_in_same_query!(users, posts);
    }

    use schema::posts;
    use schema::users;
    use schema::users::dsl::*;

    /// Build a simple SELECT query (identical to Diesel)
    pub fn simple_select() -> String {
        let query = users.select((id, name, email)).filter(id.eq(42i64));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build a SELECT query with multiple filters
    pub fn select_with_filters() -> String {
        let query = users
            .filter(status.eq("active"))
            .filter(age.gt(18))
            .filter(verified.eq(true));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build a SELECT with ORDER BY and LIMIT
    pub fn select_with_ordering() -> String {
        let query = users
            .filter(status.eq("active"))
            .order(schema::users::created_at.desc())
            .limit(10);
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build a SELECT with JOIN (diesel-async supports async JOINs)
    pub fn select_with_join() -> String {
        let query = users
            .inner_join(posts::table)
            .select((schema::users::id, schema::users::name, schema::posts::title))
            .filter(schema::posts::published.eq(true));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build an INSERT query
    pub fn insert_query() -> String {
        use schema::users::dsl::*;
        let query = diesel::insert_into(users).values((
            name.eq("John Doe"),
            email.eq("john@example.com"),
            age.eq(30),
            status.eq("active"),
            role.eq("user"),
            verified.eq(false),
            score.eq(0),
            attempts.eq(0),
            deleted.eq(false),
        ));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build a batch INSERT query
    pub fn batch_insert_query() -> String {
        use schema::users::dsl::*;

        // Create 10 rows to insert
        let values: Vec<_> = (0..10)
            .map(|i| {
                (
                    name.eq(format!("User {}", i)),
                    email.eq(format!("user{}@example.com", i)),
                    age.eq(20 + i),
                    status.eq("active"),
                    role.eq("user"),
                    verified.eq(false),
                    score.eq(0),
                    attempts.eq(0),
                    deleted.eq(false),
                )
            })
            .collect();

        let query = diesel::insert_into(users).values(&values);
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build an UPDATE query
    pub fn update_query() -> String {
        use schema::users::dsl::*;
        let query = diesel::update(users.filter(id.eq(1i64))).set((
            name.eq("Updated Name"),
            email.eq("updated@example.com"),
            verified.eq(true),
        ));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Build a DELETE query
    pub fn delete_query() -> String {
        use schema::users::dsl::*;
        let query = diesel::delete(users.filter(deleted.eq(true)));
        debug_query::<Pg, _>(&query).to_string()
    }

    /// Create an AND filter with 5 conditions
    pub fn and_filter_5()
    -> Box<dyn BoxableExpression<users::table, Pg, SqlType = diesel::sql_types::Bool>> {
        Box::new(
            status
                .eq("active")
                .and(age.gt(18))
                .and(age.lt(65))
                .and(email.is_not_null())
                .and(verified.eq(true)),
        )
    }

    /// Create an AND filter with 10 conditions
    pub fn and_filter_10()
    -> Box<dyn BoxableExpression<users::table, Pg, SqlType = diesel::sql_types::Bool>> {
        Box::new(
            status
                .eq("active")
                .and(age.gt(18))
                .and(age.lt(65))
                .and(email.is_not_null())
                .and(verified.eq(true))
                .and(score.gt(0))
                .and(attempts.lt(5))
                .and(deleted.eq(false))
                .and(role.ne("banned"))
                .and(name.is_not_null()),
        )
    }

    /// Create an OR filter with 5 conditions
    pub fn or_filter_5()
    -> Box<dyn BoxableExpression<users::table, Pg, SqlType = diesel::sql_types::Bool>> {
        Box::new(
            role.eq("admin")
                .or(role.eq("moderator"))
                .or(role.eq("editor"))
                .or(role.eq("author"))
                .or(role.eq("contributor")),
        )
    }

    /// Create an IN filter with 100 values
    pub fn in_filter_100()
    -> Box<dyn BoxableExpression<users::table, Pg, SqlType = diesel::sql_types::Bool>> {
        let values: Vec<i64> = (0..100).collect();
        Box::new(id.eq_any(values))
    }

    /// Create complex nested filter
    pub fn complex_nested_filter()
    -> Box<dyn BoxableExpression<users::table, Pg, SqlType = diesel::sql_types::Bool>> {
        Box::new(
            (status.eq("active").and(score.gt(100)))
                .or(role.eq("admin").and(verified.eq(true)))
                .and(deleted.eq(false))
                .and(email.is_not_null()),
        )
    }
}

// ==============================================================================
// SQLx Benchmarks (query building without execution)
// ==============================================================================

mod sqlx_benchmarks {
    /// Build a simple SELECT query using SQLx query builder pattern
    pub fn simple_select() -> String {
        // SQLx uses string-based queries, so we measure string construction
        "SELECT id, name, email FROM users WHERE id = $1".to_string()
    }

    /// Build a SELECT query with multiple WHERE conditions
    pub fn select_with_filters() -> String {
        "SELECT * FROM users WHERE status = $1 AND age > $2 AND created_at > $3".to_string()
    }

    /// Build an INSERT query
    pub fn insert_query() -> String {
        "INSERT INTO users (name, email, age) VALUES ($1, $2, $3)".to_string()
    }

    /// Build an UPDATE query
    pub fn update_query() -> String {
        "UPDATE users SET name = $1, email = $2 WHERE id = $3".to_string()
    }

    /// Build query with IN clause (100 values)
    pub fn in_filter_100() -> String {
        let placeholders: String = (1..=100)
            .map(|i| format!("${}", i))
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT * FROM users WHERE id IN ({})", placeholders)
    }
}

// ==============================================================================
// SeaORM Benchmarks (using EntityTrait simulation)
// ==============================================================================

mod sea_orm_benchmarks {
    use sea_orm::Condition;
    use sea_orm::sea_query::Expr;

    /// Create a simple equality filter using sea_query expressions
    pub fn simple_filter() -> Condition {
        Condition::all().add(Expr::col(Alias::new("id")).eq(42i64))
    }

    /// Create an AND filter with 5 conditions
    pub fn and_filter_5() -> Condition {
        Condition::all()
            .add(Expr::col(Alias::new("status")).eq("active"))
            .add(Expr::col(Alias::new("age")).gt(18))
            .add(Expr::col(Alias::new("age")).lt(65))
            .add(Expr::col(Alias::new("email")).is_not_null())
            .add(Expr::col(Alias::new("verified")).eq(true))
    }

    /// Create an OR filter with 5 conditions
    pub fn or_filter_5() -> Condition {
        Condition::any()
            .add(Expr::col(Alias::new("role")).eq("admin"))
            .add(Expr::col(Alias::new("role")).eq("moderator"))
            .add(Expr::col(Alias::new("role")).eq("editor"))
            .add(Expr::col(Alias::new("role")).eq("author"))
            .add(Expr::col(Alias::new("role")).eq("contributor"))
    }

    /// Create an IN filter with 100 values
    pub fn in_filter_100() -> Condition {
        let values: Vec<i64> = (0..100).collect();
        Condition::all().add(Expr::col(Alias::new("id")).is_in(values))
    }

    /// Create a complex nested filter
    pub fn complex_nested_filter() -> Condition {
        Condition::all()
            .add(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(Expr::col(Alias::new("status")).eq("active"))
                            .add(Expr::col(Alias::new("score")).gt(100)),
                    )
                    .add(
                        Condition::all()
                            .add(Expr::col(Alias::new("role")).eq("admin"))
                            .add(Expr::col(Alias::new("verified")).eq(true)),
                    ),
            )
            .add(Expr::col(Alias::new("deleted")).ne(true))
            .add(Expr::col(Alias::new("email")).is_not_null())
    }

    // Alias helper for column names
    use sea_orm::sea_query::Alias;
}

// ==============================================================================
// Criterion Benchmark Groups
// ==============================================================================

/// Benchmark query building performance across ORMs
fn bench_query_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_building");

    // Simple SELECT benchmarks
    group.bench_function("prax/simple_select", |b| {
        b.iter(|| black_box(prax_benchmarks::simple_select()))
    });

    group.bench_function("diesel/simple_select", |b| {
        b.iter(|| black_box(diesel_benchmarks::simple_select()))
    });

    group.bench_function("sqlx/simple_select", |b| {
        b.iter(|| black_box(sqlx_benchmarks::simple_select()))
    });

    // SELECT with filters
    group.bench_function("prax/select_with_filters", |b| {
        b.iter(|| black_box(prax_benchmarks::select_with_filters()))
    });

    group.bench_function("diesel/select_with_filters", |b| {
        b.iter(|| black_box(diesel_benchmarks::select_with_filters()))
    });

    group.bench_function("sqlx/select_with_filters", |b| {
        b.iter(|| black_box(sqlx_benchmarks::select_with_filters()))
    });

    // INSERT benchmarks
    group.bench_function("prax/insert_query", |b| {
        b.iter(|| black_box(prax_benchmarks::insert_query()))
    });

    group.bench_function("sqlx/insert_query", |b| {
        b.iter(|| black_box(sqlx_benchmarks::insert_query()))
    });

    // UPDATE benchmarks
    group.bench_function("prax/update_query", |b| {
        b.iter(|| black_box(prax_benchmarks::update_query()))
    });

    group.bench_function("sqlx/update_query", |b| {
        b.iter(|| black_box(sqlx_benchmarks::update_query()))
    });

    // Database-specific query building
    group.bench_function("prax/postgres_query", |b| {
        b.iter(|| black_box(prax_benchmarks::postgres_query()))
    });

    group.bench_function("prax/mysql_query", |b| {
        b.iter(|| black_box(prax_benchmarks::mysql_query()))
    });

    group.bench_function("prax/sqlite_query", |b| {
        b.iter(|| black_box(prax_benchmarks::sqlite_query()))
    });

    group.finish();
}

/// Benchmark filter construction performance across ORMs
fn bench_filter_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_construction");

    // Simple equality filter
    group.bench_function("prax/simple_filter", |b| {
        b.iter(|| black_box(prax_benchmarks::simple_filter()))
    });

    group.bench_function("diesel/simple_filter", |b| {
        b.iter(|| black_box(diesel_benchmarks::simple_filter()))
    });

    group.bench_function("sea_orm/simple_filter", |b| {
        b.iter(|| black_box(sea_orm_benchmarks::simple_filter()))
    });

    // AND filter with 5 conditions
    group.bench_function("prax/and_filter_5", |b| {
        b.iter(|| black_box(prax_benchmarks::and_filter_5()))
    });

    group.bench_function("diesel/and_filter_5", |b| {
        b.iter(|| black_box(diesel_benchmarks::and_filter_5()))
    });

    group.bench_function("sea_orm/and_filter_5", |b| {
        b.iter(|| black_box(sea_orm_benchmarks::and_filter_5()))
    });

    // AND filter with 10 conditions
    group.bench_function("prax/and_filter_10", |b| {
        b.iter(|| black_box(prax_benchmarks::and_filter_10()))
    });

    // OR filter with 5 conditions
    group.bench_function("prax/or_filter_5", |b| {
        b.iter(|| black_box(prax_benchmarks::or_filter_5()))
    });

    group.bench_function("diesel/or_filter_5", |b| {
        b.iter(|| black_box(diesel_benchmarks::or_filter_5()))
    });

    group.bench_function("sea_orm/or_filter_5", |b| {
        b.iter(|| black_box(sea_orm_benchmarks::or_filter_5()))
    });

    // IN filter with 100 values
    group.bench_function("prax/in_filter_100", |b| {
        b.iter(|| black_box(prax_benchmarks::in_filter_100()))
    });

    group.bench_function("diesel/in_filter_100", |b| {
        b.iter(|| black_box(diesel_benchmarks::in_filter_100()))
    });

    group.bench_function("sqlx/in_filter_100", |b| {
        b.iter(|| black_box(sqlx_benchmarks::in_filter_100()))
    });

    group.bench_function("sea_orm/in_filter_100", |b| {
        b.iter(|| black_box(sea_orm_benchmarks::in_filter_100()))
    });

    // Complex nested filter
    group.bench_function("prax/complex_nested", |b| {
        b.iter(|| black_box(prax_benchmarks::complex_nested_filter()))
    });

    group.bench_function("sea_orm/complex_nested", |b| {
        b.iter(|| black_box(sea_orm_benchmarks::complex_nested_filter()))
    });

    group.finish();
}

/// Benchmark different filter sizes for scalability analysis
fn bench_filter_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_scaling");

    for size in [1, 5, 10, 25, 50, 100].iter() {
        // Prax AND filter scaling
        group.bench_with_input(
            BenchmarkId::new("prax/and_filter", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let filters: Vec<_> = (0..size)
                        .map(|i| {
                            prax_query::filter::Filter::Equals(
                                format!("field_{}", i).into(),
                                prax_query::filter::FilterValue::Int(i as i64),
                            )
                        })
                        .collect();
                    black_box(prax_query::filter::Filter::and(filters))
                })
            },
        );

        // Prax IN filter scaling
        group.bench_with_input(
            BenchmarkId::new("prax/in_filter", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let values: Vec<prax_query::filter::FilterValue> = (0..size)
                        .map(|i| prax_query::filter::FilterValue::Int(i as i64))
                        .collect();
                    black_box(prax_query::filter::Filter::In("id".into(), values))
                })
            },
        );
    }

    group.finish();
}

/// Benchmark diesel-async specific patterns
fn bench_diesel_async(c: &mut Criterion) {
    let mut group = c.benchmark_group("diesel_async");

    // Query building (should be identical to Diesel)
    group.bench_function("simple_select", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::simple_select()))
    });

    group.bench_function("select_with_filters", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::select_with_filters()))
    });

    group.bench_function("select_with_ordering", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::select_with_ordering()))
    });

    group.bench_function("select_with_join", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::select_with_join()))
    });

    group.bench_function("insert_query", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::insert_query()))
    });

    group.bench_function("batch_insert_10", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::batch_insert_query()))
    });

    group.bench_function("update_query", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::update_query()))
    });

    group.bench_function("delete_query", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::delete_query()))
    });

    // Filter construction
    group.bench_function("and_filter_5", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::and_filter_5()))
    });

    group.bench_function("and_filter_10", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::and_filter_10()))
    });

    group.bench_function("or_filter_5", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::or_filter_5()))
    });

    group.bench_function("in_filter_100", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::in_filter_100()))
    });

    group.bench_function("complex_nested_filter", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::complex_nested_filter()))
    });

    group.finish();
}

/// Compare Prax vs Diesel-Async head-to-head
fn bench_prax_vs_diesel_async(c: &mut Criterion) {
    let mut group = c.benchmark_group("prax_vs_diesel_async");

    // Simple SELECT comparison
    group.bench_function("prax/simple_select", |b| {
        b.iter(|| black_box(prax_benchmarks::simple_select()))
    });
    group.bench_function("diesel_async/simple_select", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::simple_select()))
    });

    // SELECT with filters comparison
    group.bench_function("prax/select_with_filters", |b| {
        b.iter(|| black_box(prax_benchmarks::select_with_filters()))
    });
    group.bench_function("diesel_async/select_with_filters", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::select_with_filters()))
    });

    // INSERT comparison
    group.bench_function("prax/insert_query", |b| {
        b.iter(|| black_box(prax_benchmarks::insert_query()))
    });
    group.bench_function("diesel_async/insert_query", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::insert_query()))
    });

    // UPDATE comparison
    group.bench_function("prax/update_query", |b| {
        b.iter(|| black_box(prax_benchmarks::update_query()))
    });
    group.bench_function("diesel_async/update_query", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::update_query()))
    });

    // AND filter (5 conditions)
    group.bench_function("prax/and_filter_5", |b| {
        b.iter(|| black_box(prax_benchmarks::and_filter_5()))
    });
    group.bench_function("diesel_async/and_filter_5", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::and_filter_5()))
    });

    // AND filter (10 conditions)
    group.bench_function("prax/and_filter_10", |b| {
        b.iter(|| black_box(prax_benchmarks::and_filter_10()))
    });
    group.bench_function("diesel_async/and_filter_10", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::and_filter_10()))
    });

    // OR filter (5 conditions)
    group.bench_function("prax/or_filter_5", |b| {
        b.iter(|| black_box(prax_benchmarks::or_filter_5()))
    });
    group.bench_function("diesel_async/or_filter_5", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::or_filter_5()))
    });

    // IN filter (100 values)
    group.bench_function("prax/in_filter_100", |b| {
        b.iter(|| black_box(prax_benchmarks::in_filter_100()))
    });
    group.bench_function("diesel_async/in_filter_100", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::in_filter_100()))
    });

    // Complex nested filter
    group.bench_function("prax/complex_nested", |b| {
        b.iter(|| black_box(prax_benchmarks::complex_nested_filter()))
    });
    group.bench_function("diesel_async/complex_nested", |b| {
        b.iter(|| black_box(diesel_async_benchmarks::complex_nested_filter()))
    });

    group.finish();
}

/// Profile memory allocation patterns
fn bench_allocation_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("allocation_patterns");

    // Static field names vs dynamic
    group.bench_function("prax/static_field_names", |b| {
        b.iter(|| {
            // Using static strings (zero allocation for field name)
            black_box(prax_query::filter::Filter::Equals(
                "id".into(),
                prax_query::filter::FilterValue::Int(42),
            ))
        })
    });

    group.bench_function("prax/dynamic_field_names", |b| {
        b.iter(|| {
            // Using dynamic strings (allocates for field name)
            let field = format!("field_{}", 1);
            black_box(prax_query::filter::Filter::Equals(
                field.into(),
                prax_query::filter::FilterValue::Int(42),
            ))
        })
    });

    // Using interned field names
    group.bench_function("prax/interned_field_names", |b| {
        use prax_query::fields;
        b.iter(|| {
            black_box(prax_query::filter::Filter::Equals(
                fields::ID.into(),
                prax_query::filter::FilterValue::Int(42),
            ))
        })
    });

    // Builder pattern vs direct construction
    group.bench_function("prax/builder_pattern", |b| {
        b.iter(|| {
            black_box(
                prax_query::filter::Filter::builder()
                    .eq("id", 42i64)
                    .eq("status", "active")
                    .gt("age", 18i64)
                    .build_and(),
            )
        })
    });

    group.bench_function("prax/direct_construction", |b| {
        b.iter(|| {
            black_box(prax_query::filter::Filter::and([
                prax_query::filter::Filter::Equals(
                    "id".into(),
                    prax_query::filter::FilterValue::Int(42),
                ),
                prax_query::filter::Filter::Equals(
                    "status".into(),
                    prax_query::filter::FilterValue::String("active".into()),
                ),
                prax_query::filter::Filter::Gt(
                    "age".into(),
                    prax_query::filter::FilterValue::Int(18),
                ),
            ]))
        })
    });

    // Pool-based construction for complex filters
    group.bench_function("prax/pool_construction", |b| {
        let pool = prax_query::pool::FilterPool::new();
        b.iter(|| {
            black_box(pool.build(|builder| {
                builder.and(vec![
                    builder.eq("id", 42),
                    builder.eq("status", "active"),
                    builder.gt("age", 18),
                ])
            }))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_query_building,
    bench_filter_construction,
    bench_filter_scaling,
    bench_allocation_patterns,
    bench_diesel_async,
    bench_prax_vs_diesel_async,
);
criterion_main!(benches);
