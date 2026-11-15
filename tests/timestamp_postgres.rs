//! Live-Postgres regression tests: a model field declared
//! `chrono::DateTime<Utc>` on a `timestamp without time zone` column (what
//! Prisma's `DateTime` maps to) must bind and decode. Before the fix,
//! binding failed ("not a valid ISO-8601 naive datetime for TIMESTAMP:
//! trailing input", from the `Z`) and decoding failed ("error deserializing
//! column"). TIMESTAMP values are UTC wall time; TIMESTAMPTZ is unchanged.
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 cargo test --test timestamp_postgres -- --include-ignored
//! ```

#![cfg(test)]

use std::time::Duration;

mod common;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use common::e2e_url;
use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::traits::QueryEngine;

#[derive(Debug, Model)]
#[prax(table = "ts_pg_events")]
pub struct Event {
    #[prax(id, auto)]
    pub id: i32,
    /// `TIMESTAMP` (without time zone) column.
    pub at: DateTime<Utc>,
    /// `TIMESTAMPTZ` column.
    pub at_tz: DateTime<Utc>,
    pub maybe_at: Option<DateTime<Utc>>,
}

client!(Event);

async fn pool(url: String) -> PgPool {
    PgPoolBuilder::new()
        .url(url)
        .max_connections(2)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, micros: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
        + chrono::Duration::microseconds(micros as i64)
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn generated_client_round_trips_datetime_utc_on_timestamp_columns() {
    let Some(url) = e2e_url() else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let pool = pool(url).await;
    let conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS ts_pg_events ( \
             id SERIAL PRIMARY KEY, \
             at TIMESTAMP NOT NULL, \
             at_tz TIMESTAMPTZ NOT NULL, \
             maybe_at TIMESTAMP)",
    )
    .await
    .expect("create table");

    let client = PraxClient::new(PgEngine::new(pool.clone()));
    let at = utc(2026, 11, 16, 18, 8, 52, 477_200);

    // create: binds DateTime<Utc> into TIMESTAMP, decodes the RETURNING row.
    let created = client
        .event()
        .create()
        .set("at", at)
        .set("at_tz", at)
        .set("maybe_at", at)
        .exec()
        .await
        .expect("create event with DateTime<Utc> on a TIMESTAMP column");
    assert_eq!(created.at, at);
    assert_eq!(created.at_tz, at);
    assert_eq!(created.maybe_at, Some(at));

    // Stored as the UTC wall time; TIMESTAMPTZ is the same instant.
    let row = conn
        .query_one(
            "SELECT at, at_tz, maybe_at FROM ts_pg_events WHERE id = $1",
            &[&created.id],
        )
        .await
        .expect("select");
    assert_eq!(row.get::<_, NaiveDateTime>(0), at.naive_utc());
    assert_eq!(row.get::<_, DateTime<Utc>>(1), at);
    assert_eq!(row.get::<_, Option<NaiveDateTime>>(2), Some(at.naive_utc()));

    // find: decodes TIMESTAMP into DateTime<Utc> as UTC.
    let found = client
        .event()
        .find_unique()
        .r#where(event::id::equals(created.id))
        .exec()
        .await
        .expect("find event");
    assert_eq!(found.at, at);
    assert_eq!(found.at_tz, at);

    // update: binds the new value and decodes the returned rows.
    let later = utc(2027, 1, 2, 3, 4, 5, 6);
    let updated = client
        .event()
        .update()
        .r#where(event::id::equals(created.id))
        .set("at", later)
        .set("maybe_at", FilterValue::Null)
        .exec()
        .await
        .expect("update event");
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].at, later);
    assert_eq!(updated[0].maybe_at, None);
    let row = conn
        .query_one("SELECT at FROM ts_pg_events WHERE id = $1", &[&created.id])
        .await
        .expect("select after update");
    assert_eq!(row.get::<_, NaiveDateTime>(0), later.naive_utc());

    conn.execute("DELETE FROM ts_pg_events WHERE id = $1", &[&created.id])
        .await
        .expect("cleanup");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn raw_binds_convert_offset_datetimes_to_utc_for_timestamp() {
    let Some(url) = e2e_url() else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let pool = pool(url).await;
    let conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS ts_pg_raw ( \
             id SERIAL PRIMARY KEY, at TIMESTAMP NOT NULL, at_tz TIMESTAMPTZ NOT NULL)",
    )
    .await
    .expect("create table");
    let engine = PgEngine::new(pool.clone());

    // +02:00 is converted to UTC for TIMESTAMP; TIMESTAMPTZ keeps the
    // instant; a naive string is stored as-is.
    engine
        .execute_raw(
            "INSERT INTO ts_pg_raw (id, at, at_tz) VALUES (-1, $1, $2), (-2, $3, $2)",
            vec![
                FilterValue::String("2024-01-02T03:04:05+02:00".into()),
                FilterValue::String("2024-01-02T03:04:05+02:00".into()),
                FilterValue::String("2024-01-02T03:04:05".into()),
            ],
        )
        .await
        .expect("insert");
    let rows = conn
        .query(
            "SELECT at, at_tz FROM ts_pg_raw WHERE id IN (-1, -2) ORDER BY id DESC",
            &[],
        )
        .await
        .expect("select");
    let expected_at_tz = utc(2024, 1, 2, 1, 4, 5, 0);
    assert_eq!(
        rows[0].get::<_, NaiveDateTime>(0),
        expected_at_tz.naive_utc()
    );
    assert_eq!(rows[0].get::<_, DateTime<Utc>>(1), expected_at_tz);
    assert_eq!(
        rows[1].get::<_, NaiveDateTime>(0),
        utc(2024, 1, 2, 3, 4, 5, 0).naive_utc()
    );

    // A TIMESTAMP comparison with a DateTime<Utc> parameter matches.
    let n = engine
        .count(
            "SELECT count(*) FROM ts_pg_raw WHERE id = -1 AND at = $1",
            vec![FilterValue::from(expected_at_tz)],
        )
        .await
        .expect("count");
    assert_eq!(n, 1);

    conn.execute("DELETE FROM ts_pg_raw WHERE id IN (-1, -2)", &[])
        .await
        .expect("cleanup");
}
