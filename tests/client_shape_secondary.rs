//! Proves the derive-emitted `Client<E>` accepts every backend that
//! implements `QueryEngine`, not just the four built-in SQL drivers.
//! Compile-time only; no live database involvement.
//!
//! If a secondary backend stops satisfying the trait surface — missing
//! `FromRow`, changed `query_many` signature, etc. — this test breaks
//! compilation so the regression can't land silently.

#![cfg(test)]

use prax_orm::{Model, PraxClient, client};

#[derive(Model, Debug)]
#[prax(table = "probes")]
struct Probe {
    #[prax(id, auto)]
    id: i32,
    name: String,
}

client!(Probe);

#[test]
fn mongo_engine_satisfies_client_surface() {
    fn _check(engine: prax_mongodb::MongoEngine) {
        let client = PraxClient::new(engine);
        let _ = client.probe().find_many();
        let _ = client.probe().find_unique();
        let _ = client.probe().find_first();
        let _ = client.probe().count();
    }
}

#[test]
fn sqlx_engine_satisfies_client_surface() {
    fn _check(engine: prax_sqlx::SqlxEngine) {
        let client = PraxClient::new(engine);
        let _ = client.probe().find_many();
        let _ = client.probe().create();
        let _ = client.probe().update();
        let _ = client.probe().delete();
    }
}
