//! Shape test for `PraxClient<E>` + `client!` macro. Exercises every
//! accessor generated from a two-model application without touching a
//! live database — the check is compile-time.

use prax_orm::{Model, PraxClient, client};

#[derive(Model)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    email: String,
}

#[derive(Model)]
#[prax(table = "posts")]
struct Post {
    #[prax(id, auto)]
    id: i32,
    title: String,
}

client!(User, Post);

#[test]
fn client_has_user_and_post_accessors() {
    fn _check<E: prax_query::traits::QueryEngine>(engine: E) {
        let client = PraxClient::new(engine);
        let _ = client.user().find_many();
        let _ = client.user().find_unique();
        let _ = client.user().create();
        let _ = client.user().update();
        let _ = client.user().delete();
        let _ = client.user().count();
        let _ = client.post().find_many();
        let _ = client.post().create();
    }
}

#[test]
fn client_engine_is_cloneable() {
    fn _check<E: prax_query::traits::QueryEngine>(engine: E) {
        let client = PraxClient::new(engine);
        let _copy = client.clone();
    }
}
