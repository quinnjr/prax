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
