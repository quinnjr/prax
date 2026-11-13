//! `String[]` scalar-list fields round-trip through Postgres text arrays:
//! `Vec<String>: ToFilterValue` produces a `FilterValue::Array` of strings,
//! which the engine binds as `TEXT[]`, and `Vec<String>: FromColumn` reads
//! the column back through `PgRow::get_string_array`.
//!
//! Gated by `PRAX_E2E=1` and requires `POSTGRES_URL` pointing at a
//! reachable Postgres instance, like `uuid_binding.rs`.
//!
//! ```sh
//! docker compose up -d postgres
//! docker compose run --rm test-postgres
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_postgres::row_ref::PgRow;
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::{FilterValue, ToFilterValue};
use prax_query::row::FromColumn;
use prax_query::traits::QueryEngine;

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("text_array_{prefix}_{pid}_{n}")
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
async fn string_lists_round_trip_through_text_array_columns() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let engine = PgEngine::new(pool.clone());
    let table = unique_table("rt");

    engine
        .execute_raw(
            &format!(
                "CREATE TABLE {table} (id INT PRIMARY KEY, scopes TEXT[] NOT NULL \
                 DEFAULT ARRAY[]::TEXT[], tags VARCHAR(20)[])"
            ),
            vec![],
        )
        .await
        .expect("create table");

    let scopes = vec!["read".to_string(), "write".to_string()];
    engine
        .execute_raw(
            &format!("INSERT INTO {table} (id, scopes, tags) VALUES (1, $1, $2), (2, $3, NULL)"),
            vec![
                scopes.to_filter_value(),
                vec!["a".to_string()].to_filter_value(),
                Vec::<String>::new().to_filter_value(),
            ],
        )
        .await
        .expect("INSERT binding string lists into TEXT[] / VARCHAR[] columns");

    // A string array also binds wherever SQL expects a text array, e.g. an
    // overlap filter in a raw query.
    let touched = engine
        .execute_raw(
            &format!("UPDATE {table} SET id = id WHERE scopes && $1"),
            vec![FilterValue::Array(vec![FilterValue::String(
                "write".into(),
            )])],
        )
        .await
        .expect("text-array parameter in a raw filter");
    assert_eq!(touched, 1);

    let conn = pool.get().await.expect("conn");
    let rows = conn
        .query(
            &format!("SELECT id, scopes, tags FROM {table} ORDER BY id"),
            &[],
        )
        .await
        .expect("select");
    let rows: Vec<PgRow> = rows.into_iter().map(PgRow::from).collect();

    assert_eq!(
        Vec::<String>::from_column(&rows[0], "scopes").unwrap(),
        scopes
    );
    assert_eq!(
        Option::<Vec<String>>::from_column(&rows[0], "tags").unwrap(),
        Some(vec!["a".to_string()])
    );
    assert!(
        Vec::<String>::from_column(&rows[1], "scopes")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        Option::<Vec<String>>::from_column(&rows[1], "tags").unwrap(),
        None
    );
    // A scalar column is not an array.
    assert!(Vec::<String>::from_column(&rows[0], "id").is_err());

    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .ok();
}
