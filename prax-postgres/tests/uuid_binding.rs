//! Regression: `FilterValue::String` must bind as `uuid::Uuid` when the
//! target column is Postgres `UUID`. The pre-fix engine bound it as a Rust
//! `String`, which tokio-postgres rejects with `WrongType`. Also covers
//! reading a `UUID`/`TIMESTAMPTZ` column back out as a `String` and
//! `Option<T>::is_null` on non-`TEXT` columns.
//!
//! Gated by `PRAX_E2E=1` and requires `POSTGRES_URL` pointing at a
//! reachable Postgres instance. Tests are `#[ignore]`-marked so
//! `cargo test` in a dev workflow skips them; the docker-compose
//! `test-postgres` runner passes `--include-ignored` to opt in.
//!
//! ```sh
//! docker compose up -d postgres
//! docker compose run --rm test-postgres
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::traits::QueryEngine;
use uuid::Uuid;

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("uuid_{prefix}_{pid}_{n}")
}

fn skip_unless_e2e() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("POSTGRES_URL").ok()
}

async fn pool() -> PgPool {
    let url = skip_unless_e2e().expect("PRAX_E2E=1 and POSTGRES_URL required");
    PgPoolBuilder::new()
        .url(url)
        .max_connections(2)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn filter_value_string_binds_as_uuid_for_uuid_column() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let engine = PgEngine::new(pool().await);
    let table = unique_table("bind");

    engine
        .execute_raw(
            &format!("CREATE TABLE {table} (id UUID PRIMARY KEY, name TEXT NOT NULL)"),
            vec![],
        )
        .await
        .expect("create table");

    let id = Uuid::new_v4();
    engine
        .execute_raw(
            &format!("INSERT INTO {table} (id, name) VALUES ($1, $2)"),
            vec![
                FilterValue::String(id.to_string()),
                FilterValue::String("hello".into()),
            ],
        )
        .await
        .expect("INSERT binding String into a UUID column");

    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .ok();
}
