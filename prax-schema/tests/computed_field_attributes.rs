//! Tests for the structured GeneratedAttribute / AggregateAttribute
//! payloads on FieldAttributes.

use prax_schema::ast::{AggregateAttribute, AggregateKind, FieldAttributes, GeneratedAttribute};

#[test]
fn generated_attribute_round_trip() {
    let g = GeneratedAttribute {
        expression: "a || b".into(),
        stored: true,
    };
    let json = serde_json::to_string(&g).unwrap();
    let back: GeneratedAttribute = serde_json::from_str(&json).unwrap();
    assert_eq!(g, back);
}

#[test]
fn aggregate_count_has_no_field() {
    let a = AggregateAttribute {
        kind: AggregateKind::Count,
        relation: "posts".into(),
        field: None,
    };
    assert_eq!(a.kind, AggregateKind::Count);
    assert!(a.field.is_none());
}

#[test]
fn aggregate_sum_has_field() {
    let a = AggregateAttribute {
        kind: AggregateKind::Sum,
        relation: "posts".into(),
        field: Some("views".into()),
    };
    assert_eq!(a.field.as_deref(), Some("views"));
}

#[test]
fn field_attributes_default_has_none() {
    let f = FieldAttributes::default();
    assert!(f.generated.is_none());
    assert!(f.aggregate.is_none());
}
