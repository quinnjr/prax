//! Live-SQLite regression test: `Vec<u8>` / `BytesFieldUpdate` must land in
//! the column as a real `blob` (raw bytes), not base64 text.
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test bytes_sqlite -- --include-ignored
//! ```

#![cfg(test)]

mod common;

use std::time::Duration;

use common::e2e_enabled;
use prax_orm::{Model, PraxClient, client};
use prax_query::filter::FilterValue;
use prax_query::inputs::{BytesFieldUpdate, UpdateInput, WriteOp};
use prax_query::traits::QueryEngine;
use prax_sqlite::{SqliteConfig, SqliteEngine, SqlitePool};
use tempfile::TempDir;

#[derive(Debug, Model)]
#[prax(table = "bytes_sqlite_blobs")]
pub struct Blob {
    #[prax(id, auto)]
    pub id: i32,
    pub b: Vec<u8>,
}

client!(Blob);

struct SqliteTest {
    engine: SqliteEngine,
    pool: SqlitePool,
    _tempdir: TempDir,
}

async fn setup() -> Option<SqliteTest> {
    if !e2e_enabled() {
        return None;
    }
    let tempdir = tempfile::tempdir().expect("tempdir");
    let path = tempdir.path().join("bytes.sqlite");
    let pool: SqlitePool = SqlitePool::builder()
        .config(SqliteConfig::file(&path))
        .max_connections(2)
        .connection_timeout(Duration::from_secs(5))
        .build()
        .await
        .expect("build sqlite pool");
    let conn = pool.get().await.expect("acquire conn");
    conn.execute_batch(
        "CREATE TABLE bytes_sqlite_blobs (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             b BLOB NOT NULL
         )",
    )
    .await
    .expect("create bytes_sqlite_blobs");
    drop(conn);
    Some(SqliteTest {
        engine: SqliteEngine::new(pool.clone()),
        pool,
        _tempdir: tempdir,
    })
}

/// `(typeof(b), b)` for one row, read through the raw rusqlite connection.
async fn raw_typeof_and_bytes(pool: &SqlitePool, id: i32) -> (String, Vec<u8>) {
    let conn = pool.get().await.expect("acquire conn");
    conn.inner()
        .call(move |c| {
            Ok(c.query_row(
                "SELECT typeof(b), b FROM bytes_sqlite_blobs WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )?)
        })
        .await
        .expect("select typeof(b), b")
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 (no container needed)"]
async fn bytes_create_and_update_store_blob() {
    let Some(t) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let client = PraxClient::new(t.engine.clone());

    // Prax create path.
    let created_bytes = vec![0x00u8, 0x9F, 0xFF];
    let created = client
        .blob()
        .create()
        .set("b", created_bytes.clone())
        .exec()
        .await
        .expect("create blob");
    assert_eq!(created.b, created_bytes);
    assert_eq!(
        raw_typeof_and_bytes(&t.pool, created.id).await,
        ("blob".to_string(), created_bytes)
    );

    // BytesFieldUpdate through the update-input IR.
    let updated_bytes = vec![0xFFu8, 0x00, 0x80, 0x01];
    let v = blob::BlobUpdateInput {
        b: Some(BytesFieldUpdate {
            set: Some(updated_bytes.clone()),
        }),
    }
    .into_ir()
    .into_iter()
    .find_map(|(c, op)| match (c.as_str(), op) {
        ("b", WriteOp::Set(v)) => Some(v),
        _ => None,
    })
    .expect("b set op");
    let n = t
        .engine
        .execute_raw(
            "UPDATE bytes_sqlite_blobs SET b = ? WHERE id = ?",
            vec![v, FilterValue::Int(created.id as i64)],
        )
        .await
        .expect("update");
    assert_eq!(n, 1);
    assert_eq!(
        raw_typeof_and_bytes(&t.pool, created.id).await,
        ("blob".to_string(), updated_bytes)
    );
}
