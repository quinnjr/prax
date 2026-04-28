use prax_orm::Model;

#[derive(Model)]
#[prax(table = "users")]
struct User {
    #[prax(id)]
    id: i32,
    email: String,
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
