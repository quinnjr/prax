//! End-to-end CRUD test routing every operation through `PraxClient`
//! against a live PostgreSQL container.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`-marked so `cargo test` stays
//! fast. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test client_postgres -- --include-ignored
//! ```
//!
//! The test walks the whole `PraxClient<E>` → `user::Client<E>` →
//! operation-builder → engine path end-to-end: create, find_many,
//! update, delete_many, count. It's the first integration test that
//! proves the generated Client API wires together correctly.

#![cfg(test)]

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "client_pg_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

fn postgres_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
        // Compose publishes Postgres on host 5432 via `network_mode: host`.
        // The plan's 5433 guess was stale; keep the env var override for
        // deployments that remap the port.
        "postgres://prax:prax_test_password@localhost:5432/prax_test".into()
    }))
}

async fn setup() -> Option<PraxClient<PgEngine>> {
    let url = postgres_url()?;
    let pool: PgPool = PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres");

    let conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "DROP TABLE IF EXISTS client_pg_users; \
         CREATE TABLE client_pg_users ( \
             id SERIAL PRIMARY KEY, \
             email TEXT UNIQUE NOT NULL, \
             name TEXT \
         )",
    )
    .await
    .expect("create client_pg_users");
    drop(conn);

    Some(PraxClient::new(PgEngine::new(pool)))
}

#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn client_crud_roundtrip() {
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // CREATE
    let alice = c
        .user()
        .create()
        .set("email", "alice@pg.example.com")
        .set("name", "Alice")
        .exec()
        .await
        .expect("create alice");
    assert_eq!(alice.email, "alice@pg.example.com");
    assert_eq!(alice.name.as_deref(), Some("Alice"));
    assert!(alice.id > 0, "auto id should be assigned");

    // FIND_MANY
    let all = c
        .user()
        .find_many()
        .exec()
        .await
        .expect("find_many after create");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, alice.id);

    // UPDATE by equality on id (WhereParam -> Filter conversion)
    let updated = c
        .user()
        .update()
        .r#where(user::id::equals(alice.id))
        .set("name", "Alicia")
        .exec()
        .await
        .expect("update alice");
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].name.as_deref(), Some("Alicia"));
    assert_eq!(updated[0].id, alice.id);

    // COUNT
    let before_delete = c.user().count().exec().await.expect("count before delete");
    assert_eq!(before_delete, 1);

    // DELETE_MANY using a string-contains filter
    let deleted = c
        .user()
        .delete_many()
        .r#where(user::email::contains("pg.example.com"))
        .exec()
        .await
        .expect("delete_many");
    assert_eq!(deleted, 1);

    // Verify empty
    let remaining = c.user().count().exec().await.expect("count after delete");
    assert_eq!(remaining, 0);
}
