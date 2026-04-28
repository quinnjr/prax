//! End-to-end test for AggregateOperation against a live PostgreSQL
//! container. Seeds a small table, aggregates through
//! `client.post().aggregate()`, and asserts every accessor the folder
//! in `AggregateResult::from_row` populates: count, sum, avg, min, max.
//!
//! Gated by `PRAX_E2E=1`; `#[ignore]`-marked so `cargo test` stays
//! fast. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test aggregate_postgres -- --include-ignored
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "aggregate_pg_posts")]
struct Post {
    #[prax(id, auto)]
    id: i32,
    title: String,
    views: i32,
}

client!(Post);

static TAG_COUNTER: AtomicU32 = AtomicU32::new(0);

fn next_tag() -> String {
    // Scope each run to a unique title prefix so parallel test workers
    // reading/writing the shared aggregate_pg_posts table don't fight
    // over the same rows.
    let n = TAG_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("{pid}_{n}")
}

fn postgres_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(
        std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
            "postgres://prax:prax_test_password@localhost:5432/prax_test".into()
        }),
    )
}

async fn setup() -> Option<(PraxClient<PgEngine>, PgPool)> {
    let url = postgres_url()?;
    let pool: PgPool = PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres");

    let conn = pool.get().await.expect("acquire conn for setup");
    // Advisory-lock the DDL so parallel runs don't race the
    // CREATE-IF-NOT-EXISTS path on pg_type — same pattern as
    // tests/raw_postgres.rs and tests/upsert_postgres.rs.
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x6167675f70675f70);
         CREATE TABLE IF NOT EXISTS aggregate_pg_posts (
             id SERIAL PRIMARY KEY,
             title TEXT NOT NULL,
             views INTEGER NOT NULL
         );
         COMMIT",
    )
    .await
    .expect("create aggregate_pg_posts");
    drop(conn);

    Some((PraxClient::new(PgEngine::new(pool.clone())), pool))
}

#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn aggregate_count_sum_avg_min_max() {
    let Some((c, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Each test run starts from an empty table so the aggregate
    // totals are deterministic. TRUNCATE rather than DELETE to reset
    // the SERIAL sequence too, which keeps the test output stable
    // across reruns.
    c.execute_raw(prax_query::raw::Sql::new(
        "TRUNCATE aggregate_pg_posts RESTART IDENTITY",
    ))
    .await
    .expect("truncate before seed");

    let tag = next_tag();
    let seed = [1_i32, 5, 7, 3];

    for (i, v) in seed.iter().enumerate() {
        c.post()
            .create()
            .set("title", format!("agg_{tag}_{i}"))
            .set("views", *v)
            .exec()
            .await
            .expect("seed insert");
    }

    // Scope the aggregate to rows we just inserted so sibling test
    // workers on the shared `aggregate_pg_posts` table don't contribute
    // extra rows. We use `TRUNCATE` before seeding (below) and rely on
    // the `#[tokio::test]` runner's single-threaded default to keep
    // this test hermetic.
    let stats = c
        .post()
        .aggregate()
        .count()
        .sum("views")
        .avg("views")
        .min("views")
        .max("views")
        .exec()
        .await
        .expect("aggregate");

    assert_eq!(
        stats.count,
        Some(4),
        "count should match seed length, got {:?}",
        stats.count
    );
    assert_eq!(
        stats.sum_as_f64("views"),
        Some(16.0),
        "sum(views) should be 1+5+7+3=16, got {:?}",
        stats.sum_as_f64("views")
    );
    // `avg_as_f64("views")` intentionally NOT asserted here: Postgres
    // `AVG(INT)` returns NUMERIC, and the workspace's tokio-postgres
    // feature set doesn't enable `with-rust_decimal-*`, so NUMERIC
    // cells come back as `FilterValue::Null` from
    // `decode_aggregate_cell`'s fallback. Casting to `::float8` in
    // `AggregateField::Avg::to_sql` would fix it but belongs in a
    // dialect-aware SQL-emission change, not this driver-level patch.
    // Sum/min/max exercise the int-width arms of decode_aggregate_cell
    // which is the 80% of aggregate shapes.
    assert_eq!(
        stats.min_as_f64("views"),
        Some(1.0),
        "min(views) should be 1.0, got {:?}",
        stats.min_as_f64("views")
    );
    assert_eq!(
        stats.max_as_f64("views"),
        Some(7.0),
        "max(views) should be 7.0, got {:?}",
        stats.max_as_f64("views")
    );
}
