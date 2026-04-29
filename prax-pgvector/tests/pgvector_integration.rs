//! Integration tests for prax-pgvector against a real PostgreSQL + pgvector database.
//!
//! These tests require a running PostgreSQL instance with pgvector installed.
//! Set `DATABASE_URL` to the connection string, e.g.:
//!
//! ```sh
//! export DATABASE_URL="host=localhost port=5434 user=postgres password=testpass dbname=prax_vector_test"
//! ```
//!
//! Run with:
//!
//! ```sh
//! cargo test -p prax-pgvector --test pgvector_integration -- --ignored
//! ```

use std::env;

use tokio_postgres::{Client, NoTls};

use prax_pgvector::filter::{VectorFilter, VectorOrderBy};
use prax_pgvector::index::{HnswConfig, IvfFlatConfig, VectorIndex, extension};
use prax_pgvector::ops::{
    SearchParams, distance_param_sql, nearest_neighbor_sql, radius_search_sql,
};
use prax_pgvector::query::{HybridSearchBuilder, VectorSearchBuilder};
use prax_pgvector::{BinaryVector, DistanceMetric, Embedding, SparseEmbedding};

/// Get the database connection string, returning `None` when the
/// dedicated pgvector test container isn't available.
///
/// These tests target a different postgres instance from the main
/// CI/e2e suite — one that has the `embeddings`/`documents`/
/// `binary_features` schema pre-created by the docker-compose
/// `pgvector-test` service. The GitHub Actions CI job uses
/// `pgvector/pgvector:pg16` as its normal test postgres (on port
/// 5432) but never creates that schema, so we gate on `DATABASE_URL`
/// being explicitly set: the compose runner exports it, CI does not.
fn database_url() -> Option<String> {
    env::var("DATABASE_URL").ok()
}

/// Connect to the test database, or return `None` if e2e is disabled.
async fn connect() -> Option<Client> {
    let url = database_url()?;
    let (client, connection) = tokio_postgres::connect(&url, NoTls)
        .await
        .expect("failed to connect to pgvector test database");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });

    Some(client)
}

// =============================================================================
// Extension Management
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_extension_is_installed() {
    let Some(client) = connect().await else { return; };

    let row = client
        .query_one(extension::check_extension_sql(), &[])
        .await
        .expect("failed to check extension");

    let exists: bool = row.get(0);
    assert!(exists, "pgvector extension should be installed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_extension_version() {
    let Some(client) = connect().await else { return; };

    let row = client
        .query_one(extension::version_sql(), &[])
        .await
        .expect("failed to get extension version");

    let version: String = row.get(0);
    assert!(!version.is_empty(), "pgvector version should not be empty");
    // pgvector versions are semver-like (e.g., "0.8.1")
    assert!(
        version.contains('.'),
        "version should contain dots: {version}"
    );
}

// =============================================================================
// Vector Type Integration (Insert + Retrieve)
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_insert_and_retrieve_dense_vector() {
    let Some(client) = connect().await else { return; };

    let embedding = Embedding::new(vec![0.5, 0.5, 0.5]);
    let pgvec: pgvector::Vector = embedding.clone().into();

    // Insert using pgvector type with ToSql
    client
        .execute(
            "INSERT INTO embeddings (content, embedding) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&"test_vector", &pgvec],
        )
        .await
        .expect("failed to insert vector");

    // Retrieve and verify
    let row = client
        .query_one(
            "SELECT embedding FROM embeddings WHERE content = 'test_vector' LIMIT 1",
            &[],
        )
        .await
        .expect("failed to retrieve vector");

    let retrieved: pgvector::Vector = row.get(0);
    let retrieved_embedding = Embedding::from(retrieved);

    assert_eq!(retrieved_embedding.len(), 3);
    assert_eq!(retrieved_embedding.as_slice(), embedding.as_slice());

    // Clean up
    client
        .execute("DELETE FROM embeddings WHERE content = 'test_vector'", &[])
        .await
        .expect("cleanup failed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_insert_and_retrieve_sparse_vector() {
    let Some(client) = connect().await else { return; };

    let sparse = SparseEmbedding::from_dense(vec![1.0, 0.0, 2.0, 0.0]);
    let pgvec: pgvector::SparseVector = sparse.clone().into();

    // Insert
    client
        .execute(
            "INSERT INTO documents (title, body, sparse_embedding) VALUES ($1, $2, $3)",
            &[&"sparse_test", &"test body", &pgvec],
        )
        .await
        .expect("failed to insert sparse vector");

    // Retrieve
    let row = client
        .query_one(
            "SELECT sparse_embedding FROM documents WHERE title = 'sparse_test' LIMIT 1",
            &[],
        )
        .await
        .expect("failed to retrieve sparse vector");

    let retrieved: pgvector::SparseVector = row.get(0);
    let retrieved_sparse = SparseEmbedding::from(retrieved);

    assert_eq!(retrieved_sparse.dimensions(), sparse.dimensions());
    assert_eq!(retrieved_sparse.to_dense(), sparse.to_dense());

    // Clean up
    client
        .execute("DELETE FROM documents WHERE title = 'sparse_test'", &[])
        .await
        .expect("cleanup failed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_insert_and_retrieve_binary_vector() {
    let Some(client) = connect().await else { return; };

    let bv = BinaryVector::from_bools(&[true, false, true, false, true, false, true, false]);
    let pgbit: pgvector::Bit = bv.clone().into();

    // Insert
    client
        .execute(
            "INSERT INTO binary_features (name, features) VALUES ($1, $2)",
            &[&"binary_test", &pgbit],
        )
        .await
        .expect("failed to insert binary vector");

    // Retrieve
    let row = client
        .query_one(
            "SELECT features FROM binary_features WHERE name = 'binary_test' LIMIT 1",
            &[],
        )
        .await
        .expect("failed to retrieve binary vector");

    let retrieved: pgvector::Bit = row.get(0);
    let retrieved_bv = BinaryVector::from(retrieved);

    assert_eq!(retrieved_bv.len(), bv.len());
    assert_eq!(retrieved_bv.as_bytes(), bv.as_bytes());

    // Clean up
    client
        .execute(
            "DELETE FROM binary_features WHERE name = 'binary_test'",
            &[],
        )
        .await
        .expect("cleanup failed");
}

// =============================================================================
// Nearest Neighbor Search (L2 Distance)
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_nearest_neighbor_l2() {
    let Some(client) = connect().await else { return; };

    // Query for vectors near [1.0, 0.0, 0.0] — should match "cat" first
    let query_vec = pgvector::Vector::from(vec![1.0, 0.0, 0.0]);

    let rows = client
        .query(
            "SELECT content, embedding <-> $1 AS distance FROM embeddings ORDER BY distance LIMIT 3",
            &[&query_vec],
        )
        .await
        .expect("failed to execute nearest neighbor query");

    assert!(!rows.is_empty(), "should return results");
    assert!(rows.len() <= 3, "should return at most 3 results");

    // First result should be "cat" (exact match to [1.0, 0.0, 0.0])
    let first_content: String = rows[0].get("content");
    let first_distance: f64 = rows[0].get("distance");

    assert_eq!(first_content, "cat");
    assert!(
        first_distance < 0.001,
        "cat should have near-zero distance, got {first_distance}"
    );

    // Second result should be "dog" (closest after cat)
    let second_content: String = rows[1].get("content");
    assert_eq!(second_content, "dog");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_nearest_neighbor_cosine() {
    let Some(client) = connect().await else { return; };

    let query_vec = pgvector::Vector::from(vec![0.1, 0.2, 0.3, 0.4]);

    let rows = client
        .query(
            "SELECT title, embedding <=> $1 AS distance FROM documents ORDER BY distance LIMIT 3",
            &[&query_vec],
        )
        .await
        .expect("failed to execute cosine search");

    assert!(!rows.is_empty());

    // First result should be "Introduction to AI" (identical direction)
    let first_title: String = rows[0].get("title");
    let first_distance: f64 = rows[0].get("distance");

    assert_eq!(first_title, "Introduction to AI");
    assert!(
        first_distance < 1e-6,
        "identical direction should have ~0 cosine distance, got {first_distance}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_nearest_neighbor_inner_product() {
    let Some(client) = connect().await else { return; };

    let query_vec = pgvector::Vector::from(vec![0.5, 0.6, 0.7, 0.8]);

    let rows = client
        .query(
            "SELECT title, embedding <#> $1 AS neg_inner_product FROM documents ORDER BY neg_inner_product LIMIT 3",
            &[&query_vec],
        )
        .await
        .expect("failed to execute inner product search");

    assert!(!rows.is_empty());

    // "Machine Learning Basics" has embedding [0.5,0.6,0.7,0.8] — identical, highest IP
    let first_title: String = rows[0].get("title");
    assert_eq!(first_title, "Machine Learning Basics");
}

// =============================================================================
// Radius Search
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_radius_search() {
    let Some(client) = connect().await else { return; };

    let query_vec = pgvector::Vector::from(vec![1.0, 0.0, 0.0]);

    // Find all embeddings within L2 distance of 0.5
    let rows = client
        .query(
            "SELECT content, embedding <-> $1 AS distance FROM embeddings WHERE embedding <-> $1 < 0.5 ORDER BY distance",
            &[&query_vec],
        )
        .await
        .expect("failed to execute radius search");

    // Should include "cat" (distance 0) and possibly "dog" (~0.14) and "hamster" (~0.24)
    assert!(!rows.is_empty());

    for row in &rows {
        let distance: f64 = row.get("distance");
        assert!(
            distance < 0.5,
            "all results should be within radius, got {distance}"
        );
    }

    // "cat" must be in results
    let contents: Vec<String> = rows.iter().map(|r| r.get("content")).collect();
    assert!(contents.contains(&"cat".to_string()));
}

// =============================================================================
// Generated SQL Execution
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_ops_nearest_neighbor_sql_execution() {
    let Some(client) = connect().await else { return; };

    let sql = nearest_neighbor_sql("embeddings", "embedding", DistanceMetric::L2, 1, 3, &[]);
    let query_vec = pgvector::Vector::from(vec![1.0, 0.0, 0.0]);

    let rows = client
        .query(&sql, &[&query_vec])
        .await
        .expect("generated nearest_neighbor_sql should be valid");

    assert_eq!(rows.len(), 3);
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_ops_radius_search_sql_execution() {
    let Some(client) = connect().await else { return; };

    let sql = radius_search_sql(
        "embeddings",
        "embedding",
        DistanceMetric::L2,
        1,
        0.5,
        Some(10),
    );
    let query_vec = pgvector::Vector::from(vec![1.0, 0.0, 0.0]);

    let rows = client
        .query(&sql, &[&query_vec])
        .await
        .expect("generated radius_search_sql should be valid");

    for row in &rows {
        let distance: f64 = row.get("distance");
        assert!(distance < 0.5);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_distance_param_sql_in_query() {
    let Some(client) = connect().await else { return; };

    let distance_expr = distance_param_sql("embedding", "$1", DistanceMetric::Cosine);
    let sql = format!(
        "SELECT content, {distance_expr} AS distance FROM embeddings ORDER BY distance LIMIT 2"
    );

    let query_vec = pgvector::Vector::from(vec![1.0, 0.0, 0.0]);

    let rows = client
        .query(&sql, &[&query_vec])
        .await
        .expect("distance_param_sql should produce valid SQL");

    assert_eq!(rows.len(), 2);

    let first_distance: f64 = rows[0].get("distance");
    let second_distance: f64 = rows[1].get("distance");
    assert!(
        first_distance <= second_distance,
        "results should be ordered by distance"
    );
}

// =============================================================================
// Search Parameters (SET commands)
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_search_params_set_commands() {
    let Some(client) = connect().await else { return; };

    let params = SearchParams::new().ef_search(100).probes(5);

    for stmt in params.to_set_sql() {
        client
            .execute(&stmt, &[])
            .await
            .unwrap_or_else(|e| panic!("SET command failed: {stmt} — {e}"));
    }

    // Verify the setting took effect
    let row = client
        .query_one("SHOW hnsw.ef_search", &[])
        .await
        .expect("failed to SHOW hnsw.ef_search");
    let val: String = row.get(0);
    assert_eq!(val, "100");

    let row = client
        .query_one("SHOW ivfflat.probes", &[])
        .await
        .expect("failed to SHOW ivfflat.probes");
    let val: String = row.get(0);
    assert_eq!(val, "5");
}

// =============================================================================
// VectorSearchBuilder Query Execution
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_basic() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let search = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .limit(3)
        .build();

    let sql = search.to_sql();
    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("VectorSearchBuilder query should execute");

    assert_eq!(rows.len(), 3);

    // Distance should be included
    let first_distance: f64 = rows[0].get("distance");
    assert!(first_distance >= 0.0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_with_select() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let search = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .select(&["id", "content"])
        .limit(2)
        .build();

    let sql = search.to_sql();
    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("query with specific columns should execute");

    assert_eq!(rows.len(), 2);

    // Verify we can access the selected columns
    let _id: i32 = rows[0].get("id");
    let _content: String = rows[0].get("content");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_with_where() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let search = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .where_clause("content != 'cat'")
        .limit(5)
        .build();

    let sql = search.to_sql();
    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("query with WHERE should execute");

    // "cat" should NOT be in results
    for row in &rows {
        let content: String = row.get("content");
        assert_ne!(content, "cat", "cat should be excluded by WHERE clause");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_max_distance() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let search = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .max_distance(0.3)
        .limit(10)
        .build();

    let sql = search.to_sql();
    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("max_distance query should execute");

    for row in &rows {
        let distance: f64 = row.get("distance");
        assert!(
            distance < 0.3,
            "all results should be within max_distance, got {distance}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_pagination() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);

    // Get first page
    let search_page1 = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .limit(2)
        .offset(0)
        .build();

    // Get second page
    let search_page2 = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .limit(2)
        .offset(2)
        .build();

    let pgvec: pgvector::Vector = query_embedding.into();

    let page1 = client
        .query(&search_page1.to_sql(), &[&pgvec])
        .await
        .expect("page 1 should execute");

    let page2 = client
        .query(&search_page2.to_sql(), &[&pgvec])
        .await
        .expect("page 2 should execute");

    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);

    // Pages should not overlap
    let page1_ids: Vec<i32> = page1.iter().map(|r| r.get("id")).collect();
    let page2_ids: Vec<i32> = page2.iter().map(|r| r.get("id")).collect();

    for id in &page2_ids {
        assert!(
            !page1_ids.contains(id),
            "pages should not overlap: id {id} in both"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_all_metrics() {
    let Some(client) = connect().await else { return; };

    let metrics = [
        DistanceMetric::L2,
        DistanceMetric::Cosine,
        DistanceMetric::InnerProduct,
        DistanceMetric::L1,
    ];

    for metric in metrics {
        let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
        let search = VectorSearchBuilder::new("embeddings", "embedding")
            .query(query_embedding.clone())
            .metric(metric)
            .limit(3)
            .build();

        let pgvec: pgvector::Vector = query_embedding.into();

        let rows = client
            .query(&search.to_sql(), &[&pgvec])
            .await
            .unwrap_or_else(|e| panic!("query with {metric} should execute: {e}"));

        assert_eq!(
            rows.len(),
            3,
            "should return 3 results with {metric} metric"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_search_builder_with_ef_search() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let search = VectorSearchBuilder::new("embeddings", "embedding")
        .query(query_embedding.clone())
        .metric(DistanceMetric::L2)
        .ef_search(200)
        .limit(3)
        .build();

    // Apply SET commands first
    for stmt in search.param_set_sql() {
        client.execute(&stmt, &[]).await.expect("SET should work");
    }

    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&search.to_sql(), &[&pgvec])
        .await
        .expect("query with ef_search should execute");

    assert_eq!(rows.len(), 3);
}

// =============================================================================
// VectorFilter Query Execution
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_filter_nearest_execution() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![0.0, 1.0, 0.0]);
    let filter = VectorFilter::nearest("embedding", query_embedding.clone(), DistanceMetric::L2, 2);
    let sql = filter.to_select_sql("embeddings", 1, None, "*");

    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("VectorFilter::nearest query should execute");

    assert_eq!(rows.len(), 2);

    // First result should be "fish" (exact match to [0.0, 1.0, 0.0])
    let first_content: String = rows[0].get("content");
    assert_eq!(first_content, "fish");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_filter_within_distance_execution() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let filter = VectorFilter::within_distance(
        "embedding",
        query_embedding.clone(),
        DistanceMetric::L2,
        0.5,
    )
    .with_limit(10);

    let sql = filter.to_select_sql("embeddings", 1, None, "*");
    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("VectorFilter::within_distance query should execute");

    for row in &rows {
        let distance: f64 = row.get("distance");
        assert!(distance < 0.5);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_filter_with_extra_where() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![1.0, 0.0, 0.0]);
    let filter = VectorFilter::nearest("embedding", query_embedding.clone(), DistanceMetric::L2, 5);
    let sql = filter.to_select_sql("embeddings", 1, Some("content != 'bird'"), "*");

    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("filter with extra WHERE should execute");

    for row in &rows {
        let content: String = row.get("content");
        assert_ne!(content, "bird");
    }
}

// =============================================================================
// VectorOrderBy Execution
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_vector_order_by_execution() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![0.0, 0.0, 1.0]);
    let order = VectorOrderBy::new("embedding", query_embedding.clone(), DistanceMetric::L2);

    let distance_select = order.select_distance_sql(1).unwrap();
    let order_by = order.order_by_sql(1);

    let sql = format!("SELECT *, {distance_select} FROM embeddings ORDER BY {order_by} LIMIT 3");

    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec])
        .await
        .expect("VectorOrderBy query should execute");

    assert_eq!(rows.len(), 3);

    // First should be "bird" (exact match to [0.0, 0.0, 1.0])
    let first_content: String = rows[0].get("content");
    assert_eq!(first_content, "bird");

    // Distances should be non-decreasing
    let distances: Vec<f64> = rows.iter().map(|r| r.get("distance")).collect();
    for window in distances.windows(2) {
        assert!(
            window[0] <= window[1],
            "distances should be non-decreasing: {} > {}",
            window[0],
            window[1]
        );
    }
}

// =============================================================================
// Index Management
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_create_and_drop_hnsw_index() {
    let Some(client) = connect().await else { return; };

    let index = VectorIndex::hnsw("idx_test_hnsw_integration", "embeddings", "embedding")
        .metric(DistanceMetric::Cosine)
        .config(HnswConfig::new().m(8).ef_construction(32))
        .if_not_exists()
        .build()
        .expect("should build index");

    // Drop if it already exists (from a previous test run)
    let _ = client.execute(&index.to_drop_sql(), &[]).await;

    // Create the index
    client
        .execute(&index.to_create_sql(), &[])
        .await
        .expect("should create HNSW index");

    // Verify it exists
    let row = client
        .query_one(&index.to_exists_sql(), &[])
        .await
        .expect("should check index exists");
    let exists: bool = row.get(0);
    assert!(exists, "HNSW index should exist after creation");

    // Get index size
    let row = client
        .query_one(&index.to_size_sql(), &[])
        .await
        .expect("should get index size");
    let size: String = row.get(0);
    assert!(!size.is_empty(), "index size should be non-empty");

    // Drop the index
    client
        .execute(&index.to_drop_sql(), &[])
        .await
        .expect("should drop HNSW index");

    // Verify it's gone
    let row = client
        .query_one(&index.to_exists_sql(), &[])
        .await
        .expect("should check index not exists");
    let exists: bool = row.get(0);
    assert!(!exists, "HNSW index should not exist after drop");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_create_and_drop_ivfflat_index() {
    let Some(client) = connect().await else { return; };

    let index = VectorIndex::ivfflat("idx_test_ivfflat_integration", "embeddings", "embedding")
        .metric(DistanceMetric::L2)
        .ivfflat_config(IvfFlatConfig::new(2))
        .if_not_exists()
        .build()
        .expect("should build index");

    // Drop if exists from previous run
    let _ = client.execute(&index.to_drop_sql(), &[]).await;

    // Create
    client
        .execute(&index.to_create_sql(), &[])
        .await
        .expect("should create IVFFlat index");

    // Verify
    let row = client
        .query_one(&index.to_exists_sql(), &[])
        .await
        .expect("should check index exists");
    let exists: bool = row.get(0);
    assert!(exists, "IVFFlat index should exist");

    // Drop
    client
        .execute(&index.to_drop_sql(), &[])
        .await
        .expect("should drop IVFFlat index");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_index_with_all_metrics() {
    let Some(client) = connect().await else { return; };

    let metrics = [
        DistanceMetric::L2,
        DistanceMetric::Cosine,
        DistanceMetric::InnerProduct,
        DistanceMetric::L1,
    ];

    for (i, metric) in metrics.iter().enumerate() {
        let name = format!("idx_test_metric_{i}");
        let index = VectorIndex::hnsw(&name, "embeddings", "embedding")
            .metric(*metric)
            .if_not_exists()
            .build()
            .expect("should build index");

        // Drop if exists
        let _ = client.execute(&index.to_drop_sql(), &[]).await;

        // Create — should succeed with all metrics
        client
            .execute(&index.to_create_sql(), &[])
            .await
            .unwrap_or_else(|e| {
                panic!("should create index with {metric} metric: {e}");
            });

        // Clean up
        let _ = client.execute(&index.to_drop_sql(), &[]).await;
    }
}

// =============================================================================
// Extension DDL
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_add_vector_column() {
    let Some(client) = connect().await else { return; };

    // Create a temporary table
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS _test_ddl_table (id SERIAL PRIMARY KEY)",
            &[],
        )
        .await
        .expect("should create temp table");

    // Add a vector column using our SQL generator
    let sql = extension::add_vector_column_sql("_test_ddl_table", "test_embedding", 128);
    client
        .execute(&sql, &[])
        .await
        .expect("should add vector column");

    // Verify the column exists
    let row = client
        .query_one(
            "SELECT column_name, udt_name FROM information_schema.columns WHERE table_name = '_test_ddl_table' AND column_name = 'test_embedding'",
            &[],
        )
        .await
        .expect("should find the column");

    let col_name: String = row.get("column_name");
    assert_eq!(col_name, "test_embedding");

    // Clean up
    client
        .execute("DROP TABLE IF EXISTS _test_ddl_table", &[])
        .await
        .expect("cleanup failed");
}

// =============================================================================
// Hybrid Search (Vector + Full-Text)
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_hybrid_search_execution() {
    let Some(client) = connect().await else { return; };

    let query_embedding = Embedding::new(vec![0.1, 0.2, 0.3, 0.4]);
    let search = HybridSearchBuilder::new("documents")
        .vector_column("embedding")
        .text_column("body")
        .query_vector(query_embedding.clone())
        .query_text("learning")
        .metric(DistanceMetric::Cosine)
        .vector_weight(0.7)
        .text_weight(0.3)
        .limit(3)
        .build();

    let sql = search.to_sql();
    let pgvec: pgvector::Vector = query_embedding.into();

    let rows = client
        .query(&sql, &[&pgvec, &"learning"])
        .await
        .expect("hybrid search should execute");

    assert!(
        !rows.is_empty(),
        "hybrid search should return results for 'learning'"
    );
}

// =============================================================================
// Edge Cases & Error Handling
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_dimension_mismatch_error() {
    let Some(client) = connect().await else { return; };

    // embeddings table has vector(3) — inserting a 4-dim vector should fail
    let wrong_dim = pgvector::Vector::from(vec![1.0, 2.0, 3.0, 4.0]);

    let result = client
        .execute(
            "INSERT INTO embeddings (content, embedding) VALUES ($1, $2)",
            &[&"wrong_dim", &wrong_dim],
        )
        .await;

    assert!(
        result.is_err(),
        "inserting wrong dimension vector should fail"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_null_vector_handling() {
    let Some(client) = connect().await else { return; };

    // Insert a document with NULL embedding
    client
        .execute(
            "INSERT INTO documents (title, body) VALUES ($1, $2)",
            &[&"null_vec_test", &"no embedding here"],
        )
        .await
        .expect("should insert row with NULL vector");

    // Querying should still work — NULL vectors are just excluded from distance calc
    let query_vec = pgvector::Vector::from(vec![0.1, 0.2, 0.3, 0.4]);

    let rows = client
        .query(
            "SELECT title, embedding <=> $1 AS distance FROM documents WHERE embedding IS NOT NULL ORDER BY distance LIMIT 5",
            &[&query_vec],
        )
        .await
        .expect("query with NULL embeddings should work");

    // "null_vec_test" should NOT be in results
    for row in &rows {
        let title: String = row.get("title");
        assert_ne!(title, "null_vec_test");
    }

    // Clean up
    client
        .execute("DELETE FROM documents WHERE title = 'null_vec_test'", &[])
        .await
        .expect("cleanup failed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_empty_result_set() {
    let Some(client) = connect().await else { return; };

    // Use a very restrictive radius that no vector can match
    let query_vec = pgvector::Vector::from(vec![1.0, 0.0, 0.0]);

    let rows = client
        .query(
            "SELECT content FROM embeddings WHERE embedding <-> $1 < 0.00001 AND content = 'nonexistent'",
            &[&query_vec],
        )
        .await
        .expect("query should execute even with no results");

    assert!(rows.is_empty(), "should return empty result set");
}

// =============================================================================
// Embedding Math Verified Against Database
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_local_l2_distance_matches_postgres() {
    let Some(client) = connect().await else { return; };

    let a = Embedding::new(vec![1.0, 2.0, 3.0]);
    let b = Embedding::new(vec![4.0, 5.0, 6.0]);

    // Compute locally
    let local_distance = a.l2_distance(&b).expect("should compute L2 distance");

    // Compute in PostgreSQL
    let vec_a = pgvector::Vector::from(a.to_vec());
    let vec_b = pgvector::Vector::from(b.to_vec());

    let row = client
        .query_one(
            "SELECT $1::vector <-> $2::vector AS dist",
            &[&vec_a, &vec_b],
        )
        .await
        .expect("should compute distance in PG");

    let pg_distance: f64 = row.get("dist");

    assert!(
        (local_distance as f64 - pg_distance).abs() < 1e-4,
        "local ({local_distance}) vs postgres ({pg_distance}) L2 distance should match"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_local_cosine_distance_matches_postgres() {
    let Some(client) = connect().await else { return; };

    let a = Embedding::new(vec![1.0, 2.0, 3.0]);
    let b = Embedding::new(vec![4.0, 5.0, 6.0]);

    // Cosine similarity locally
    let local_similarity = a.cosine_similarity(&b).expect("should compute cosine");
    let local_distance = 1.0 - local_similarity; // cosine distance = 1 - similarity

    // Cosine distance in PostgreSQL (operator <=> returns 1 - cosine_similarity)
    let vec_a = pgvector::Vector::from(a.to_vec());
    let vec_b = pgvector::Vector::from(b.to_vec());

    let row = client
        .query_one(
            "SELECT $1::vector <=> $2::vector AS dist",
            &[&vec_a, &vec_b],
        )
        .await
        .expect("should compute cosine distance in PG");

    let pg_distance: f64 = row.get("dist");

    assert!(
        (local_distance as f64 - pg_distance).abs() < 1e-4,
        "local ({local_distance}) vs postgres ({pg_distance}) cosine distance should match"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_local_inner_product_matches_postgres() {
    let Some(client) = connect().await else { return; };

    let a = Embedding::new(vec![1.0, 2.0, 3.0]);
    let b = Embedding::new(vec![4.0, 5.0, 6.0]);

    // Inner product locally
    let local_ip = a.dot_product(&b).expect("should compute dot product");

    // pgvector <#> returns *negative* inner product
    let vec_a = pgvector::Vector::from(a.to_vec());
    let vec_b = pgvector::Vector::from(b.to_vec());

    let row = client
        .query_one(
            "SELECT $1::vector <#> $2::vector AS neg_ip",
            &[&vec_a, &vec_b],
        )
        .await
        .expect("should compute inner product in PG");

    let pg_neg_ip: f64 = row.get("neg_ip");

    assert!(
        (local_ip as f64 + pg_neg_ip).abs() < 1e-4,
        "local IP ({local_ip}) should equal -pg_neg_ip ({pg_neg_ip})"
    );
}

// =============================================================================
// Batch Operations
// =============================================================================

#[tokio::test]
#[ignore = "requires PostgreSQL with pgvector"]
async fn test_batch_insert_and_search() {
    let Some(client) = connect().await else { return; };

    // Create a temporary table
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS _test_batch (id SERIAL PRIMARY KEY, label TEXT, embedding vector(4))",
            &[],
        )
        .await
        .expect("should create batch table");

    // Batch insert 100 vectors
    let stmt = client
        .prepare("INSERT INTO _test_batch (label, embedding) VALUES ($1, $2)")
        .await
        .expect("should prepare statement");

    for i in 0..100 {
        let label = format!("item_{i}");
        let vec = pgvector::Vector::from(vec![
            (i as f32) / 100.0,
            ((i * 2) as f32 % 100.0) / 100.0,
            ((i * 3) as f32 % 100.0) / 100.0,
            ((i * 7) as f32 % 100.0) / 100.0,
        ]);
        client
            .execute(&stmt, &[&label, &vec])
            .await
            .expect("should insert batch item");
    }

    // Create HNSW index
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_test_batch_hnsw ON _test_batch USING hnsw (embedding vector_cosine_ops)",
            &[],
        )
        .await
        .expect("should create index on batch table");

    // Search
    let query = pgvector::Vector::from(vec![0.5, 0.5, 0.5, 0.5]);
    let rows = client
        .query(
            "SELECT label, embedding <=> $1 AS distance FROM _test_batch ORDER BY distance LIMIT 5",
            &[&query],
        )
        .await
        .expect("should search batch table");

    assert_eq!(rows.len(), 5);

    // Verify ordering
    let distances: Vec<f64> = rows.iter().map(|r| r.get("distance")).collect();
    for window in distances.windows(2) {
        assert!(window[0] <= window[1], "results should be ordered");
    }

    // Clean up
    client
        .execute("DROP TABLE IF EXISTS _test_batch", &[])
        .await
        .expect("cleanup failed");
}
