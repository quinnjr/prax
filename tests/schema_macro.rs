//! Confirms the `prax_schema!` macro emits Model + FromRow + Client<E>
//! with the same surface area as `#[derive(Model)]`.

prax_orm::prax_schema!("tests/fixtures/basic.prax");

#[test]
fn schema_generates_client_with_every_operation() {
    fn _check<E: prax_query::traits::QueryEngine>(engine: E) {
        let client = user::Client::new(engine);
        let _ = client.find_many();
        let _ = client.find_unique();
        let _ = client.find_first();
        let _ = client.create();
        let _ = client.create_many();
        let _ = client.update();
        let _ = client.update_many();
        let _ = client.upsert();
        let _ = client.delete();
        let _ = client.delete_many();
        let _ = client.count();
    }
}

#[test]
fn schema_implements_model_and_fromrow() {
    fn _assert_impls<T: prax_query::traits::Model + prax_query::row::FromRow>() {}
    _assert_impls::<user::User>();
    // model name in .prax is `User`, so the default table name is `User`
    // (the `@@map("...")` attribute overrides this).
    assert_eq!(
        <user::User as prax_query::traits::Model>::TABLE_NAME,
        "User"
    );
}

#[test]
fn schema_user_implements_model_with_pk() {
    use prax_query::filter::FilterValue;
    use prax_query::traits::ModelWithPk;
    let u = user::User {
        id: 7,
        email: "e@f.g".into(),
        name: Some("n".into()),
    };
    assert_eq!(u.pk_value(), FilterValue::Int(7));
    assert_eq!(
        u.get_column_value("email"),
        Some(FilterValue::String("e@f.g".into()))
    );
    assert_eq!(
        u.get_column_value("name"),
        Some(FilterValue::String("n".into()))
    );
    assert_eq!(u.get_column_value("missing"), None);
}
