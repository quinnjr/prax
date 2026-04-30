//! End-to-end CRUD test routing every operation through `PraxClient`
//! against a live MySQL container.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`-marked so `cargo test` stays
//! fast. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test client_mysql -- --include-ignored
//! ```
//!
//! Mirrors `tests/client_postgres.rs` — create, find_many, update,
//! count, delete_many, final count — but against MySQL so the driver's
//! parameter binding and RowRef bridge get exercised end-to-end.

#![cfg(test)]

use std::time::Duration;

use prax_mysql::{MysqlEngine, MysqlPool, MysqlPoolBuilder};
use prax_orm::{Model, PraxClient, client};

#[derive(Model, Debug)]
#[prax(table = "client_mysql_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

fn mysql_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("MYSQL_URL").unwrap_or_else(|_| {
        // docker-compose binds MySQL to host 3307 via `network_mode: host`.
        "mysql://prax:prax_test_password@localhost:3307/prax_test".into()
    }))
}

async fn setup() -> Option<PraxClient<MysqlEngine>> {
    let url = mysql_url()?;
    let pool: MysqlPool = MysqlPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to mysql");

    let mut conn = pool.get().await.expect("acquire conn");
    conn.execute("DROP TABLE IF EXISTS client_mysql_users")
        .await
        .expect("drop old client_mysql_users");
    conn.execute(
        "CREATE TABLE client_mysql_users ( \
             id INT AUTO_INCREMENT PRIMARY KEY, \
             email VARCHAR(255) UNIQUE NOT NULL, \
             name VARCHAR(255) \
         )",
    )
    .await
    .expect("create client_mysql_users");
    drop(conn);

    Some(PraxClient::new(MysqlEngine::new(pool)))
}

#[tokio::test]
#[ignore = "requires docker-compose mysql (PRAX_E2E=1)"]
async fn client_crud_roundtrip() {
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // CREATE
    let alice = c
        .user()
        .create()
        .set("email", "alice@mysql.example.com")
        .set("name", "Alice")
        .exec()
        .await
        .expect("create alice");
    assert_eq!(alice.email, "alice@mysql.example.com");
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
        .r#where(user::email::contains("mysql.example.com"))
        .exec()
        .await
        .expect("delete_many");
    assert_eq!(deleted, 1);

    // Verify empty
    let remaining = c.user().count().exec().await.expect("count after delete");
    assert_eq!(remaining, 0);
}
