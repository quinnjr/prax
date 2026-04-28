//! Integration tests for `PraxClient::transaction` against a live
//! MySQL server.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]` so the default `cargo test`
//! run skips them. Opt in with:
//!
//! ```sh
//! docker compose up -d mysql
//! PRAX_E2E=1 cargo test --test tx_mysql -- --include-ignored --nocapture
//! ```
//!
//! Mirrors `tests/tx_postgres.rs` — share the `tx_mysql_users` table
//! across both tests and namespace rows by email prefix so parallel
//! runs stay hermetic on the unique-email constraint.

#![cfg(test)]

use std::time::Duration;

use prax_mysql::{MysqlEngine, MysqlPool, MysqlPoolBuilder};
use prax_orm::{Model, PraxClient, client};
use prax_query::error::{QueryError, QueryResult};
use prax_query::raw::Sql;

#[derive(Debug, Model)]
#[prax(table = "tx_mysql_users")]
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
    Some(
        std::env::var("MYSQL_URL")
            .unwrap_or_else(|_| "mysql://prax:prax_test_password@localhost:3307/prax_test".into()),
    )
}

async fn build_pool(url: String) -> MysqlPool {
    MysqlPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to mysql")
}

async fn setup() -> Option<(PraxClient<MysqlEngine>, MysqlPool)> {
    let url = mysql_url()?;
    let pool = build_pool(url).await;

    let mut conn = pool.get().await.expect("acquire conn for setup");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tx_mysql_users ( \
             id INT AUTO_INCREMENT PRIMARY KEY, \
             email VARCHAR(255) UNIQUE NOT NULL, \
             name VARCHAR(255) \
         )",
    )
    .await
    .expect("create tx_mysql_users");
    drop(conn);

    Some((PraxClient::new(MysqlEngine::new(pool.clone())), pool))
}

#[tokio::test]
#[ignore = "requires docker-compose mysql (PRAX_E2E=1)"]
async fn transaction_rolls_back_on_error() {
    let Some((client, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    let email = "tx_rollback@mysql.example.com";

    client
        .execute_raw(Sql::new("DELETE FROM tx_mysql_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let result: QueryResult<()> = client
        .transaction(|tx| async move {
            tx.user()
                .create()
                .set("email", "tx_rollback@mysql.example.com")
                .set("name", "Rolled Back")
                .exec()
                .await?;

            Err(QueryError::internal("intentional rollback trigger"))
        })
        .await;

    assert!(
        result.is_err(),
        "closure returned Err, tx should surface it"
    );

    let rows: Vec<User> = client
        .query_raw(
            Sql::new("SELECT id, email, name FROM tx_mysql_users WHERE email = ").bind(email),
        )
        .await
        .expect("post-rollback read");
    assert!(
        rows.is_empty(),
        "rollback did not happen: found {} row(s) with email {}",
        rows.len(),
        email
    );
}

#[tokio::test]
#[ignore = "requires docker-compose mysql (PRAX_E2E=1)"]
async fn transaction_commits_on_ok() {
    let Some((client, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    let email = "tx_commit@mysql.example.com";

    client
        .execute_raw(Sql::new("DELETE FROM tx_mysql_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let created_id: i32 = client
        .transaction(|tx| async move {
            let u = tx
                .user()
                .create()
                .set("email", "tx_commit@mysql.example.com")
                .set("name", "Committed")
                .exec()
                .await?;
            Ok(u.id)
        })
        .await
        .expect("commit-on-ok transaction");
    assert!(created_id > 0, "expected auto-assigned PK from INSERT");

    let rows: Vec<User> = client
        .query_raw(
            Sql::new("SELECT id, email, name FROM tx_mysql_users WHERE email = ").bind(email),
        )
        .await
        .expect("post-commit read");
    assert_eq!(rows.len(), 1, "commit did not persist the row");
    assert_eq!(rows[0].id, created_id);
    assert_eq!(rows[0].name.as_deref(), Some("Committed"));

    client
        .execute_raw(Sql::new("DELETE FROM tx_mysql_users WHERE email = ").bind(email))
        .await
        .expect("post-test cleanup");
}
