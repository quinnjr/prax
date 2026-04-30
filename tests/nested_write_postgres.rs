//! Integration test for nested writes via `CreateOperation::with(...)`
//! against a live PostgreSQL server (Task 27).
//!
//! Verifies the end-to-end nested-write path:
//!
//! 1. `c.user().create().set(...).with(user::posts::create(...))` builds a
//!    [`prax_query::nested::NestedWriteOp::Create`] alongside the parent
//!    insert.
//! 2. `exec()` sees a non-empty nested queue and wraps the whole thing
//!    in an implicit transaction.
//! 3. The parent `INSERT ... RETURNING` runs first and yields the
//!    auto-assigned PK; each nested `INSERT` then splices the FK from
//!    that PK.
//! 4. A failure inside any nested child rolls back the parent too.
//!
//! Gated by `PRAX_E2E=1`. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test nested_write_postgres -- --include-ignored --nocapture
//! ```

#![cfg(test)]

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::error::QueryResult;
use prax_query::filter::FilterValue;

/// Serialize tests inside this file — both tests mutate the shared
/// `nested_{users,posts}` tables and run in parallel by default, so
/// one test's TRUNCATE in setup can wipe out another test's in-flight
/// state. A process-wide mutex guarantees they run one at a time.
fn test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Child model with FK back to the parent. `Clone` is required for
/// `.include()` round-trips; we don't use include() here but keep the
/// derive uniform with `relations_postgres.rs`.
#[derive(Model, Debug, Clone)]
#[prax(table = "nested_posts")]
pub struct Post {
    #[prax(id, auto)]
    pub id: i32,
    pub title: String,
    pub author_id: i32,
}

/// Parent model. The `posts` relation is what exposes
/// `user::posts::create(...)` and `user::posts::connect(...)` at the
/// call site.
#[derive(Model, Debug, Clone)]
#[prax(table = "nested_users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(unique)]
    pub email: String,
    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    pub posts: Vec<Post>,
}

client!(User, Post);

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

async fn setup() -> Option<PraxClient<PgEngine>> {
    let url = postgres_url()?;
    let pool: PgPool = PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres");

    let conn = pool.get().await.expect("acquire conn");
    // Use the advisory-lock + CREATE TABLE IF NOT EXISTS pattern from
    // `raw_postgres.rs` so parallel test runs don't race on the
    // pg_class unique index mid-CREATE. TRUNCATE at the end gives each
    // test a clean slate without racing on DROP.
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x6e6573746564300a);
         CREATE TABLE IF NOT EXISTS nested_users (
             id SERIAL PRIMARY KEY,
             email TEXT UNIQUE NOT NULL
         );
         CREATE TABLE IF NOT EXISTS nested_posts (
             id SERIAL PRIMARY KEY,
             title TEXT NOT NULL,
             author_id INTEGER NOT NULL REFERENCES nested_users(id)
         );
         TRUNCATE nested_posts, nested_users RESTART IDENTITY CASCADE;
         COMMIT",
    )
    .await
    .expect("create nested_{users,posts}");
    drop(conn);

    Some(PraxClient::new(PgEngine::new(pool)))
}

// `test_lock` returns a std::sync::MutexGuard held across `.await` to
// serialize the two tests against the shared Postgres tables. The mutex
// has no async contention — only this test process touches it — so the
// await-holding-lock lint does not apply.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn create_user_with_nested_posts() {
    let _guard = test_lock();
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Parent INSERT plus two nested post INSERTs, all inside a single
    // implicit transaction wrapped by `CreateOperation` because the
    // `nested` queue is non-empty.
    let u: User = c
        .user()
        .create()
        .set("email", "nw@x.com")
        .with(user::posts::create(vec![
            vec![("title".into(), FilterValue::String("p1".into()))],
            vec![("title".into(), FilterValue::String("p2".into()))],
        ]))
        .exec()
        .await
        .expect("nested create");
    assert!(u.id > 0, "auto-id should be assigned");

    // The nested-write engine populated `author_id` from the parent's
    // pk_value(), so filtering posts by that FK should yield both
    // children.
    let posts: Vec<Post> = c
        .post()
        .find_many()
        .r#where(post::author_id::equals(u.id))
        .exec()
        .await
        .expect("find posts by author_id");
    assert_eq!(posts.len(), 2, "both nested children should be inserted");
    let mut titles: Vec<_> = posts.iter().map(|p| p.title.as_str().to_owned()).collect();
    titles.sort();
    assert_eq!(titles, vec!["p1".to_string(), "p2".to_string()]);
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn nested_write_failure_rolls_back_parent() {
    let _guard = test_lock();
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // The nested payload references a non-existent column on
    // nested_posts, so Postgres rejects the child INSERT with an
    // "undefined column" error. That failure must also roll back the
    // parent INSERT — the whole operation is wrapped in a single tx.
    let result: QueryResult<User> = c
        .user()
        .create()
        .set("email", "rollback@x.com")
        .with(user::posts::create(vec![vec![(
            "nonexistent_column".into(),
            FilterValue::String("p1".into()),
        )]]))
        .exec()
        .await;
    assert!(
        result.is_err(),
        "nested child failure must surface as Err, got: {:?}",
        result.ok().map(|u| u.id)
    );

    // The parent row must not exist — if it does, rollback didn't
    // happen and the whole tx wrapping is broken.
    let users: Vec<User> = c
        .user()
        .find_many()
        .r#where(user::email::equals("rollback@x.com".into()))
        .exec()
        .await
        .expect("post-rollback find");
    assert!(
        users.is_empty(),
        "parent INSERT should have rolled back; found {} row(s)",
        users.len()
    );
}
