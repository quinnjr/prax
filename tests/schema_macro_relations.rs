//! Regression test for the schema-path relation_helpers bug: `prax_schema!`
//! over a schema containing relations must emit cross-model module paths
//! that resolve (`crate::<model>::<Model>`), populate relation fields via
//! FromRow defaults, and expose fetch()/select relation helpers.
//!
//! Before the fix this file failed to compile (E0433 "too many leading
//! super", E0425 unresolved `Post`/`Author`, E0063 missing relation field
//! in initializer, E0599 missing Select variant, E0277 FilterValue::From).

prax_orm::prax_schema!("tests/fixtures/relations.prax");

#[test]
fn relation_models_compile_and_default_relation_fields() {
    // List relation (`posts: Vec<Post>`) defaults empty; the cross-model
    // type `Post` must resolve from inside `mod author`.
    let a = author::Author {
        id: 1,
        name: "Ada".into(),
        posts: Vec::new(),
    };
    assert_eq!(a.posts.len(), 0);

    // Single relation (`author: Option<Box<Author>>`) — always optional and
    // boxed regardless of the schema modifier, so FromRow's Default (None)
    // holds even when the schema declares the relation as required, and
    // self-recursive relations stay finitely sized.
    let p = post::Post {
        id: 1,
        title: "t".into(),
        author_id: 1,
        author: None,
    };
    assert!(p.author.is_none());
}

#[test]
fn relation_fetch_helper_returns_include_spec() {
    // The schema path's relation field module exposes `fetch()` returning
    // an `IncludeSpec` keyed by the field name — mirroring the derive
    // path's `relation_accessors::emit` output so both codegen paths share
    // the same include API (fetch() -> IncludeSpec, not include() ->
    // IncludeParam).
    let spec = author::posts::fetch();
    assert_eq!(spec.relation_name, "posts");

    let spec2 = post::author::fetch();
    assert_eq!(spec2.relation_name, "author");
}

#[test]
fn scalar_filter_helpers_still_work_alongside_relations() {
    // A model that has relation fields must still generate working scalar
    // WhereParam filters for its columns.
    let w = author::name::equals("Ada".to_string());
    assert!(matches!(w, author::WhereParam::Name(_)));
    assert_eq!(author::name::COLUMN, "name");
}

#[test]
fn multi_word_model_name_resolves_relation_paths() {
    // `BlogPost` -> module `blog_post`, struct `BlogPost`. The single
    // relation `author Author?` must resolve `super::author::Author` from
    // inside the snake_cased module and default to None via FromRow.
    let bp = blog_post::BlogPost {
        id: 1,
        title: "t".into(),
        author_id: 1,
        author: None,
    };
    assert!(bp.author.is_none());
    let spec = blog_post::author::fetch();
    assert_eq!(spec.relation_name, "author");
}

#[test]
fn self_relation_resolves_to_own_module() {
    // `Category { parent Category?, children Category[] }` — the relation
    // target is the model itself. `super::category::Category` must resolve
    // back to this model; the field names differ from the module name so the
    // collision guard does not fire.
    let c = category::Category {
        id: 2,
        name: "rust".into(),
        parent_id: 1,
        parent: None,
        children: Vec::new(),
    };
    assert!(c.parent.is_none());
    assert_eq!(c.children.len(), 0);
    assert_eq!(category::parent::fetch().relation_name, "parent");
    assert_eq!(category::children::fetch().relation_name, "children");
}

// --- ModelRelationLoader bound (relation execution) ------------------------
//
// `FindManyOperation::exec` requires `M: ModelRelationLoader<E>`
// unconditionally. The schema path must emit this impl or `.exec()` on any
// schema-path model is a compile error (E0277). This asserts the bound is
// satisfied for a schema-path model; it does not exercise the (deferred)
// functional relation loading — the stub impl errors at runtime on includes.

use prax_query::error::QueryError;
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model as ModelTrait, ModelRelationLoader, QueryEngine};

#[derive(Clone)]
struct MockEngine;

impl QueryEngine for MockEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
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

fn assert_loader<E: QueryEngine, M: ModelRelationLoader<E>>() {}

#[test]
fn schema_path_models_implement_relation_loader() {
    // Compile-time proof the bound `exec` requires is satisfied for every
    // schema-path model (including those with no relations resolvable yet).
    assert_loader::<MockEngine, author::Author>();
    assert_loader::<MockEngine, post::Post>();
    assert_loader::<MockEngine, blog_post::BlogPost>();
    assert_loader::<MockEngine, category::Category>();
}

#[tokio::test]
async fn schema_path_include_errors_loudly_until_relation_loading_lands() {
    // The stub loader errors on any relation name rather than silently
    // no-op'ing. This pins the documented behavior: includes on schema-path
    // models are rejected at runtime until functional relation loading lands.
    let mut parents = vec![author::Author {
        id: 1,
        name: "Ada".into(),
        posts: Vec::new(),
    }];
    let spec = author::posts::fetch();
    let err = <author::Author as ModelRelationLoader<MockEngine>>::load_relation(
        &MockEngine,
        &mut parents,
        &spec,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("unknown relation"));
}
