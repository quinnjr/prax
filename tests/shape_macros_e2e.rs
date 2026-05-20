//! End-to-end smoke tests for the phase-4 shape macros.
//!
//! Each test asserts:
//!   1. The macro can be invoked (it compiles).
//!   2. The result has the expected concrete type (the per-model
//!      typed input struct from phase-2 codegen).
//!
//! Composition with the read macros is exercised in Task 7's added
//! tests; this file ships the per-macro happy-path proof as each task
//! lands.

#![allow(dead_code)]
#![allow(unused_imports)]

// `prax_orm::prax_schema!` emits the per-model module that the shape
// macros refer to in their lowered output.
prax_orm::prax_schema!("prax/schema.prax");

use prax_query::dialect::SqlDialect;
use prax_query::error::QueryError;
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model as ModelTrait, QueryEngine};

// Mirrors `tests/read_macros_e2e.rs::MockEngine` so the composition
// tests below can build a `Client<E>` accessor for `find_many!` etc.
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
fn where_macro_returns_user_where_input() {
    let w: user::UserWhereInput = prax_orm::r#where!(User, {
        id: { equals: 1 },
    });
    // `UserWhereInput` is a struct with `id: Option<IntFilter>`; if
    // the macro emitted the wrong type, this binding would not compile.
    let _ = w;
}

#[test]
fn where_macro_raw_identifier_path_compiles() {
    // `where` is a Rust keyword and cannot appear as a bare path
    // segment, so callers must use the `r#where!` raw-identifier form
    // when reaching through `prax_orm::`. (At the unprefixed call site
    // `where!(...)` would also fail to parse.) Documented this way for
    // discoverability.
    let _w: user::UserWhereInput = prax_orm::r#where!(User, { active: true });
}

#[test]
fn where_macro_with_spread_composes() {
    let base = prax_orm::r#where!(User, { active: true });
    // Spread the base value into a wider filter; this exercises that
    // the macro emits a real `Default`-able struct value.
    let w = user::UserWhereInput {
        email: Some(prax_query::inputs::StringFilter::equals("alice@x.com")),
        ..base
    };
    let _: user::UserWhereInput = w;
}

#[test]
fn select_macro_returns_user_select() {
    let s: user::UserSelect = prax_orm::select!(User, {
        id: true,
        email: true,
    });
    // The selection struct is `Default + Clone`; round-trip via clone
    // to make sure the value is fully owned.
    let _ = s.clone();
}

#[test]
fn select_macro_default_when_empty() {
    // Empty selection block is allowed — produces a UserSelect with
    // all Option::None which lowers to Select::All at runtime.
    let _s: user::UserSelect = prax_orm::select!(User, {});
}

#[test]
fn include_macro_returns_user_include() {
    // `User` in the workspace fixture schema has no relation fields,
    // so the include block is empty. The macro should still resolve
    // and emit a `UserInclude` value.
    let _i: user::UserInclude = prax_orm::include!(User, {});
}

#[test]
fn order_by_macro_single_block_returns_order_by() {
    let ob: prax_query::types::OrderBy = prax_orm::order_by!(User, { created_at: desc });
    assert!(!ob.is_empty());
}

#[test]
fn order_by_macro_list_returns_multi_field_order_by() {
    let ob: prax_query::types::OrderBy = prax_orm::order_by!(User, [
        { active: desc },
        { email: asc },
    ]);
    // Multi-field sort lowers to OrderBy::Fields with two entries.
    match ob {
        prax_query::types::OrderBy::Fields(fs) => assert_eq!(fs.len(), 2),
        other => panic!("expected OrderBy::Fields, got {:?}", other),
    }
}

#[test]
fn cursor_macro_id_column_returns_where_unique_input() {
    let _c: user::UserWhereUniqueInput = prax_orm::cursor!(User, { id: 42 });
}

#[test]
fn cursor_macro_unique_email_column_returns_where_unique_input() {
    let _c: user::UserWhereUniqueInput = prax_orm::cursor!(User, { email: "alice@x.com" });
}

// ---------------------------------------------------------------------------
// Composition with the phase-3 read macros.
//
// These tests exercise the spread-composition story from the plan intro:
// shape values precomputed via the phase-4 macros plugged into a phase-3
// read-macro builder. Each chain is built and lowered to SQL against the
// MockEngine dialect to confirm the trait bounds line up.
// ---------------------------------------------------------------------------

#[test]
fn where_value_composes_into_find_many_via_spread() {
    let client = AppClient::new();
    let base = prax_orm::r#where!(User, { active: true });
    let op = prax_orm::find_many!(client.user, {
        where: {
            ..base,
            email: { contains: "@x.com" },
        },
        take: 10,
    });
    // Build SQL to prove the chain type-checks against the engine.
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}

#[test]
fn include_value_composes_into_find_many_builder() {
    let client = AppClient::new();
    // Empty include block for the workspace `User` fixture (no relations);
    // the test still exercises the trait bound `with_include_input<I:
    // IncludeInput>` matches the macro's emitted type.
    let inc = prax_orm::include!(User, {});
    let op = prax_orm::find_many!(client.user, {
        where: { active: true },
    })
    .with_include_input(inc);
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}

#[test]
fn select_value_composes_into_find_many_builder() {
    let client = AppClient::new();
    let sel = prax_orm::select!(User, {
        id: true,
        email: true,
    });
    let op = prax_orm::find_many!(client.user, {
        where: { active: true },
    })
    .with_select_input(sel);
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}

#[test]
fn order_by_value_composes_into_find_many_builder() {
    let client = AppClient::new();
    let ob = prax_orm::order_by!(User, { created_at: desc });
    let op = prax_orm::find_many!(client.user, {
        where: { active: true },
    })
    .order_by(ob);
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}

#[test]
fn cursor_value_composes_into_find_unique() {
    let client = AppClient::new();
    let c = prax_orm::cursor!(User, { id: 1 });
    // `find_unique`'s `with_where_input` takes a `WhereUniqueInput`,
    // which is exactly what `cursor!` returns. The shape macro thus
    // doubles as a reusable unique-lookup key.
    let op = client.user.find_unique().with_where_input(c);
    let _ = op.build_sql(&prax_query::dialect::Postgres);
}
