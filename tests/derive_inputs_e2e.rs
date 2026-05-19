//! End-to-end test: `#[derive(Model)]` emits per-model typed input
//! structs that lower correctly through phase-1's runtime traits.
//!
//! This test lives in `prax-orm` (not `prax-codegen`) because the
//! derive macro emits code referencing `::prax_orm::_prax_prelude`,
//! which can't be reached from `prax-codegen`'s own dev-dependencies
//! due to the circular dependency between the proc-macro crate and
//! the umbrella crate.

use prax_orm::Model;
use prax_query::filter::Filter;
use prax_query::inputs::{BoolFilter, IntNullableFilter, StringFilter, WhereInput};

#[derive(Model, Debug, Clone)]
#[prax(table = "users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i64,
    #[prax(unique)]
    pub email: String,
    pub name: Option<String>,
    pub age: Option<i32>,
    pub active: bool,
}

#[test]
fn user_where_input_default_lowers_to_filter_none() {
    let w = user::UserWhereInput::default();
    assert!(matches!(w.into_ir(), Filter::None));
}

#[test]
fn user_where_input_with_three_operators_lowers_to_and_of_three() {
    let w = user::UserWhereInput {
        email: Some(StringFilter::contains("@example.com")),
        active: Some(BoolFilter::equals(true)),
        age: Some(IntNullableFilter {
            gte: Some(18),
            ..Default::default()
        }),
        ..Default::default()
    };

    let f = w.into_ir();
    match f {
        Filter::And(parts) => assert_eq!(
            parts.len(),
            3,
            "expected exactly 3 active filters AND'd together"
        ),
        other => panic!("expected Filter::And(3), got {:?}", other),
    }
}

#[test]
fn user_where_input_round_trips_through_serde_json() {
    let w = user::UserWhereInput {
        email: Some(StringFilter::contains("@x.com")),
        ..Default::default()
    };
    let json = serde_json::to_string(&w).unwrap();
    let back: user::UserWhereInput = serde_json::from_str(&json).unwrap();
    let contains = back.email.as_ref().and_then(|f| f.contains.as_deref());
    assert_eq!(contains, Some("@x.com"));
}

#[test]
fn user_where_unique_input_lowers_to_filter_equals() {
    use prax_query::filter::FilterValue;
    use prax_query::inputs::WhereUniqueInput;

    let w = user::UserWhereUniqueInput::Email("alice@example.com".into());
    match w.into_ir() {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "email");
            assert_eq!(s, "alice@example.com");
        }
        other => panic!("expected Filter::Equals on email, got {:?}", other),
    }
}
