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

#[allow(unused_imports)]
use prax_query::inputs::{
    BigIntFilter, BigIntNullableFilter, BoolFilter, BoolNullableFilter, BytesFilter,
    BytesNullableFilter, DateFilter, DateNullableFilter, DateTimeFilter, DateTimeNullableFilter,
    DecimalFilter, DecimalNullableFilter, EnumFilter, EnumNullableFilter, FloatFilter,
    FloatNullableFilter, IntFilter, IntNullableFilter, JsonFilter, JsonNullableFilter, TimeFilter,
    TimeNullableFilter, UuidFilter, UuidNullableFilter,
};

#[test]
fn int_filter_equals_lowers() {
    let f = IntFilter::equals(42i32);
    let filter = f.into_filter("age");
    assert_eq!(filter, Filter::Equals("age".into(), FilterValue::Int(42)));
}

#[test]
fn int_filter_gt_lowers() {
    let f = IntFilter::gt(18i32);
    let filter = f.into_filter("age");
    assert_eq!(filter, Filter::Gt("age".into(), FilterValue::Int(18)));
}

#[test]
fn int_filter_in_list_lowers() {
    let f = IntFilter {
        in_list: Some(vec![1, 2, 3]),
        ..Default::default()
    };
    let filter = f.into_filter("id");
    match filter {
        Filter::In(col, values) => {
            assert_eq!(col, "id");
            assert_eq!(values.len(), 3);
        }
        other => panic!("expected Filter::In, got {:?}", other),
    }
}

#[test]
fn int_nullable_filter_is_null() {
    let f = IntNullableFilter {
        is_null: Some(true),
        ..Default::default()
    };
    let filter = f.into_filter("deleted_at");
    assert_eq!(filter, Filter::IsNull("deleted_at".into()));
}

#[test]
fn bool_filter_equals_lowers() {
    let f = BoolFilter::equals(true);
    let filter = f.into_filter("active");
    assert_eq!(
        filter,
        Filter::Equals("active".into(), FilterValue::Bool(true))
    );
}

#[test]
fn big_int_filter_equals_lowers() {
    let f = BigIntFilter::equals(9_999_999_999i64);
    let filter = f.into_filter("counter");
    assert_eq!(
        filter,
        Filter::Equals("counter".into(), FilterValue::Int(9_999_999_999))
    );
}

#[test]
fn float_filter_equals_lowers() {
    let f = FloatFilter::equals(2.71f64);
    let filter = f.into_filter("score");
    assert_eq!(
        filter,
        Filter::Equals("score".into(), FilterValue::Float(2.71))
    );
}

#[test]
fn decimal_filter_equals_lowers() {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let v = Decimal::from_str("12.34").unwrap();
    let f = DecimalFilter::equals(v);
    let filter = f.into_filter("amount");
    // Decimal flows through as a string in FilterValue::String — see lowering.
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "amount");
            assert_eq!(s, "12.34");
        }
        other => panic!("expected Decimal Equals to string, got {:?}", other),
    }
}

#[test]
fn uuid_filter_equals_lowers() {
    use uuid::Uuid;
    let id = Uuid::nil();
    let f = UuidFilter::equals(id);
    let filter = f.into_filter("id");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "id");
            assert_eq!(s, id.to_string());
        }
        other => panic!("expected Uuid Equals to string, got {:?}", other),
    }
}

#[test]
fn json_filter_equals_lowers() {
    let f = JsonFilter {
        equals: Some(serde_json::json!({"k": 1})),
        ..Default::default()
    };
    let filter = f.into_filter("data");
    match filter {
        Filter::Equals(col, FilterValue::Json(v)) => {
            assert_eq!(col, "data");
            assert_eq!(v, serde_json::json!({"k": 1}));
        }
        other => panic!("expected Json Equals, got {:?}", other),
    }
}

#[test]
fn bytes_filter_equals_lowers() {
    let f = BytesFilter::equals(vec![1u8, 2, 3]);
    let filter = f.into_filter("blob");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "blob");
            assert!(!s.is_empty());
        }
        other => panic!("expected Bytes Equals (base64-encoded), got {:?}", other),
    }
}

#[test]
fn datetime_filter_equals_lowers() {
    use chrono::{TimeZone, Utc};
    let dt = Utc.with_ymd_and_hms(2026, 5, 18, 12, 0, 0).unwrap();
    let f = DateTimeFilter::equals(dt);
    let filter = f.into_filter("created_at");
    // Encoded as RFC3339 string.
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "created_at");
            assert!(s.starts_with("2026-05-18T12:00:00"));
        }
        other => panic!("expected DateTime Equals, got {:?}", other),
    }
}

#[test]
fn date_filter_equals_lowers() {
    use chrono::NaiveDate;
    let d = NaiveDate::from_ymd_opt(2026, 5, 18).unwrap();
    let f = DateFilter::equals(d);
    let filter = f.into_filter("birthday");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "birthday");
            assert_eq!(s, "2026-05-18");
        }
        other => panic!("expected Date Equals, got {:?}", other),
    }
}

#[test]
fn time_filter_equals_lowers() {
    use chrono::NaiveTime;
    let t = NaiveTime::from_hms_opt(13, 45, 0).unwrap();
    let f = TimeFilter::equals(t);
    let filter = f.into_filter("opens_at");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "opens_at");
            assert_eq!(s, "13:45:00");
        }
        other => panic!("expected Time Equals, got {:?}", other),
    }
}

#[test]
fn enum_filter_equals_lowers() {
    let f: EnumFilter<&str> = EnumFilter::equals("Admin");
    let filter = f.into_filter("role");
    assert_eq!(
        filter,
        Filter::Equals("role".into(), FilterValue::String("Admin".into()))
    );
}
