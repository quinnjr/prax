//! Integration tests against in-memory SQLite for the vector feature.
//!
//! Gated by the `vector` feature. Run with:
//! ```
//! cargo test -p prax-sqlite --features vector --test vector_integration
//! ```
//!
//! **NOTE:** Tests that actually execute vector SQL (i.e., call sqlite-vector-rs
//! functions at runtime) are marked `#[ignore]` because sqlite-vector-rs is built
//! as a loadable extension (cdylib), which is not automatically built when prax-sqlite
//! depends on it as an rlib. To run these tests, you must first build the
//! sqlite-vector-rs extension manually and ensure it's available via the
//! `SQLITE_VECTOR_RS_LIB` environment variable.
//!
//! The `test_pool_creation_succeeds_even_without_cdylib` test verifies that pool
//! creation works regardless (extension registration now soft-fails gracefully).

#![cfg(feature = "vector")]

use prax_sqlite::vector::prelude::*;
use prax_sqlite::{SqliteConfig, SqlitePool};

async fn make_pool() -> SqlitePool {
    SqlitePool::new(SqliteConfig::memory()).await.unwrap()
}

#[tokio::test]
async fn test_pool_creation_succeeds_even_without_cdylib() {
    // This test verifies that pool creation works even if the sqlite-vector-rs
    // cdylib is not available. The extension registration now soft-fails gracefully.
    let pool = make_pool().await;
    // Pool creation succeeds
    assert!(pool.available_permits() > 0);
    // Getting a connection succeeds
    let _conn = pool.get().await.unwrap();
}

#[tokio::test]
#[ignore = "requires sqlite-vector-rs cdylib to be built and available"]
async fn test_pool_autoregisters_vector_extension() {
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
            .map_err(|e| tokio_rusqlite::Error::Rusqlite(e))
        })
        .await
        .unwrap();
    assert_eq!(dims, 3);
}

#[tokio::test]
#[ignore = "requires sqlite-vector-rs cdylib to be built and available"]
async fn test_create_virtual_table_via_vector_index_builder() {
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();

    let ddl = VectorIndex::new("docs_vectors")
        .rowid_column("doc_id")
        .column(
            "embedding",
            VectorElementType::Float4,
            4,
            DistanceMetric::Cosine,
            Some(VectorIndexKind::Hnsw),
        )
        .to_create_sql();

    conn.execute_batch(&ddl).await.unwrap();
}

#[tokio::test]
#[ignore = "requires sqlite-vector-rs cdylib to be built and available"]
async fn test_insert_and_topk_search() {
    let pool = make_pool().await;
    let conn = pool.get().await.unwrap();

    conn.execute_batch(
        "CREATE TABLE documents (id INTEGER PRIMARY KEY, title TEXT NOT NULL);\n\
         CREATE VIRTUAL TABLE documents_vectors USING vector(\n\
         rowid_column='document_id',\n\
         embedding='float4[4] cosine hnsw');",
    )
    .await
    .unwrap();

    conn.execute(
        "INSERT INTO documents (id, title) VALUES (1, 'A'), (2, 'B'), (3, 'C')",
    )
    .await
    .unwrap();

    conn.execute_batch(
        "INSERT INTO documents_vectors (document_id, embedding) \
         VALUES (1, vector_from_json('[1.0, 0.0, 0.0, 0.0]', 'float4'));\n\
         INSERT INTO documents_vectors (document_id, embedding) \
         VALUES (2, vector_from_json('[0.0, 1.0, 0.0, 0.0]', 'float4'));\n\
         INSERT INTO documents_vectors (document_id, embedding) \
         VALUES (3, vector_from_json('[0.9, 0.1, 0.0, 0.0]', 'float4'));",
    )
    .await
    .unwrap();

    // Query: find the top 2 documents closest to [1,0,0,0].
    let results: Vec<(i64, f64)> = conn
        .inner()
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT documents.id, \
                 vector_distance(v.embedding, vector_from_json('[1.0, 0.0, 0.0, 0.0]', 'float4'), 'cosine', 'float4') AS distance \
                 FROM documents_vectors v \
                 JOIN documents ON documents.id = v.document_id \
                 ORDER BY distance \
                 LIMIT 2",
            )?;
            let rows = stmt
                .query_map([], |row: &rusqlite::Row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            Ok(rows)
        })
        .await
        .unwrap();

    assert_eq!(results.len(), 2);
    // Document 1 (exact match) must come first.
    assert_eq!(results[0].0, 1);
    // Document 3 (close) should come second; document 2 (orthogonal) must not be in top 2.
    assert_eq!(results[1].0, 3);
}
