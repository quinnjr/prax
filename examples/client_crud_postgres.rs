//! End-to-end Prax client example against PostgreSQL.
//!
//! Walks the full CRUD cycle through the typed `PraxClient<E>` API
//! emitted by `#[derive(Model)]` + `prax::client!`: create, find_many,
//! update, count, delete_many. Mirrors the shape of
//! `tests/client_postgres.rs` so it doubles as a live sanity check
//! after touching the client surface.
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_POSTGRES_URL=postgres://prax:prax_test_password@localhost:5432/prax_test \
//!     cargo run --example client_crud_postgres
//! ```
//!
//! The compose file uses `network_mode: host`, so the Postgres service
//! binds the host's 5432 directly. Set `PRAX_POSTGRES_URL` to override
//! the DSN for deployments that remap the port.

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "example_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

// Wire `user()` onto PraxClient<E> via the generated extension trait.
client!(User);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5432/prax_test".into());

    let pool: PgPool = PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await?;

    // Fresh table each run so the example is idempotent. `example_users`
    // avoids colliding with the tables the integration tests reset.
    let conn = pool.get().await?;
    conn.batch_execute(
        "DROP TABLE IF EXISTS example_users; \
         CREATE TABLE example_users ( \
             id SERIAL PRIMARY KEY, \
             email TEXT UNIQUE NOT NULL, \
             name TEXT \
         )",
    )
    .await?;
    drop(conn);

    let client = PraxClient::new(PgEngine::new(pool));

    let alice = client
        .user()
        .create()
        .set("email", "alice@example.com")
        .set("name", "Alice")
        .exec()
        .await?;
    println!("Created:  {alice:?}");

    let all = client.user().find_many().exec().await?;
    println!("All:      {all:?}");

    let updated = client
        .user()
        .update()
        .r#where(user::id::equals(alice.id))
        .set("name", "Alicia")
        .exec()
        .await?;
    println!("Updated:  {updated:?}");

    let count_before = client.user().count().exec().await?;
    println!("Count:    {count_before}");

    let deleted = client
        .user()
        .delete_many()
        .r#where(user::email::contains("example.com"))
        .exec()
        .await?;
    println!("Deleted:  {deleted} rows");

    Ok(())
}
