//! End-to-end smoke tests for the phase-3 read-operation macros.
//!
//! These tests build the operation expression via the macro and call
//! `build_sql` on the resulting operation against a mock dialect to
//! confirm the chain compiled. They don't `.exec()` because spinning up
//! a real engine is out of scope for the macro test.
//!
//! Schema discovery walks up from `CARGO_MANIFEST_DIR` and finds
//! `prax.toml` at the workspace root, which points at
//! `prax/schema.prax`. That fixture defines `User`. Relations and the
//! full surface are exercised by `prax-codegen`'s in-crate lowering
//! tests (`prax-codegen/src/macros/lower/*` `#[cfg(test)]` modules).

#![allow(dead_code)]
#![allow(unused_imports)]

// `prax_orm::prax_schema!` emits the per-model module and `Client<E>`
// accessor that the `find_many!` macro chains onto.
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
fn find_many_compiles_basic_where() {
    let client = AppClient::new();
    let op = prax_orm::find_many!(client.user, {
        where: { email: { contains: "@example.com" } },
        take: 10,
    });
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}

#[test]
fn find_many_compiles_with_order_by_and_skip() {
    let client = AppClient::new();
    let op = prax_orm::find_many!(client.user, {
        where: { active: true },
        order_by: { created_at: desc },
        skip: 0,
        take: 25,
    });
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}
