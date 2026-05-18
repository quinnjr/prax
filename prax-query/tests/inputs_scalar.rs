use prax_query::filter::{Filter, FilterValue};
use prax_query::inputs::{QueryMode, ScalarFilter, StringFilter, StringNullableFilter};

#[test]
fn string_filter_equals_lowers_to_filter_equals() {
    let f = StringFilter::equals("alice@example.com");
    let filter = f.into_filter("email");
    assert_eq!(
        filter,
        Filter::Equals(
            "email".into(),
            FilterValue::String("alice@example.com".into())
        )
    );
}

#[test]
fn string_filter_contains_lowers_to_filter_contains() {
    let f = StringFilter::contains("@example.com");
    let filter = f.into_filter("email");
    assert!(matches!(filter, Filter::Contains(_, _)));
}

#[test]
fn string_filter_combines_with_and_when_multiple_ops_set() {
    let f = StringFilter {
        contains: Some("@x.com".into()),
        starts_with: Some("a".into()),
        ..Default::default()
    };
    let filter = f.into_filter("email");
    match filter {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}

#[test]
fn string_nullable_filter_is_null_lowers_to_is_null() {
    let f = StringNullableFilter {
        is_null: Some(true),
        ..Default::default()
    };
    let filter = f.into_filter("name");
    assert_eq!(filter, Filter::IsNull("name".into()));
}

#[test]
fn string_nullable_filter_is_not_null_lowers_to_is_not_null() {
    let f = StringNullableFilter {
        is_null: Some(false),
        ..Default::default()
    };
    let filter = f.into_filter("name");
    assert_eq!(filter, Filter::IsNotNull("name".into()));
}

#[test]
fn query_mode_default_is_default() {
    assert_eq!(QueryMode::default(), QueryMode::Default);
}

#[test]
fn string_filter_from_scalar_shortcut() {
    let f: StringFilter = "alice@x.com".into();
    assert_eq!(f.equals, Some("alice@x.com".into()));
}
