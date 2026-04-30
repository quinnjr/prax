//! Integration test for `.include()` on `FindManyOperation` against a
//! live PostgreSQL server (Task 22).
//!
//! Verifies the end-to-end eager-loading path:
//!
//! 1. `find_many()` issues the parent SELECT and hydrates Vec of User.
//! 2. Each queued `.include(user::posts::fetch())` triggers a single
//!    follow-up `SELECT * FROM rel_posts WHERE author_id IN (...)`.
//! 3. `ModelRelationLoader::load_relation` (emitted by the derive)
//!    buckets the children by FK and splices them onto the parent
//!    slice.
//!
//! Gated by `PRAX_E2E=1`. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test relations_postgres -- --include-ignored --nocapture
//! ```

#![cfg(test)]

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

/// Child model. `#[derive(Clone)]` is required because the relation
/// loader stitches children onto parents via `Vec::clone` — this is
/// the caller-side ergonomic cost of not using `Rc`/`Arc` in the
/// executor.
#[derive(Model, Debug, Clone, PartialEq)]
#[prax(table = "rel_posts")]
pub struct Post {
    #[prax(id, auto)]
    pub id: i32,
    pub title: String,
    pub author_id: i32,
}

/// Parent model — declares the `posts` relation.
#[derive(Model, Debug, Clone, PartialEq)]
#[prax(table = "rel_users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    pub email: String,
    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    pub posts: Vec<Post>,
}

client!(User, Post);

fn postgres_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
        // Matches the other e2e tests (client_postgres.rs). Docker
        // Compose publishes Postgres on 5432.
        "postgres://prax:prax_test_password@localhost:5432/prax_test".into()
    }))
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
    // DROP+CREATE is fine: the test owns both tables end-to-end. If
    // a concurrent run is in flight, re-running just starts from a
    // clean slate.
    conn.batch_execute(
        "DROP TABLE IF EXISTS rel_posts; \
         DROP TABLE IF EXISTS rel_users; \
         CREATE TABLE rel_users ( \
             id SERIAL PRIMARY KEY, \
             email TEXT UNIQUE NOT NULL \
         ); \
         CREATE TABLE rel_posts ( \
             id SERIAL PRIMARY KEY, \
             title TEXT NOT NULL, \
             author_id INTEGER NOT NULL REFERENCES rel_users(id) \
         )",
    )
    .await
    .expect("create rel_users/rel_posts");
    drop(conn);

    Some(PraxClient::new(PgEngine::new(pool)))
}

#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn find_many_include_posts_stitches_children_onto_parents() {
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Seed a user + three posts. Use the generated client for writes
    // too — that exercises the whole create path for a FK-bearing
    // child model.
    let alice = c
        .user()
        .create()
        .set("email", "alice@rel.example.com")
        .exec()
        .await
        .expect("create alice");
    assert!(alice.id > 0);

    for title in ["First", "Second", "Third"] {
        c.post()
            .create()
            .set("title", title)
            .set("author_id", alice.id)
            .exec()
            .await
            .expect("create post");
    }

    // Sanity-check before the include: the child table has three
    // rows. If this assertion fires, the failure is in the seed
    // path, not the loader.
    let all_posts = c
        .post()
        .find_many()
        .exec()
        .await
        .expect("find posts directly");
    assert_eq!(all_posts.len(), 3, "seeded three posts");

    // The load-bearing assertion: find_many with .include() returns
    // the single user with all three posts attached.
    let users = c
        .user()
        .find_many()
        .include(user::posts::fetch())
        .exec()
        .await
        .expect("find_many with include");

    assert_eq!(users.len(), 1, "exactly one seeded user");
    assert_eq!(users[0].id, alice.id);
    assert_eq!(users[0].posts.len(), 3, "all three posts attached");

    let mut titles: Vec<_> = users[0].posts.iter().map(|p| p.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, vec!["First", "Second", "Third"]);

    for post in &users[0].posts {
        assert_eq!(post.author_id, alice.id);
    }
}
