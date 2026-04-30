//! Unit tests for the per-relation codegen accessors (Task 21).
//!
//! Verifies that `#[prax(relation(...))]` on a `Vec<Target>` field:
//! - emits a `<model>::<field>::Relation` zero-sized type with a
//!   `RelationMeta` impl carrying the right name, kind, and FK, and
//! - emits a `<model>::<field>::fetch()` helper that produces an
//!   `IncludeSpec` keyed by the field's Rust name.
//!
//! These tests run as part of the default `cargo test` workspace run —
//! they touch only the compile-time metadata path, so no DB is needed.

#![cfg(test)]

use prax_orm::Model;
use prax_query::relations::{RelationKind, RelationMeta};

// Forward declarations so the derive can reference them. Relations are
// compile-time metadata only; no runtime linkage to the target struct
// is needed by the `RelationMeta` impl itself.
#[derive(Model, Debug, Clone)]
#[prax(table = "posts")]
pub struct Post {
    #[prax(id, auto)]
    pub id: i32,
    pub title: String,
    pub author_id: i32,
}

#[derive(Model, Debug)]
#[prax(table = "users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    pub posts: Vec<Post>,
}

#[test]
fn user_posts_relation_meta() {
    assert_eq!(<user::posts::Relation as RelationMeta>::NAME, "posts");
    assert_eq!(
        <user::posts::Relation as RelationMeta>::FOREIGN_KEY,
        "author_id"
    );
    assert_eq!(<user::posts::Relation as RelationMeta>::LOCAL_KEY, "id");
    assert!(matches!(
        <user::posts::Relation as RelationMeta>::KIND,
        RelationKind::HasMany,
    ));
}

#[test]
fn user_posts_fetch_produces_include_spec() {
    let spec = user::posts::fetch();
    assert_eq!(spec.relation_name, "posts");
}
