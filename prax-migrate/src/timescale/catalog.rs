//! The live database's TimescaleDB state, built from plain catalog rows.
//!
//! prax-migrate has no database driver: the CLI runs the `queries` and fills
//! [`CatalogRows`], and [`TimescaleCatalog::from_rows`] assembles the state.
//!
//! The SQL in `queries`, the column indices in `cols` and the `*Row`
//! structs are plumbing shared with `prax-cli`. They follow the catalog
//! layout of the supported TimescaleDB versions and are not a stable API:
//! they may change in any release.

use std::collections::{BTreeMap, BTreeSet};
use std::iter::Peekable;
use std::str::Chars;

use prax_schema::ast::PgInterval;

use crate::error::{MigrateResult, MigrationError};

/// An interval decomposed as `(months, days, micros)`.
pub type IntervalParts = (i32, i32, i64);

/// Oldest TimescaleDB version Prax manages, as `(major, minor)`. Older
/// servers lack the `hypertable_columnstore_settings` view the catalog reads.
pub const TIMESCALE_MIN_VERSION: (u32, u32) = (2, 18);

/// Microseconds in an hour, for decomposing intervals in SQL.
const MICROS_PER_HOUR: i64 = 3_600_000_000;

/// Microseconds in a minute, for decomposing intervals in SQL.
const MICROS_PER_MINUTE: i64 = 60_000_000;

/// Marker on the first line of a continuous-aggregate definition record.
pub const CAGG_RECORD_MARKER: &str = "prax:cagg:v1";

/// Build the `COMMENT ON VIEW` text recording an aggregate's authored SQL.
#[must_use]
pub fn aggregate_record(sql: &str) -> String {
    format!("{CAGG_RECORD_MARKER}\n{sql}")
}

/// Extract the recorded SQL from a view comment, if it carries the marker.
#[must_use]
pub fn parse_aggregate_record(comment: &str) -> Option<&str> {
    comment
        .strip_prefix(CAGG_RECORD_MARKER)
        .and_then(|rest| rest.strip_prefix('\n'))
}

/// Whether a TimescaleDB version string is at least
/// [`TIMESCALE_MIN_VERSION`].
#[must_use]
pub fn timescale_version_supported(version: &str) -> bool {
    let (major, minor) = TIMESCALE_MIN_VERSION;
    timescale_version_at_least(version, major, minor)
}

/// Whether a TimescaleDB version string is at least `major.minor`. An
/// unparsable version is treated as older.
#[must_use]
pub fn timescale_version_at_least(version: &str, major: u32, minor: u32) -> bool {
    let mut parts = version.split('.');
    let have_major = parts.next().and_then(|p| p.trim().parse::<u32>().ok());
    let have_minor = parts.next().and_then(|p| p.trim().parse::<u32>().ok());
    match (have_major, have_minor) {
        (Some(have_major), Some(have_minor)) => (have_major, have_minor) >= (major, minor),
        _ => false,
    }
}

/// SQL selecting `(months, days, micros)` for an interval expression, cast to
/// `int4, int4, int8` and labelled `{alias}_months`, `{alias}_days` and
/// `{alias}_micros`. A `NULL` expression yields three `NULL`s.
#[must_use]
pub fn interval_parts_sql(expr: &str, alias: &str) -> String {
    format!(
        "(EXTRACT(YEAR FROM {expr}) * 12 + EXTRACT(MONTH FROM {expr}))::int4 AS {alias}_months, \
         EXTRACT(DAY FROM {expr})::int4 AS {alias}_days, \
         (EXTRACT(HOUR FROM {expr}) * {MICROS_PER_HOUR} \
         + EXTRACT(MINUTE FROM {expr}) * {MICROS_PER_MINUTE} \
         + EXTRACT(MICROSECONDS FROM {expr}))::int8 AS {alias}_micros"
    )
}

/// SQL run by the CLI to fill [`CatalogRows`]. Every query but `EXTENSIONS` is
/// scoped to the introspected schema, passed as `$1`, so a database with the
/// same relation names in several schemas (e.g. schema per tenant) never mixes
/// their state.
///
/// Every output column carries an alias; `cols` holds each
/// query's column indices, in `SELECT` order. Not a stable API.
#[doc(hidden)]
pub mod queries {
    use super::interval_parts_sql;

    /// `(name, version)` of every installed extension.
    pub const EXTENSIONS: &str =
        "SELECT extname::text AS name, extversion::text AS version FROM pg_extension";

    /// `(table_name, columnstore_enabled)`.
    pub const HYPERTABLES: &str = "SELECT hypertable_name::text AS table_name, \
         compression_enabled AS columnstore_enabled \
         FROM timescaledb_information.hypertables WHERE hypertable_schema = $1";

    /// `(table_name, column_name, is_time, chunk_interval_{months,days,micros},
    /// num_partitions)`.
    #[must_use]
    pub fn dimensions_sql() -> String {
        format!(
            "SELECT hypertable_name::text AS table_name, column_name::text AS column_name, \
             dimension_type = 'Time' AS is_time, {chunk_interval}, \
             num_partitions::int2 AS num_partitions \
             FROM timescaledb_information.dimensions WHERE hypertable_schema = $1 \
             ORDER BY hypertable_name, dimension_number",
            chunk_interval = interval_parts_sql("time_interval", "chunk_interval"),
        )
    }

    /// `(relation, segment_by, order_by)` for the hypertables in schema `$1`;
    /// `relation` is regclass text.
    pub const COLUMNSTORE_SETTINGS: &str = "SELECT s.hypertable::text AS relation, \
         s.segmentby::text AS segment_by, s.orderby::text AS order_by \
         FROM timescaledb_information.hypertable_columnstore_settings s \
         JOIN pg_class c ON c.oid = s.hypertable::oid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1";

    /// `(proc_name, relation, drop_after, compress_after, start_offset,
    /// end_offset, schedule_interval)`, each interval as three
    /// `{name}_{months,days,micros}` columns.
    ///
    /// Scoped to schema `$1`: columnstore and retention jobs of its hypertables,
    /// and refresh jobs of its continuous aggregates. A refresh job names either
    /// the aggregate's view (2.30) or its materialization hypertable in
    /// `_timescaledb_internal` (2.18), so the latter is matched through the
    /// schema's aggregates.
    #[must_use]
    pub fn jobs_sql() -> String {
        let config =
            |key: &str| interval_parts_sql(&format!("(j.config->>'{key}')::interval"), key);
        format!(
            "SELECT j.proc_name::text AS proc_name, j.hypertable_name::text AS relation, \
             {drop_after}, {compress_after}, {start_offset}, {end_offset}, {schedule} \
             FROM timescaledb_information.jobs j \
             WHERE j.proc_name IN ('policy_compression', 'policy_retention', \
                                   'policy_refresh_continuous_aggregate') \
               AND (j.hypertable_schema = $1 \
                    OR (j.proc_name = 'policy_refresh_continuous_aggregate' \
                        AND (j.hypertable_schema, j.hypertable_name) IN ( \
                            SELECT materialization_hypertable_schema, \
                                   materialization_hypertable_name \
                            FROM timescaledb_information.continuous_aggregates \
                            WHERE view_schema = $1)))",
            drop_after = config("drop_after"),
            compress_after = config("compress_after"),
            start_offset = config("start_offset"),
            end_offset = config("end_offset"),
            schedule = interval_parts_sql("j.schedule_interval", "schedule_interval"),
        )
    }

    /// `(view_name, hypertable_name, materialization_hypertable_name,
    /// materialized_only, view_definition, view_comment)`.
    ///
    /// `hypertable_name` is qualified as `schema.name` when the aggregate reads
    /// a hypertable of another schema (a parent aggregate's materialization
    /// hypertable is left bare), so it never matches this schema's hypertables.
    pub const AGGREGATES: &str = "SELECT a.view_name::text AS view_name, \
         CASE WHEN a.hypertable_schema = $1 \
                OR (a.hypertable_schema, a.hypertable_name) IN ( \
                    SELECT p.materialization_hypertable_schema, \
                           p.materialization_hypertable_name \
                    FROM timescaledb_information.continuous_aggregates p \
                    WHERE p.view_schema = $1) \
              THEN a.hypertable_name::text \
              ELSE (a.hypertable_schema || '.' || a.hypertable_name)::text END \
              AS hypertable_name, \
         a.materialization_hypertable_name::text AS materialization_hypertable_name, \
         a.materialized_only AS materialized_only, \
         a.view_definition::text AS view_definition, \
         obj_description(format('%I.%I', a.view_schema, a.view_name)::regclass, 'pg_class') \
              AS view_comment \
         FROM timescaledb_information.continuous_aggregates a WHERE a.view_schema = $1";

    /// `(relation, column_name, udt_name, nullable)` for the aggregates' views.
    pub const COLUMNS: &str = "SELECT table_name::text AS relation, \
         column_name::text AS column_name, udt_name::text AS udt_name, \
         is_nullable = 'YES' AS nullable \
         FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name IN ( \
             SELECT view_name FROM timescaledb_information.continuous_aggregates \
             WHERE view_schema = $1) \
         ORDER BY table_name, ordinal_position";

    /// `(view_name, is_materialized)` for views and materialized views.
    pub const VIEWS: &str = "SELECT c.relname::text AS view_name, \
         c.relkind = 'm' AS is_materialized \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('v', 'm')";
}

/// Column indices of each `queries` result, in `SELECT` order. An interval
/// index names the first of its three `(months, days, micros)` columns. Not a
/// stable API.
#[doc(hidden)]
pub mod cols {
    /// Number of columns an interval occupies: `(months, days, micros)`.
    pub const INTERVAL_WIDTH: usize = 3;

    /// `queries::EXTENSIONS`.
    pub mod extensions {
        /// Extension name.
        pub const NAME: usize = 0;
        /// Extension version.
        pub const VERSION: usize = 1;
    }

    /// `queries::HYPERTABLES`.
    pub mod hypertables {
        /// Hypertable name.
        pub const TABLE: usize = 0;
        /// Whether columnstore is enabled.
        pub const COLUMNSTORE_ENABLED: usize = 1;
    }

    /// `queries::dimensions_sql`.
    pub mod dimensions {
        /// Hypertable name.
        pub const TABLE: usize = 0;
        /// Dimension column.
        pub const COLUMN: usize = 1;
        /// Whether this is a time dimension.
        pub const IS_TIME: usize = 2;
        /// Chunk interval (three columns).
        pub const CHUNK_INTERVAL: usize = 3;
        /// Partition count.
        pub const PARTITIONS: usize = CHUNK_INTERVAL + super::INTERVAL_WIDTH;
    }

    /// `queries::COLUMNSTORE_SETTINGS`.
    pub mod columnstore_settings {
        /// Hypertable as regclass text.
        pub const RELATION: usize = 0;
        /// `segmentby` list.
        pub const SEGMENT_BY: usize = 1;
        /// `orderby` list.
        pub const ORDER_BY: usize = 2;
    }

    /// `queries::jobs_sql`.
    pub mod jobs {
        use super::INTERVAL_WIDTH;

        /// Job procedure name.
        pub const PROC_NAME: usize = 0;
        /// Relation the job targets.
        pub const RELATION: usize = 1;
        /// Retention `drop_after` (three columns).
        pub const DROP_AFTER: usize = 2;
        /// Columnstore `compress_after` (three columns).
        pub const COMPRESS_AFTER: usize = DROP_AFTER + INTERVAL_WIDTH;
        /// Refresh `start_offset` (three columns).
        pub const START_OFFSET: usize = COMPRESS_AFTER + INTERVAL_WIDTH;
        /// Refresh `end_offset` (three columns).
        pub const END_OFFSET: usize = START_OFFSET + INTERVAL_WIDTH;
        /// Job schedule interval (three columns).
        pub const SCHEDULE_INTERVAL: usize = END_OFFSET + INTERVAL_WIDTH;
    }

    /// `queries::AGGREGATES`.
    pub mod aggregates {
        /// View name.
        pub const VIEW: usize = 0;
        /// Source hypertable (possibly `schema.name`).
        pub const HYPERTABLE: usize = 1;
        /// Own materialization hypertable.
        pub const MATERIALIZATION_HYPERTABLE: usize = 2;
        /// Whether the view is materialized-only.
        pub const MATERIALIZED_ONLY: usize = 3;
        /// Postgres-normalized view definition.
        pub const VIEW_DEFINITION: usize = 4;
        /// View comment.
        pub const COMMENT: usize = 5;
    }

    /// `queries::COLUMNS`.
    pub mod columns {
        /// View name.
        pub const RELATION: usize = 0;
        /// Column name.
        pub const NAME: usize = 1;
        /// Postgres `udt_name`.
        pub const UDT_NAME: usize = 2;
        /// Whether the column is nullable.
        pub const NULLABLE: usize = 3;
    }

    /// `queries::VIEWS`.
    pub mod views {
        /// View name.
        pub const NAME: usize = 0;
        /// Whether it is a materialized view.
        pub const IS_MATERIALIZED: usize = 1;
    }
}

/// Row of `pg_extension`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRow {
    /// Extension name.
    pub name: String,
    /// Extension version.
    pub version: String,
}

/// Row of `timescaledb_information.hypertables`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypertableRow {
    /// Hypertable name.
    pub table: String,
    /// Whether columnstore (compression) is enabled.
    pub columnstore_enabled: bool,
}

/// Row of `timescaledb_information.dimensions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimensionRow {
    /// Hypertable name.
    pub table: String,
    /// Dimension column.
    pub column: String,
    /// Whether this is the time dimension.
    pub is_time: bool,
    /// Chunk interval parts (time dimensions).
    pub interval: Option<IntervalParts>,
    /// Partition count (space dimensions).
    pub partitions: Option<i16>,
}

/// Row of `timescaledb_information.hypertable_columnstore_settings`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnstoreSettingsRow {
    /// Relation, possibly regclass text with identifier quoting.
    pub relation: String,
    /// `segmentby` column list text.
    pub segment_by: Option<String>,
    /// `orderby` column list text.
    pub order_by: Option<String>,
}

/// Row of `timescaledb_information.jobs` for a Timescale policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    /// `policy_compression`, `policy_retention` or `policy_refresh_continuous_aggregate`.
    pub proc_name: String,
    /// Relation the job targets (hypertable, or aggregate materialization hypertable).
    pub relation: String,
    /// Retention `drop_after`.
    pub drop_after: Option<IntervalParts>,
    /// Compression `compress_after`.
    pub after: Option<IntervalParts>,
    /// Refresh `start_offset`.
    pub start_offset: Option<IntervalParts>,
    /// Refresh `end_offset`.
    pub end_offset: Option<IntervalParts>,
    /// Job schedule interval.
    pub schedule_interval: IntervalParts,
}

/// Row of `timescaledb_information.continuous_aggregates`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateRow {
    /// User-facing view name.
    pub view: String,
    /// Source hypertable (or parent materialization hypertable); `schema.name`
    /// for a hypertable of another schema.
    pub hypertable_name: String,
    /// This aggregate's own materialization hypertable.
    pub materialization_hypertable: String,
    /// Whether the view is materialized-only.
    pub materialized_only: bool,
    /// Postgres-normalized view definition.
    pub view_definition: String,
    /// View comment. When Prax created the aggregate it holds the authored
    /// SQL, prefixed by [`CAGG_RECORD_MARKER`] (see [`aggregate_record`]).
    pub comment: Option<String>,
}

/// Column of an aggregate's view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRow {
    /// View name.
    pub relation: String,
    /// Column name.
    pub name: String,
    /// Postgres `udt_name`.
    pub udt_name: String,
    /// Whether the column is nullable.
    pub nullable: bool,
}

/// A view or materialized view in the introspected schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewRow {
    /// View name.
    pub name: String,
    /// Whether it is a materialized view.
    pub is_materialized: bool,
}

/// All rows needed to build a [`TimescaleCatalog`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogRows {
    /// Installed extensions.
    pub extensions: Vec<ExtensionRow>,
    /// Hypertables.
    pub hypertables: Vec<HypertableRow>,
    /// Dimensions.
    pub dimensions: Vec<DimensionRow>,
    /// Columnstore settings.
    pub columnstore_settings: Vec<ColumnstoreSettingsRow>,
    /// Policy jobs.
    pub jobs: Vec<JobRow>,
    /// Continuous aggregates.
    pub aggregates: Vec<AggregateRow>,
    /// Aggregate view columns.
    pub columns: Vec<ColumnRow>,
    /// Plain and materialized views.
    pub views: Vec<ViewRow>,
}

/// Columnstore `segmentby`/`orderby` settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnstoreSettings {
    /// Segment-by columns.
    pub segment_by: Vec<String>,
    /// Order-by columns; the flag is `true` for descending.
    pub order_by: Vec<(String, bool)>,
}

/// Columnstore configuration of a hypertable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnstoreState {
    /// Segment/order settings.
    pub settings: ColumnstoreSettings,
    /// Columnstore policy interval, if a policy exists.
    pub after: Option<PgInterval>,
}

/// A hypertable and its policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypertableState {
    /// Table name.
    pub table: String,
    /// Time dimension column.
    pub time_column: String,
    /// Chunk interval.
    pub chunk_interval: PgInterval,
    /// Space dimension `(column, partitions)`.
    pub space: Option<(String, i16)>,
    /// Columnstore configuration, when enabled.
    pub columnstore: Option<ColumnstoreState>,
    /// Retention `drop_after`.
    pub retention: Option<PgInterval>,
}

/// A continuous-aggregate refresh policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatePolicy {
    /// Refresh window start offset.
    pub start_offset: Option<PgInterval>,
    /// Refresh window end offset.
    pub end_offset: Option<PgInterval>,
    /// Job schedule interval.
    pub schedule_interval: PgInterval,
}

/// A continuous aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateState {
    /// View name.
    pub view: String,
    /// Source relation: the parent aggregate's view, or the source hypertable.
    pub source: String,
    /// Recorded authored SQL; `None` when the view carries no record.
    pub sql: Option<String>,
    /// Whether the view is materialized-only.
    pub materialized_only: bool,
    /// Refresh policy.
    pub policy: Option<AggregatePolicy>,
}

/// Column of an aggregate's view (used by `db pull`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogColumn {
    /// Column name.
    pub name: String,
    /// Postgres `udt_name`.
    pub udt_name: String,
    /// Whether the column is nullable.
    pub nullable: bool,
}

/// The live database's TimescaleDB state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimescaleCatalog {
    /// Installed `timescaledb` version.
    pub version: String,
    /// Hypertables by table name.
    pub hypertables: BTreeMap<String, HypertableState>,
    /// Continuous aggregates by view name.
    pub aggregates: BTreeMap<String, AggregateState>,
    /// Postgres-normalized `view_definition` per aggregate (`db pull` only).
    pub aggregate_definitions: BTreeMap<String, String>,
    /// Columns per aggregate view (`db pull` only).
    pub aggregate_columns: BTreeMap<String, Vec<CatalogColumn>>,
    /// Non-aggregate views; the value is whether the view is materialized.
    pub other_views: BTreeMap<String, bool>,
    /// Valid hypertables Prax cannot represent (table to reason), omitted
    /// from `hypertables`.
    pub unsupported_hypertables: BTreeMap<String, String>,
    /// Valid continuous aggregates Prax cannot represent (view to reason),
    /// omitted from `aggregates` and `other_views`; `db pull` reports them
    /// instead of rendering a view.
    pub unsupported_aggregates: BTreeMap<String, String>,
}

fn interval(parts: IntervalParts) -> PgInterval {
    PgInterval::new(parts.0, parts.1, parts.2)
}

/// Read one SQL identifier from `chars`: a double-quoted identifier (`""`
/// inside it becomes `"`), or an unquoted run ending before whitespace or a
/// char in `stops`. The terminating char is left unread.
fn read_identifier(chars: &mut Peekable<Chars<'_>>, stops: &[char]) -> String {
    let mut name = String::new();
    if chars.next_if_eq(&'"').is_some() {
        while let Some(c) = chars.next() {
            if c != '"' {
                name.push(c);
            } else if chars.next_if_eq(&'"').is_some() {
                name.push('"');
            } else {
                break;
            }
        }
    } else {
        while let Some(c) = chars.next_if(|c| !c.is_whitespace() && !stops.contains(c)) {
            name.push(c);
        }
    }
    name
}

/// Reduce regclass text to the relation name: the last identifier of an
/// optionally schema-qualified name, split on unquoted dots.
fn unquote_relation(s: &str) -> String {
    let mut chars = s.trim().chars().peekable();
    let mut name = read_identifier(&mut chars, &['.']);
    while chars.next_if_eq(&'.').is_some() {
        name = read_identifier(&mut chars, &['.']);
    }
    name
}

/// Parse a Timescale `segmentby`/`orderby` list into `(column, descending)`.
///
/// Each column may carry `ASC` or `DESC` and the NULLS ordering that is the
/// default for that direction (`NULLS LAST` for ascending, `NULLS FIRST` for
/// descending).
///
/// # Errors
///
/// Returns a reason naming the column when it carries a non-default NULLS
/// ordering or any other modifier, which Prax cannot represent.
pub(crate) fn parse_column_list(s: &str) -> Result<Vec<(String, bool)>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        if chars.peek().is_none() {
            break;
        }
        let name = read_identifier(&mut chars, &[',']);
        let mut modifiers = String::new();
        while let Some(c) = chars.next_if(|c| *c != ',') {
            modifiers.push(c);
        }
        let descending = parse_ordering(&modifiers).ok_or_else(|| {
            format!(
                "column \"{name}\" has ordering \"{}\"; only ASC or DESC with the default \
                 NULLS ordering is supported",
                modifiers.trim()
            )
        })?;
        out.push((name, descending));
    }
    Ok(out)
}

/// Parse a column's ordering modifiers, `[ASC | DESC] [NULLS FIRST | NULLS
/// LAST]` (case-insensitive), into whether it is descending, or `None` when
/// they hold a non-default NULLS ordering or anything else.
fn parse_ordering(modifiers: &str) -> Option<bool> {
    let lowered = modifiers.to_ascii_lowercase();
    let words: Vec<&str> = lowered.split_whitespace().collect();
    let (descending, nulls) = match words.as_slice() {
        ["asc", rest @ ..] => (false, rest),
        ["desc", rest @ ..] => (true, rest),
        rest => (false, rest),
    };
    // Postgres defaults: NULLS LAST when ascending, NULLS FIRST when descending.
    let default_nulls = if descending { "first" } else { "last" };
    match nulls {
        [] => Some(descending),
        ["nulls", position] if *position == default_nulls => Some(descending),
        _ => None,
    }
}

fn malformed(relation: &str, what: &str) -> MigrationError {
    MigrationError::Database(format!(
        "malformed TimescaleDB catalog for '{relation}': {what}"
    ))
}

impl TimescaleCatalog {
    /// The error to report when the installed TimescaleDB is older than
    /// [`TIMESCALE_MIN_VERSION`], or `None` when the version is supported.
    #[must_use]
    pub fn version_error(&self) -> Option<String> {
        if timescale_version_supported(&self.version) {
            return None;
        }
        let (major, minor) = TIMESCALE_MIN_VERSION;
        Some(format!(
            "TimescaleDB {} is installed, but Prax requires TimescaleDB {major}.{minor} or \
             newer for @@timescale.* attributes",
            self.version
        ))
    }

    /// Build the catalog from rows. Returns `None` when the `timescaledb`
    /// extension is not installed.
    ///
    /// Valid state Prax cannot represent is recorded with a reason instead of
    /// failing. `unsupported_hypertables` gets every hypertable with an
    /// integer time dimension, more than one time or space dimension, a
    /// columnstore column with a non-default NULLS ordering, or a columnstore
    /// or retention policy without its interval (`*_created_before`).
    /// `unsupported_aggregates` gets every continuous aggregate built
    /// (directly or through parent aggregates) on such a hypertable or on a
    /// relation that is not a hypertable of this schema.
    ///
    /// An installed extension older than [`TIMESCALE_MIN_VERSION`] is still
    /// reported: the caller reads no other rows, so the catalog holds only the
    /// version.
    ///
    /// # Errors
    ///
    /// Returns `MigrationError::Database` when a hypertable has no time
    /// dimension.
    pub fn from_rows(rows: CatalogRows) -> MigrateResult<Option<Self>> {
        let Some(ext) = rows.extensions.iter().find(|e| e.name == "timescaledb") else {
            return Ok(None);
        };
        let version = ext.version.clone();

        let mut hypertables = BTreeMap::new();
        let mut unsupported_hypertables = BTreeMap::new();
        for h in &rows.hypertables {
            let (time_dims, space_dims): (Vec<&DimensionRow>, Vec<&DimensionRow>) = rows
                .dimensions
                .iter()
                .filter(|d| d.table == h.table)
                .partition(|d| d.is_time);
            let Some(time) = time_dims.first() else {
                return Err(malformed(&h.table, "hypertable has no time dimension"));
            };
            if time_dims.len() > 1 || space_dims.len() > 1 {
                unsupported_hypertables.insert(
                    h.table.clone(),
                    "hypertables with more than one time or space dimension are not supported"
                        .to_string(),
                );
                continue;
            }
            let Some(chunk_interval) = time.interval.map(interval) else {
                unsupported_hypertables.insert(
                    h.table.clone(),
                    "integer time dimensions are not supported".to_string(),
                );
                continue;
            };
            let space = space_dims
                .first()
                .and_then(|d| d.partitions.map(|p| (d.column.clone(), p)));

            let columnstore = if h.columnstore_enabled {
                let settings = rows
                    .columnstore_settings
                    .iter()
                    .find(|s| unquote_relation(&s.relation) == h.table);
                let parse = |list: Option<&str>, label: &str| {
                    list.map(parse_column_list)
                        .transpose()
                        .map(Option::unwrap_or_default)
                        .map_err(|why| format!("columnstore {label} {why}"))
                };
                let segment_by = parse(settings.and_then(|s| s.segment_by.as_deref()), "segmentby");
                let order_by = parse(settings.and_then(|s| s.order_by.as_deref()), "orderby");
                match (segment_by, order_by) {
                    (Ok(segment_by), Ok(order_by)) => Some(ColumnstoreState {
                        settings: ColumnstoreSettings {
                            segment_by: segment_by.into_iter().map(|(n, _)| n).collect(),
                            order_by,
                        },
                        after: None,
                    }),
                    (Err(why), _) | (_, Err(why)) => {
                        unsupported_hypertables.insert(h.table.clone(), why);
                        continue;
                    }
                }
            } else {
                None
            };

            hypertables.insert(
                h.table.clone(),
                HypertableState {
                    table: h.table.clone(),
                    time_column: time.column.clone(),
                    chunk_interval,
                    space,
                    columnstore,
                    retention: None,
                },
            );
        }

        let mut aggregates = BTreeMap::new();
        let mut aggregate_definitions = BTreeMap::new();
        for a in &rows.aggregates {
            let source = rows
                .aggregates
                .iter()
                .find(|p| p.materialization_hypertable == a.hypertable_name)
                .map_or_else(|| a.hypertable_name.clone(), |p| p.view.clone());
            aggregates.insert(
                a.view.clone(),
                AggregateState {
                    view: a.view.clone(),
                    source,
                    sql: a
                        .comment
                        .as_deref()
                        .and_then(parse_aggregate_record)
                        .map(str::to_string),
                    materialized_only: a.materialized_only,
                    policy: None,
                },
            );
            aggregate_definitions.insert(a.view.clone(), a.view_definition.clone());
        }

        for j in &rows.jobs {
            match j.proc_name.as_str() {
                // A columnstore or retention policy sets its interval on the
                // hypertable; one without it (`*_created_before`) makes the
                // hypertable unsupported.
                "policy_compression" | "policy_retention" => {
                    let Some(h) = hypertables.get_mut(&j.relation) else {
                        continue;
                    };
                    let (slot, parts, missing) = if j.proc_name == "policy_compression" {
                        let Some(cs) = h.columnstore.as_mut() else {
                            continue;
                        };
                        (
                            &mut cs.after,
                            j.after,
                            "compress_after interval (compress_created_before)",
                        )
                    } else {
                        (
                            &mut h.retention,
                            j.drop_after,
                            "drop_after interval (drop_created_before)",
                        )
                    };
                    match parts {
                        Some(parts) => *slot = Some(interval(parts)),
                        None => {
                            hypertables.remove(&j.relation);
                            unsupported_hypertables.insert(
                                j.relation.clone(),
                                format!("{} without a {missing} is not supported", j.proc_name),
                            );
                        }
                    }
                }
                "policy_refresh_continuous_aggregate" => {
                    let view = rows
                        .aggregates
                        .iter()
                        .find(|a| {
                            a.view == j.relation || a.materialization_hypertable == j.relation
                        })
                        .map(|a| a.view.clone());
                    if let Some(state) = view.and_then(|v| aggregates.get_mut(&v)) {
                        state.policy = Some(AggregatePolicy {
                            start_offset: j.start_offset.map(interval),
                            end_offset: j.end_offset.map(interval),
                            schedule_interval: interval(j.schedule_interval),
                        });
                    }
                }
                _ => {}
            }
        }

        // Aggregates built on a relation Prax cannot represent (an unsupported
        // hypertable, or a relation that is not a hypertable of this schema)
        // can never be declared in a schema, so they are left unmanaged.
        let unsupported_aggregates =
            unsupported_aggregate_reasons(&aggregates, &hypertables, &unsupported_hypertables);
        for view in unsupported_aggregates.keys() {
            aggregates.remove(view);
            aggregate_definitions.remove(view);
        }

        let mut aggregate_columns: BTreeMap<String, Vec<CatalogColumn>> = BTreeMap::new();
        for c in &rows.columns {
            if aggregates.contains_key(&c.relation) {
                aggregate_columns
                    .entry(c.relation.clone())
                    .or_default()
                    .push(CatalogColumn {
                        name: c.name.clone(),
                        udt_name: c.udt_name.clone(),
                        nullable: c.nullable,
                    });
            }
        }

        let other_views = rows
            .views
            .iter()
            .filter(|v| {
                !aggregates.contains_key(&v.name) && !unsupported_aggregates.contains_key(&v.name)
            })
            .map(|v| (v.name.clone(), v.is_materialized))
            .collect();

        Ok(Some(Self {
            version,
            hypertables,
            aggregates,
            aggregate_definitions,
            aggregate_columns,
            other_views,
            unsupported_hypertables,
            unsupported_aggregates,
        }))
    }
}

/// The aggregates whose source chain (through parent aggregates) ends at a
/// relation that is an unsupported hypertable or not a hypertable of this
/// schema, mapped to the reason.
fn unsupported_aggregate_reasons(
    aggregates: &BTreeMap<String, AggregateState>,
    hypertables: &BTreeMap<String, HypertableState>,
    unsupported_hypertables: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for view in aggregates.keys() {
        // Follow parent aggregates to the relation the chain is built on; a
        // cycle (impossible in TimescaleDB) stops at the repeated view.
        let mut seen = BTreeSet::new();
        let mut relation = view;
        while let Some(parent) = aggregates.get(relation) {
            if !seen.insert(relation) {
                break;
            }
            relation = &parent.source;
        }
        let reason = if let Some(why) = unsupported_hypertables.get(relation) {
            format!(
                "it is built on \"{relation}\", whose TimescaleDB setup Prax cannot manage: {why}"
            )
        } else if hypertables.contains_key(relation) {
            continue;
        } else {
            format!("it is built on \"{relation}\", which is not a hypertable in this schema")
        };
        out.insert(view.clone(), reason);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::ast::PgInterval;

    fn days(d: i32) -> PgInterval {
        PgInterval::new(0, d, 0)
    }

    fn base_rows() -> CatalogRows {
        CatalogRows {
            extensions: vec![ExtensionRow {
                name: "timescaledb".into(),
                version: "2.18.0".into(),
            }],
            ..Default::default()
        }
    }

    fn agg(view: &str, hyper: &str, mat: &str, comment: Option<&str>) -> AggregateRow {
        AggregateRow {
            view: view.into(),
            hypertable_name: hyper.into(),
            materialization_hypertable: mat.into(),
            materialized_only: false,
            view_definition: "SELECT 1".into(),
            comment: comment.map(Into::into),
        }
    }

    fn metric_rows() -> CatalogRows {
        let mut r = base_rows();
        r.hypertables = vec![HypertableRow {
            table: "Metric".into(),
            columnstore_enabled: true,
        }];
        r.dimensions = vec![
            DimensionRow {
                table: "Metric".into(),
                column: "time".into(),
                is_time: true,
                interval: Some((0, 1, 0)),
                partitions: None,
            },
            DimensionRow {
                table: "Metric".into(),
                column: "deviceId".into(),
                is_time: false,
                interval: None,
                partitions: Some(4),
            },
        ];
        r.columnstore_settings = vec![ColumnstoreSettingsRow {
            relation: "\"Metric\"".into(),
            segment_by: Some("\"deviceId\"".into()),
            order_by: Some("\"time\" DESC".into()),
        }];
        r
    }

    fn job(proc_name: &str, relation: &str) -> JobRow {
        JobRow {
            proc_name: proc_name.into(),
            relation: relation.into(),
            drop_after: None,
            after: None,
            start_offset: None,
            end_offset: None,
            schedule_interval: (0, 0, 3_600_000_000),
        }
    }

    #[test]
    fn test_from_rows_without_extension_is_none() {
        let rows = CatalogRows {
            extensions: vec![ExtensionRow {
                name: "vector".into(),
                version: "0.7.0".into(),
            }],
            ..Default::default()
        };
        assert!(TimescaleCatalog::from_rows(rows).unwrap().is_none());
    }

    #[test]
    fn test_from_rows_builds_hypertable() {
        let mut rows = metric_rows();
        let mut c = job("policy_compression", "Metric");
        c.after = Some((0, 7, 0));
        let mut r = job("policy_retention", "Metric");
        r.drop_after = Some((0, 90, 0));
        rows.jobs = vec![c, r];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.version, "2.18.0");
        assert_eq!(
            cat.hypertables["Metric"],
            HypertableState {
                table: "Metric".into(),
                time_column: "time".into(),
                chunk_interval: days(1),
                space: Some(("deviceId".into(), 4)),
                columnstore: Some(ColumnstoreState {
                    settings: ColumnstoreSettings {
                        segment_by: vec!["deviceId".into()],
                        order_by: vec![("time".into(), true)],
                    },
                    after: Some(days(7)),
                }),
                retention: Some(days(90)),
            }
        );
    }

    #[test]
    fn test_hypertable_without_columnstore() {
        let mut rows = metric_rows();
        rows.hypertables[0].columnstore_enabled = false;
        rows.columnstore_settings.clear();
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.hypertables["Metric"].columnstore, None);
    }

    #[test]
    fn test_hypertable_without_time_dimension_is_error() {
        let mut rows = metric_rows();
        rows.dimensions.remove(0);
        let err = TimescaleCatalog::from_rows(rows).unwrap_err().to_string();
        assert!(err.contains("Metric"), "{err}");
    }

    #[test]
    fn test_compression_job_without_interval_is_unsupported() {
        let mut rows = metric_rows();
        rows.jobs = vec![job("policy_compression", "Metric")];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(!cat.hypertables.contains_key("Metric"));
        assert!(cat.unsupported_hypertables["Metric"].contains("policy_compression"));
    }

    #[test]
    fn test_retention_job_without_drop_after_is_unsupported() {
        let mut rows = metric_rows();
        rows.jobs = vec![job("policy_retention", "Metric")];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(!cat.hypertables.contains_key("Metric"));
        assert!(cat.unsupported_hypertables["Metric"].contains("policy_retention"));
    }

    #[test]
    fn test_integer_time_hypertable_is_unsupported() {
        let mut rows = metric_rows();
        rows.dimensions[0].interval = None;
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(cat.hypertables.is_empty());
        assert!(cat.unsupported_hypertables["Metric"].contains("integer time"));
    }

    #[test]
    fn test_columnstore_settings_match_schema_qualified_relation() {
        for rel in [
            "myschema.\"Metric\"",
            "\"my.schema\".\"Metric\"",
            "\"Metric\"",
        ] {
            let mut rows = metric_rows();
            rows.columnstore_settings[0].relation = rel.into();
            let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
            let cs = cat.hypertables["Metric"].columnstore.as_ref().unwrap();
            assert_eq!(
                cs.settings.segment_by,
                vec!["deviceId".to_string()],
                "{rel}"
            );
        }
        assert_eq!(unquote_relation("myschema.metric"), "metric");
    }

    #[test]
    fn test_parse_column_list() {
        assert_eq!(
            parse_column_list("\"time\" DESC, device_id"),
            Ok(vec![
                ("time".to_string(), true),
                ("device_id".to_string(), false)
            ])
        );
        assert_eq!(
            parse_column_list("\"we\"\"ird\" ASC NULLS LAST"),
            Ok(vec![("we\"ird".to_string(), false)])
        );
        assert_eq!(
            parse_column_list("\"a,b\" desc nulls first,c nulls last"),
            Ok(vec![("a,b".to_string(), true), ("c".to_string(), false)])
        );
        assert_eq!(parse_column_list(""), Ok(Vec::new()));
    }

    #[test]
    fn test_parse_column_list_rejects_non_default_nulls_ordering() {
        for list in [
            "\"time\" DESC NULLS LAST",
            "device_id NULLS FIRST",
            "device_id ASC NULLS FIRST",
        ] {
            let err = parse_column_list(list).unwrap_err();
            assert!(err.contains("NULLS"), "{list}: {err}");
        }
        let err = parse_column_list("ok, \"time\" DESC NULLS LAST").unwrap_err();
        assert!(err.contains("column \"time\""), "{err}");
    }

    #[test]
    fn test_parse_column_list_rejects_unknown_modifiers() {
        for list in [
            "\"time\" COLLATE \"C\"",
            "time DESC DESC",
            "time NULLS",
            "time NULLS MIDDLE",
            "time NULLS FIRST DESC",
        ] {
            let err = parse_column_list(list).unwrap_err();
            assert!(err.contains("column \"time\""), "{list}: {err}");
        }
    }

    #[test]
    fn test_non_default_nulls_ordering_hypertable_is_unsupported() {
        let mut rows = metric_rows();
        rows.columnstore_settings[0].order_by = Some("\"time\" DESC NULLS LAST".into());
        add_hierarchy(&mut rows, "Metric");
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(!cat.hypertables.contains_key("Metric"));
        let reason = &cat.unsupported_hypertables["Metric"];
        assert!(
            reason.contains("orderby") && reason.contains("column \"time\""),
            "{reason}"
        );
        assert_hierarchy_unsupported(&cat, &["built on \"Metric\"", "NULLS LAST"]);
    }

    #[test]
    fn test_multiple_space_dimensions_hypertable_is_unsupported() {
        let mut rows = metric_rows();
        rows.dimensions.push(DimensionRow {
            table: "Metric".into(),
            column: "siteId".into(),
            is_time: false,
            interval: None,
            partitions: Some(2),
        });
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(cat.hypertables.is_empty());
        assert!(
            cat.unsupported_hypertables["Metric"].contains("more than one time or space dimension"),
            "{}",
            cat.unsupported_hypertables["Metric"]
        );
    }

    #[test]
    fn test_multiple_time_dimensions_hypertable_is_unsupported() {
        let mut rows = metric_rows();
        rows.dimensions.push(DimensionRow {
            table: "Metric".into(),
            column: "recordedAt".into(),
            is_time: true,
            interval: Some((0, 7, 0)),
            partitions: None,
        });
        add_hierarchy(&mut rows, "Metric");
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(cat.hypertables.is_empty());
        assert!(
            cat.unsupported_hypertables["Metric"].contains("more than one time or space dimension")
        );
        assert_hierarchy_unsupported(&cat, &["more than one time or space dimension"]);
    }

    #[test]
    fn test_old_version_extension_row_yields_version_only_catalog() {
        // Below TIMESCALE_MIN_VERSION the CLI reads only the extension row.
        let rows = CatalogRows {
            extensions: vec![ExtensionRow {
                name: "timescaledb".into(),
                version: "2.17.2".into(),
            }],
            ..Default::default()
        };
        assert!(!timescale_version_supported("2.17.2"));
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.version, "2.17.2");
        assert!(cat.hypertables.is_empty());
        assert!(cat.aggregates.is_empty());
        assert!(cat.unsupported_hypertables.is_empty());
        assert!(cat.unsupported_aggregates.is_empty());
        assert!(cat.other_views.is_empty());
    }

    #[test]
    fn test_aggregate_parent_derivation() {
        let mut rows = metric_rows();
        rows.aggregates = vec![
            agg("HourlyMetric", "Metric", "_materialized_hypertable_2", None),
            agg(
                "DailyMetric",
                "_materialized_hypertable_2",
                "_materialized_hypertable_3",
                None,
            ),
        ];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.aggregates["DailyMetric"].source, "HourlyMetric");
        assert_eq!(cat.aggregates["HourlyMetric"].source, "Metric");
        assert_eq!(cat.aggregate_definitions["HourlyMetric"], "SELECT 1");
    }

    #[test]
    fn test_aggregate_record_parsing() {
        let mut rows = metric_rows();
        rows.aggregates = vec![
            agg("A", "Metric", "_m1", Some("prax:cagg:v1\nSELECT 1")),
            agg("B", "Metric", "_m2", None),
            agg("C", "Metric", "_m3", Some("hand written")),
        ];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.aggregates["A"].sql.as_deref(), Some("SELECT 1"));
        assert_eq!(cat.aggregates["B"].sql, None);
        assert_eq!(cat.aggregates["C"].sql, None);
        assert_eq!(aggregate_record("SELECT 1"), "prax:cagg:v1\nSELECT 1");
    }

    #[test]
    fn test_refresh_job_maps_by_view_or_materialization() {
        let mut rows = metric_rows();
        rows.aggregates = vec![
            agg("A", "Metric", "_m1", None),
            agg("B", "Metric", "_m2", None),
        ];
        let mut ja = job("policy_refresh_continuous_aggregate", "A");
        ja.start_offset = Some((0, 3, 0));
        let jb = job("policy_refresh_continuous_aggregate", "_m2");
        rows.jobs = vec![ja, jb];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        let a = cat.aggregates["A"].policy.as_ref().unwrap();
        assert_eq!(a.start_offset, Some(days(3)));
        assert_eq!(a.end_offset, None);
        assert!(cat.aggregates["B"].policy.is_some());
    }

    #[test]
    fn test_other_views_exclude_aggregates() {
        let mut rows = metric_rows();
        rows.aggregates = vec![agg("HourlyMetric", "Metric", "_m1", None)];
        rows.views = vec![
            ViewRow {
                name: "HourlyMetric".into(),
                is_materialized: false,
            },
            ViewRow {
                name: "report".into(),
                is_materialized: true,
            },
        ];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.other_views.len(), 1);
        assert!(cat.other_views["report"]);
    }

    /// `HourlyMetric` over `source` and `DailyMetric` over `HourlyMetric`, with
    /// refresh policies, view rows and columns.
    fn add_hierarchy(rows: &mut CatalogRows, source: &str) {
        rows.aggregates = vec![
            agg("HourlyMetric", source, "_m1", None),
            agg("DailyMetric", "_m1", "_m2", None),
        ];
        rows.jobs
            .push(job("policy_refresh_continuous_aggregate", "HourlyMetric"));
        rows.jobs
            .push(job("policy_refresh_continuous_aggregate", "_m2"));
        for view in ["HourlyMetric", "DailyMetric"] {
            rows.views.push(ViewRow {
                name: view.into(),
                is_materialized: false,
            });
            rows.columns.push(ColumnRow {
                relation: view.into(),
                name: "bucket".into(),
                udt_name: "timestamptz".into(),
                nullable: true,
            });
        }
    }

    /// Both aggregates of [`add_hierarchy`] are unsupported (with a reason
    /// containing each of `reason_parts`) and appear nowhere else.
    #[track_caller]
    fn assert_hierarchy_unsupported(cat: &TimescaleCatalog, reason_parts: &[&str]) {
        for view in ["HourlyMetric", "DailyMetric"] {
            let reason = &cat.unsupported_aggregates[view];
            for part in reason_parts {
                assert!(reason.contains(part), "{view}: {reason}");
            }
            assert!(!cat.unsupported_hypertables.contains_key(view), "{view}");
            assert!(!cat.aggregates.contains_key(view), "{view}");
            assert!(!cat.aggregate_definitions.contains_key(view), "{view}");
            assert!(!cat.aggregate_columns.contains_key(view), "{view}");
            assert!(!cat.other_views.contains_key(view), "{view}");
        }
    }

    #[test]
    fn test_aggregates_over_integer_time_hypertable_are_unsupported() {
        let mut rows = metric_rows();
        rows.dimensions[0].interval = None;
        add_hierarchy(&mut rows, "Metric");
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_hierarchy_unsupported(
            &cat,
            &[
                "built on \"Metric\"",
                "integer time dimensions are not supported",
            ],
        );
        assert!(!cat.unsupported_aggregates.contains_key("Metric"));
    }

    #[test]
    fn test_aggregates_over_created_before_policy_hypertable_are_unsupported() {
        let mut rows = metric_rows();
        rows.jobs = vec![job("policy_retention", "Metric")];
        add_hierarchy(&mut rows, "Metric");
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_hierarchy_unsupported(&cat, &["built on \"Metric\"", "drop_created_before"]);
    }

    #[test]
    fn test_aggregates_over_unknown_relation_are_unsupported() {
        // A hypertable in another schema (the query qualifies its name), and
        // a relation missing from the catalog.
        for source in ["other.Metric", "Gone"] {
            let mut rows = metric_rows();
            add_hierarchy(&mut rows, source);
            let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
            assert_hierarchy_unsupported(
                &cat,
                &[
                    &format!("built on \"{source}\""),
                    "not a hypertable in this schema",
                ],
            );
            assert!(cat.hypertables.contains_key("Metric"));
        }
    }

    #[test]
    fn test_aggregates_over_supported_hypertable_are_kept() {
        let mut rows = metric_rows();
        add_hierarchy(&mut rows, "Metric");
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(cat.unsupported_hypertables.is_empty() && cat.unsupported_aggregates.is_empty());
        assert_eq!(cat.aggregates["DailyMetric"].source, "HourlyMetric");
        assert!(cat.aggregates["DailyMetric"].policy.is_some());
        assert_eq!(cat.aggregate_columns.len(), 2);
    }

    #[test]
    fn test_timescale_version_supported() {
        assert!(!timescale_version_supported("2.17.2"));
        assert!(timescale_version_supported("2.18.0"));
        assert!(timescale_version_supported("2.30.2"));
        assert!(timescale_version_supported("3.0.0"));
        assert!(!timescale_version_supported("garbage"));
        let (major, minor) = TIMESCALE_MIN_VERSION;
        assert!(timescale_version_supported(&format!("{major}.{minor}.0")));
        assert!(!timescale_version_supported(&format!(
            "{major}.{}.9",
            minor - 1
        )));
    }

    #[test]
    fn test_version_error() {
        let catalog = |version: &str| TimescaleCatalog {
            version: version.into(),
            ..Default::default()
        };
        let (major, minor) = TIMESCALE_MIN_VERSION;
        assert_eq!(catalog(&format!("{major}.{minor}.0")).version_error(), None);
        assert_eq!(catalog("3.0.0").version_error(), None);
        for version in ["2.17.2", "garbage"] {
            assert_eq!(
                catalog(version).version_error(),
                Some(format!(
                    "TimescaleDB {version} is installed, but Prax requires TimescaleDB \
                     {major}.{minor} or newer for @@timescale.* attributes"
                ))
            );
        }
    }

    #[test]
    fn test_queries_are_schema_scoped() {
        use super::queries::*;
        for (name, sql) in [
            ("HYPERTABLES", HYPERTABLES.to_string()),
            ("dimensions_sql", dimensions_sql()),
            ("COLUMNSTORE_SETTINGS", COLUMNSTORE_SETTINGS.to_string()),
            ("jobs_sql", jobs_sql()),
            ("AGGREGATES", AGGREGATES.to_string()),
            ("COLUMNS", COLUMNS.to_string()),
            ("VIEWS", VIEWS.to_string()),
        ] {
            assert!(sql.contains("= $1"), "{name} is not scoped to the schema");
        }
        assert!(!EXTENSIONS.contains('$'));
    }

    /// The alias of each top-level `SELECT` output column of `sql`, in order
    /// (empty for a column without `AS`). Skips parenthesized and
    /// single-quoted text, and stops at the top-level `FROM`.
    fn select_aliases(sql: &str) -> Vec<String> {
        let body = sql.trim_start().strip_prefix("SELECT ").unwrap();
        let mut items = Vec::new();
        let mut depth = 0usize;
        let mut quoted = false;
        let mut start = 0;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                '\'' => quoted = !quoted,
                _ if quoted => {}
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    items.push(&body[start..i]);
                    start = i + 1;
                }
                _ if depth == 0 && body[i..].starts_with(" FROM ") => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        items.push(&body[start..end]);
        items
            .iter()
            .map(|item| {
                item.trim()
                    .rsplit_once(" AS ")
                    .map(|(_, alias)| alias.trim().to_string())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// `(index, alias)` for the three columns of an interval at `idx`.
    fn interval_cols(idx: usize, alias: &str) -> [(usize, String); 3] {
        [
            (idx, format!("{alias}_months")),
            (idx + 1, format!("{alias}_days")),
            (idx + 2, format!("{alias}_micros")),
        ]
    }

    #[test]
    fn test_query_aliases_match_column_indices() {
        fn named(pairs: &[(usize, &str)]) -> Vec<(usize, String)> {
            pairs.iter().map(|(i, a)| (*i, (*a).to_string())).collect()
        }

        let mut dimensions = named(&[
            (cols::dimensions::TABLE, "table_name"),
            (cols::dimensions::COLUMN, "column_name"),
            (cols::dimensions::IS_TIME, "is_time"),
            (cols::dimensions::PARTITIONS, "num_partitions"),
        ]);
        dimensions.extend(interval_cols(
            cols::dimensions::CHUNK_INTERVAL,
            "chunk_interval",
        ));

        let mut jobs = named(&[
            (cols::jobs::PROC_NAME, "proc_name"),
            (cols::jobs::RELATION, "relation"),
        ]);
        for (idx, alias) in [
            (cols::jobs::DROP_AFTER, "drop_after"),
            (cols::jobs::COMPRESS_AFTER, "compress_after"),
            (cols::jobs::START_OFFSET, "start_offset"),
            (cols::jobs::END_OFFSET, "end_offset"),
            (cols::jobs::SCHEDULE_INTERVAL, "schedule_interval"),
        ] {
            jobs.extend(interval_cols(idx, alias));
        }

        let cases = [
            (
                "EXTENSIONS",
                queries::EXTENSIONS.to_string(),
                named(&[
                    (cols::extensions::NAME, "name"),
                    (cols::extensions::VERSION, "version"),
                ]),
            ),
            (
                "HYPERTABLES",
                queries::HYPERTABLES.to_string(),
                named(&[
                    (cols::hypertables::TABLE, "table_name"),
                    (
                        cols::hypertables::COLUMNSTORE_ENABLED,
                        "columnstore_enabled",
                    ),
                ]),
            ),
            ("dimensions_sql", queries::dimensions_sql(), dimensions),
            (
                "COLUMNSTORE_SETTINGS",
                queries::COLUMNSTORE_SETTINGS.to_string(),
                named(&[
                    (cols::columnstore_settings::RELATION, "relation"),
                    (cols::columnstore_settings::SEGMENT_BY, "segment_by"),
                    (cols::columnstore_settings::ORDER_BY, "order_by"),
                ]),
            ),
            ("jobs_sql", queries::jobs_sql(), jobs),
            (
                "AGGREGATES",
                queries::AGGREGATES.to_string(),
                named(&[
                    (cols::aggregates::VIEW, "view_name"),
                    (cols::aggregates::HYPERTABLE, "hypertable_name"),
                    (
                        cols::aggregates::MATERIALIZATION_HYPERTABLE,
                        "materialization_hypertable_name",
                    ),
                    (cols::aggregates::MATERIALIZED_ONLY, "materialized_only"),
                    (cols::aggregates::VIEW_DEFINITION, "view_definition"),
                    (cols::aggregates::COMMENT, "view_comment"),
                ]),
            ),
            (
                "COLUMNS",
                queries::COLUMNS.to_string(),
                named(&[
                    (cols::columns::RELATION, "relation"),
                    (cols::columns::NAME, "column_name"),
                    (cols::columns::UDT_NAME, "udt_name"),
                    (cols::columns::NULLABLE, "nullable"),
                ]),
            ),
            (
                "VIEWS",
                queries::VIEWS.to_string(),
                named(&[
                    (cols::views::NAME, "view_name"),
                    (cols::views::IS_MATERIALIZED, "is_materialized"),
                ]),
            ),
        ];

        for (name, sql, mut expected) in cases {
            expected.sort();
            let aliases = select_aliases(&sql);
            let expected_aliases: Vec<String> = expected.iter().map(|(_, a)| a.clone()).collect();
            let indices: Vec<usize> = expected.iter().map(|(i, _)| *i).collect();
            assert_eq!(
                indices,
                (0..expected.len()).collect::<Vec<_>>(),
                "{name}: column indices are not 0..{}",
                expected.len()
            );
            assert_eq!(aliases, expected_aliases, "{name}: SELECT aliases");
        }
    }

    #[test]
    fn test_select_aliases_helper() {
        assert_eq!(
            select_aliases(
                "SELECT f(a, 'x,y') AS one, CASE WHEN b IN (SELECT c FROM d) THEN 1 END AS two \
                 FROM t WHERE e AS f"
            ),
            vec!["one".to_string(), "two".to_string()]
        );
        assert_eq!(
            select_aliases("SELECT a, b AS c FROM t"),
            vec![String::new(), "c".into()]
        );
    }

    #[test]
    fn test_timescale_version_at_least() {
        assert!(timescale_version_at_least("2.30.2", 2, 30));
        assert!(timescale_version_at_least("3.0.0", 2, 30));
        assert!(!timescale_version_at_least("2.29.9", 2, 30));
        assert!(!timescale_version_at_least("2.18.2", 2, 30));
        assert!(!timescale_version_at_least("garbage", 2, 30));
    }

    #[test]
    fn test_interval_parts_sql_casts() {
        let s = interval_parts_sql("x", "iv");
        assert!(s.contains("::int4") && s.contains("::int8"));
        assert!(
            s.contains("* 3600000000") && s.contains("* 60000000"),
            "{s}"
        );
        assert_eq!(
            select_aliases(&format!("SELECT {s} FROM t")),
            vec!["iv_months", "iv_days", "iv_micros"]
        );
    }
}
