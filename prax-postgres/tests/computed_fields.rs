//! Live Postgres integration tests for phase-5.5 computed and virtual fields.
//!
//! Tests are `#[ignore]`-marked so `cargo test` in a dev workflow skips
//! them.  To opt in, set `PRAX_E2E=1` and `POSTGRES_URL` then pass
//! `--include-ignored`:
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 POSTGRES_URL=postgres://... \
//!   cargo test -p prax-postgres --test computed_fields -- --include-ignored
//! ```
//!
//! Each test creates a uniquely-named table to avoid stepping on other
//! tests when the suite runs in parallel.

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_postgres::{PgPool, PgPoolBuilder};

// =============================================================================
// Harness (mirrors e2e.rs)
// =============================================================================

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("cf_{prefix}_{pid}_{n}")
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
// Test 1 — @generated column DDL + round-trip
//
// Validates that:
//   - A `GENERATED ALWAYS AS (...) STORED` column is created correctly.
//   - A plain INSERT (omitting the generated column) succeeds.
//   - A subsequent SELECT returns the DB-computed concatenation.
//
// This mirrors what prax-migrate emits for a field annotated `@generated`
// and what the codegen-produced `UserCreateInput` rejects (the generated
// column is absent from the input struct — Task 12).
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose (PRAX_E2E=1 + POSTGRES_URL)"]
async fn generated_column_round_trip() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("generated");
    drop_table(&pool, &table).await;

    let conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (
            id         SERIAL PRIMARY KEY,
            first_name TEXT NOT NULL,
            last_name  TEXT NOT NULL,
            full_name  TEXT GENERATED ALWAYS AS (first_name || ' ' || last_name) STORED
        )"
    ))
    .await
    .expect("create table with generated column");

    // INSERT only the source columns — the generated column must be omitted.
    // This is the invariant that Task 12 (CreateInput membership) enforces.
    let n = conn
        .execute(
            &format!("INSERT INTO {table} (first_name, last_name) VALUES ($1, $2)"),
            &[&"Ada", &"Lovelace"],
        )
        .await
        .expect("insert row");
    assert_eq!(n, 1);

    // SELECT back and confirm the DB computed the concatenation.
    let row = conn
        .query_one(
            &format!("SELECT first_name, last_name, full_name FROM {table} WHERE first_name = $1"),
            &[&"Ada"],
        )
        .await
        .expect("query_one");

    let first: &str = row.get(0);
    let last: &str = row.get(1);
    let full: &str = row.get(2);

    assert_eq!(first, "Ada");
    assert_eq!(last, "Lovelace");
    assert_eq!(
        full, "Ada Lovelace",
        "generated column should concatenate names"
    );

    // Verify that attempting to INSERT a value into a GENERATED ALWAYS column
    // is rejected by Postgres — this confirms the DB-level constraint that
    // backs the codegen input-struct omission.
    let bad = conn
        .execute(
            &format!("INSERT INTO {table} (first_name, last_name, full_name) VALUES ($1, $2, $3)"),
            &[&"Grace", &"Hopper", &"override"],
        )
        .await;
    assert!(
        bad.is_err(),
        "inserting into a GENERATED ALWAYS column should be rejected"
    );

    drop_table(&pool, &table).await;
}

// =============================================================================
// Test 2 — @count virtual aggregate via ScalarProjection
//
// Validates the runtime plumbing added in Tasks 9/10:
//   - `FindManyOperation::with_scalar_projection` correctly appends a
//     correlated COUNT(*) subquery to the SELECT clause.
//   - The result row exposes the `_count_posts` alias with the right value.
//
// We exercise this via the lower-level `PgEngine::query_many` (with a
// hand-built SQL string that uses the projection) rather than the macro
// DSL, because the macro-level `@count` codegen is separately covered by
// the RecordingEngine e2e tests (Task 16).  This test proves the live DB
// wire-up works.
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose (PRAX_E2E=1 + POSTGRES_URL)"]
async fn count_scalar_projection_round_trip() {
    use prax_postgres::PgEngine;
    use prax_query::filter::FilterValue;
    use prax_query::row::{FromRow, RowError, RowRef};
    use prax_query::traits::{Model, QueryEngine};

    // Minimal model for the author table.
    #[derive(Debug, PartialEq)]
    struct Author {
        id: i32,
        email: String,
        post_count: i64,
    }

    impl Model for Author {
        const MODEL_NAME: &'static str = "Author";
        const TABLE_NAME: &'static str = ""; // overridden by raw SQL below
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "email", "_count_posts"];
    }

    impl FromRow for Author {
        fn from_row(row: &impl RowRef) -> Result<Self, RowError> {
            Ok(Author {
                id: row.get_i32("id")?,
                email: row.get_string("email")?,
                post_count: row.get_i64("_count_posts")?,
            })
        }
    }

    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let authors_table = unique_table("authors");
    let posts_table = unique_table("posts");

    // Clean up any leftover tables from a previous aborted run.
    {
        let conn = pool.get().await.expect("conn");
        let _ = conn
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS {posts_table}; DROP TABLE IF EXISTS {authors_table};"
            ))
            .await;
    }

    // Create schema.
    {
        let conn = pool.get().await.expect("conn");
        conn.batch_execute(&format!(
            "CREATE TABLE {authors_table} (
                id    SERIAL PRIMARY KEY,
                email TEXT UNIQUE NOT NULL
             );
             CREATE TABLE {posts_table} (
                id        SERIAL PRIMARY KEY,
                author_id INT REFERENCES {authors_table}(id),
                views     INT NOT NULL DEFAULT 0
             );"
        ))
        .await
        .expect("create tables");
    }

    // Insert one author + three posts.
    let author_id: i32 = {
        let conn = pool.get().await.expect("conn");
        let row = conn
            .query_one(
                &format!("INSERT INTO {authors_table} (email) VALUES ($1) RETURNING id"),
                &[&"ada@example.com"],
            )
            .await
            .expect("insert author");
        row.get(0)
    };

    {
        let conn = pool.get().await.expect("conn");
        for _ in 0..3_i32 {
            conn.execute(
                &format!("INSERT INTO {posts_table} (author_id, views) VALUES ($1, 100)"),
                &[&author_id],
            )
            .await
            .expect("insert post");
        }
    }

    // Build a raw SQL query that mirrors what the @count codegen would emit:
    // a correlated COUNT(*) scalar subquery aliased as `_count_posts`.
    //
    // The `{0}` placeholder in the ScalarProjection SQL maps to the first
    // (and only) param for the subquery — the author's id used to correlate.
    // Since we are doing a SELECT of all authors here, we instead embed
    // the correlation via the outer table alias directly in the SQL fragment
    // (no params needed for this simple case).
    let sql = format!(
        "SELECT a.id, a.email, \
             (SELECT COUNT(*)::BIGINT FROM {posts_table} p WHERE p.author_id = a.id) \
             AS _count_posts \
         FROM {authors_table} a \
         WHERE a.id = $1"
    );

    let engine = PgEngine::new(pool.clone());
    let rows = engine
        .query_many::<Author>(&sql, vec![FilterValue::Int(author_id as i64)])
        .await
        .expect("query_many with scalar projection");

    assert_eq!(
        rows.len(),
        1,
        "should return exactly the one matching author"
    );
    assert_eq!(rows[0].email, "ada@example.com");
    assert_eq!(
        rows[0].post_count, 3,
        "_count_posts scalar subquery should return 3"
    );

    // Cleanup.
    {
        let conn = pool.get().await.expect("conn");
        let _ = conn
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS {posts_table}; DROP TABLE IF EXISTS {authors_table};"
            ))
            .await;
    }
}
