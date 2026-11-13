//! `String[]` scalar-list fields round-trip through Postgres text arrays on
//! the SQLx engine: `Vec<String>: ToFilterValue` produces an array of
//! strings (`FilterValue::Array`), which `prax-sqlx` binds as `TEXT[]`, and the row decoder reads
//! `TEXT[]` / `VARCHAR[]` cells back so `Vec<String>: FromColumn` works.
//!
//! Gated by `PRAX_E2E=1` and requires `POSTGRES_URL` pointing at a
//! reachable Postgres instance, like the prax-postgres e2e tests.

#![cfg(feature = "postgres")]

use std::sync::atomic::{AtomicU32, Ordering};

use prax_query::filter::{FilterValue, ToFilterValue};
use prax_query::row::FromColumn;
use prax_sqlx::row_ref::SqlxRowRef;
use prax_sqlx::{SqlxConfig, SqlxEngine};

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table() -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("sqlx_text_array_{}_{n}", std::process::id())
}

fn postgres_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("POSTGRES_URL").ok()
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn string_lists_round_trip_through_text_array_columns() {
    let Some(url) = postgres_url() else {
        return;
    };
    let engine = SqlxEngine::new(SqlxConfig::from_url(url).expect("config"))
        .await
        .expect("connect to postgres");
    let table = unique_table();

    engine
        .raw_execute(
            &format!(
                "CREATE TABLE {table} (id INT PRIMARY KEY, scopes TEXT[] NOT NULL \
                 DEFAULT ARRAY[]::TEXT[], tags VARCHAR(20)[])"
            ),
            &[],
        )
        .await
        .expect("create table");

    let scopes = vec!["read".to_string(), "write".to_string()];
    engine
        .raw_execute(
            &format!("INSERT INTO {table} (id, scopes, tags) VALUES (1, $1, $2), (2, $3, NULL)"),
            &[
                scopes.to_filter_value(),
                vec!["a".to_string()].to_filter_value(),
                Vec::<String>::new().to_filter_value(),
            ],
        )
        .await
        .expect("INSERT binding string lists into TEXT[] / VARCHAR[] columns");

    let touched = engine
        .raw_execute(
            &format!("UPDATE {table} SET id = id WHERE scopes && $1"),
            &[FilterValue::Array(vec![FilterValue::String(
                "write".into(),
            )])],
        )
        .await
        .expect("text-array parameter in a raw filter");
    assert_eq!(touched, 1);

    let rows = engine
        .raw_query_many(
            &format!("SELECT id, scopes, tags FROM {table} ORDER BY id"),
            &[],
        )
        .await
        .expect("select");
    let rows: Vec<SqlxRowRef> = rows
        .iter()
        .map(|r| SqlxRowRef::from_sqlx(r).expect("decode row"))
        .collect();

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
    assert!(Vec::<String>::from_column(&rows[0], "id").is_err());

    engine
        .raw_execute(&format!("DROP TABLE IF EXISTS {table}"), &[])
        .await
        .ok();
}
