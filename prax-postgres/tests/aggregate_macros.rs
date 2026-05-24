//! Live Postgres integration for phase-6 aggregate operations.
//! Gated #[ignore] — requires PRAX_E2E=1 and POSTGRES_URL.
//! Run: PRAX_E2E=1 POSTGRES_URL=... cargo test -p prax-postgres --test aggregate_macros -- --ignored
//!
//! Exercises the runtime AggregateOperation / GroupByOperation (what the
//! aggregate!/group_by!/count! macros lower to) against real Postgres.
//! The macro front-end is covered by trybuild fixtures (compile-level)
//! and the codegen unit tests; the schema-path relation_helpers bug
//! prevents end-to-end macro use in a test crate (see
//! tests/aggregate_macros_e2e.rs).
//!
//! Because Model::TABLE_NAME is a `&'static str` const we cannot set it to a
//! runtime-generated unique table name.  Tests therefore call
//! `AggregateOperation::build_sql` / `GroupByOperation::build_sql` for the
//! SQL shape and then execute via the raw `QueryEngine::aggregate_query` path,
//! swapping the static TABLE_NAME for the dynamic table name in the SQL
//! string.  This is the same execution path that `exec()` takes internally.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::operations::having;
use prax_query::operations::{
    AggregateOperation, AggregateResult, GroupByOperation, GroupByResult,
};
use prax_query::traits::{Model, QueryEngine};

// =============================================================================
// Harness (mirrors computed_fields.rs)
// =============================================================================

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("agg_{prefix}_{pid}_{n}")
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
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

async fn drop_table(pool: &PgPool, table: &str) {
    let conn = pool.get().await.expect("acquire conn for cleanup");
    let _ = conn
        .batch_execute(&format!("DROP TABLE IF EXISTS {table}"))
        .await;
}

// =============================================================================
// Minimal Model stubs required by the typed operation builders.
// =============================================================================

struct CountModel;
impl Model for CountModel {
    const MODEL_NAME: &'static str = "CountModel";
    const TABLE_NAME: &'static str = "count_model_placeholder";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "email"];
}

struct ScoreModel;
impl Model for ScoreModel {
    const MODEL_NAME: &'static str = "ScoreModel";
    const TABLE_NAME: &'static str = "score_model_placeholder";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "score"];
}

struct TeamModel;
impl Model for TeamModel {
    const MODEL_NAME: &'static str = "TeamModel";
    const TABLE_NAME: &'static str = "team_model_placeholder";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "team_id", "score"];
}

// =============================================================================
// Test 1 — COUNT(*) round-trip
//
// Creates a table with 5 rows and verifies COUNT(*) returns 5.
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose (PRAX_E2E=1 + POSTGRES_URL)"]
async fn count_select_round_trip() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("count");
    drop_table(&pool, &table).await;

    {
        let conn = pool.get().await.expect("conn");
        conn.batch_execute(&format!(
            "CREATE TABLE {table} (id SERIAL PRIMARY KEY, email TEXT)"
        ))
        .await
        .expect("create table");

        // 3 rows with email, 2 with NULL
        conn.batch_execute(&format!(
            "INSERT INTO {table} (email) VALUES \
             ('a@example.com'), ('b@example.com'), ('c@example.com'), (NULL), (NULL)"
        ))
        .await
        .expect("insert rows");
    }

    let engine = PgEngine::new(pool.clone());
    let dialect = engine.dialect();

    // Build SQL via the operation builder, then swap in the real table name.
    let op: AggregateOperation<CountModel, PgEngine> = AggregateOperation::new().count();
    let (sql, params) = op.build_sql(dialect);
    let sql = sql.replace(CountModel::TABLE_NAME, &table);

    let mut rows = engine
        .aggregate_query(&sql, params)
        .await
        .expect("aggregate_query");

    let result = AggregateResult::from_row(rows.pop().unwrap_or_default());

    assert_eq!(
        result.count,
        Some(5),
        "COUNT(*) should be 5 (includes NULLs)"
    );

    drop_table(&pool, &table).await;
}

// =============================================================================
// Test 2 — SUM / AVG / COUNT(*) round-trip
//
// Inserts scores 10, 20, 30 and validates aggregate results.
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose (PRAX_E2E=1 + POSTGRES_URL)"]
async fn aggregate_sum_avg_count_round_trip() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("score");
    drop_table(&pool, &table).await;

    {
        let conn = pool.get().await.expect("conn");
        conn.batch_execute(&format!(
            "CREATE TABLE {table} (id SERIAL PRIMARY KEY, score INT NOT NULL)"
        ))
        .await
        .expect("create table");

        conn.batch_execute(&format!(
            "INSERT INTO {table} (score) VALUES (10), (20), (30)"
        ))
        .await
        .expect("insert rows");
    }

    let engine = PgEngine::new(pool.clone());
    let dialect = engine.dialect();

    let op: AggregateOperation<ScoreModel, PgEngine> =
        AggregateOperation::new().count().sum("score").avg("score");
    let (sql, params) = op.build_sql(dialect);
    let sql = sql.replace(ScoreModel::TABLE_NAME, &table);

    let mut rows = engine
        .aggregate_query(&sql, params)
        .await
        .expect("aggregate_query");

    let result = AggregateResult::from_row(rows.pop().unwrap_or_default());

    assert_eq!(result.count, Some(3), "COUNT(*) should be 3");

    let sum = result
        .sum_as_f64("score")
        .expect("sum(score) should be present");
    assert!(
        (sum - 60.0).abs() < 0.001,
        "SUM(score) should be 60, got {sum}"
    );

    let avg = result
        .avg_as_f64("score")
        .expect("avg(score) should be present");
    assert!(
        (avg - 20.0).abs() < 0.001,
        "AVG(score) should be 20.0, got {avg}"
    );

    drop_table(&pool, &table).await;
}

// =============================================================================
// Test 3 — GROUP BY + HAVING round-trip
//
// team 1 → 2 rows, team 2 → 4 rows.
// HAVING COUNT(*) > 3 should return only team 2 with count == 4.
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose (PRAX_E2E=1 + POSTGRES_URL)"]
async fn group_by_with_having_round_trip() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("team");
    drop_table(&pool, &table).await;

    {
        let conn = pool.get().await.expect("conn");
        conn.batch_execute(&format!(
            "CREATE TABLE {table} (id SERIAL PRIMARY KEY, team_id INT NOT NULL, score INT NOT NULL)"
        ))
        .await
        .expect("create table");

        // team 1 → 2 rows, team 2 → 4 rows
        conn.batch_execute(&format!(
            "INSERT INTO {table} (team_id, score) VALUES \
             (1, 10), (1, 20), \
             (2, 30), (2, 40), (2, 50), (2, 60)"
        ))
        .await
        .expect("insert rows");
    }

    let engine = PgEngine::new(pool.clone());
    let dialect = engine.dialect();

    let op: GroupByOperation<TeamModel, PgEngine> =
        GroupByOperation::new(vec!["team_id".to_string()])
            .count()
            .having(having::count_gt(3.0));
    let (sql, params) = op.build_sql(dialect);
    let sql = sql.replace(TeamModel::TABLE_NAME, &table);

    let raw_rows = engine
        .aggregate_query(&sql, params)
        .await
        .expect("aggregate_query for group_by");

    // Split raw rows into GroupByResult (same logic as GroupByOperation::exec).
    let group_columns = ["team_id"];
    let results: Vec<GroupByResult> = raw_rows
        .into_iter()
        .map(|row| {
            let mut group_values: HashMap<String, serde_json::Value> = HashMap::new();
            let mut agg_map: HashMap<String, FilterValue> = HashMap::new();
            for (k, v) in row {
                if group_columns.contains(&k.as_str()) {
                    let json_val = match &v {
                        FilterValue::Int(n) => serde_json::Value::from(*n),
                        FilterValue::Float(f) => serde_json::json!(*f),
                        FilterValue::String(s) => serde_json::Value::String(s.clone()),
                        FilterValue::Bool(b) => serde_json::Value::Bool(*b),
                        _ => serde_json::Value::Null,
                    };
                    group_values.insert(k, json_val);
                } else {
                    agg_map.insert(k, v);
                }
            }
            GroupByResult {
                group_values,
                aggregates: AggregateResult::from_row(agg_map),
            }
        })
        .collect();

    assert_eq!(
        results.len(),
        1,
        "HAVING COUNT(*) > 3 should return exactly one group"
    );

    let team_id = results[0]
        .group_values
        .get("team_id")
        .and_then(serde_json::Value::as_i64)
        .expect("team_id should be present as integer");
    assert_eq!(team_id, 2, "the surviving group should be team 2");

    let count = results[0]
        .aggregates
        .count
        .expect("COUNT(*) should be present in aggregates");
    assert_eq!(count, 4, "team 2 has 4 rows");

    drop_table(&pool, &table).await;
}
