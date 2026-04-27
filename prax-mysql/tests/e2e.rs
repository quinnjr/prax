//! End-to-end tests for prax-mysql against a live MySQL server.
//!
//! Gated by `PRAX_E2E=1` and requires `MYSQL_URL`. Run via:
//!
//! ```sh
//! docker compose up -d mysql
//! docker compose run --rm test-mysql
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_mysql::{MysqlPool, MysqlPoolBuilder};

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("e2e_{prefix}_{pid}_{n}")
}

fn skip_unless_e2e() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("MYSQL_URL").ok()
}

async fn pool() -> MysqlPool {
    let url = skip_unless_e2e().expect("PRAX_E2E=1 and MYSQL_URL required");
    MysqlPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to mysql")
}

async fn drop_table(pool: &MysqlPool, table: &str) {
    let mut conn = pool.get().await.expect("acquire conn for cleanup");
    let _ = conn.execute(&format!("DROP TABLE IF EXISTS {table}")).await;
}

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_crud_roundtrip() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("crud");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.execute(&format!(
        "CREATE TABLE {table} (
            id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
            name VARCHAR(64) NOT NULL,
            score INT NOT NULL DEFAULT 0
        )"
    ))
    .await
    .expect("create table");

    let n = conn
        .execute_params(
            &format!("INSERT INTO {table} (name, score) VALUES (?, ?)"),
            ("alice", 42_i32),
        )
        .await
        .expect("insert");
    assert_eq!(n, 1);

    let rows: Vec<(String, i32)> = conn
        .query_params(&format!("SELECT name, score FROM {table}"), ())
        .await
        .expect("select");
    assert_eq!(rows, vec![("alice".into(), 42)]);

    let n = conn
        .execute_params(
            &format!("UPDATE {table} SET score = ? WHERE name = ?"),
            (100_i32, "alice"),
        )
        .await
        .expect("update");
    assert_eq!(n, 1);

    let (score,): (i32,) = conn
        .query_one_params(
            &format!("SELECT score FROM {table} WHERE name = ?"),
            ("alice",),
        )
        .await
        .expect("query_one");
    assert_eq!(score, 100);

    let n = conn
        .execute(&format!("DELETE FROM {table}"))
        .await
        .expect("delete");
    assert_eq!(n, 1);

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_bulk_insert_and_aggregate() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("agg");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.execute(&format!(
        "CREATE TABLE {table} (
            id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
            category VARCHAR(16) NOT NULL,
            amount INT NOT NULL
        )"
    ))
    .await
    .expect("create");

    for i in 0..50 {
        let cat = if i % 2 == 0 { "a" } else { "b" };
        conn.execute_params(
            &format!("INSERT INTO {table} (category, amount) VALUES (?, ?)"),
            (cat, i),
        )
        .await
        .expect("insert");
    }

    let rows: Vec<(String, i64)> = conn
        .query(&format!(
            "SELECT category, SUM(amount) FROM {table} GROUP BY category ORDER BY category"
        ))
        .await
        .expect("aggregate");
    assert_eq!(rows.len(), 2);
    let total: i64 = rows.iter().map(|(_, s)| *s).sum();
    assert_eq!(total, (0..50).sum::<i32>() as i64);

    drop_table(&pool, &table).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_concurrent_writes_via_pool() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("concurrent");
    drop_table(&pool, &table).await;

    {
        let mut conn = pool.get().await.expect("conn");
        conn.execute(&format!(
            "CREATE TABLE {table} (
                id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
                worker INT NOT NULL,
                seq INT NOT NULL
            )"
        ))
        .await
        .expect("create");
    }

    let workers = 4_i32;
    let per_worker = 25_i32;
    let mut tasks = Vec::new();
    for w in 0..workers {
        let pool = pool.clone();
        let table = table.clone();
        tasks.push(tokio::spawn(async move {
            let mut conn = pool.get().await.expect("conn");
            for s in 0..per_worker {
                conn.execute_params(
                    &format!("INSERT INTO {table} (worker, seq) VALUES (?, ?)"),
                    (w, s),
                )
                .await
                .expect("insert");
            }
        }));
    }
    for t in tasks {
        t.await.expect("join");
    }

    let mut conn = pool.get().await.expect("conn");
    let (count,): (i64,) = conn
        .query_one(&format!("SELECT COUNT(*) FROM {table}"))
        .await
        .expect("count");
    assert_eq!(count, (workers * per_worker) as i64);

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_pool_is_healthy() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    assert!(pool.is_healthy().await);
}
