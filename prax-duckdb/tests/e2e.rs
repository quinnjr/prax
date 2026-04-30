//! End-to-end tests for prax-duckdb.
//!
//! DuckDB is an embedded OLAP engine — no Docker container required. The
//! tests remain `#[ignore]` by default so the hot dev workflow stays
//! fast, and the `test-duckdb` compose service sets `PRAX_E2E=1` and
//! passes `--include-ignored`.
//!
//! Tests cover the analytical surface area DuckDB users actually care
//! about: aggregations, window functions, Parquet round-trip, and
//! JSON/CSV ingestion.

#![cfg(test)]

use prax_duckdb::{DuckDbConfig, DuckDbPool};
use prax_query::filter::FilterValue;
use tempfile::TempDir;

fn skip_unless_e2e() -> bool {
    std::env::var("PRAX_E2E").ok().as_deref() == Some("1")
}

async fn pool() -> DuckDbPool {
    // In-memory keeps tests hermetic and fast; analytical workloads that
    // care about persistence are covered by the Parquet round-trip below.
    DuckDbPool::new(DuckDbConfig::in_memory())
        .await
        .expect("create in-memory duckdb pool")
}

#[tokio::test]
#[ignore = "DuckDB E2E — run via `docker compose run --rm test-duckdb`"]
async fn e2e_crud_and_returning() {
    if !skip_unless_e2e() {
        return;
    }
    let pool = pool().await;
    let conn = pool.get().await.expect("conn");

    conn.execute_batch(
        "CREATE TABLE widgets (
            id BIGINT PRIMARY KEY,
            name VARCHAR NOT NULL,
            score INT NOT NULL
        )",
    )
    .await
    .expect("create");

    for (i, name, score) in [(1_i64, "a", 10_i32), (2, "b", 20), (3, "c", 30)] {
        conn.execute(
            "INSERT INTO widgets (id, name, score) VALUES (?, ?, ?)",
            &[
                FilterValue::Int(i),
                FilterValue::String(name.into()),
                FilterValue::Int(score.into()),
            ],
        )
        .await
        .expect("insert");
    }

    // SELECT with parameter
    let rows = conn
        .query(
            "SELECT name, score FROM widgets WHERE score >= ? ORDER BY score",
            &[FilterValue::Int(20)],
        )
        .await
        .expect("select");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], serde_json::json!("b"));
    assert_eq!(rows[1]["name"], serde_json::json!("c"));

    // UPDATE
    let n = conn
        .execute(
            "UPDATE widgets SET score = score + ? WHERE id = ?",
            &[FilterValue::Int(5), FilterValue::Int(1)],
        )
        .await
        .expect("update");
    assert_eq!(n, 1);

    let row = conn
        .query_one(
            "SELECT score FROM widgets WHERE id = ?",
            &[FilterValue::Int(1)],
        )
        .await
        .expect("query_one");
    assert_eq!(row["score"], serde_json::json!(15));

    // DELETE
    let n = conn
        .execute("DELETE FROM widgets", &[])
        .await
        .expect("delete");
    assert_eq!(n, 3);
}

#[tokio::test]
#[ignore = "DuckDB E2E — run via `docker compose run --rm test-duckdb`"]
async fn e2e_aggregations_and_window_functions() {
    if !skip_unless_e2e() {
        return;
    }
    let pool = pool().await;
    let conn = pool.get().await.expect("conn");

    // Seed with a tiny sales fact table that exercises grouping and
    // window functions — the core DuckDB use case.
    conn.execute_batch(
        "CREATE TABLE sales (
            region VARCHAR NOT NULL,
            day DATE NOT NULL,
            amount DECIMAL(10,2) NOT NULL
        );
        INSERT INTO sales VALUES
            ('north', DATE '2025-01-01', 100),
            ('north', DATE '2025-01-02', 150),
            ('north', DATE '2025-01-03', 120),
            ('south', DATE '2025-01-01',  50),
            ('south', DATE '2025-01-02',  75),
            ('south', DATE '2025-01-03', 200);",
    )
    .await
    .expect("seed");

    // Aggregation
    let rows = conn
        .query(
            "SELECT region, SUM(amount) AS total FROM sales GROUP BY region ORDER BY region",
            &[],
        )
        .await
        .expect("agg");
    assert_eq!(rows.len(), 2);
    // Totals: north = 370, south = 325
    assert_eq!(rows[0]["region"], serde_json::json!("north"));
    assert_eq!(rows[1]["region"], serde_json::json!("south"));

    // Window function — cumulative revenue per region. Cast to DOUBLE
    // so the JSON round-trip emits numbers instead of DECIMAL strings,
    // giving us stable equality assertions.
    let rows = conn
        .query(
            "SELECT region, day,
                    CAST(SUM(amount) OVER (PARTITION BY region ORDER BY day
                                           ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
                         AS DOUBLE) AS cum
             FROM sales
             ORDER BY region, day",
            &[],
        )
        .await
        .expect("window");
    assert_eq!(rows.len(), 6);
    let last_north = rows
        .iter()
        .rfind(|r| r["region"] == serde_json::json!("north"))
        .expect("north last");
    assert_eq!(last_north["cum"], serde_json::json!(370.0));
    let last_south = rows
        .iter()
        .rfind(|r| r["region"] == serde_json::json!("south"))
        .expect("south last");
    assert_eq!(last_south["cum"], serde_json::json!(325.0));
}

#[tokio::test]
#[ignore = "DuckDB E2E — run via `docker compose run --rm test-duckdb`"]
async fn e2e_parquet_roundtrip() {
    if !skip_unless_e2e() {
        return;
    }
    let pool = pool().await;
    let conn = pool.get().await.expect("conn");
    let tmp: TempDir = tempfile::tempdir().expect("tempdir");
    let parquet_path = tmp.path().join("fact.parquet");
    let path_str = parquet_path.to_string_lossy().into_owned();

    conn.execute_batch(
        "CREATE TABLE fact (id INTEGER, v DOUBLE);
         INSERT INTO fact SELECT i, sqrt(i::DOUBLE) FROM generate_series(1, 1000) AS t(i);",
    )
    .await
    .expect("seed");

    // Export to Parquet
    conn.copy_to_parquet("SELECT * FROM fact", &path_str)
        .await
        .expect("copy_to_parquet");
    assert!(parquet_path.exists(), "parquet file should exist");

    // Re-import and assert the row count + a couple of values survive.
    let rows = conn
        .query(
            &format!("SELECT COUNT(*) AS n FROM read_parquet('{}')", path_str),
            &[],
        )
        .await
        .expect("read_parquet count");
    assert_eq!(rows[0]["n"], serde_json::json!(1000));

    let rows = conn
        .query(
            &format!(
                "SELECT id, v FROM read_parquet('{}') WHERE id IN (1, 100, 1000) ORDER BY id",
                path_str
            ),
            &[],
        )
        .await
        .expect("read_parquet select");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"], serde_json::json!(1));
    assert_eq!(rows[1]["id"], serde_json::json!(100));
    assert_eq!(rows[2]["id"], serde_json::json!(1000));
}

#[tokio::test]
#[ignore = "DuckDB E2E — run via `docker compose run --rm test-duckdb`"]
async fn e2e_transaction_rollback_preserves_state() {
    if !skip_unless_e2e() {
        return;
    }
    let pool = pool().await;
    let conn = pool.get().await.expect("conn");

    conn.execute_batch("CREATE TABLE acct (id INTEGER, bal INTEGER)")
        .await
        .expect("create");
    conn.execute(
        "INSERT INTO acct VALUES (?, ?)",
        &[FilterValue::Int(1), FilterValue::Int(100)],
    )
    .await
    .expect("seed");

    conn.execute_batch("BEGIN TRANSACTION")
        .await
        .expect("begin");
    conn.execute(
        "UPDATE acct SET bal = ? WHERE id = ?",
        &[FilterValue::Int(0), FilterValue::Int(1)],
    )
    .await
    .expect("update");
    conn.execute_batch("ROLLBACK").await.expect("rollback");

    let row = conn
        .query_one("SELECT bal FROM acct WHERE id = 1", &[])
        .await
        .expect("query");
    assert_eq!(row["bal"], serde_json::json!(100));
}
