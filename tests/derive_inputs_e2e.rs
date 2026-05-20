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

// Second fixture exercising the temporal / binary / json / uuid scalar
// surface so a regression in their codegen lowering is caught by tests.
#[derive(Model, Debug, Clone)]
#[prax(table = "audit_events")]
pub struct AuditEvent {
    #[prax(id, auto)]
    pub id: i64,
    #[prax(unique)]
    pub event_id: ::uuid::Uuid,
    pub occurred_at: ::chrono::DateTime<::chrono::Utc>,
    pub payload: Vec<u8>,
    pub metadata: ::serde_json::Value,
    pub amount: Option<::rust_decimal::Decimal>,
}

#[test]
fn audit_event_where_input_supports_temporal_binary_and_json() {
    use prax_query::inputs::{
        BytesFilter, DateTimeFilter, DecimalNullableFilter, ScalarFilter, UuidFilter,
    };

    let dt = ::chrono::Utc::now();
    let id = ::uuid::Uuid::nil();
    let _w = audit_event::AuditEventWhereInput {
        event_id: Some(UuidFilter::equals(id)),
        occurred_at: Some(DateTimeFilter::equals(dt)),
        payload: Some(BytesFilter::equals(vec![1u8, 2, 3])),
        amount: Some(DecimalNullableFilter {
            is_null: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Confirm UuidFilter lowers correctly.
    let f = UuidFilter::equals(id).into_filter("event_id");
    match f {
        Filter::Equals(col, prax_query::filter::FilterValue::String(s)) => {
            assert_eq!(col, "event_id");
            assert_eq!(s, id.to_string());
        }
        other => panic!("expected UuidFilter Equals, got {:?}", other),
    }
}

#[test]
fn audit_event_where_unique_input_uuid_variant_lowers() {
    use prax_query::filter::FilterValue;
    use prax_query::inputs::WhereUniqueInput;

    let id = ::uuid::Uuid::nil();
    let w = audit_event::AuditEventWhereUniqueInput::EventId(id);
    match w.into_ir() {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "event_id");
            assert_eq!(s, id.to_string());
        }
        other => panic!(
            "expected Filter::Equals on event_id column, got {:?}",
            other
        ),
    }
}

#[test]
fn audit_event_metadata_json_nullable_filter_lowers() {
    use prax_query::inputs::{JsonNullableFilter, ScalarFilter};

    let f = JsonNullableFilter {
        is_null: Some(false),
        ..Default::default()
    };
    let filter = f.into_filter("metadata");
    assert_eq!(filter, Filter::IsNotNull("metadata".into()));
}

#[test]
fn audit_event_select_emits_chosen_columns() {
    use prax_query::inputs::SelectInput;

    let s = audit_event::AuditEventSelect {
        id: Some(true),
        event_id: Some(true),
        occurred_at: Some(true),
        ..Default::default()
    };
    match s.into_ir() {
        prax_query::types::Select::Fields(cols) => {
            assert!(cols.contains(&"id".to_string()));
            assert!(cols.contains(&"event_id".to_string()));
            assert!(cols.contains(&"occurred_at".to_string()));
            assert_eq!(cols.len(), 3);
        }
        other => panic!("expected Select::Fields, got {:?}", other),
    }
}

#[test]
fn user_create_input_lowers_required_and_optional_fields() {
    use prax_query::filter::FilterValue;
    use prax_query::inputs::CreateInput;

    // `id` is `@id @auto` and excluded from CreateInput by codegen.
    // Required: `email`, `active`. Optional: `name`, `age`.
    let c = user::UserCreateInput {
        email: "alice@example.com".into(),
        name: Some("Alice".into()),
        age: None,
        active: true,
    };
    let payload = c.into_ir();
    // Required fields always emit; `name` emits because it's `Some(...)`;
    // `age = None` is skipped — codegen leaves nullable / has_default
    // fields out of the payload when unset.
    let cols: Vec<&str> = payload.iter().map(|(c, _)| c.as_str()).collect();
    assert!(cols.contains(&"email"), "missing email; got {cols:?}");
    assert!(cols.contains(&"active"), "missing active; got {cols:?}");
    assert!(cols.contains(&"name"), "missing name; got {cols:?}");
    assert!(
        !cols.contains(&"age"),
        "age should be omitted; got {cols:?}"
    );

    // Values round-trip through the `Into<FilterValue>` impls.
    let email_val = payload.iter().find(|(c, _)| c == "email").map(|(_, v)| v);
    assert!(matches!(email_val, Some(FilterValue::String(s)) if s == "alice@example.com"));
    let active_val = payload.iter().find(|(c, _)| c == "active").map(|(_, v)| v);
    assert_eq!(active_val, Some(&FilterValue::Bool(true)));
}

#[test]
fn user_update_input_lowers_set_and_arithmetic_ops() {
    use prax_query::filter::FilterValue;
    use prax_query::inputs::{IntNullableFieldUpdate, StringFieldUpdate, UpdateInput, WriteOp};

    let u = user::UserUpdateInput {
        email: Some(StringFieldUpdate {
            set: Some("bob@example.com".into()),
        }),
        age: Some(IntNullableFieldUpdate {
            increment: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    };
    let payload = u.into_ir();
    // Each Option<*FieldUpdate> wrapper lowers to at least one
    // (column, WriteOp) pair. Set + Increment surface as distinct
    // variants in the output.
    let has_email_set = payload.iter().any(|(c, op)| {
        c == "email" && matches!(op, WriteOp::Set(FilterValue::String(s)) if s == "bob@example.com")
    });
    let has_age_increment = payload
        .iter()
        .any(|(c, op)| c == "age" && matches!(op, WriteOp::Increment(FilterValue::Int(1))));
    assert!(
        has_email_set,
        "expected email = Set(\"bob...\"); got {payload:?}"
    );
    assert!(has_age_increment, "expected age += 1; got {payload:?}");
}

#[test]
fn user_update_input_lowers_unset_on_nullable() {
    use prax_query::inputs::{StringNullableFieldUpdate, UpdateInput, WriteOp};

    // `name: Option<String>` is nullable, so the wrapper carries `unset`.
    let u = user::UserUpdateInput {
        name: Some(StringNullableFieldUpdate {
            unset: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let payload = u.into_ir();
    let has_unset = payload
        .iter()
        .any(|(c, op)| c == "name" && matches!(op, WriteOp::Unset));
    assert!(has_unset, "expected name = Unset; got {payload:?}");
}
