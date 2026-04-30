//! End-to-end test for `UpsertOperation` against a live PostgreSQL
//! container. Exercises both the INSERT and UPDATE arms of the
//! dialect-aware `ON CONFLICT (…) DO UPDATE SET …` clause via
//! `client.user().upsert()`.
//!
//! Gated by `PRAX_E2E=1`; `#[ignore]`-marked so `cargo test` stays
//! fast. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test upsert_postgres -- --include-ignored
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "upsert_pg_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

static TAG_COUNTER: AtomicU32 = AtomicU32::new(0);

fn next_tag() -> String {
    // Scope each test to a unique email suffix so parallel runs on the
    // same shared table cannot collide on the UNIQUE(email) constraint.
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
    // CREATE-IF-NOT-EXISTS path on pg_type. See tests/raw_postgres.rs
    // for background on why IF NOT EXISTS alone isn't enough.
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x7570736572745f70);
         CREATE TABLE IF NOT EXISTS upsert_pg_users (
             id SERIAL PRIMARY KEY,
             email TEXT NOT NULL UNIQUE,
             name TEXT
         );
         COMMIT",
    )
    .await
    .expect("create upsert_pg_users");
    drop(conn);

    Some((PraxClient::new(PgEngine::new(pool.clone())), pool))
}

#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn upsert_insert_then_update_targets_same_row() {
    let Some((c, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Keep each run hermetic: use a per-test email so we don't trip
    // over rows left by other tests sharing the upsert_pg_users table.
    let email = format!("upsert_{}@example.com", next_tag());

    // First upsert: no row exists -> INSERT path. Dialect emits
    // `ON CONFLICT (email) DO UPDATE SET name = $N RETURNING *`.
    let u1 = c
        .user()
        .upsert()
        .on_conflict(["email"])
        .create_set("email", email.as_str())
        .create_set("name", "A")
        .update_set("name", "B")
        .exec()
        .await
        .expect("first upsert (insert path)");

    assert_eq!(u1.email, email);
    assert_eq!(
        u1.name.as_deref(),
        Some("A"),
        "INSERT path stores the create-side value"
    );
    assert!(u1.id > 0, "SERIAL id should be assigned");

    // Second upsert: row now exists -> UPDATE path via the conflict
    // clause. Same create-side payload (so the attempt still conflicts
    // on email), but a different update-side `name`.
    let u2 = c
        .user()
        .upsert()
        .on_conflict(["email"])
        .create_set("email", email.as_str())
        .create_set("name", "A")
        .update_set("name", "C")
        .exec()
        .await
        .expect("second upsert (update path)");

    assert_eq!(
        u2.id, u1.id,
        "upsert should target the same row on second call"
    );
    assert_eq!(
        u2.name.as_deref(),
        Some("C"),
        "UPDATE path should apply the update-side value"
    );
}
