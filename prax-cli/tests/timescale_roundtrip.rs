//! Docker-gated TimescaleDB round-trip tests (spec §8, §11).
//!
//! Each test introspects a live TimescaleDB through the same calls `prax
//! migrate` uses (`introspect_database` + `schema_from_database` for the
//! structure, `introspect_postgres_catalog` for the extension and Timescale
//! state), generates the migration, applies it with `batch_execute`, then
//! introspects again and asserts the re-diff is **empty**. The `probe_*` tests
//! pin the TimescaleDB behaviour the implementation relies on (spec §8).
//!
//! Every test is `#[ignore]` and self-skips unless `PRAX_E2E=1` and
//! `TIMESCALE_URL` is set, following the workspace convention. Each test owns
//! a fresh schema `t_<name>` (dropped at the end) and the tests run one at a
//! time, because each first drops the `t_*` schemas a failed run left behind.
//! Run via docker compose:
//!
//! ```sh
//! docker compose up -d timescaledb
//! PRAX_E2E=1 TIMESCALE_URL=postgres://prax:prax_test_password@localhost:5434/prax_test \
//!   cargo test -p prax-orm-cli --features postgres --test timescale_roundtrip -- --ignored
//! ```
//!
//! The floor version runs the same way with
//! `TIMESCALE_IMAGE=timescale/timescaledb:2.18.2-pg17 docker compose up -d timescaledb`.

#![cfg(feature = "postgres")]

use prax_cli::commands::introspect::{
    IntrospectionOptions, PostgresCatalog, format_as_prax, introspect_database,
    introspect_postgres_catalog,
};
use prax_cli::commands::schema_from_db::schema_from_database;
use prax_cli::config::Config;
use prax_migrate::timescale::catalog::{ColumnstoreSettings, aggregate_record};
use prax_migrate::timescale::{TimescaleChangeKind, quote_literal};
use prax_migrate::{
    IntrospectionConfig, MigrateResult, MigrationError, SchemaDiff, SchemaDiffer, SqlBackend,
    SqlDialect, TimescaleCatalog,
};
use prax_schema::ast::PgInterval;
use tokio::sync::{Mutex, MutexGuard};
use tokio_postgres::{Client, NoTls};

/// Serializes the tests: each one starts by dropping every leftover `t_*`
/// schema, which would pull the schema out from under a concurrent test.
static SERIAL: Mutex<()> = Mutex::const_new(());

const PROVIDER: &str = "postgresql";
const DAY_MICROS: i64 = 86_400_000_000;
const HOUR_MICROS: i64 = 3_600_000_000;

/// A live TimescaleDB session scoped to one test schema.
struct Ts {
    client: Client,
    url: String,
    schema: String,
    _serial: MutexGuard<'static, ()>,
}

/// The output of a generated migration.
struct Generated {
    up: String,
    skipped: Vec<String>,
    warnings: Vec<String>,
}

/// Database error text including the server message, which
/// `tokio_postgres::Error`'s `Display` omits.
fn pg_err(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{} ({})", db.message(), db.code().code()),
        None => e.to_string(),
    }
}

impl Ts {
    /// Connect and create a fresh schema `t_<name>`, or `None` when the live
    /// suite is not enabled.
    async fn connect(name: &str) -> Option<Self> {
        if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
            eprintln!("skipping: PRAX_E2E not set");
            return None;
        }
        let Ok(url) = std::env::var("TIMESCALE_URL") else {
            eprintln!("skipping: TIMESCALE_URL not set");
            return None;
        };
        let serial = SERIAL.lock().await;
        let (client, connection) = tokio_postgres::connect(&url, NoTls)
            .await
            .expect("connect to TIMESCALE_URL");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        // Policy jobs start running in background workers as soon as they are
        // added and race the end-of-test DROP SCHEMA: on 2.30.2 a retention
        // job hitting a dropped hypertable segfaulted, restarting the server
        // under the next test. The tests only read policy configuration, so
        // stop this database's job scheduler (idempotent; it stays stopped
        // until started again or the server restarts).
        client
            .batch_execute("SELECT _timescaledb_functions.stop_background_workers()")
            .await
            .unwrap_or_else(|e| panic!("stop background workers: {}", pg_err(&e)));

        // A test that panicked earlier leaves its schema behind.
        let leftovers = client
            .query(
                "SELECT nspname::text FROM pg_namespace WHERE nspname LIKE 't\\_%'",
                &[],
            )
            .await
            .expect("list leftover test schemas");
        for row in leftovers {
            let old: String = row.get(0);
            client
                .batch_execute(&format!("DROP SCHEMA \"{old}\" CASCADE"))
                .await
                .unwrap_or_else(|e| panic!("drop leftover schema {old}: {}", pg_err(&e)));
        }

        let schema = format!("t_{name}");
        client
            .batch_execute(&format!(
                "CREATE SCHEMA \"{schema}\"; SET search_path TO \"{schema}\", public"
            ))
            .await
            .unwrap_or_else(|e| panic!("create schema {schema}: {}", pg_err(&e)));
        Some(Self {
            client,
            url,
            schema,
            _serial: serial,
        })
    }

    /// Point the session (search path, introspection and diffs) at another
    /// existing schema.
    async fn use_schema(&mut self, schema: &str) {
        self.exec(&format!("SET search_path TO \"{schema}\", public"))
            .await;
        self.schema = schema.to_string();
    }

    /// Drop the test schema.
    async fn finish(self) {
        self.exec(&format!("DROP SCHEMA \"{}\" CASCADE", self.schema))
            .await;
    }

    /// Run SQL in the test schema, panicking with the server message and the
    /// SQL on failure.
    async fn exec(&self, sql: &str) {
        if let Err(e) = self.client.batch_execute(sql).await {
            panic!("SQL failed: {}\n--- SQL ---\n{sql}", pg_err(&e));
        }
    }

    /// Run SQL in the test schema and return the server message on failure.
    async fn try_exec(&self, sql: &str) -> Result<(), String> {
        self.client.batch_execute(sql).await.map_err(|e| pg_err(&e))
    }

    /// Run a statement inside `BEGIN … ROLLBACK`; the server message if it
    /// failed.
    async fn try_rolled_back(&self, sql: &str) -> Result<(), String> {
        let result = self.try_exec(&format!("BEGIN; {sql};")).await;
        self.exec("ROLLBACK").await;
        result
    }

    /// Run a query returning one `bigint`.
    async fn count(&self, sql: &str) -> i64 {
        self.client
            .query_one(sql, &[])
            .await
            .unwrap_or_else(|e| panic!("query failed: {}\n{sql}", pg_err(&e)))
            .get(0)
    }

    /// Run a query returning one text column.
    async fn strings(&self, sql: &str) -> Vec<String> {
        self.client
            .query(sql, &[])
            .await
            .unwrap_or_else(|e| panic!("query failed: {}\n{sql}", pg_err(&e)))
            .iter()
            .map(|r| r.get(0))
            .collect()
    }

    /// The installed TimescaleDB version.
    async fn version(&self) -> String {
        self.strings("SELECT extversion::text FROM pg_extension WHERE extname = 'timescaledb'")
            .await
            .pop()
            .expect("timescaledb is installed")
    }

    /// Introspect the test schema the way `prax migrate` does.
    async fn introspect(&self) -> (prax_schema::Schema, PostgresCatalog) {
        let options = IntrospectionOptions {
            schema: Some(self.schema.clone()),
            ..Default::default()
        };
        let db = introspect_database(PROVIDER, &self.url, &options)
            .await
            .expect("introspect database");
        let source = schema_from_database(&db, IntrospectionConfig::default())
            .expect("map introspected schema")
            .schema;
        let catalog = introspect_postgres_catalog(PROVIDER, &self.url, Some(&self.schema))
            .await
            .expect("read catalog")
            .expect("postgres provider yields a catalog");
        (source, catalog)
    }

    /// The Timescale catalog of the test schema.
    async fn catalog(&self) -> TimescaleCatalog {
        self.introspect()
            .await
            .1
            .timescale
            .expect("timescaledb is installed")
    }

    /// Diff `prax` against the database, as `generate_migration_sql` does.
    async fn try_diff(&self, prax: &str) -> MigrateResult<SchemaDiff> {
        let target = prax_schema::validate_schema(prax)
            .unwrap_or_else(|e| panic!("target schema validates: {e}\n{prax}"));
        let (source, catalog) = self.introspect().await;
        let mut differ = SchemaDiffer::new(target)
            .with_source(source)
            .with_installed_extensions(catalog.extensions);
        if let Some(ts) = catalog.timescale {
            differ = differ.with_timescale_catalog(ts);
        }
        differ.diff()
    }

    async fn diff(&self, prax: &str) -> SchemaDiff {
        self.try_diff(prax)
            .await
            .unwrap_or_else(|e| panic!("diff: {e}"))
    }

    /// Generate the migration for `prax`, stripping gated changes unless
    /// `allow_destructive`.
    async fn generate(&self, prax: &str, allow_destructive: bool) -> Generated {
        let mut diff = self.diff(prax).await;
        let skipped = if allow_destructive {
            Vec::new()
        } else {
            strip_destructive(&mut diff)
        };
        let migration = SqlDialect::for_backend(SqlBackend::Postgres).generate_migration(&diff);
        Generated {
            up: migration.up,
            skipped,
            warnings: migration.warnings,
        }
    }

    /// Introspect → generate → apply → introspect → diff. Returns the applied
    /// `up` SQL and the re-diff.
    async fn roundtrip(&self, prax: &str, allow_destructive: bool) -> (String, SchemaDiff) {
        let generated = self.generate(prax, allow_destructive).await;
        self.exec(&generated.up).await;
        (generated.up, self.diff(prax).await)
    }

    /// Round-trip `prax` and assert the re-diff is empty.
    async fn apply_clean(&self, prax: &str, allow_destructive: bool) -> String {
        let (up, rediff) = self.roundtrip(prax, allow_destructive).await;
        assert_empty(&rediff, &up);
        up
    }

    /// Convert `limit` chunks of `"Metric"` to the columnstore (all when
    /// `None`).
    async fn convert_chunks(&self, limit: Option<u32>) {
        let limit = limit.map_or_else(String::new, |n| format!(" LIMIT {n}"));
        self.exec(&format!(
            "DO $$ DECLARE c regclass; BEGIN \
             FOR c IN SELECT show_chunks('\"Metric\"'){limit} LOOP \
             CALL convert_to_columnstore(c); END LOOP; END $$"
        ))
        .await;
    }

    /// Number of `"Metric"` chunks in the columnstore.
    async fn compressed_chunks(&self) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM timescaledb_information.chunks \
             WHERE hypertable_schema = '{}' AND hypertable_name = 'Metric' AND is_compressed",
            self.schema
        ))
        .await
    }
}

/// The CLI's `strip_destructive` (private to `commands::migrate`) for these
/// fixtures: they never produce generic drops, which is asserted, so only the
/// gated Timescale changes are stripped.
fn strip_destructive(diff: &mut SchemaDiff) -> Vec<String> {
    let notes = diff.strip_timescale_gated();
    assert!(
        diff.drop_models.is_empty()
            && diff.drop_views.is_empty()
            && diff.drop_indexes.is_empty()
            && diff.drop_enums.is_empty()
            && diff
                .alter_models
                .iter()
                .all(|a| a.drop_fields.is_empty() && a.drop_foreign_keys.is_empty()),
        "fixture produced generic drops: {}",
        diff.summary()
    );
    notes
}

#[track_caller]
fn assert_empty(diff: &SchemaDiff, up: &str) {
    assert!(
        diff.is_empty(),
        "expected an empty re-diff, got: {}\n{:#?}\n--- applied SQL ---\n{up}",
        diff.summary(),
        diff.timescale.changes
    );
}

/// Whether `diff` failed with `InvalidMigration`.
fn is_invalid_migration(diff: &MigrateResult<SchemaDiff>) -> bool {
    matches!(diff, Err(MigrationError::InvalidMigration(_)))
}

/// Major and minor of a version string such as `2.18.2`.
fn major_minor(version: &str) -> (u32, u32) {
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

// ============================================================================
// Fixtures
// ============================================================================

const DATASOURCE: &str = r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [timescaledb]
}
"#;

/// A `Metric` model with `extra` fields and `attrs` model attributes.
fn metric(extra: &str, attrs: &str) -> String {
    format!(
        r#"
model Metric {{
  time     DateTime
  deviceId String
  value    Float
{extra}
  @@id([deviceId, time])
{attrs}
}}
"#
    )
}

/// 100 hourly rows over four devices.
const INSERT_METRICS: &str = "INSERT INTO \"Metric\" (\"time\", \"deviceId\", \"value\") \
     SELECT now() - make_interval(hours => i), 'd' || (i % 4), i \
     FROM generate_series(1, 100) i";

const HOURLY_SQL: &str = r#"
SELECT time_bucket('1 hour', "time") AS bucket, "deviceId", avg("value") AS "avgValue"
FROM "Metric"
GROUP BY 1, 2
"#;

/// The `HourlyMetric` aggregate over `"Metric"` with the given SQL.
fn hourly(sql: &str) -> String {
    hourly_with(
        r#"scheduleInterval: "1 hour", startOffset: "3 days", endOffset: "1 hour""#,
        sql,
    )
}

/// The `HourlyMetric` aggregate with the given policy arguments.
fn hourly_with(policy: &str, sql: &str) -> String {
    format!(
        r#"
view HourlyMetric {{
  bucket   DateTime
  deviceId String
  avgValue Float

  @@timescale.continuous(source: Metric, {policy})
}}

@@sql("HourlyMetric", """{sql}""")
"#
    )
}

/// The `DailyMetric` aggregate over `HourlyMetric`.
const DAILY: &str = r#"
view DailyMetric {
  day      DateTime
  deviceId String
  avgValue Float

  @@timescale.continuous(source: HourlyMetric, scheduleInterval: "1 day", startOffset: "7 days", endOffset: "1 day")
}

@@sql("DailyMetric", """
SELECT time_bucket('1 day', bucket) AS day, "deviceId", avg("avgValue") AS "avgValue"
FROM "HourlyMetric"
GROUP BY 1, 2
""")
"#;

/// The spec §3.1 example.
fn spec_example() -> String {
    let model = metric(
        "",
        r#"  @@timescale.hypertable(time, chunkInterval: "1 day", space: hash(deviceId, 4))
  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "7 days")
  @@timescale.retention(dropAfter: "90 days")"#,
    );
    format!("{DATASOURCE}{model}{}", hourly(HOURLY_SQL))
}

/// A plain `"Metric"` hypertable created with raw SQL.
const RAW_HYPERTABLE: &str = r#"
CREATE TABLE "Metric" ("time" timestamptz NOT NULL, "deviceId" text NOT NULL,
  "value" double precision NOT NULL, PRIMARY KEY ("deviceId", "time"));
SELECT create_hypertable('"Metric"', by_range('time', INTERVAL '1 day'), create_default_indexes => false);
"#;

/// Raw two-level aggregate hierarchy over [`RAW_HYPERTABLE`], with policies.
const RAW_HIERARCHY: &str = r#"
CREATE MATERIALIZED VIEW "HourlyMetric" WITH (timescaledb.continuous) AS
SELECT time_bucket('1 hour', "time") AS bucket, "deviceId", avg("value") AS "avgValue"
FROM "Metric" GROUP BY 1, 2 WITH NO DATA;
CREATE MATERIALIZED VIEW "DailyMetric" WITH (timescaledb.continuous) AS
SELECT time_bucket('1 day', bucket) AS day, "deviceId", avg("avgValue") AS "avgValue"
FROM "HourlyMetric" GROUP BY 1, 2 WITH NO DATA;
SELECT add_continuous_aggregate_policy('"HourlyMetric"', start_offset => INTERVAL '3 days',
  end_offset => INTERVAL '1 hour', schedule_interval => INTERVAL '1 hour');
SELECT add_continuous_aggregate_policy('"DailyMetric"', start_offset => INTERVAL '7 days',
  end_offset => INTERVAL '1 day', schedule_interval => INTERVAL '1 day');
"#;

// ============================================================================
// Round trips
// ============================================================================

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn roundtrip_spec_example_is_empty() {
    let Some(ts) = Ts::connect("roundtrip_spec_example_is_empty").await else {
        return;
    };
    let up = ts.apply_clean(&spec_example(), false).await;
    assert!(up.contains("create_hypertable"), "{up}");
    assert!(up.contains("add_retention_policy"), "{up}");
    assert!(up.contains("add_continuous_aggregate_policy"), "{up}");

    let catalog = ts.catalog().await;
    let metric = &catalog.hypertables["Metric"];
    assert_eq!(metric.space, Some(("deviceId".to_string(), 4)));
    assert!(metric.retention.is_some());
    assert!(
        metric
            .columnstore
            .as_ref()
            .is_some_and(|c| c.after.is_some())
    );
    let hourly = &catalog.aggregates["HourlyMetric"];
    assert_eq!(hourly.source, "Metric");
    assert!(hourly.sql.is_some() && hourly.policy.is_some());
    ts.finish().await;
}

/// The same hypertable and aggregate names in two schemas, with different
/// chunk, columnstore, retention and refresh settings: each schema's catalog
/// sees only its own (the jobs and columnstore-settings queries are scoped to
/// the introspected schema) and each re-diffs empty.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn catalog_queries_are_schema_scoped() {
    let Some(mut ts) = Ts::connect("catalog_queries_are_schema_scoped").await else {
        return;
    };
    let main = ts.schema.clone();
    let other = format!("{main}_other");
    ts.exec(&format!("CREATE SCHEMA \"{other}\"")).await;

    let main_prax = spec_example();
    let other_model = metric(
        "",
        r#"  @@timescale.hypertable(time, chunkInterval: "2 days")
  @@timescale.columnstore(orderBy: [desc(time)], after: "3 days")
  @@timescale.retention(dropAfter: "30 days")"#,
    );
    let other_prax = format!(
        "{DATASOURCE}{other_model}{}",
        hourly_with(
            r#"scheduleInterval: "2 hours", startOffset: "2 days", endOffset: "2 hours""#,
            HOURLY_SQL
        )
    );

    // Apply each schema, then re-diff both with the other one present.
    ts.use_schema(&other).await;
    ts.apply_clean(&other_prax, false).await;
    ts.use_schema(&main).await;
    ts.apply_clean(&main_prax, false).await;
    ts.use_schema(&other).await;
    assert_empty(&ts.diff(&other_prax).await, "(other schema re-diff)");
    let other_catalog = ts.catalog().await;
    ts.use_schema(&main).await;
    assert_empty(&ts.diff(&main_prax).await, "(main schema re-diff)");
    let main_catalog = ts.catalog().await;

    let main_metric = &main_catalog.hypertables["Metric"];
    let other_metric = &other_catalog.hypertables["Metric"];
    assert_eq!(main_metric.retention, Some(PgInterval::new(0, 90, 0)));
    assert_eq!(other_metric.retention, Some(PgInterval::new(0, 30, 0)));
    assert_eq!(
        main_metric
            .columnstore
            .as_ref()
            .map(|c| (&c.settings.segment_by, c.after)),
        Some((
            &vec!["deviceId".to_string()],
            Some(PgInterval::new(0, 7, 0))
        ))
    );
    assert_eq!(
        other_metric
            .columnstore
            .as_ref()
            .map(|c| (&c.settings.segment_by, c.after)),
        Some((&Vec::new(), Some(PgInterval::new(0, 3, 0))))
    );
    let start = |catalog: &TimescaleCatalog| {
        catalog.aggregates["HourlyMetric"]
            .policy
            .as_ref()
            .and_then(|p| p.start_offset)
    };
    assert_eq!(start(&main_catalog), Some(PgInterval::new(0, 3, 0)));
    assert_eq!(start(&other_catalog), Some(PgInterval::new(0, 2, 0)));

    ts.exec(&format!("DROP SCHEMA \"{other}\" CASCADE")).await;
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn policy_changes_roundtrip() {
    let Some(ts) = Ts::connect("policy_changes_roundtrip").await else {
        return;
    };
    ts.apply_clean(&spec_example(), false).await;

    // Lengthened retention, a new columnstore `after` and a new refresh
    // policy: all ungated, each a remove followed by an add.
    let model = metric(
        "",
        r#"  @@timescale.hypertable(time, chunkInterval: "1 day", space: hash(deviceId, 4))
  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "14 days")
  @@timescale.retention(dropAfter: "120 days")"#,
    );
    let changed = format!(
        "{DATASOURCE}{model}{}",
        hourly_with(
            r#"scheduleInterval: "2 hours", startOffset: "5 days", endOffset: "2 hours", materializedOnly: false"#,
            HOURLY_SQL
        )
    );
    let generated = ts.generate(&changed, false).await;
    assert!(generated.skipped.is_empty(), "{:?}", generated.skipped);
    for call in [
        "remove_columnstore_policy",
        "add_columnstore_policy",
        "remove_retention_policy",
        "add_retention_policy",
        "remove_continuous_aggregate_policy",
        "add_continuous_aggregate_policy",
        "timescaledb.materialized_only = false",
    ] {
        assert!(
            generated.up.contains(call),
            "missing {call}:\n{}",
            generated.up
        );
    }
    ts.exec(&generated.up).await;
    assert_empty(&ts.diff(&changed).await, &generated.up);

    let catalog = ts.catalog().await;
    let metric = &catalog.hypertables["Metric"];
    assert_eq!(metric.retention, Some(PgInterval::new(0, 120, 0)));
    assert_eq!(
        metric.columnstore.as_ref().and_then(|c| c.after),
        Some(PgInterval::new(0, 14, 0))
    );
    let hourly = &catalog.aggregates["HourlyMetric"];
    assert!(!hourly.materialized_only);
    assert_eq!(
        hourly.policy.as_ref().map(|p| p.schedule_interval),
        Some(PgInterval::new(0, 0, 2 * HOUR_MICROS))
    );
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn roundtrip_map_and_mixed_case_is_empty() {
    let Some(ts) = Ts::connect("roundtrip_map_and_mixed_case_is_empty").await else {
        return;
    };
    let prax = format!(
        r#"{DATASOURCE}
model SensorReading {{
  recordedAt   DateTime @map("recorded_at")
  sensorId     String
  siteCode     String   @map("site_code")
  readingValue Float    @map("ReadingValue")

  @@id([sensorId, recordedAt])
  @@map("sensor_readings")
  @@timescale.hypertable(recordedAt, chunkInterval: "12 hours", space: hash(sensorId, 2))
  @@timescale.columnstore(segmentBy: [siteCode], orderBy: [recordedAt, desc(readingValue)], after: "3 days")
  @@timescale.retention(dropAfter: "30 days")
}}

view SiteHourly {{
  bucket   DateTime
  siteCode String   @map("site_code")
  maxValue Float

  @@map("site_hourly")
  @@timescale.continuous(source: SensorReading, scheduleInterval: "30 minutes", startOffset: "2 days", endOffset: "1 hour", materializedOnly: false)
}}

@@sql("SiteHourly", """
SELECT time_bucket('1 hour', recorded_at) AS bucket, site_code, max("ReadingValue") AS "maxValue"
FROM sensor_readings
GROUP BY 1, 2
""")
"#
    );
    ts.apply_clean(&prax, false).await;

    let catalog = ts.catalog().await;
    let table = &catalog.hypertables["sensor_readings"];
    assert_eq!(table.time_column, "recorded_at");
    assert_eq!(table.space, Some(("sensorId".to_string(), 2)));
    assert_eq!(
        table.columnstore.as_ref().map(|c| &c.settings),
        Some(&ColumnstoreSettings {
            segment_by: vec!["site_code".to_string()],
            order_by: vec![
                ("recorded_at".to_string(), false),
                ("ReadingValue".to_string(), true)
            ],
        })
    );
    let site = &catalog.aggregates["site_hourly"];
    assert_eq!(site.source, "sensor_readings");
    assert!(!site.materialized_only);
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn convert_populated_table() {
    let Some(ts) = Ts::connect("convert_populated_table").await else {
        return;
    };
    let plain = format!("{DATASOURCE}{}", metric("", ""));
    ts.apply_clean(&plain, false).await;
    ts.exec(INSERT_METRICS).await;

    let hyper = format!(
        "{DATASOURCE}{}",
        metric(
            "",
            r#"  @@timescale.hypertable(time, chunkInterval: "1 day")"#
        )
    );
    let generated = ts.generate(&hyper, false).await;
    assert!(
        generated.up.contains("migrate_data => true"),
        "{}",
        generated.up
    );
    assert!(
        generated
            .warnings
            .iter()
            .any(|w| w.contains("Converting existing table")),
        "{:?}",
        generated.warnings
    );
    ts.exec(&generated.up).await;
    assert_eq!(ts.count("SELECT count(*) FROM \"Metric\"").await, 100);
    assert!(
        ts.count("SELECT count(*) FROM show_chunks('\"Metric\"')")
            .await
            > 1
    );
    assert_empty(&ts.diff(&hyper).await, &generated.up);
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn retention_add_skipped_then_applied() {
    let Some(ts) = Ts::connect("retention_add_skipped_then_applied").await else {
        return;
    };
    let hypertable = r#"  @@timescale.hypertable(time, chunkInterval: "1 day")"#;
    ts.apply_clean(&format!("{DATASOURCE}{}", metric("", hypertable)), false)
        .await;

    let with_retention = format!(
        "{DATASOURCE}{}",
        metric(
            "",
            &format!("{hypertable}\n  @@timescale.retention(dropAfter: \"30 days\")")
        )
    );
    let skipped = ts.generate(&with_retention, false).await;
    assert!(
        !skipped.up.contains("add_retention_policy"),
        "{}",
        skipped.up
    );
    assert!(
        skipped
            .skipped
            .iter()
            .any(|n| n.contains("retention") && n.contains("Metric")),
        "{:?}",
        skipped.skipped
    );
    ts.exec(&skipped.up).await;
    let pending = ts.diff(&with_retention).await;
    assert!(
        pending
            .timescale
            .changes
            .iter()
            .any(|c| matches!(c.kind, TimescaleChangeKind::ReplaceRetentionPolicy { .. })),
        "{:#?}",
        pending.timescale.changes
    );

    let up = ts.apply_clean(&with_retention, true).await;
    assert!(up.contains("add_retention_policy"), "{up}");
    assert_eq!(
        ts.catalog().await.hypertables["Metric"].retention,
        Some(PgInterval::new(0, 30, 0))
    );
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn columnstore_settings_change_and_disable() {
    let Some(ts) = Ts::connect("columnstore_settings_change_and_disable").await else {
        return;
    };
    let model = |columnstore: &str| {
        format!(
            "{DATASOURCE}{}",
            metric(
                "  region   String?\n",
                &format!("  @@timescale.hypertable(time, chunkInterval: \"1 day\")\n{columnstore}")
            )
        )
    };
    let by_device = model(
        r#"  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "30 days")"#,
    );
    ts.apply_clean(&by_device, false).await;
    ts.exec(INSERT_METRICS).await;

    let by_region = model(
        r#"  @@timescale.columnstore(segmentBy: [region], orderBy: [desc(time)], after: "30 days")"#,
    );
    let up = ts.apply_clean(&by_region, false).await;
    assert!(up.contains("timescaledb.segmentby = '\"region\"'"), "{up}");

    ts.convert_chunks(Some(1)).await;
    assert_eq!(ts.compressed_chunks().await, 1);

    let without = model("");
    let gated = ts.generate(&without, false).await;
    assert!(!gated.up.contains("convert_to_rowstore"), "{}", gated.up);
    assert!(!gated.skipped.is_empty());

    let up = ts.apply_clean(&without, true).await;
    assert!(up.contains("convert_to_rowstore"), "{up}");
    assert_eq!(ts.compressed_chunks().await, 0);
    assert_eq!(ts.count("SELECT count(*) FROM \"Metric\"").await, 100);
    assert!(
        ts.catalog().await.hypertables["Metric"]
            .columnstore
            .is_none()
    );
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn aggregate_sql_change_skipped_then_recreated() {
    let Some(ts) = Ts::connect("aggregate_sql_change_skipped_then_recreated").await else {
        return;
    };
    let model = metric(
        "",
        r#"  @@timescale.hypertable(time, chunkInterval: "1 day")"#,
    );
    let v1 = format!("{DATASOURCE}{model}{}{DAILY}", hourly(HOURLY_SQL));
    ts.apply_clean(&v1, false).await;
    ts.exec(INSERT_METRICS).await;

    let changed_sql = r#"
SELECT time_bucket('1 hour', "time") AS bucket, "deviceId", avg("value") AS "avgValue"
FROM "Metric"
WHERE "value" >= 0
GROUP BY 1, 2
"#;
    let v2 = format!("{DATASOURCE}{model}{}{DAILY}", hourly(changed_sql));

    let skipped = ts.generate(&v2, false).await;
    assert!(
        !skipped.up.contains("DROP MATERIALIZED VIEW"),
        "{}",
        skipped.up
    );
    assert!(
        skipped.skipped.iter().any(|n| n.contains("HourlyMetric")),
        "{:?}",
        skipped.skipped
    );
    ts.exec(&skipped.up).await;
    assert!(!ts.diff(&v2).await.is_empty());

    let up = ts.apply_clean(&v2, true).await;
    let position = |needle: &str| {
        up.find(needle)
            .unwrap_or_else(|| panic!("missing {needle}:\n{up}"))
    };
    assert!(
        position("DROP MATERIALIZED VIEW IF EXISTS \"DailyMetric\"")
            < position("DROP MATERIALIZED VIEW IF EXISTS \"HourlyMetric\"")
    );
    assert!(
        position("CREATE MATERIALIZED VIEW \"HourlyMetric\"")
            < position("CREATE MATERIALIZED VIEW \"DailyMetric\"")
    );
    let catalog = ts.catalog().await;
    assert!(
        catalog.aggregates["HourlyMetric"]
            .sql
            .as_deref()
            .is_some_and(|s| s.contains("WHERE"))
    );
    assert_eq!(catalog.aggregates["DailyMetric"].source, "HourlyMetric");
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn aggregate_record_survives_hostile_sql() {
    let Some(ts) = Ts::connect("aggregate_record_survives_hostile_sql").await else {
        return;
    };
    let hostile = r#"
SELECT time_bucket('1 hour', "time") AS bucket, "deviceId", avg("value") AS "avgValue"
FROM "Metric"
WHERE "deviceId" <> 'it''s a "quoted" $$ value'
GROUP BY 1, 2
"#;
    let prax = format!(
        "{DATASOURCE}{}{}",
        metric(
            "",
            r#"  @@timescale.hypertable(time, chunkInterval: "1 day")"#
        ),
        hourly(hostile)
    );
    ts.apply_clean(&prax, false).await;
    // The record holds the SQL exactly as the schema resolves it.
    let target = prax_schema::validate_schema(&prax).expect("schema validates");
    let authored = target.views["HourlyMetric"].resolved_sql(&target);
    assert!(authored.is_some_and(|s| s.contains("'it''s a \"quoted\" $$ value'\n")));
    assert_eq!(
        ts.catalog().await.aggregates["HourlyMetric"].sql.as_deref(),
        authored
    );
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn plain_view_to_aggregate() {
    let Some(ts) = Ts::connect("plain_view_to_aggregate").await else {
        return;
    };
    let model = metric(
        "",
        r#"  @@timescale.hypertable(time, chunkInterval: "1 day")"#,
    );
    ts.apply_clean(&format!("{DATASOURCE}{model}"), false).await;
    ts.exec(
        "CREATE VIEW \"HourlyMetric\" AS SELECT \"deviceId\", avg(\"value\") AS \"avgValue\" \
         FROM \"Metric\" GROUP BY 1",
    )
    .await;

    let up = ts
        .apply_clean(&format!("{DATASOURCE}{model}{}", hourly(HOURLY_SQL)), false)
        .await;
    assert!(
        up.find("DROP VIEW IF EXISTS \"HourlyMetric\"")
            < up.find("CREATE MATERIALIZED VIEW \"HourlyMetric\""),
        "{up}"
    );
    assert!(ts.catalog().await.aggregates.contains_key("HourlyMetric"));
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn source_column_type_change_recreates_aggregate() {
    let Some(ts) = Ts::connect("source_column_type_change_recreates_aggregate").await else {
        return;
    };
    let model = |value_type: &str| {
        format!(
            r#"
model Metric {{
  time     DateTime
  deviceId String
  value    {value_type}

  @@id([deviceId, time])
  @@timescale.hypertable(time, chunkInterval: "1 day")
}}
"#
        )
    };
    let v1 = format!("{DATASOURCE}{}{}", model("Int"), hourly(HOURLY_SQL));
    ts.apply_clean(&v1, false).await;
    ts.exec(INSERT_METRICS).await;

    let v2 = format!("{DATASOURCE}{}{}", model("BigInt"), hourly(HOURLY_SQL));
    let skipped = ts.generate(&v2, false).await;
    assert!(!skipped.up.contains("ALTER COLUMN"), "{}", skipped.up);
    assert!(!skipped.skipped.is_empty());

    let up = ts.apply_clean(&v2, true).await;
    assert!(
        up.contains("DROP MATERIALIZED VIEW IF EXISTS \"HourlyMetric\""),
        "{up}"
    );
    assert!(
        up.contains("CREATE MATERIALIZED VIEW \"HourlyMetric\""),
        "{up}"
    );
    assert_eq!(ts.count("SELECT count(*) FROM \"Metric\"").await, 100);
    ts.finish().await;
}

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn db_pull_output_rediffs_empty() {
    let Some(ts) = Ts::connect("db_pull_output_rediffs_empty").await else {
        return;
    };
    ts.apply_clean(&spec_example(), false).await;

    let options = IntrospectionOptions {
        schema: Some(ts.schema.clone()),
        ..Default::default()
    };
    let db = introspect_database(PROVIDER, &ts.url, &options)
        .await
        .expect("introspect database");
    let timescale = introspect_postgres_catalog(PROVIDER, &ts.url, Some(&ts.schema))
        .await
        .expect("read catalog")
        .and_then(|c| c.timescale);
    let pulled = format_as_prax(
        &db,
        &Config::default_for_provider(PROVIDER),
        timescale.as_ref(),
    );
    assert!(pulled.contains("@@timescale.hypertable"), "{pulled}");
    assert!(pulled.contains("@@timescale.continuous"), "{pulled}");
    if let Err(e) = prax_schema::validate_schema(&pulled) {
        panic!("pulled schema does not validate: {e}\n{pulled}");
    }
    let rediff = ts.diff(&pulled).await;
    assert_empty(&rediff, &pulled);
    ts.finish().await;
}

// ============================================================================
// Spec §8 probes
// ============================================================================

/// §8 item 1: `hypertable_columnstore_settings` has `hypertable`, `segmentby`
/// and `orderby`, and the catalog read decodes them.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_columnstore_settings_view_shape() {
    let Some(ts) = Ts::connect("probe_columnstore_settings_view_shape").await else {
        return;
    };
    let columns = ts
        .strings(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_schema = 'timescaledb_information' \
             AND table_name = 'hypertable_columnstore_settings' ORDER BY ordinal_position",
        )
        .await;
    eprintln!("hypertable_columnstore_settings columns: {columns:?}");
    for expected in ["hypertable", "segmentby", "orderby"] {
        assert!(columns.iter().any(|c| c == expected), "{columns:?}");
    }

    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(
        "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, \
         timescaledb.segmentby = '\"deviceId\"', timescaledb.orderby = '\"time\" DESC')",
    )
    .await;
    let raw = ts
        .strings(&format!(
            "SELECT concat_ws(' | ', hypertable::text, segmentby, orderby) \
             FROM timescaledb_information.hypertable_columnstore_settings \
             WHERE hypertable = '\"{}\".\"Metric\"'::regclass",
            ts.schema
        ))
        .await;
    eprintln!("hypertable_columnstore_settings row: {raw:?}");
    let catalog = ts.catalog().await;
    assert_eq!(
        catalog.hypertables["Metric"]
            .columnstore
            .as_ref()
            .map(|c| &c.settings),
        Some(&ColumnstoreSettings {
            segment_by: vec!["deviceId".to_string()],
            order_by: vec![("time".to_string(), true)],
        })
    );
    ts.finish().await;
}

/// §8 item 2: enabling the columnstore with `ALTER TABLE` adds no policy.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_alter_table_adds_no_implicit_policy() {
    let Some(ts) = Ts::connect("probe_alter_table_adds_no_implicit_policy").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(
        "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, \
         timescaledb.segmentby = '', timescaledb.orderby = '\"time\" DESC')",
    )
    .await;
    let jobs = ts
        .count(&format!(
            "SELECT count(*) FROM timescaledb_information.jobs \
             WHERE hypertable_schema = '{}' AND hypertable_name = 'Metric'",
            ts.schema
        ))
        .await;
    assert_eq!(jobs, 0, "ALTER TABLE … enable_columnstore added a job");
    let catalog = ts.catalog().await;
    let columnstore = catalog.hypertables["Metric"]
        .columnstore
        .clone()
        .expect("columnstore enabled");
    assert_eq!(columnstore.after, None);
    assert!(columnstore.settings.segment_by.is_empty());
    ts.finish().await;
}

/// §8 item 3: `add_columnstore_policy` is a procedure taking `after`, stored
/// as `compress_after`.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_add_columnstore_policy_after_param() {
    let Some(ts) = Ts::connect("probe_add_columnstore_policy_after_param").await else {
        return;
    };
    let kinds = ts
        .strings(
            "SELECT proname || ':' || prokind::text FROM pg_proc \
             WHERE proname IN ('add_columnstore_policy', 'remove_columnstore_policy') \
             ORDER BY 1",
        )
        .await;
    assert_eq!(
        kinds,
        ["add_columnstore_policy:p", "remove_columnstore_policy:p"]
    );
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(
        "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, \
         timescaledb.segmentby = '\"deviceId\"', timescaledb.orderby = '\"time\" DESC'); \
         CALL add_columnstore_policy('\"Metric\"', after => INTERVAL '7 days')",
    )
    .await;
    let config = ts
        .strings(&format!(
            "SELECT config::text FROM timescaledb_information.jobs \
             WHERE hypertable_schema = '{}' AND proc_name = 'policy_compression'",
            ts.schema
        ))
        .await;
    eprintln!("policy_compression config: {config:?}");
    assert!(config[0].contains("\"compress_after\""), "{config:?}");
    let catalog = ts.catalog().await;
    assert_eq!(
        catalog.hypertables["Metric"]
            .columnstore
            .as_ref()
            .and_then(|c| c.after),
        Some(PgInterval::new(0, 7, 0))
    );
    ts.exec("CALL remove_columnstore_policy('\"Metric\"', if_exists => true)")
        .await;
    ts.finish().await;
}

/// §8 item 4: foreign keys referencing a hypertable are accepted on every
/// supported version, so validation rule 6 was dropped: a schema with such a
/// relation validates, applies and round-trips.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_fk_to_hypertable_accepted() {
    let Some(ts) = Ts::connect("probe_fk_to_hypertable_accepted").await else {
        return;
    };
    let prax = format!(
        r#"{DATASOURCE}
model Metric {{
  time     DateTime
  deviceId String
  value    Float
  alerts   Alert[]

  @@id([deviceId, time])
  @@timescale.hypertable(time, chunkInterval: "1 day")
}}

model Alert {{
  id       Int      @id
  deviceId String
  time     DateTime
  metric   Metric   @relation(fields: [deviceId, time], references: [deviceId, time], map: "Alert_metric_fkey")
}}
"#
    );
    ts.apply_clean(&prax, false).await;

    // A foreign key added after the table is already a hypertable.
    let raw = ts
        .try_exec(
            "CREATE TABLE \"RawAlert\" (id int PRIMARY KEY, \"deviceId\" text NOT NULL, \
             \"time\" timestamptz NOT NULL, \
             FOREIGN KEY (\"deviceId\", \"time\") REFERENCES \"Metric\" (\"deviceId\", \"time\"))",
        )
        .await;
    eprintln!("FK to an existing hypertable: {raw:?}");
    assert_eq!(raw, Ok(()));
    ts.finish().await;
}

/// §8 item 5: a column cannot be in both `segmentby` and `orderby` (rule 8).
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_segmentby_orderby_overlap_rejected() {
    let Some(ts) = Ts::connect("probe_segmentby_orderby_overlap_rejected").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    let result = ts
        .try_exec(
            "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, \
             timescaledb.segmentby = '\"deviceId\"', \
             timescaledb.orderby = '\"deviceId\", \"time\" DESC')",
        )
        .await;
    eprintln!("segmentby/orderby overlap: {result:?}");
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("both ordering and segmenting")),
        "{result:?}"
    );
    let overlapping = format!(
        "{DATASOURCE}{}",
        metric(
            "",
            "  @@timescale.hypertable(time)\n  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [deviceId, desc(time)])"
        )
    );
    assert!(prax_schema::validate_schema(&overlapping).is_err());
    ts.finish().await;
}

/// §8 item 6: chunk intervals decode to the same `fixed_micros()` whichever
/// unit they were declared in, so the differ compares them that way.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_chunk_interval_decodes_fixed() {
    let Some(ts) = Ts::connect("probe_chunk_interval_decodes_fixed").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(
        "CREATE TABLE \"Hours\" (\"time\" timestamptz NOT NULL); \
         SELECT create_hypertable('\"Hours\"', by_range('time', INTERVAL '24 hours'), \
           create_default_indexes => false)",
    )
    .await;
    let catalog = ts.catalog().await;
    let day = catalog.hypertables["Metric"].chunk_interval;
    let hours = catalog.hypertables["Hours"].chunk_interval;
    eprintln!("chunk intervals: '1 day' -> {day:?}, '24 hours' -> {hours:?}");
    assert_eq!(day.fixed_micros(), Some(DAY_MICROS));
    assert_eq!(hours.fixed_micros(), Some(DAY_MICROS));

    // A schema declaring the other unit does not churn.
    let declared = format!(
        "{DATASOURCE}{}",
        metric(
            "",
            r#"  @@timescale.hypertable(time, chunkInterval: "24 hours")"#
        )
    );
    let diff = ts.diff(&declared).await;
    assert!(
        !diff
            .timescale
            .changes
            .iter()
            .any(|c| matches!(c.kind, TimescaleChangeKind::SetChunkInterval { .. })),
        "{:#?}",
        diff.timescale.changes
    );
    ts.finish().await;
}

/// §8 item 7: policy intervals keep their `(months, days, micros)` parts in
/// `jobs.config` (`1 day` is not stored as `24:00:00`), so they compare
/// structurally.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_policy_interval_parts_preserved() {
    let Some(ts) = Ts::connect("probe_policy_interval_parts_preserved").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(
        "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, \
         timescaledb.segmentby = '\"deviceId\"', timescaledb.orderby = '\"time\" DESC'); \
         CALL add_columnstore_policy('\"Metric\"', after => INTERVAL '2 days 12 hours'); \
         SELECT add_retention_policy('\"Metric\"', drop_after => INTERVAL '1 day')",
    )
    .await;
    ts.exec(RAW_HIERARCHY).await;

    let catalog = ts.catalog().await;
    let metric = &catalog.hypertables["Metric"];
    eprintln!(
        "retention {:?}, columnstore after {:?}",
        metric.retention,
        metric.columnstore.as_ref().and_then(|c| c.after)
    );
    assert_eq!(metric.retention, Some(PgInterval::new(0, 1, 0)));
    assert_eq!(
        metric.columnstore.as_ref().and_then(|c| c.after),
        Some(PgInterval::new(0, 2, 12 * HOUR_MICROS))
    );
    let policy = catalog.aggregates["HourlyMetric"]
        .policy
        .clone()
        .expect("refresh policy");
    assert_eq!(policy.start_offset, Some(PgInterval::new(0, 3, 0)));
    assert_eq!(policy.end_offset, Some(PgInterval::new(0, 0, HOUR_MICROS)));
    assert_eq!(policy.schedule_interval, PgInterval::new(0, 0, HOUR_MICROS));

    ts.exec(
        "SELECT remove_retention_policy('\"Metric\"'); \
         SELECT add_retention_policy('\"Metric\"', drop_after => INTERVAL '24 hours')",
    )
    .await;
    assert_eq!(
        ts.catalog().await.hypertables["Metric"].retention,
        Some(PgInterval::new(0, 0, 24 * HOUR_MICROS))
    );
    ts.finish().await;
}

/// §8 item 8: `COMMENT ON VIEW` and `obj_description` work on an aggregate's
/// view, including a multi-line comment.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_comment_on_aggregate_view() {
    let Some(ts) = Ts::connect("probe_comment_on_aggregate_view").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(RAW_HIERARCHY).await;
    let sql = "SELECT 1\nFROM \"Metric\" -- it's \"multi-line\" $$";
    ts.exec(&format!(
        "COMMENT ON VIEW \"HourlyMetric\" IS {}",
        quote_literal(&aggregate_record(sql))
    ))
    .await;
    let catalog = ts.catalog().await;
    assert_eq!(catalog.aggregates["HourlyMetric"].sql.as_deref(), Some(sql));
    assert_eq!(catalog.aggregates["DailyMetric"].sql, None);
    ts.finish().await;
}

/// §8 item 9: the generated columnstore teardown (`CALL convert_to_rowstore`
/// in a `DO` loop over `show_chunks`) works inside a transaction.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_convert_to_rowstore_in_do_block() {
    let Some(ts) = Ts::connect("probe_convert_to_rowstore_in_do_block").await else {
        return;
    };
    let hypertable = r#"  @@timescale.hypertable(time, chunkInterval: "1 day")"#;
    let with_columnstore = format!(
        "{DATASOURCE}{}",
        metric(
            "",
            &format!(
                "{hypertable}\n  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)])"
            )
        )
    );
    ts.apply_clean(&with_columnstore, false).await;
    ts.exec(INSERT_METRICS).await;
    ts.convert_chunks(None).await;
    assert!(ts.compressed_chunks().await > 1);

    let without = format!("{DATASOURCE}{}", metric("", hypertable));
    let generated = ts.generate(&without, true).await;
    assert!(
        generated.up.contains("DO $prax$") && generated.up.contains("convert_to_rowstore"),
        "{}",
        generated.up
    );
    let applied = ts
        .try_exec(&format!("BEGIN;\n{}\nCOMMIT;", generated.up))
        .await;
    eprintln!("columnstore teardown in a transaction: {applied:?}");
    assert_eq!(applied, Ok(()), "{}", generated.up);
    assert_eq!(ts.compressed_chunks().await, 0);
    assert_eq!(ts.count("SELECT count(*) FROM \"Metric\"").await, 100);
    assert_empty(&ts.diff(&without).await, &generated.up);
    ts.finish().await;
}

/// §8 item 10: which column alters TimescaleDB rejects on a hypertable with
/// the columnstore enabled and a converted chunk, and that the differ refuses
/// exactly those with `InvalidMigration`.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_columnstore_alter_column_restrictions() {
    let Some(ts) = Ts::connect("probe_columnstore_alter_column_restrictions").await else {
        return;
    };
    let model_with = |extra: &str, segment_by: &str| {
        format!(
            "{DATASOURCE}{}",
            metric(
                extra,
                &format!(
                    "  @@timescale.hypertable(time, chunkInterval: \"1 day\")\n  \
                     @@timescale.columnstore({segment_by}orderBy: [desc(time)])"
                )
            )
        )
    };
    let model = |extra: &str| model_with(extra, "segmentBy: [region], ");
    let base = "  region   String?\n  note     String?\n  extra    Int?\n";
    ts.apply_clean(&model(base), false).await;
    ts.exec(INSERT_METRICS).await;
    ts.exec("UPDATE \"Metric\" SET region = 'r', extra = 1")
        .await;
    ts.convert_chunks(Some(1)).await;
    assert_eq!(ts.compressed_chunks().await, 1);

    let version = ts.version().await;
    let alter_type = ts
        .try_rolled_back("ALTER TABLE \"Metric\" ALTER COLUMN \"value\" TYPE real")
        .await;
    let drop_segmentby = ts
        .try_rolled_back("ALTER TABLE \"Metric\" DROP COLUMN \"region\"")
        .await;
    let drop_other = ts
        .try_rolled_back("ALTER TABLE \"Metric\" DROP COLUMN \"note\"")
        .await;
    let set_not_null = ts
        .try_rolled_back("ALTER TABLE \"Metric\" ALTER COLUMN \"extra\" SET NOT NULL")
        .await;
    let drop_not_null = ts
        .try_rolled_back("ALTER TABLE \"Metric\" ALTER COLUMN \"note\" DROP NOT NULL")
        .await;
    eprintln!(
        "TimescaleDB {version}: TYPE {alter_type:?}; DROP segmentby column {drop_segmentby:?}; \
         DROP other column {drop_other:?}; SET NOT NULL {set_not_null:?}; \
         DROP NOT NULL {drop_not_null:?}"
    );
    assert!(alter_type.is_err());
    assert!(drop_segmentby.is_err());
    assert_eq!(drop_other, Ok(()));
    assert_eq!(drop_not_null, Ok(()));
    // SET NOT NULL is rejected on 2.18 and accepted on 2.30.
    assert_eq!(set_not_null.is_err(), major_minor(&version) < (2, 30));

    // The differ refuses the rejected alters and allows the others.
    let type_change = model("  region   String?\n  note     String?\n  extra    BigInt?\n");
    assert!(is_invalid_migration(&ts.try_diff(&type_change).await));
    let drop_region = model_with("  note     String?\n  extra    Int?\n", "");
    assert!(is_invalid_migration(&ts.try_diff(&drop_region).await));
    let drop_note = model("  region   String?\n  extra    Int?\n");
    assert!(ts.try_diff(&drop_note).await.is_ok());
    let required = model("  region   String?\n  note     String?\n  extra    Int\n");
    assert_eq!(
        is_invalid_migration(&ts.try_diff(&required).await),
        set_not_null.is_err()
    );
    ts.finish().await;
}

/// §8 item 11: a hierarchical aggregate's `hypertable_name` is its parent's
/// materialization hypertable, so the catalog maps it back to the parent view.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_hierarchical_parent_mapping() {
    let Some(ts) = Ts::connect("probe_hierarchical_parent_mapping").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(RAW_HIERARCHY).await;
    let matched = ts
        .count(&format!(
            "SELECT count(*) FROM timescaledb_information.continuous_aggregates child \
             JOIN timescaledb_information.continuous_aggregates parent \
               ON child.hypertable_name = parent.materialization_hypertable_name \
             WHERE child.view_schema = '{0}' AND parent.view_schema = '{0}' \
               AND child.view_name = 'DailyMetric' AND parent.view_name = 'HourlyMetric'",
            ts.schema
        ))
        .await;
    assert_eq!(matched, 1);
    let catalog = ts.catalog().await;
    assert_eq!(catalog.aggregates["DailyMetric"].source, "HourlyMetric");
    assert_eq!(catalog.aggregates["HourlyMetric"].source, "Metric");
    ts.finish().await;
}

/// §8 item 12: which relation a refresh job's `hypertable_name` names; the
/// catalog accepts the view or its materialization hypertable.
#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn probe_refresh_job_relation() {
    let Some(ts) = Ts::connect("probe_refresh_job_relation").await else {
        return;
    };
    ts.exec(RAW_HYPERTABLE).await;
    ts.exec(RAW_HIERARCHY).await;
    let names = ts
        .strings(&format!(
            "SELECT CASE WHEN j.hypertable_name = a.view_name THEN 'view' \
                         WHEN j.hypertable_name = a.materialization_hypertable_name \
                           THEN 'materialization hypertable' \
                         ELSE 'other: ' || j.hypertable_name END \
             FROM timescaledb_information.jobs j \
             JOIN timescaledb_information.continuous_aggregates a \
               ON (j.config->>'mat_hypertable_id')::int = ( \
                  SELECT id FROM _timescaledb_catalog.hypertable \
                  WHERE table_name = a.materialization_hypertable_name) \
             WHERE j.proc_name = 'policy_refresh_continuous_aggregate' \
               AND a.view_schema = '{}'",
            ts.schema
        ))
        .await;
    eprintln!("refresh job hypertable_name names the {names:?}");
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.iter().all(|n| !n.starts_with("other")), "{names:?}");
    let catalog = ts.catalog().await;
    assert!(catalog.aggregates["HourlyMetric"].policy.is_some());
    assert!(catalog.aggregates["DailyMetric"].policy.is_some());
    ts.finish().await;
}
