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
//! time: the catalog's job and columnstore-settings queries are not
//! schema-scoped. Run via docker compose:
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
    IntrospectionOptions, PostgresCatalog, introspect_database, introspect_postgres_catalog,
};
use prax_cli::commands::schema_from_db::schema_from_database;
use prax_migrate::{
    IntrospectionConfig, MigrateResult, SchemaDiff, SchemaDiffer, SqlBackend, SqlDialect,
};
use tokio::sync::{Mutex, MutexGuard};
use tokio_postgres::{Client, NoTls};

/// Serializes the tests: the jobs and columnstore-settings catalog queries are
/// unfiltered by schema, so two tests with a `"Metric"` hypertable would see
/// each other's policies.
static SERIAL: Mutex<()> = Mutex::const_new(());

const PROVIDER: &str = "postgresql";

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

        // A test that panicked earlier leaves its schema behind; its policies
        // would leak into this test's catalog read.
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
    async fn catalog(&self) -> prax_migrate::TimescaleCatalog {
        self.introspect()
            .await
            .1
            .timescale
            .expect("timescaledb is installed")
    }

    /// Diff `prax` against the database, as `generate_migration_sql` does.
    async fn try_diff(&self, prax: &str) -> MigrateResult<SchemaDiff> {
        let target = prax_schema::validate_schema(prax).expect("target schema validates");
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
        self.try_diff(prax).await.expect("diff")
    }

    /// Generate the migration for `prax`, stripping gated changes unless
    /// `allow_destructive`.
    async fn generate(&self, prax: &str, allow_destructive: bool) -> Generated {
        let mut diff = self.diff(prax).await;
        if !allow_destructive {
            strip_destructive(&mut diff);
        }
        let migration = SqlDialect::for_backend(SqlBackend::Postgres).generate_migration(&diff);
        Generated { up: migration.up }
    }

    /// Introspect → generate → apply → introspect → diff. Returns the applied
    /// `up` SQL and the re-diff.
    async fn roundtrip(&self, prax: &str, allow_destructive: bool) -> (String, SchemaDiff) {
        let generated = self.generate(prax, allow_destructive).await;
        self.exec(&generated.up).await;
        (generated.up, self.diff(prax).await)
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

const DATASOURCE: &str = r#"
datasource db {
  provider   = "postgresql"
  url        = env("DATABASE_URL")
  extensions = [timescaledb]
}
"#;

/// The spec §3.1 example.
fn spec_example() -> String {
    format!(
        r#"{DATASOURCE}
model Metric {{
  time     DateTime
  deviceId String
  value    Float

  @@id([deviceId, time])
  @@timescale.hypertable(time, chunkInterval: "1 day", space: hash(deviceId, 4))
  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "7 days")
  @@timescale.retention(dropAfter: "90 days")
}}

view HourlyMetric {{
  bucket   DateTime
  deviceId String
  avgValue Float

  @@timescale.continuous(source: Metric, scheduleInterval: "1 hour", startOffset: "3 days", endOffset: "1 hour")
}}

@@sql("HourlyMetric", """
SELECT time_bucket('1 hour', "time") AS bucket, "deviceId", avg("value") AS "avgValue"
FROM "Metric"
GROUP BY 1, 2
""")
"#
    )
}

// ============================================================================
// Round trips
// ============================================================================

#[tokio::test]
#[ignore = "requires TimescaleDB via docker-compose"]
async fn roundtrip_spec_example_is_empty() {
    let Some(ts) = Ts::connect("roundtrip_spec_example_is_empty").await else {
        return;
    };
    let (up, rediff) = ts.roundtrip(&spec_example(), false).await;
    assert!(up.contains("create_hypertable"), "{up}");
    assert!(up.contains("add_retention_policy"), "{up}");
    assert!(up.contains("add_continuous_aggregate_policy"), "{up}");
    assert_empty(&rediff, &up);

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
