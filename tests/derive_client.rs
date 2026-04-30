//! Confirms `#[derive(Model)]` emits a `Client<E>` with all operation accessors.

use prax_orm::Model;

#[derive(Model)]
#[prax(table = "posts")]
struct Post {
    #[prax(id, auto)]
    id: i32,
    title: String,
}

#[test]
fn post_client_surfaces_every_operation() {
    fn _check<E: prax_query::traits::QueryEngine>(engine: E) {
        let client = post::Client::new(engine);
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
