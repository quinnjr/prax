use prax_orm::Model;

#[derive(Model)]
#[prax(table = "users")]
struct User {
    #[prax(id)]
    id: i32,
    email: String,
    age: i32,
    bio: Option<String>,
}

#[test]
fn where_param_converts_to_filter() {
    let param: user::WhereParam = user::email::equals("a@b.c".to_string());
    let filter: prax_query::filter::Filter = param.into();
    match filter {
        prax_query::filter::Filter::Equals(ref col, _) => assert_eq!(col.as_ref(), "email"),
        _ => panic!("unexpected filter variant: {:?}", filter),
    }
}

#[test]
fn numeric_field_supports_comparison() {
    let param = user::age::gt(18);
    let filter: prax_query::filter::Filter = param.into();
    match filter {
        prax_query::filter::Filter::Gt(ref col, _) => assert_eq!(col.as_ref(), "age"),
        _ => panic!("expected Gt, got {:?}", filter),
    }
}

#[test]
fn string_field_supports_contains() {
    let param = user::email::contains("example");
    let filter: prax_query::filter::Filter = param.into();
    match filter {
        prax_query::filter::Filter::Contains(
            ref col,
            prax_query::filter::FilterValue::String(ref s),
        ) => {
            assert_eq!(col.as_ref(), "email");
            assert_eq!(s, "example");
        }
        _ => panic!(
            "expected Contains with FilterValue::String, got {:?}",
            filter
        ),
    }
}

#[test]
fn optional_field_supports_null_checks() {
    let param = user::bio::is_null();
    let filter: prax_query::filter::Filter = param.into();
    assert!(matches!(
        filter,
        prax_query::filter::Filter::IsNull(ref col) if col.as_ref() == "bio"
    ));

    let param = user::bio::is_not_null();
    let filter: prax_query::filter::Filter = param.into();
    assert!(matches!(
        filter,
        prax_query::filter::Filter::IsNotNull(ref col) if col.as_ref() == "bio"
    ));
}

#[test]
fn not_maps_to_not_equals() {
    let param = user::email::not("x@y.z".to_string());
    let filter: prax_query::filter::Filter = param.into();
    match filter {
        prax_query::filter::Filter::NotEquals(ref col, _) => assert_eq!(col.as_ref(), "email"),
        _ => panic!("expected NotEquals, got {:?}", filter),
    }
}

// Qualified paths (e.g. `chrono::NaiveDate`) must classify the same as their
// unqualified siblings — classify_field_type matches on the last path
// segment's identifier — and the runtime FilterValue must have matching
// From impls so the emitted `.gt(v).into()` chain actually compiles against
// a typed value.
#[derive(Model)]
#[prax(table = "events")]
struct Event {
    #[prax(id)]
    id: i32,
    when: chrono::NaiveDate,
}

#[test]
fn qualified_chrono_naive_date_emits_comparison_ops() {
    let d = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let filter: prax_query::filter::Filter = event::when::gt(d).into();
    match filter {
        prax_query::filter::Filter::Gt(ref col, prax_query::filter::FilterValue::String(ref s)) => {
            assert_eq!(col.as_ref(), "when");
            assert_eq!(s, "2020-01-01");
        }
        _ => panic!("expected Gt/String on chrono::NaiveDate, got {filter:?}"),
    }
}

// ============================================================================
// Coverage for codegen-emitted operators that weren't previously tested.
// Each test here guards against a specific regression: if classify_field_type
// or generate_field_module_from_derive silently stops emitting one of these
// variants, the corresponding test fails to compile, not just fails at runtime.
// ============================================================================

#[test]
fn numeric_field_emits_lt() {
    let filter: prax_query::filter::Filter = user::age::lt(18).into();
    match filter {
        prax_query::filter::Filter::Lt(ref col, prax_query::filter::FilterValue::Int(18)) => {
            assert_eq!(col.as_ref(), "age");
        }
        _ => panic!("expected Lt(\"age\", Int(18)), got {filter:?}"),
    }
}

#[test]
fn numeric_field_emits_lte() {
    let filter: prax_query::filter::Filter = user::age::lte(18).into();
    match filter {
        prax_query::filter::Filter::Lte(ref col, prax_query::filter::FilterValue::Int(18)) => {
            assert_eq!(col.as_ref(), "age");
        }
        _ => panic!("expected Lte(\"age\", Int(18)), got {filter:?}"),
    }
}

#[test]
fn numeric_field_emits_gte() {
    let filter: prax_query::filter::Filter = user::age::gte(18).into();
    match filter {
        prax_query::filter::Filter::Gte(ref col, prax_query::filter::FilterValue::Int(18)) => {
            assert_eq!(col.as_ref(), "age");
        }
        _ => panic!("expected Gte(\"age\", Int(18)), got {filter:?}"),
    }
}

#[test]
fn string_field_emits_starts_with() {
    let filter: prax_query::filter::Filter = user::email::starts_with("joe").into();
    match filter {
        prax_query::filter::Filter::StartsWith(
            ref col,
            prax_query::filter::FilterValue::String(ref s),
        ) => {
            assert_eq!(col.as_ref(), "email");
            assert_eq!(s, "joe");
        }
        _ => panic!("expected StartsWith(\"email\", \"joe\"), got {filter:?}"),
    }
}

#[test]
fn string_field_emits_ends_with() {
    let filter: prax_query::filter::Filter = user::email::ends_with(".com").into();
    match filter {
        prax_query::filter::Filter::EndsWith(
            ref col,
            prax_query::filter::FilterValue::String(ref s),
        ) => {
            assert_eq!(col.as_ref(), "email");
            assert_eq!(s, ".com");
        }
        _ => panic!("expected EndsWith(\"email\", \".com\"), got {filter:?}"),
    }
}

#[test]
fn numeric_field_emits_in() {
    let filter: prax_query::filter::Filter = user::age::in_(vec![18, 21, 30]).into();
    match filter {
        prax_query::filter::Filter::In(ref col, ref list) => {
            assert_eq!(col.as_ref(), "age");
            assert_eq!(list.len(), 3);
            assert_eq!(list[0], prax_query::filter::FilterValue::Int(18));
            assert_eq!(list[1], prax_query::filter::FilterValue::Int(21));
            assert_eq!(list[2], prax_query::filter::FilterValue::Int(30));
        }
        _ => panic!("expected In(\"age\", [18, 21, 30]), got {filter:?}"),
    }
}

#[test]
fn numeric_field_emits_not_in() {
    let filter: prax_query::filter::Filter = user::age::not_in(vec![18, 21]).into();
    match filter {
        prax_query::filter::Filter::NotIn(ref col, ref list) => {
            assert_eq!(col.as_ref(), "age");
            assert_eq!(list.len(), 2);
            assert_eq!(list[0], prax_query::filter::FilterValue::Int(18));
            assert_eq!(list[1], prax_query::filter::FilterValue::Int(21));
        }
        _ => panic!("expected NotIn(\"age\", [18, 21]), got {filter:?}"),
    }
}

// ----------------------------------------------------------------------------
// Boolean + UUID + Decimal fields
// ----------------------------------------------------------------------------

// Separate struct because User already wires `age: i32` as Numeric and the
// test corpus splits by field-type category. `bool` has a very small op
// surface (classify_field_type::Boolean emits only Equals/Not) so this model
// pins the minimal contract.
#[derive(Model)]
#[prax(table = "sessions")]
struct Session {
    #[prax(id)]
    id: i32,
    active: bool,
    session_id: uuid::Uuid,
    price: rust_decimal::Decimal,
}

#[test]
fn bool_field_equals_emits_filter_value_bool() {
    let filter: prax_query::filter::Filter = session::active::equals(true).into();
    match filter {
        prax_query::filter::Filter::Equals(
            ref col,
            prax_query::filter::FilterValue::Bool(true),
        ) => {
            assert_eq!(col.as_ref(), "active");
        }
        _ => panic!("expected Equals(\"active\", Bool(true)), got {filter:?}"),
    }
}

#[test]
fn uuid_field_equals_emits_filter_value_string() {
    let u = uuid::Uuid::nil();
    let filter: prax_query::filter::Filter = session::session_id::equals(u).into();
    // Uuids serialize through FilterValue::String (via the Uuid -> FilterValue
    // From impl) so the hyphenated 36-char form lands in the payload.
    match filter {
        prax_query::filter::Filter::Equals(
            ref col,
            prax_query::filter::FilterValue::String(ref s),
        ) => {
            assert_eq!(col.as_ref(), "session_id");
            assert_eq!(s, "00000000-0000-0000-0000-000000000000");
        }
        _ => panic!("expected Equals(\"session_id\", String(\"00…0\")), got {filter:?}"),
    }
}

#[test]
fn decimal_field_gt_emits_filter_value_string() {
    use rust_decimal::Decimal;
    let filter: prax_query::filter::Filter = session::price::gt(Decimal::from(100)).into();
    // Decimal -> FilterValue goes through Decimal::to_string() to avoid
    // f64 rounding.
    match filter {
        prax_query::filter::Filter::Gt(ref col, prax_query::filter::FilterValue::String(ref s)) => {
            assert_eq!(col.as_ref(), "price");
            assert_eq!(s, "100");
        }
        _ => panic!("expected Gt(\"price\", String(\"100\")), got {filter:?}"),
    }
}

// ----------------------------------------------------------------------------
// Logical combinators — these don't live on the per-field module, but we
// pin them here so the derive's From<WhereParam> path plus Filter::and2 /
// or2 / Not compose into the shape downstream dialects expect.
// ----------------------------------------------------------------------------

#[test]
fn filter_and2_composes_two_filters() {
    let a: prax_query::filter::Filter = user::age::gt(18).into();
    let b: prax_query::filter::Filter = user::email::equals("a@b.c".to_string()).into();
    let combined = prax_query::filter::Filter::and2(a, b);
    match combined {
        prax_query::filter::Filter::And(ref filters) => {
            assert_eq!(filters.len(), 2);
            assert!(matches!(
                filters[0],
                prax_query::filter::Filter::Gt(ref c, _) if c.as_ref() == "age"
            ));
            assert!(matches!(
                filters[1],
                prax_query::filter::Filter::Equals(ref c, _) if c.as_ref() == "email"
            ));
        }
        _ => panic!("expected And([Gt, Equals]), got {combined:?}"),
    }
}

#[test]
fn filter_or2_composes_two_filters() {
    let a: prax_query::filter::Filter = user::email::equals("x@y.z".to_string()).into();
    let b: prax_query::filter::Filter = user::email::equals("a@b.c".to_string()).into();
    let combined = prax_query::filter::Filter::or2(a, b);
    match combined {
        prax_query::filter::Filter::Or(ref filters) => {
            assert_eq!(filters.len(), 2);
            assert!(matches!(
                filters[0],
                prax_query::filter::Filter::Equals(ref c, _) if c.as_ref() == "email"
            ));
            assert!(matches!(
                filters[1],
                prax_query::filter::Filter::Equals(ref c, _) if c.as_ref() == "email"
            ));
        }
        _ => panic!("expected Or([Equals, Equals]), got {combined:?}"),
    }
}

#[test]
fn filter_not_wraps_inner_filter() {
    let inner: prax_query::filter::Filter = user::age::lt(18).into();
    let not_filter = prax_query::filter::Filter::Not(Box::new(inner));
    match not_filter {
        prax_query::filter::Filter::Not(ref boxed) => match boxed.as_ref() {
            prax_query::filter::Filter::Lt(col, prax_query::filter::FilterValue::Int(18)) => {
                assert_eq!(col.as_ref(), "age");
            }
            other => panic!("expected inner Lt, got {other:?}"),
        },
        _ => panic!("expected Not(Lt), got {not_filter:?}"),
    }
}
