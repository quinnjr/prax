use prax_orm::Model;
use prax_query::traits::Model as QueryModel;

#[derive(Model)]
#[prax(table = "authors")]
struct Author {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

fn assert_impls<T: prax_query::row::FromRow + prax_query::traits::Model>() {}

#[test]
fn author_has_model_and_fromrow_impls() {
    assert_impls::<Author>();
    assert_eq!(Author::TABLE_NAME, "authors");
    assert_eq!(Author::PRIMARY_KEY, &["id"]);
}
