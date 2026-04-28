//! End-to-end tests for prax-mssql against a live SQL Server instance.
//!
//! Gated by `PRAX_E2E=1` and requires `MSSQL_URL`.
//!
//! ```sh
//! docker compose up -d mssql
//! docker compose run --rm test-mssql
//! ```
//!
//! MSSQL assigns object names per-database and cleaning up is slow, so
//! each test creates a uniquely named table and drops it at the end.

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_mssql::{MssqlConfig, MssqlPool};

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
    std::env::var("MSSQL_URL").ok()
}

async fn pool() -> MssqlPool {
    let url = skip_unless_e2e().expect("PRAX_E2E=1 and MSSQL_URL required");
    let config = MssqlConfig::from_connection_string(&url).expect("parse mssql url");
    MssqlPool::builder()
        .config(config)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(15))
        .trust_cert(true)
        .build()
        .await
        .expect("connect to mssql")
}

async fn drop_table(pool: &MssqlPool, table: &str) {
    let mut conn = pool.get().await.expect("cleanup conn");
    let _ = conn
        .batch_execute(&format!(
            "IF OBJECT_ID('dbo.{table}', 'U') IS NOT NULL DROP TABLE dbo.{table}"
        ))
        .await;
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_crud_roundtrip() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("crud");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE dbo.{table} (
            id INT IDENTITY(1,1) PRIMARY KEY,
            name NVARCHAR(64) NOT NULL,
            score INT NOT NULL
        )"
    ))
    .await
    .expect("create table");

    // INSERT — tiberius uses @P1, @P2, ... for parameter markers.
    let n = conn
        .execute(
            &format!("INSERT INTO dbo.{table} (name, score) VALUES (@P1, @P2)"),
            &[&"alice", &42_i32],
        )
        .await
        .expect("insert");
    assert_eq!(n, 1);

    // SELECT
    let rows = conn
        .query(&format!("SELECT name, score FROM dbo.{table}"), &[])
        .await
        .expect("select");
    assert_eq!(rows.len(), 1);
    let name: &str = rows[0].get(0).expect("name");
    let score: i32 = rows[0].get(1).expect("score");
    assert_eq!(name, "alice");
    assert_eq!(score, 42);

    // UPDATE
    let n = conn
        .execute(
            &format!("UPDATE dbo.{table} SET score = @P1 WHERE name = @P2"),
            &[&100_i32, &"alice"],
        )
        .await
        .expect("update");
    assert_eq!(n, 1);

    // DELETE
    let n = conn
        .execute(&format!("DELETE FROM dbo.{table}"), &[])
        .await
        .expect("delete");
    assert_eq!(n, 1);

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_transaction_commit_and_rollback() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("tx");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE dbo.{table} (id INT IDENTITY(1,1) PRIMARY KEY, v INT NOT NULL)"
    ))
    .await
    .expect("create");

    // Commit path
    conn.begin_transaction().await.expect("begin");
    conn.execute(
        &format!("INSERT INTO dbo.{table} (v) VALUES (@P1)"),
        &[&1_i32],
    )
    .await
    .expect("insert 1");
    conn.commit().await.expect("commit");

    // Rollback path
    conn.begin_transaction().await.expect("begin");
    conn.execute(
        &format!("INSERT INTO dbo.{table} (v) VALUES (@P1)"),
        &[&999_i32],
    )
    .await
    .expect("insert doomed");
    conn.rollback().await.expect("rollback");

    let rows = conn
        .query(&format!("SELECT v FROM dbo.{table} ORDER BY v"), &[])
        .await
        .expect("select");
    let vs: Vec<i32> = rows.iter().map(|r| r.get::<i32, _>(0).unwrap()).collect();
    assert_eq!(vs, vec![1], "only committed row should survive");

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_query_opt_missing_row() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("opt");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!("CREATE TABLE dbo.{table} (id INT PRIMARY KEY)"))
        .await
        .expect("create");

    let row = conn
        .query_opt(&format!("SELECT id FROM dbo.{table} WHERE id = 1"), &[])
        .await
        .expect("query_opt");
    assert!(row.is_none());

    drop_table(&pool, &table).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_concurrent_writes_via_pool() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("conc");
    drop_table(&pool, &table).await;

    {
        let mut conn = pool.get().await.expect("conn");
        conn.batch_execute(&format!(
            "CREATE TABLE dbo.{table} (
                id INT IDENTITY(1,1) PRIMARY KEY,
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
                conn.execute(
                    &format!("INSERT INTO dbo.{table} (worker, seq) VALUES (@P1, @P2)"),
                    &[&w, &s],
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
    let row = conn
        .query_one(&format!("SELECT COUNT(*) FROM dbo.{table}"), &[])
        .await
        .expect("count");
    let count: i32 = row.get(0).expect("count");
    assert_eq!(count, workers * per_worker);

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_pool_is_healthy() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    assert!(pool.is_healthy().await);
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_query_many_typed_decodes_rows() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    use prax_mssql::MssqlEngine;
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
        const TABLE_NAME: &'static str = "e2e_mssql_typed";
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
    let engine = MssqlEngine::new(pool.clone());

    // Drop and create table
    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "IF OBJECT_ID('dbo.{table}', 'U') IS NOT NULL DROP TABLE dbo.{table}"
    ))
    .await
    .expect("drop table");
    conn.batch_execute(&format!(
        "CREATE TABLE dbo.{table} (id INT IDENTITY(1,1) PRIMARY KEY, email NVARCHAR(255) NOT NULL)"
    ))
    .await
    .expect("create table");
    conn.batch_execute(&format!(
        "INSERT INTO dbo.{table} (email) VALUES ('a@x.com'),('b@x.com')"
    ))
    .await
    .expect("insert");
    drop(conn);

    let rows = engine
        .query_many::<Person>(
            &format!("SELECT id, email FROM dbo.{table} ORDER BY id"),
            Vec::<FilterValue>::new(),
        )
        .await
        .expect("query_many");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].email, "a@x.com");
    assert_eq!(rows[1].email, "b@x.com");

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_row_ref_primitive_reads() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    use prax_mssql::row_ref::MssqlRowRef;
    use prax_query::row::RowRef;

    let pool = pool().await;
    let mut conn = pool.get().await.expect("conn");

    let rows = conn
        .query("SELECT 42 AS n, N'hello' AS s", &[])
        .await
        .expect("query");
    let row = rows.into_iter().next().expect("row present");

    let row_ref = MssqlRowRef::from_row(&row).expect("from_row");
    assert_eq!(row_ref.get_i32("n").unwrap(), 42);
    assert_eq!(row_ref.get_str("s").unwrap(), "hello");
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_row_ref_null_vs_absent_column() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    use prax_mssql::row_ref::MssqlRowRef;
    use prax_query::row::{RowError, RowRef};

    let pool = pool().await;
    let mut conn = pool.get().await.expect("conn");

    let rows = conn
        .query(
            "SELECT CAST(42 AS INT) AS present, CAST(NULL AS INT) AS nulled",
            &[],
        )
        .await
        .expect("query");
    let row = rows.into_iter().next().expect("row present");
    let r = MssqlRowRef::from_row(&row).expect("from_row");

    // Present column with a value → Ok(Some(_)).
    assert_eq!(r.get_i32_opt("present").unwrap(), Some(42));

    // Present column whose value is NULL → Ok(None).
    assert_eq!(r.get_i32_opt("nulled").unwrap(), None);

    // Absent column (not in the SELECT list) → Err(ColumnNotFound).
    let err = r.get_i32_opt("missing").unwrap_err();
    assert!(
        matches!(err, RowError::ColumnNotFound(ref col) if col == "missing"),
        "expected ColumnNotFound for absent column, got {err:?}",
    );
}

#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_query_one_missing_row_returns_not_found() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    use prax_mssql::MssqlEngine;
    use prax_query::error::ErrorCode;
    use prax_query::filter::FilterValue;
    use prax_query::row::{FromRow, RowError, RowRef};
    use prax_query::traits::{Model, QueryEngine};

    #[derive(Debug)]
    struct Person {
        id: i32,
        email: String,
    }
    impl Model for Person {
        const MODEL_NAME: &'static str = "Person";
        const TABLE_NAME: &'static str = "e2e_mssql_typed";
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

    let table = unique_table("one_missing");
    let pool = pool().await;
    let engine = MssqlEngine::new(pool);
    engine
        .execute_raw(
            &format!("IF OBJECT_ID('dbo.{table}', 'U') IS NOT NULL DROP TABLE dbo.{table}"),
            vec![],
        )
        .await
        .unwrap();
    engine
        .execute_raw(
            &format!(
                "CREATE TABLE dbo.{table} (id INT IDENTITY(1,1) PRIMARY KEY, email NVARCHAR(255) NOT NULL)"
            ),
            vec![],
        )
        .await
        .unwrap();

    let err = engine
        .query_one::<Person>(
            &format!("SELECT id, email FROM dbo.{table} WHERE id = 999"),
            Vec::<FilterValue>::new(),
        )
        .await
        .unwrap_err();

    assert_eq!(
        err.code,
        ErrorCode::RecordNotFound,
        "expected NotFound for missing row, got {err:?}"
    );

    engine
        .execute_raw(&format!("DROP TABLE dbo.{table}"), vec![])
        .await
        .unwrap();
}

/// Regression test for the `query_one` tail-materialization contract.
///
/// `query_one` nominally means "fetch exactly one row", but the driver
/// implementations disagree on what happens when the SQL returns 2+:
///
/// - `tokio-postgres::query_one` errors on 0 or 2+ rows.
/// - `mysql_async::exec_first` silently takes the first row.
/// - `rusqlite::rows.next()` silently takes the first row.
/// - `tiberius::query_one` (what this engine uses) silently takes the
///   first row and discards the rest.
///
/// This test locks down MSSQL's "first row wins" behavior so that if
/// someone later switches to an implementation that errors on 2+, the
/// change surfaces here and gets a deliberate CHANGELOG / migration
/// update rather than slipping in silently.
#[tokio::test]
#[ignore = "requires running MSSQL via docker-compose"]
async fn e2e_query_one_with_multiple_rows_behavior() {
    if skip_unless_e2e().is_none() {
        return;
    }
    use prax_mssql::MssqlEngine;
    use prax_query::filter::FilterValue;
    use prax_query::row::{FromRow, RowError, RowRef};
    use prax_query::traits::{Model, QueryEngine};

    #[derive(Debug)]
    struct Person {
        id: i32,
        email: String,
    }
    impl Model for Person {
        const MODEL_NAME: &'static str = "Person";
        const TABLE_NAME: &'static str = "e2e_mssql_one_multi";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "email"];
    }
    impl FromRow for Person {
        fn from_row(r: &impl RowRef) -> Result<Self, RowError> {
            Ok(Person {
                id: r.get_i32("id")?,
                email: r.get_string("email")?,
            })
        }
    }

    let table = unique_table("one_multi");
    let pool = pool().await;
    let engine = MssqlEngine::new(pool.clone());

    engine
        .execute_raw(
            &format!("IF OBJECT_ID('dbo.{table}', 'U') IS NOT NULL DROP TABLE dbo.{table}"),
            vec![],
        )
        .await
        .unwrap();
    engine
        .execute_raw(
            &format!(
                "CREATE TABLE dbo.{table} (id INT IDENTITY(1,1) PRIMARY KEY, \
                 email NVARCHAR(255) NOT NULL)"
            ),
            vec![],
        )
        .await
        .unwrap();
    engine
        .execute_raw(
            &format!("INSERT INTO dbo.{table} (email) VALUES ('a@x.com'),('b@x.com')"),
            vec![],
        )
        .await
        .unwrap();

    let result = engine
        .query_one::<Person>(
            &format!("SELECT id, email FROM dbo.{table} ORDER BY id"),
            Vec::<FilterValue>::new(),
        )
        .await;

    // Observed: tiberius' `query_one` yields the first row and drops the
    // rest. Callers that want Postgres-style "error on 2+" must add
    // `TOP 2` + check the row count themselves.
    match result {
        Ok(p) => {
            assert_eq!(
                p.email, "a@x.com",
                "MSSQL query_one should return the first row (by ORDER BY) \
                 when 2+ rows match; this documents tiberius' 'take first' \
                 semantics and must not regress silently."
            );
        }
        Err(e) => {
            panic!(
                "MSSQL engine is documented to return the first row on a \
                 multi-row query_one (tiberius::Client::query_one \
                 semantics). If the driver has changed to erroring, update \
                 this test, the Unreleased CHANGELOG migration guide, and \
                 any callers that relied on the implicit 'take first' \
                 behavior. Got: {e:?}"
            );
        }
    }

    drop_table(&pool, &table).await;
}
