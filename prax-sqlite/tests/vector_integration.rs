//! Integration tests for the `vector` feature against the real
//! sqlite-vector-rs loadable extension.
//!
//! The extension is a cdylib loaded at runtime, so these tests need it
//! built and named by `SQLITE_VECTOR_RS_LIB` (CI's "SQLite vector
//! extension" job does both). Without it they skip, unless
//! `PRAX_VECTOR_E2E=1` is set, in which case a missing library fails the
//! test instead of silently skipping it.
//!
//! ```sh
//! git clone --branch v0.3.1 https://github.com/quinnjr/sqlite-vector-rs
//! (cd sqlite-vector-rs && cargo build --release --lib)
//! SQLITE_VECTOR_RS_LIB=$PWD/sqlite-vector-rs/target/release/libsqlite_vector_rs.so \
//!   PRAX_VECTOR_E2E=1 cargo test -p prax-sqlite --features vector --test vector_integration
//! ```

#![cfg(feature = "vector")]

use prax_sqlite::vector::prelude::*;
use prax_sqlite::{SqliteConfig, SqliteConnection, SqlitePool};

/// Whether the extension library is configured. Panics when it isn't but
/// `PRAX_VECTOR_E2E=1` asks for the tests to run.
fn vector_lib_available() -> bool {
    let available = std::env::var_os("SQLITE_VECTOR_RS_LIB").is_some();
    if !available && std::env::var("PRAX_VECTOR_E2E").as_deref() == Ok("1") {
        panic!("PRAX_VECTOR_E2E=1 but SQLITE_VECTOR_RS_LIB is not set");
    }
    if !available {
        eprintln!("skipping: SQLITE_VECTOR_RS_LIB not set");
    }
    available
}

async fn make_pool() -> SqlitePool {
    SqlitePool::new(SqliteConfig::memory()).await.unwrap()
}

/// `(id, distance)` for each row of a vector search query.
async fn id_distance_rows(conn: &SqliteConnection, sql: String) -> Vec<(i64, f64)> {
    conn.inner()
        .call(move |c| {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>("id")?, row.get::<_, f64>("distance")?))
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
}

/// Main table `documents` plus a 4-dim cosine vector table built with
/// [`VectorIndex`]; three documents linked by rowid.
async fn seed_documents(conn: &SqliteConnection, index: Option<VectorIndexKind>) {
    let ddl = VectorIndex::new(
        vector_table_name("documents", "embedding"),
        VectorElementType::Float4,
        4,
        DistanceMetric::Cosine,
    )
    .index(index)
    .to_create_sql();
    conn.execute_batch(&format!(
        "CREATE TABLE documents (id INTEGER PRIMARY KEY, title TEXT NOT NULL);\n{ddl}"
    ))
    .await
    .unwrap();

    conn.execute_batch(
        "INSERT INTO documents (id, title) VALUES (10, 'cat'), (20, 'car'), (30, 'lion');\n\
         INSERT INTO documents_embedding_vectors (rowid, vector) VALUES \
         (10, vector_from_json('[1.0, 0.0, 0.0, 0.0]', 'float4')), \
         (20, vector_from_json('[0.0, 1.0, 0.0, 0.0]', 'float4')), \
         (30, vector_from_json('[0.9, 0.1, 0.0, 0.0]', 'float4'));",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_pool_creation_succeeds_without_extension() {
    // Registration soft-fails, so a pool works with or without the library.
    let pool = make_pool().await;
    assert!(pool.available_permits() > 0);
    let _conn = pool.get().await.unwrap();
}

#[tokio::test]
async fn test_pool_autoregisters_vector_extension() {
    if !vector_lib_available() {
        return;
    }
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();
    let dims: i64 = conn
        .inner()
        .call(|c| {
            c.query_row(
                "SELECT vector_dims(vector_from_json('[1.0, 2.0, 3.0]', 'float4'), 'float4')",
                [],
                |row| row.get(0),
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)
        })
        .await
        .unwrap();
    assert_eq!(dims, 3);
}

#[tokio::test]
async fn test_vector_index_ddl_is_accepted_for_every_mode_and_metric() {
    if !vector_lib_available() {
        return;
    }
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();
    for (name, index) in [("t_hnsw", Some(VectorIndexKind::Hnsw)), ("t_exact", None)] {
        for metric in [
            DistanceMetric::Cosine,
            DistanceMetric::L2,
            DistanceMetric::InnerProduct,
        ] {
            let idx = VectorIndex::new(
                format!("{name}_{}", metric.as_sql()),
                VectorElementType::Float4,
                8,
                metric,
            )
            .index(index);
            conn.execute_batch(&idx.to_create_sql()).await.unwrap();
            conn.execute_batch(&idx.to_drop_sql()).await.unwrap();
        }
    }
}

#[tokio::test]
async fn test_topk_search_with_builder_hnsw() {
    if !vector_lib_available() {
        return;
    }
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();
    seed_documents(&conn, Some(VectorIndexKind::Hnsw)).await;

    let query = Embedding::new(vec![1.0, 0.0, 0.0, 0.0]).unwrap();
    let sql = VectorSearchBuilder::new("documents", "embedding")
        .query_embedding(&query)
        .limit(2)
        .to_sql()
        .unwrap();
    let rows = id_distance_rows(&conn, sql).await;

    let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
    assert_eq!(ids, vec![10, 30], "nearest two, closest first: {rows:?}");
    assert!(rows[0].1 <= rows[1].1);
    assert!(rows[0].1.abs() < 1e-6, "exact match has ~0 cosine distance");
}

#[tokio::test]
async fn test_topk_search_with_builder_exact_mode() {
    if !vector_lib_available() {
        return;
    }
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();
    seed_documents(&conn, None).await;

    let sql = VectorSearchBuilder::new("documents", "embedding")
        .query_json("[0.0, 1.0, 0.0, 0.0]")
        .limit(1)
        .to_sql()
        .unwrap();
    let rows = id_distance_rows(&conn, sql).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, 20);
}

#[tokio::test]
async fn test_hybrid_search_with_fts5() {
    if !vector_lib_available() {
        return;
    }
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();
    seed_documents(&conn, Some(VectorIndexKind::Hnsw)).await;
    conn.execute_batch(
        "CREATE VIRTUAL TABLE documents_fts USING fts5(title);\n\
         INSERT INTO documents_fts (rowid, title) SELECT id, title FROM documents;",
    )
    .await
    .unwrap();

    // Vector side (2 candidates) ranks 10 then 30; the text side matches
    // only 20 ('car').
    let sql = HybridSearchBuilder::new("documents")
        .vector_table(vector_table_name("documents", "embedding"))
        .fts_table("documents_fts")
        .query_vector_json("[1.0, 0.0, 0.0, 0.0]")
        .query_text("car")
        .candidates(2)
        .to_sql()
        .unwrap();
    let ids: Vec<i64> = conn
        .inner()
        .call(move |c| {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, i64>("id"))?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            Ok(rows)
        })
        .await
        .unwrap();

    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![10, 20, 30], "union of both sides: {ids:?}");
    // 10 (vector rank 1) and 20 (text rank 1) tie on score and both
    // outrank 30 (vector rank 2 only).
    assert_eq!(ids[2], 30, "{ids:?}");
}
