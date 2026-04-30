//! Example: minimal RAG pipeline with prax-sqlite vector support.
//!
//! Run with:
//!   cargo run --example vector_rag -p prax-sqlite --features vector

#![cfg(feature = "vector")]

use prax_sqlite::vector::prelude::*;
use prax_sqlite::{SqliteConfig, SqlitePool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = SqlitePool::new(SqliteConfig::memory()).await?;
    let conn = pool.get().await?;

    // 1. Create the main documents table and its vector companion.
    conn.execute_batch(
        "CREATE TABLE documents (\n    \
         id INTEGER PRIMARY KEY,\n    \
         title TEXT NOT NULL,\n    \
         content TEXT NOT NULL\n\
         );\n\
         CREATE VIRTUAL TABLE documents_vectors USING vector(\n    \
         rowid_column='document_id',\n    \
         embedding='float4[4] cosine hnsw'\n\
         );",
    )
    .await?;

    // 2. Insert a few documents and their (fake) embeddings.
    let corpus: Vec<(&str, &str, Vec<f32>)> = vec![
        (
            "cat",
            "A cat is a small furry mammal.",
            vec![1.0, 0.0, 0.0, 0.0],
        ),
        ("dog", "A dog is a loyal canine.", vec![0.9, 0.1, 0.0, 0.0]),
        (
            "car",
            "A car is a wheeled motor vehicle.",
            vec![0.0, 0.0, 1.0, 0.0],
        ),
        (
            "lion",
            "A lion is a large wild cat.",
            vec![0.95, 0.0, 0.0, 0.05],
        ),
    ];

    for (i, (title, content, emb)) in corpus.into_iter().enumerate() {
        let id = (i + 1) as i64;
        let title_owned = title.to_string();
        let content_owned = content.to_string();
        let embedding = Embedding::new(emb)?;
        let json = embedding.to_json();

        // Insert into main table
        conn.execute(&format!(
            "INSERT INTO documents (id, title, content) VALUES ({}, '{}', '{}')",
            id, title_owned, content_owned
        ))
        .await?;

        // Insert into vector table
        conn.execute(&format!(
            "INSERT INTO documents_vectors (document_id, embedding) \
             VALUES ({}, vector_from_json('{}', 'float4'))",
            id, json
        ))
        .await?;
    }

    // 3. Build a top-3 search for "cat-like".
    let query = Embedding::new(vec![1.0, 0.0, 0.0, 0.0])?;
    let search = VectorSearchBuilder::new("documents", "embedding")
        .query_embedding(&query)
        .metric(DistanceMetric::Cosine)
        .limit(3)
        .to_sql()?;

    let rows = conn.query(&search).await?;

    println!("Top matches for a cat-like query:");
    for row in rows {
        let id = row["id"].as_i64().unwrap();
        let title = row["title"].as_str().unwrap();
        let distance = row["distance"].as_f64().unwrap();
        println!("  id={} distance={:.4} title={}", id, distance, title);
    }

    Ok(())
}
