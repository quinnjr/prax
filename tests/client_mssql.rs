//! End-to-end CRUD test routing every operation through `PraxClient`
//! against a live Microsoft SQL Server container.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`-marked so `cargo test` stays
//! fast. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test client_mssql -- --include-ignored
//! ```
//!
//! Mirrors `tests/client_postgres.rs` — create, find_many, update,
//! count, delete_many, final count — but against MSSQL so the tiberius
//! driver's parameter binding and RowRef bridge get exercised
//! end-to-end.

#![cfg(test)]

use std::time::Duration;

use prax_mssql::{MssqlConfig, MssqlEngine, MssqlPool};
use prax_orm::{Model, PraxClient, client};

#[derive(Model, Debug)]
#[prax(table = "client_mssql_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

fn mssql_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("MSSQL_URL").unwrap_or_else(|_| {
        // MSSQL listens on 1433 via `network_mode: host`. Prax's ADO
        // connection-string parser splits `server=` on ',' to extract
        // host,port — so we use bare `localhost,1433` (the `tcp:` prefix
        // from Microsoft's ADO syntax makes the parser treat
        // `tcp:localhost` as a hostname and DNS lookup fails). Turn on
        // trust_cert because the container's cert is self-signed.
        "server=localhost,1433;database=master;\
         user=sa;password=Prax_Test_Password123!;\
         trustservercertificate=true"
            .into()
    }))
}

async fn setup() -> Option<PraxClient<MssqlEngine>> {
    let url = mssql_url()?;
    let config = MssqlConfig::from_connection_string(&url).expect("parse mssql url");
    let pool: MssqlPool = MssqlPool::builder()
        .config(config)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(15))
        .trust_cert(true)
        .build()
        .await
        .expect("connect to mssql");

    let mut conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "IF OBJECT_ID('dbo.client_mssql_users', 'U') IS NOT NULL \
             DROP TABLE dbo.client_mssql_users;",
    )
    .await
    .expect("drop old client_mssql_users");
    conn.batch_execute(
        "CREATE TABLE dbo.client_mssql_users ( \
             id INT IDENTITY(1,1) PRIMARY KEY, \
             email NVARCHAR(255) UNIQUE NOT NULL, \
             name NVARCHAR(255) \
         )",
    )
    .await
    .expect("create client_mssql_users");
    drop(conn);

    Some(PraxClient::new(MssqlEngine::new(pool)))
}

#[tokio::test]
#[ignore = "requires docker-compose mssql (PRAX_E2E=1)"]
async fn client_crud_roundtrip() {
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // CREATE
    let alice = c
        .user()
        .create()
        .set("email", "alice@mssql.example.com")
        .set("name", "Alice")
        .exec()
        .await
        .expect("create alice");
    assert_eq!(alice.email, "alice@mssql.example.com");
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

    // UPDATE by equality on id. Note: MSSQL is strictly typed like
    // Postgres, so if the driver's `filter_value_to_sql` binds i64
    // directly, this will fail with a type-mismatch error against the
    // INT column. See prax-mssql/src/types.rs and commit 2aba7ef for
    // the Postgres fix pattern.
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
        .r#where(user::email::contains("mssql.example.com"))
        .exec()
        .await
        .expect("delete_many");
    assert_eq!(deleted, 1);

    // Verify empty
    let remaining = c.user().count().exec().await.expect("count after delete");
    assert_eq!(remaining, 0);
}
