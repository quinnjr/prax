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

use prax_mysql::row_ref::MysqlRowRef;
use prax_mysql::{MysqlPool, MysqlPoolBuilder};
use prax_query::row::RowRef;

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

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_row_ref_primitive_reads() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let mut conn = pool.get().await.expect("conn");
    let rows = conn
        .query_raw("SELECT 42 AS n, 'hello' AS s")
        .await
        .expect("query");
    let r = MysqlRowRef::from_row(rows.into_iter().next().unwrap()).unwrap();
    assert_eq!(r.get_i32("n").unwrap(), 42);
    assert_eq!(r.get_str("s").unwrap(), "hello");
}

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_query_many_typed_decodes_rows() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    use prax_mysql::MysqlEngine;
    use prax_query::filter::FilterValue;
    use prax_query::row::{FromRow, RowError, RowRef};
    use prax_query::traits::{Model, QueryEngine};

    #[derive(Debug, PartialEq)]
    struct Person {
        id: i32,
        email: String,
    }
    impl Model for Person {
        const MODEL_NAME: &'static str = "Person";
        const TABLE_NAME: &'static str = "e2e_mysql_typed";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "email"];
    }
    impl FromRow for Person {
        fn from_row(row: &impl RowRef) -> Result<Self, RowError> {
            Ok(Person {
                id: row.get_i32("id")?,
                email: row.get_string("email")?,
            })
        }
    }

    let table = unique_table("typed");
    let pool = pool().await;
    let engine = MysqlEngine::new(pool);

    engine
        .execute_raw(
            &format!("DROP TABLE IF EXISTS {table}"),
            Vec::<FilterValue>::new(),
        )
        .await
        .unwrap();
    engine
        .execute_raw(
            &format!(
                "CREATE TABLE {table} (id INT AUTO_INCREMENT PRIMARY KEY, \
                 email VARCHAR(255) NOT NULL)"
            ),
            vec![],
        )
        .await
        .unwrap();
    engine
        .execute_raw(
            &format!("INSERT INTO {table} (email) VALUES ('a@x.com'),('b@x.com')"),
            vec![],
        )
        .await
        .unwrap();

    let rows = engine
        .query_many::<Person>(
            &format!("SELECT id, email FROM {table} ORDER BY id"),
            Vec::<FilterValue>::new(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].email, "a@x.com");
    assert_eq!(rows[1].email, "b@x.com");

    engine
        .execute_raw(&format!("DROP TABLE {table}"), vec![])
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_count_on_empty_table_is_zero_not_error() {
    if skip_unless_e2e().is_none() {
        return;
    }
    use prax_mysql::MysqlEngine;
    use prax_query::filter::FilterValue;
    use prax_query::traits::QueryEngine;

    let pool = pool().await;
    let engine = MysqlEngine::new(pool);
    let table = unique_table("count_empty");

    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .unwrap();
    engine
        .execute_raw(
            &format!("CREATE TABLE {table} (id INT PRIMARY KEY)"),
            vec![],
        )
        .await
        .unwrap();

    // COUNT(*) on an empty table returns 0 as a row — not a NULL — so
    // this must succeed with n == 0, not error out.
    let n = engine
        .count(
            &format!("SELECT COUNT(*) FROM {table}"),
            Vec::<FilterValue>::new(),
        )
        .await
        .unwrap();
    assert_eq!(n, 0);

    engine
        .execute_raw(&format!("DROP TABLE {table}"), vec![])
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires running MySQL via docker-compose"]
async fn e2e_row_ref_repeated_get_str_returns_same_slice() {
    if skip_unless_e2e().is_none() {
        return;
    }
    use mysql_async::prelude::Queryable;
    use prax_mysql::row_ref::MysqlRowRef;
    use prax_query::row::RowRef;

    let pool = pool().await;
    let mut conn = pool.get().await.unwrap();
    let rows: Vec<mysql_async::Row> = conn.inner_mut().query("SELECT 'hello' AS s").await.unwrap();
    let r = MysqlRowRef::from_row(rows.into_iter().next().unwrap()).unwrap();

    let s1 = r.get_str("s").unwrap();
    let s2 = r.get_str("s").unwrap();
    assert_eq!(s1, s2);
    // Same backing allocation — cache isn't re-computed.
    assert!(std::ptr::eq(s1.as_ptr(), s2.as_ptr()));
}
