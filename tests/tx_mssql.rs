//! Integration tests for `PraxClient::transaction` against a live
//! Microsoft SQL Server container.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]` so the default `cargo test`
//! run skips them. Opt in with:
//!
//! ```sh
//! docker compose up -d mssql
//! PRAX_E2E=1 MSSQL_URL='server=localhost,1433;database=master;\
//!   user=sa;password=Prax_Test_Password123!;\
//!   trustservercertificate=true' \
//!   cargo test --test tx_mssql -- --include-ignored --nocapture
//! ```
//!
//! Mirrors `tests/tx_postgres.rs` — two tests per file,
//! `transaction_rolls_back_on_error` and
//! `transaction_commits_on_ok`, sharing the `tx_mssql_users` table
//! and namespaced by email prefix.

#![cfg(test)]

use std::time::Duration;

use prax_mssql::{MssqlConfig, MssqlEngine, MssqlPool};
use prax_orm::{Model, PraxClient, client};
use prax_query::error::{QueryError, QueryResult};
use prax_query::raw::Sql;

#[derive(Debug, Model)]
#[prax(table = "tx_mssql_users")]
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
        // MSSQL listens on 1433 via `network_mode: host`. Use bare
        // `localhost,1433` — the `tcp:` prefix from Microsoft's ADO
        // syntax makes Prax's parser treat `tcp:localhost` as a
        // hostname and DNS lookup fails.
        "server=localhost,1433;database=master;\
         user=sa;password=Prax_Test_Password123!;\
         trustservercertificate=true"
            .into()
    }))
}

async fn build_pool(url: String) -> MssqlPool {
    let config = MssqlConfig::from_connection_string(&url).expect("parse mssql url");
    MssqlPool::builder()
        .config(config)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(15))
        .trust_cert(true)
        .build()
        .await
        .expect("connect to mssql")
}

async fn setup() -> Option<(PraxClient<MssqlEngine>, MssqlPool)> {
    let url = mssql_url()?;
    let pool = build_pool(url).await;

    let mut conn = pool.get().await.expect("acquire conn for setup");
    conn.batch_execute(
        "IF OBJECT_ID('dbo.tx_mssql_users', 'U') IS NULL \
             CREATE TABLE dbo.tx_mssql_users ( \
                 id INT IDENTITY(1,1) PRIMARY KEY, \
                 email NVARCHAR(255) UNIQUE NOT NULL, \
                 name NVARCHAR(255) \
             );",
    )
    .await
    .expect("create tx_mssql_users");
    drop(conn);

    Some((PraxClient::new(MssqlEngine::new(pool.clone())), pool))
}

#[tokio::test]
#[ignore = "requires docker-compose mssql (PRAX_E2E=1)"]
async fn transaction_rolls_back_on_error() {
    let Some((client, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    let email = "tx_rollback@mssql.example.com";

    client
        .execute_raw(Sql::new("DELETE FROM tx_mssql_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let result: QueryResult<()> = client
        .transaction(|tx| async move {
            tx.user()
                .create()
                .set("email", "tx_rollback@mssql.example.com")
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
            Sql::new("SELECT id, email, name FROM tx_mssql_users WHERE email = ").bind(email),
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
#[ignore = "requires docker-compose mssql (PRAX_E2E=1)"]
async fn transaction_commits_on_ok() {
    let Some((client, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    let email = "tx_commit@mssql.example.com";

    client
        .execute_raw(Sql::new("DELETE FROM tx_mssql_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let created_id: i32 = client
        .transaction(|tx| async move {
            let u = tx
                .user()
                .create()
                .set("email", "tx_commit@mssql.example.com")
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
            Sql::new("SELECT id, email, name FROM tx_mssql_users WHERE email = ").bind(email),
        )
        .await
        .expect("post-commit read");
    assert_eq!(rows.len(), 1, "commit did not persist the row");
    assert_eq!(rows[0].id, created_id);
    assert_eq!(rows[0].name.as_deref(), Some("Committed"));

    client
        .execute_raw(Sql::new("DELETE FROM tx_mssql_users WHERE email = ").bind(email))
        .await
        .expect("post-test cleanup");
}
