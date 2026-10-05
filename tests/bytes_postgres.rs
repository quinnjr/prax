//! Live-Postgres regression test: `BytesFieldUpdate` must bind raw bytes,
//! not base64 text (0.12.x wrote base64 text into `bytea` columns).
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 cargo test --test bytes_postgres -- --include-ignored
//! ```

#![cfg(test)]

use std::time::Duration;

use prax_orm::Model;
use prax_postgres::{PgEngine, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::inputs::{BytesFieldUpdate, UpdateInput, WriteOp};
use prax_query::traits::QueryEngine;

#[derive(Debug, Model)]
#[prax(table = "bytes_pg_blobs")]
pub struct Blob {
    #[prax(id, auto)]
    pub id: i32,
    pub b: Vec<u8>,
}

#[derive(Debug, Model)]
#[prax(table = "bytes_pg_tokens")]
pub struct Token {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(unique)]
    pub tok: Vec<u8>,
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn bytes_update_input_writes_raw_bytes() {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let url = std::env::var("POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5432/prax_test".into());
    let pool = PgPoolBuilder::new()
        .url(url)
        .max_connections(2)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres");

    let conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS bytes_pg_blobs (id SERIAL PRIMARY KEY, b BYTEA NOT NULL)",
    )
    .await
    .expect("create table");
    let id: i32 = conn
        .query_one(
            "INSERT INTO bytes_pg_blobs (b) VALUES ($1) RETURNING id",
            &[&vec![1u8]],
        )
        .await
        .expect("insert")
        .get(0);

    let ir = blob::BlobUpdateInput {
        b: Some(BytesFieldUpdate {
            set: Some(vec![0x00, 0x9F, 0xFF]),
        }),
    }
    .into_ir();
    let v = ir
        .into_iter()
        .find_map(|(c, op)| match (c.as_str(), op) {
            ("b", WriteOp::Set(v)) => Some(v),
            _ => None,
        })
        .expect("b set op");

    let engine = PgEngine::new(pool.clone());
    engine
        .execute_raw(
            "UPDATE bytes_pg_blobs SET b = $1 WHERE id = $2",
            vec![v, FilterValue::Int(id as i64)],
        )
        .await
        .expect("update");

    let row = conn
        .query_one("SELECT b FROM bytes_pg_blobs WHERE id = $1", &[&id])
        .await
        .expect("select");
    assert_eq!(row.get::<_, Vec<u8>>(0), vec![0x00, 0x9F, 0xFF]);

    conn.execute("DELETE FROM bytes_pg_blobs WHERE id = $1", &[&id])
        .await
        .expect("cleanup");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn bytes_where_unique_matches_row() {
    use prax_query::dialect::Postgres;
    use prax_query::inputs::WhereUniqueInput;

    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let url = std::env::var("POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5432/prax_test".into());
    let pool = PgPoolBuilder::new()
        .url(url)
        .max_connections(2)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres");
    let conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS bytes_pg_tokens (id SERIAL PRIMARY KEY, tok BYTEA NOT NULL UNIQUE)",
    )
    .await
    .expect("create table");
    let tok = vec![0x00u8, 0x9F, 0xFF, 0x42];
    conn.execute("DELETE FROM bytes_pg_tokens WHERE tok = $1", &[&tok])
        .await
        .expect("pre-clean");
    conn.execute("INSERT INTO bytes_pg_tokens (tok) VALUES ($1)", &[&tok])
        .await
        .expect("insert");

    let filter = token::TokenWhereUniqueInput::Tok(tok.clone()).into_ir();
    let (where_sql, params) = filter.to_sql(0, &Postgres);
    let engine = PgEngine::new(pool.clone());
    let n = engine
        .execute_raw(
            &format!("UPDATE bytes_pg_tokens SET tok = tok WHERE {where_sql}"),
            params,
        )
        .await
        .expect("lookup");
    assert_eq!(n, 1, "unique bytes lookup must match the row");

    conn.execute("DELETE FROM bytes_pg_tokens WHERE tok = $1", &[&tok])
        .await
        .expect("cleanup");
}
