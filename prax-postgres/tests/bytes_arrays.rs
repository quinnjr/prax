//! Postgres binding of `FilterValue::Bytes` (bytea) and `FilterValue::Array`
//! (array parameters). Gated by `PRAX_E2E=1` and `POSTGRES_URL`.

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::traits::QueryEngine;
use uuid::Uuid;

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("ba_{prefix}_{pid}_{n}")
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
        .max_connections(2)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

async fn count(pool: &PgPool, sql: &str, param: FilterValue) -> i64 {
    let params = prax_postgres::types::filter_values_to_params(&[param]).expect("convert param");
    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    let conn = pool.get().await.expect("conn");
    let row = conn.query_one(sql, &refs).await.expect("count query");
    row.get::<_, i64>(0)
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn bytea_round_trips_non_utf8() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let engine = PgEngine::new(pool.clone());
    let table = unique_table("bytea");
    engine
        .execute_raw(&format!("CREATE TABLE {table} (b BYTEA NOT NULL)"), vec![])
        .await
        .expect("create table");
    engine
        .execute_raw(
            &format!("INSERT INTO {table} (b) VALUES ($1)"),
            vec![FilterValue::Bytes(vec![0x00, 0x9F, 0xFF])],
        )
        .await
        .expect("insert bytes");
    let row = pool
        .get()
        .await
        .expect("conn")
        .query_one(&format!("SELECT b FROM {table}"), &[])
        .await
        .expect("select");
    assert_eq!(row.get::<_, Vec<u8>>(0), vec![0x00, 0x9F, 0xFF]);
    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn array_param_binds_for_any() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let engine = PgEngine::new(pool.clone());
    let table = unique_table("any");
    engine
        .execute_raw(
            &format!("CREATE TABLE {table} (id INT4, u UUID, s TEXT)"),
            vec![],
        )
        .await
        .expect("create table");
    let uuids: Vec<String> = (0..3).map(|_| Uuid::new_v4().to_string()).collect();
    for (i, (u, s)) in uuids.iter().zip(["x", "y", "z"]).enumerate() {
        engine
            .execute_raw(
                &format!("INSERT INTO {table} (id, u, s) VALUES ($1, $2, $3)"),
                vec![
                    FilterValue::Int(i as i64 + 1),
                    FilterValue::String(u.clone()),
                    FilterValue::String(s.into()),
                ],
            )
            .await
            .expect("insert");
    }

    let ints = FilterValue::Array(vec![FilterValue::Int(1), FilterValue::Int(3)]);
    let sql = format!("SELECT count(*) FROM {table} WHERE id = ANY($1)");
    assert_eq!(count(&pool, &sql, ints).await, 2);

    let us = FilterValue::Array(vec![FilterValue::String(uuids[0].clone())]);
    let sql = format!("SELECT count(*) FROM {table} WHERE u = ANY($1)");
    assert_eq!(count(&pool, &sql, us).await, 1);

    let ss = FilterValue::Array(vec![FilterValue::String("x".into())]);
    let sql = format!("SELECT count(*) FROM {table} WHERE s = ANY($1)");
    assert_eq!(count(&pool, &sql, ss).await, 1);

    let sql = format!("SELECT count(*) FROM {table} WHERE id = ANY($1)");
    assert_eq!(count(&pool, &sql, FilterValue::Array(vec![])).await, 0);

    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn array_column_round_trips_and_mixed_kinds_error() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let engine = PgEngine::new(pool.clone());
    let table = unique_table("col");
    engine
        .execute_raw(
            &format!("CREATE TABLE {table} (tags TEXT[] NOT NULL, ns INT8[] NOT NULL)"),
            vec![],
        )
        .await
        .expect("create table");
    engine
        .execute_raw(
            &format!("INSERT INTO {table} (tags, ns) VALUES ($1, $2)"),
            vec![
                FilterValue::Array(vec![
                    FilterValue::String("a".into()),
                    FilterValue::String("b\"q".into()),
                ]),
                FilterValue::Array(vec![FilterValue::Int(1), FilterValue::Null]),
            ],
        )
        .await
        .expect("insert arrays");
    let row = pool
        .get()
        .await
        .expect("conn")
        .query_one(&format!("SELECT tags, ns FROM {table}"), &[])
        .await
        .expect("select");
    assert_eq!(row.get::<_, Vec<String>>(0), vec!["a", "b\"q"]);
    assert_eq!(row.get::<_, Vec<Option<i64>>>(1), vec![Some(1), None]);

    let res = engine
        .execute_raw(
            &format!("INSERT INTO {table} (tags, ns) VALUES ($1, $2)"),
            vec![
                FilterValue::Array(vec![FilterValue::Int(1)]),
                FilterValue::Array(vec![]),
            ],
        )
        .await;
    assert!(res.is_err(), "Int element into TEXT[] must error");

    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn bytea_array_round_trips() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let engine = PgEngine::new(pool.clone());
    let table = unique_table("byteaarr");
    engine
        .execute_raw(
            &format!("CREATE TABLE {table} (bs BYTEA[] NOT NULL)"),
            vec![],
        )
        .await
        .expect("create table");
    engine
        .execute_raw(
            &format!("INSERT INTO {table} (bs) VALUES ($1)"),
            vec![FilterValue::Array(vec![
                FilterValue::Bytes(vec![0x00, 0xFF]),
                FilterValue::Bytes(vec![0x9F]),
            ])],
        )
        .await
        .expect("insert bytea[]");
    let row = pool
        .get()
        .await
        .expect("conn")
        .query_one(&format!("SELECT bs FROM {table}"), &[])
        .await
        .expect("select");
    assert_eq!(
        row.get::<_, Vec<Vec<u8>>>(0),
        vec![vec![0x00, 0xFF], vec![0x9F]]
    );
    engine
        .execute_raw(&format!("DROP TABLE IF EXISTS {table}"), vec![])
        .await
        .ok();
}
