//! Regression test for the schema-path relation_helpers bug: `prax_schema!`
//! over a schema containing relations must emit cross-model module paths
//! that resolve (`crate::<model>::<Model>`), populate relation fields via
//! FromRow defaults, and expose Include/Select relation variants.
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

    // Optional single relation (`author: Option<Author>`) defaults to None.
    let p = post::Post {
        id: 1,
        title: "t".into(),
        author_id: 1,
        author: None,
    };
    assert!(p.author.is_none());
}

#[test]
fn relation_include_helper_resolves_to_include_param() {
    // The relation field module exposes `include()` returning the model's
    // IncludeParam variant (relations are included, not selected/filtered
    // as scalar columns).
    assert!(matches!(
        author::posts::include(),
        author::IncludeParam::Posts
    ));
    assert!(matches!(
        post::author::include(),
        post::IncludeParam::Author
    ));
}

#[test]
fn scalar_filter_helpers_still_work_alongside_relations() {
    // A model that has relation fields must still generate working scalar
    // WhereParam filters for its columns.
    let w = author::name::equals("Ada".to_string());
    assert!(matches!(w, author::WhereParam::Name(_)));
    assert_eq!(author::name::COLUMN, "name");
}
