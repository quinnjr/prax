//! End-to-end smoke tests for the phase-5a write-operation macros.
//!
//! Each test builds the operation expression via the macro and calls
//! `build_sql` on the resulting operation against the Postgres dialect
//! to confirm the chain compiled and that the typed inputs threaded
//! through `with_create_input` / `with_update_input` lower to the
//! expected SQL.
//!
//! Schema discovery walks up from `CARGO_MANIFEST_DIR` and finds
//! `prax.toml` at the workspace root, which points at
//! `prax/schema.prax`. That fixture defines `User`.

#![allow(dead_code)]
#![allow(unused_imports)]

prax_orm::prax_schema!("prax/schema.prax");

use prax_query::dialect::SqlDialect;
use prax_query::error::QueryError;
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model as ModelTrait, QueryEngine};

#[derive(Clone)]
struct MockEngine;

impl QueryEngine for MockEngine {
    fn dialect(&self) -> &dyn SqlDialect {
        &prax_query::dialect::Postgres
    }
    fn query_many<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(QueryError::not_found("test")) })
    }
    fn query_optional<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(QueryError::not_found("test")) })
    }
    fn execute_update<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn execute_delete(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn execute_raw(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn count(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

struct AppClient {
    user: user::Client<MockEngine>,
}

impl AppClient {
    fn new() -> Self {
        Self {
            user: user::Client::new(MockEngine),
        }
    }
}

#[test]
fn create_macro_compiles_with_data_and_select() {
    let client = AppClient::new();
    let now = ::chrono::Utc::now();
    let op = prax_orm::create!(client.user, {
        data: { email: "a@x.com", name: "Alice", age: 30, active: true, created_at: @(now) },
        select: { id: true, email: true },
    });
    let (sql, params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("INSERT INTO User"), "got: {sql}");
    assert!(sql.contains("RETURNING id, email"), "got: {sql}");
    // The required fields (email, active, created_at) plus the
    // Some(...) optionals (name, age) contribute one parameter each.
    assert!(!params.is_empty());
}

#[test]
fn create_macro_compiles_without_optional_fields() {
    let client = AppClient::new();
    // Only required fields supplied — `name`/`age` are optional and
    // codegen leaves them out of the payload when omitted.
    let now = ::chrono::Utc::now();
    let op = prax_orm::create!(client.user, {
        data: { email: "b@x.com", active: true, created_at: @(now) },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("INSERT INTO User"), "got: {sql}");
    // Default Select is `*`.
    assert!(sql.contains("RETURNING *"));
}

#[test]
fn update_macro_compiles_plain_set() {
    let client = AppClient::new();
    let op = prax_orm::update!(client.user, {
        where: { id: 1 },
        data: { name: "Renamed" },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("UPDATE User SET"), "got: {sql}");
    assert!(sql.contains("name = $1"), "got: {sql}");
    assert!(sql.contains("WHERE"), "got: {sql}");
}

#[test]
fn update_macro_compiles_with_increment() {
    let client = AppClient::new();
    let op = prax_orm::update!(client.user, {
        where: { id: 1 },
        data: { age: { increment: 1 } },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("UPDATE User SET"), "got: {sql}");
    assert!(sql.contains("age = age + $1"), "got: {sql}");
}

#[test]
fn update_macro_compiles_with_unset() {
    let client = AppClient::new();
    let op = prax_orm::update!(client.user, {
        where: { id: 1 },
        data: { name: { unset: true } },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("name = NULL"), "got: {sql}");
}

#[test]
fn update_macro_compiles_mixed_ops() {
    let client = AppClient::new();
    let op = prax_orm::update!(client.user, {
        where: { id: 42 },
        data: {
            name: "Bob",
            age: { increment: 1 },
        },
        select: { id: true, age: true },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("name = $1"), "got: {sql}");
    assert!(sql.contains("age = age + $2"), "got: {sql}");
    assert!(sql.contains("RETURNING id, age"), "got: {sql}");
}

#[test]
fn upsert_macro_compiles_full_form() {
    let client = AppClient::new();
    let now = ::chrono::Utc::now();
    let op = prax_orm::upsert!(client.user, {
        where: { email: "a@x.com" },
        create: { email: "a@x.com", name: "Alice", active: true, created_at: @(now) },
        update: { name: { set: "Renamed" } },
        select: { id: true, email: true },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("INSERT INTO User"), "got: {sql}");
    assert!(sql.contains("ON CONFLICT (email)"), "got: {sql}");
    assert!(sql.contains("DO UPDATE SET"), "got: {sql}");
    assert!(sql.contains("RETURNING id, email"), "got: {sql}");
}

#[test]
fn upsert_macro_supports_atomic_update_op() {
    let client = AppClient::new();
    let now = ::chrono::Utc::now();
    let op = prax_orm::upsert!(client.user, {
        where: { id: 1 },
        create: { email: "b@x.com", active: true, created_at: @(now) },
        update: { age: { increment: 1 } },
    });
    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);
    assert!(sql.contains("ON CONFLICT (id)"), "got: {sql}");
    assert!(sql.contains("age = age + $"), "got: {sql}");
}
