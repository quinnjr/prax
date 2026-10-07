//! Live Postgres tests for the sqlx backend: `FilterValue::Array` binds and
//! `PgHelpers::notify` against a real server.
//!
//! Gated by `PRAX_E2E=1` (and optionally `POSTGRES_URL`); every test is also
//! `#[ignore]`d, so run them with `-- --include-ignored`.

#![cfg(feature = "postgres")]

use std::time::Duration;

use prax_query::filter::FilterValue;
use prax_sqlx::postgres::PgHelpers;
use prax_sqlx::row::SqlxRow;
use prax_sqlx::{SqlxConfig, SqlxEngine, SqlxError};
use sqlx::Row;
use sqlx::postgres::PgListener;

fn url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
        "postgres://prax:prax_test_password@localhost:5432/prax_test".to_string()
    }))
}

/// One-connection engine, so `TEMP` tables stay visible across calls.
async fn engine(url: &str) -> SqlxEngine {
    let config = SqlxConfig::from_url(url)
        .expect("config")
        .with_max_connections(1);
    SqlxEngine::new(config).await.expect("connect")
}

async fn select_one(
    engine: &SqlxEngine,
    sql: &str,
    params: Vec<FilterValue>,
) -> sqlx::postgres::PgRow {
    match engine.raw_query_one(sql, &params).await.expect(sql) {
        SqlxRow::Postgres(row) => row,
        _ => panic!("expected a Postgres row"),
    }
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn array_round_trips_per_kind() {
    let Some(url) = url() else { return };
    let engine = engine(&url).await;
    use FilterValue::*;

    let row = select_one(
        &engine,
        "SELECT $1::int8[]",
        vec![Array(vec![Int(1), Null, Int(-3)])],
    )
    .await;
    assert_eq!(
        row.get::<Vec<Option<i64>>, _>(0),
        vec![Some(1), None, Some(-3)]
    );

    let row = select_one(
        &engine,
        "SELECT $1::float8[]",
        vec![Array(vec![Float(1.5), Null])],
    )
    .await;
    assert_eq!(row.get::<Vec<Option<f64>>, _>(0), vec![Some(1.5), None]);

    let row = select_one(
        &engine,
        "SELECT $1::bool[]",
        vec![Array(vec![Null, Bool(true), Bool(false)])],
    )
    .await;
    assert_eq!(
        row.get::<Vec<Option<bool>>, _>(0),
        vec![None, Some(true), Some(false)]
    );

    let row = select_one(
        &engine,
        "SELECT $1::text[]",
        vec![Array(vec![String("a".into()), Null, String("it's".into())])],
    )
    .await;
    assert_eq!(
        row.get::<Vec<Option<std::string::String>>, _>(0),
        vec![Some("a".to_string()), None, Some("it's".to_string())]
    );

    let row = select_one(
        &engine,
        "SELECT $1::jsonb[]",
        vec![Array(vec![Json(serde_json::json!({"a": 1})), Null])],
    )
    .await;
    assert_eq!(
        row.get::<Vec<Option<serde_json::Value>>, _>(0),
        vec![Some(serde_json::json!({"a": 1})), None]
    );

    let row = select_one(
        &engine,
        "SELECT $1::bytea[]",
        vec![Array(vec![Bytes(vec![0, 255, 159]), Null])],
    )
    .await;
    assert_eq!(
        row.get::<Vec<Option<Vec<u8>>>, _>(0),
        vec![Some(vec![0, 255, 159]), None]
    );

    // Empty arrays bind as text[].
    let row = select_one(&engine, "SELECT $1::text[]", vec![Array(vec![])]).await;
    assert!(row.get::<Vec<Option<std::string::String>>, _>(0).is_empty());
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn any_with_int_and_uuid_columns() {
    let Some(url) = url() else { return };
    let engine = engine(&url).await;
    use FilterValue::*;

    engine
        .raw_execute(
            "CREATE TEMP TABLE sqlx_any_probe (n int4 NOT NULL, u uuid NOT NULL)",
            &[],
        )
        .await
        .expect("create");
    engine
        .raw_execute(
            "INSERT INTO sqlx_any_probe VALUES \
             (1, '67e55044-10b1-426f-9247-bb680e5fe0c8'), \
             (2, '11111111-1111-1111-1111-111111111111'), \
             (3, '22222222-2222-2222-2222-222222222222')",
            &[],
        )
        .await
        .expect("insert");

    // int8[] against an int4 column.
    let row = select_one(
        &engine,
        "SELECT count(*) FROM sqlx_any_probe WHERE n = ANY($1)",
        vec![Array(vec![Int(1), Int(3), Int(99)])],
    )
    .await;
    assert_eq!(row.get::<i64, _>(0), 2);

    // String arrays bind as text[], so the uuid column needs the cast.
    let row = select_one(
        &engine,
        "SELECT count(*) FROM sqlx_any_probe WHERE u = ANY($1::uuid[])",
        vec![Array(vec![
            String("67e55044-10b1-426f-9247-bb680e5fe0c8".into()),
            String("22222222-2222-2222-2222-222222222222".into()),
        ])],
    )
    .await;
    assert_eq!(row.get::<i64, _>(0), 2);

    // A non-uuid string fails server-side rather than matching nothing.
    let err = engine
        .raw_query_one(
            "SELECT count(*) FROM sqlx_any_probe WHERE u = ANY($1::uuid[])",
            &[Array(vec![String("not-a-uuid".into())])],
        )
        .await;
    assert!(err.is_err());
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn notify_reaches_pg_listener_with_exact_channel_and_payload() {
    let Some(url) = url() else { return };
    let engine = engine(&url).await;
    let pool = engine.pool().as_postgres().expect("postgres pool");

    let channel = format!("Mixed\"Case_{}", std::process::id());
    let payload = "it's";

    let mut listener = PgListener::connect(&url).await.expect("listener");
    listener.listen(&channel).await.expect("listen");

    PgHelpers::notify(pool, &channel, payload)
        .await
        .expect("notify");

    let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv())
        .await
        .expect("timed out waiting for notification")
        .expect("recv");
    assert_eq!(notification.channel(), channel);
    assert_eq!(notification.payload(), payload);
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn notify_validates_client_side() {
    let Some(url) = url() else { return };
    let engine = engine(&url).await;
    let pool = engine.pool().as_postgres().expect("postgres pool");

    for (channel, payload) in [
        (String::new(), "x".to_string()),
        ("a".repeat(64), "x".to_string()),
        ("a\0b".to_string(), "x".to_string()),
        ("ok".to_string(), "p".repeat(8000)),
    ] {
        let err = PgHelpers::notify(pool, &channel, &payload)
            .await
            .expect_err("must be rejected before reaching the server");
        assert!(
            matches!(err, SqlxError::Query(_)),
            "unexpected error: {err}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn database_errors_carry_the_sqlstate() {
    let Some(url) = url() else { return };
    let engine = engine(&url).await;
    let Err(err) = engine.raw_query_one("SELECT 1 / 0", &[]).await else {
        panic!("division by zero must fail");
    };
    let err = prax_query::QueryError::from(err);
    assert_eq!(err.sqlstate(), Some("22012"), "{err:?}");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn engine_errors_carry_the_sqlstate() {
    use prax_query::ErrorCode;
    use prax_query::traits::QueryEngine;

    let Some(url) = url() else { return };
    // One connection: the TEMP table is visible to the transaction too.
    let engine = engine(&url).await;
    QueryEngine::execute_raw(
        &engine,
        "CREATE TEMP TABLE sqlx_sqlstate_probe (
             email TEXT UNIQUE,
             code TEXT UNIQUE DEFERRABLE INITIALLY DEFERRED
         )",
        Vec::new(),
    )
    .await
    .expect("create probe table");
    let insert = "INSERT INTO sqlx_sqlstate_probe (email, code) VALUES ($1, $2)";
    let row = |email: &str, code: &str| {
        vec![
            FilterValue::String(email.into()),
            FilterValue::String(code.into()),
        ]
    };

    // What `PraxClient::execute_raw` runs: a unique violation.
    QueryEngine::execute_raw(&engine, insert, row("a@x", "a"))
        .await
        .expect("first insert");
    let err = QueryEngine::execute_raw(&engine, insert, row("a@x", "b"))
        .await
        .expect_err("duplicate email");
    assert_eq!(err.sqlstate(), Some("23505"), "{err:?}");
    assert_eq!(err.code, ErrorCode::UniqueConstraint, "{err:?}");

    // `count` goes through `raw_query_one`.
    let err = QueryEngine::count(
        &engine,
        "SELECT count(*) FROM sqlx_no_such_table",
        Vec::new(),
    )
    .await
    .expect_err("missing table");
    assert_eq!(err.sqlstate(), Some("42P01"), "{err:?}");

    // Inside a transaction, and at COMMIT (the deferred constraint).
    let err = engine
        .transaction(move |tx| async move {
            QueryEngine::execute_raw(&tx, insert, row("b@x", "a")).await?;
            Ok(())
        })
        .await
        .expect_err("deferred duplicate fails at COMMIT");
    assert_eq!(err.sqlstate(), Some("23505"), "{err:?}");
    let err = engine
        .transaction(move |tx| async move {
            QueryEngine::execute_raw(&tx, insert, row("a@x", "c")).await?;
            Ok(())
        })
        .await
        .expect_err("duplicate email in a transaction");
    assert_eq!(err.sqlstate(), Some("23505"), "{err:?}");
}
