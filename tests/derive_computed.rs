//! Tests that `#[derive(Model)]` picks up `#[prax(generated)]` and
//! `#[prax(count/sum/avg/min/max)]` directives and emits the
//! `GENERATED_FIELDS` / `AGGREGATE_FIELDS` metadata constants.

use prax_orm::Model;
use prax_query::traits::Model as QueryModel; // must be in scope for GENERATED_FIELDS / AGGREGATE_FIELDS

#[derive(Model, Debug, Clone, Default)]
#[prax(table = "posts")]
pub struct Post {
    #[prax(id, auto)]
    pub id: i32,
    pub author_id: i32,
    pub views: i32,
    pub created_at: String,
}

#[derive(Model, Debug, Clone, Default)]
#[prax(table = "users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(unique)]
    pub email: String,
    pub first_name: String,
    pub last_name: String,

    #[prax(generated = "first_name || ' ' || last_name", stored)]
    pub full_name: String,

    #[prax(generated = "LOWER(email)", virtual)]
    pub search_key: String,

    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    pub posts: Vec<Post>,

    #[prax(count(posts))]
    pub post_count: i64,

    #[prax(sum(posts.views))]
    pub total_views: Option<i32>,
}

#[test]
fn user_emits_generated_field_metadata() {
    assert_eq!(
        User::GENERATED_FIELDS,
        &[
            ("full_name", "first_name || ' ' || last_name", true),
            ("search_key", "LOWER(email)", false),
        ][..],
    );
}

#[test]
fn user_emits_aggregate_field_metadata() {
    assert_eq!(
        User::AGGREGATE_FIELDS,
        &[
            ("post_count", "count", "posts", None),
            ("total_views", "sum", "posts", Some("views")),
        ][..],
    );
}

#[test]
fn post_has_no_computed_metadata() {
    assert!(Post::GENERATED_FIELDS.is_empty());
    assert!(Post::AGGREGATE_FIELDS.is_empty());
}
