//! The live database's TimescaleDB state, built from plain catalog rows.
//!
//! prax-migrate has no database driver: the CLI runs the [`queries`] and fills
//! [`CatalogRows`], and [`TimescaleCatalog::from_rows`] assembles the state.

use std::collections::BTreeMap;

use prax_schema::ast::PgInterval;

use crate::error::{MigrateResult, MigrationError};

/// An interval decomposed as `(months, days, micros)`.
pub type IntervalParts = (i32, i32, i64);

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

/// Whether a TimescaleDB version string is at least 2.18.
#[must_use]
pub fn timescale_version_supported(version: &str) -> bool {
    timescale_version_at_least(version, 2, 18)
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
/// `int4, int4, int8`. A `NULL` expression yields three `NULL`s.
#[must_use]
pub fn interval_parts_sql(expr: &str) -> String {
    format!(
        "(EXTRACT(YEAR FROM {expr}) * 12 + EXTRACT(MONTH FROM {expr}))::int4, \
         EXTRACT(DAY FROM {expr})::int4, \
         (EXTRACT(HOUR FROM {expr}) * 3600000000 + EXTRACT(MINUTE FROM {expr}) * 60000000 \
         + EXTRACT(MICROSECONDS FROM {expr}))::int8"
    )
}

/// SQL run by the CLI to fill [`CatalogRows`]. Schema-scoped queries take the
/// schema name as `$1`; `COLUMNSTORE_SETTINGS` and `JOBS` are unfiltered because
/// their relations can live in `_timescaledb_internal`.
pub mod queries {
    /// `(extname, extversion)`.
    pub const EXTENSIONS: &str = "SELECT extname::text, extversion::text FROM pg_extension";

    /// `(hypertable_name, compression_enabled)`.
    pub const HYPERTABLES: &str = "SELECT hypertable_name::text, compression_enabled \
         FROM timescaledb_information.hypertables WHERE hypertable_schema = $1";

    /// `(hypertable_name, column_name, is_time, months, days, micros, num_partitions)`.
    pub const DIMENSIONS: &str = "SELECT hypertable_name::text, column_name::text, \
         dimension_type = 'Time', \
         (EXTRACT(YEAR FROM time_interval) * 12 + EXTRACT(MONTH FROM time_interval))::int4, \
         EXTRACT(DAY FROM time_interval)::int4, \
         (EXTRACT(HOUR FROM time_interval) * 3600000000 \
          + EXTRACT(MINUTE FROM time_interval) * 60000000 \
          + EXTRACT(MICROSECONDS FROM time_interval))::int8, \
         num_partitions::int2 \
         FROM timescaledb_information.dimensions WHERE hypertable_schema = $1 \
         ORDER BY hypertable_name, dimension_number";

    /// `(hypertable::text, segmentby, orderby)`.
    pub const COLUMNSTORE_SETTINGS: &str = "SELECT hypertable::text, segmentby::text, orderby::text \
         FROM timescaledb_information.hypertable_columnstore_settings";

    /// `(proc_name, hypertable_name, drop_after, compress_after, start_offset, end_offset,
    /// schedule_interval)`, each interval as three `(months, days, micros)` columns.
    pub const JOBS: &str = "SELECT proc_name::text, hypertable_name::text, \
         (EXTRACT(YEAR FROM (config->>'drop_after')::interval) * 12 \
          + EXTRACT(MONTH FROM (config->>'drop_after')::interval))::int4, \
         EXTRACT(DAY FROM (config->>'drop_after')::interval)::int4, \
         (EXTRACT(HOUR FROM (config->>'drop_after')::interval) * 3600000000 \
          + EXTRACT(MINUTE FROM (config->>'drop_after')::interval) * 60000000 \
          + EXTRACT(MICROSECONDS FROM (config->>'drop_after')::interval))::int8, \
         (EXTRACT(YEAR FROM (config->>'compress_after')::interval) * 12 \
          + EXTRACT(MONTH FROM (config->>'compress_after')::interval))::int4, \
         EXTRACT(DAY FROM (config->>'compress_after')::interval)::int4, \
         (EXTRACT(HOUR FROM (config->>'compress_after')::interval) * 3600000000 \
          + EXTRACT(MINUTE FROM (config->>'compress_after')::interval) * 60000000 \
          + EXTRACT(MICROSECONDS FROM (config->>'compress_after')::interval))::int8, \
         (EXTRACT(YEAR FROM (config->>'start_offset')::interval) * 12 \
          + EXTRACT(MONTH FROM (config->>'start_offset')::interval))::int4, \
         EXTRACT(DAY FROM (config->>'start_offset')::interval)::int4, \
         (EXTRACT(HOUR FROM (config->>'start_offset')::interval) * 3600000000 \
          + EXTRACT(MINUTE FROM (config->>'start_offset')::interval) * 60000000 \
          + EXTRACT(MICROSECONDS FROM (config->>'start_offset')::interval))::int8, \
         (EXTRACT(YEAR FROM (config->>'end_offset')::interval) * 12 \
          + EXTRACT(MONTH FROM (config->>'end_offset')::interval))::int4, \
         EXTRACT(DAY FROM (config->>'end_offset')::interval)::int4, \
         (EXTRACT(HOUR FROM (config->>'end_offset')::interval) * 3600000000 \
          + EXTRACT(MINUTE FROM (config->>'end_offset')::interval) * 60000000 \
          + EXTRACT(MICROSECONDS FROM (config->>'end_offset')::interval))::int8, \
         (EXTRACT(YEAR FROM schedule_interval) * 12 + EXTRACT(MONTH FROM schedule_interval))::int4, \
         EXTRACT(DAY FROM schedule_interval)::int4, \
         (EXTRACT(HOUR FROM schedule_interval) * 3600000000 \
          + EXTRACT(MINUTE FROM schedule_interval) * 60000000 \
          + EXTRACT(MICROSECONDS FROM schedule_interval))::int8 \
         FROM timescaledb_information.jobs \
         WHERE proc_name IN ('policy_compression', 'policy_retention', \
                             'policy_refresh_continuous_aggregate')";

    /// `(view_name, hypertable_name, materialization_hypertable_name, materialized_only,
    /// view_definition, comment)`.
    pub const AGGREGATES: &str = "SELECT view_name::text, hypertable_name::text, \
         materialization_hypertable_name::text, materialized_only, view_definition::text, \
         obj_description(format('%I.%I', view_schema, view_name)::regclass, 'pg_class') \
         FROM timescaledb_information.continuous_aggregates WHERE view_schema = $1";

    /// `(table_name, column_name, udt_name, is_nullable)` for the aggregates' views.
    pub const COLUMNS: &str = "SELECT table_name::text, column_name::text, udt_name::text, \
         is_nullable = 'YES' \
         FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name IN ( \
             SELECT view_name FROM timescaledb_information.continuous_aggregates \
             WHERE view_schema = $1) \
         ORDER BY table_name, ordinal_position";

    /// `(relname, is_materialized)` for views and materialized views.
    pub const VIEWS: &str = "SELECT c.relname::text, c.relkind = 'm' \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('v', 'm')";
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
    /// Source hypertable (or parent materialization hypertable).
    pub hypertable_name: String,
    /// This aggregate's own materialization hypertable.
    pub materialization_hypertable: String,
    /// Whether the view is materialized-only.
    pub materialized_only: bool,
    /// Postgres-normalized view definition.
    pub view_definition: String,
    /// View comment (may hold the §5.2 record).
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Valid Timescale relations Prax cannot represent (relation to reason);
    /// omitted from `hypertables`.
    pub unsupported: BTreeMap<String, String>,
}

fn interval(parts: IntervalParts) -> PgInterval {
    PgInterval::new(parts.0, parts.1, parts.2)
}

/// Reduce regclass text to the relation name: the last identifier of an
/// optionally schema-qualified name, split on unquoted dots (`""` becomes `"`).
fn unquote_relation(s: &str) -> String {
    let mut parts: Vec<String> = vec![String::new()];
    let mut chars = s.trim().chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                if let Some(p) = parts.last_mut() {
                    p.push('"');
                }
            }
            '"' => quoted = !quoted,
            '.' if !quoted => parts.push(String::new()),
            _ => {
                if let Some(p) = parts.last_mut() {
                    p.push(c);
                }
            }
        }
    }
    parts.pop().unwrap_or_default()
}

/// Parse a Timescale `segmentby`/`orderby` list into `(column, descending)`.
pub(crate) fn parse_column_list(s: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        let mut name = String::new();
        if first == '"' {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        name.push('"');
                    } else {
                        break;
                    }
                } else {
                    name.push(c);
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == ',' {
                    break;
                }
                name.push(c);
                chars.next();
            }
        }
        // Modifiers up to the next comma: only ASC/DESC matter.
        let mut descending = false;
        let mut modifiers = String::new();
        while let Some(&c) = chars.peek() {
            if c == ',' {
                break;
            }
            modifiers.push(c);
            chars.next();
        }
        for word in modifiers.split_whitespace() {
            if word.eq_ignore_ascii_case("desc") {
                descending = true;
            }
        }
        out.push((name, descending));
    }
    out
}

fn malformed(relation: &str, what: &str) -> MigrationError {
    MigrationError::Database(format!(
        "malformed TimescaleDB catalog for '{relation}': {what}"
    ))
}

impl TimescaleCatalog {
    /// Build the catalog from rows. Returns `None` when the `timescaledb`
    /// extension is not installed.
    ///
    /// # Errors
    ///
    /// Returns `MigrationError::Database` when a hypertable has no time
    /// dimension or a policy job lacks its required interval.
    pub fn from_rows(rows: CatalogRows) -> MigrateResult<Option<Self>> {
        let Some(ext) = rows.extensions.iter().find(|e| e.name == "timescaledb") else {
            return Ok(None);
        };
        let version = ext.version.clone();

        let mut hypertables = BTreeMap::new();
        let mut unsupported = BTreeMap::new();
        for h in &rows.hypertables {
            let time = rows
                .dimensions
                .iter()
                .find(|d| d.table == h.table && d.is_time)
                .ok_or_else(|| malformed(&h.table, "hypertable has no time dimension"))?;
            let Some(chunk_interval) = time.interval.map(interval) else {
                unsupported.insert(
                    h.table.clone(),
                    "integer time dimensions are not supported".to_string(),
                );
                continue;
            };
            let space = rows
                .dimensions
                .iter()
                .find(|d| d.table == h.table && !d.is_time && d.partitions.is_some())
                .and_then(|d| d.partitions.map(|p| (d.column.clone(), p)));

            let columnstore = if h.columnstore_enabled {
                let settings = rows
                    .columnstore_settings
                    .iter()
                    .find(|s| unquote_relation(&s.relation) == h.table);
                Some(ColumnstoreState {
                    settings: ColumnstoreSettings {
                        segment_by: settings
                            .and_then(|s| s.segment_by.as_deref())
                            .map(|s| parse_column_list(s).into_iter().map(|(n, _)| n).collect())
                            .unwrap_or_default(),
                        order_by: settings
                            .and_then(|s| s.order_by.as_deref())
                            .map(parse_column_list)
                            .unwrap_or_default(),
                    },
                    after: None,
                })
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
                "policy_compression" => {
                    if let Some(cs) = hypertables
                        .get_mut(&j.relation)
                        .and_then(|h| h.columnstore.as_mut())
                    {
                        match j.after {
                            Some(after) => cs.after = Some(interval(after)),
                            None => {
                                hypertables.remove(&j.relation);
                                unsupported.insert(
                                    j.relation.clone(),
                                    "policy_compression without a compress_after interval \
                                     (compress_created_before) is not supported"
                                        .to_string(),
                                );
                            }
                        }
                    }
                }
                "policy_retention" => {
                    if let Some(h) = hypertables.get_mut(&j.relation) {
                        match j.drop_after {
                            Some(drop_after) => h.retention = Some(interval(drop_after)),
                            None => {
                                hypertables.remove(&j.relation);
                                unsupported.insert(
                                    j.relation.clone(),
                                    "policy_retention without a drop_after interval \
                                     (drop_created_before) is not supported"
                                        .to_string(),
                                );
                            }
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
            .filter(|v| !aggregates.contains_key(&v.name))
            .map(|v| (v.name.clone(), v.is_materialized))
            .collect();

        Ok(Some(Self {
            version,
            hypertables,
            aggregates,
            aggregate_definitions,
            aggregate_columns,
            other_views,
            unsupported,
        }))
    }
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
        assert!(cat.unsupported["Metric"].contains("policy_compression"));
    }

    #[test]
    fn test_retention_job_without_drop_after_is_unsupported() {
        let mut rows = metric_rows();
        rows.jobs = vec![job("policy_retention", "Metric")];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(!cat.hypertables.contains_key("Metric"));
        assert!(cat.unsupported["Metric"].contains("policy_retention"));
    }

    #[test]
    fn test_integer_time_hypertable_is_unsupported() {
        let mut rows = metric_rows();
        rows.dimensions[0].interval = None;
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert!(cat.hypertables.is_empty());
        assert!(cat.unsupported["Metric"].contains("integer time"));
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
            vec![("time".to_string(), true), ("device_id".to_string(), false)]
        );
        assert_eq!(
            parse_column_list("\"we\"\"ird\" ASC NULLS LAST"),
            vec![("we\"ird".to_string(), false)]
        );
        assert_eq!(parse_column_list(""), Vec::<(String, bool)>::new());
    }

    #[test]
    fn test_aggregate_parent_derivation() {
        let mut rows = base_rows();
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
        let mut rows = base_rows();
        rows.aggregates = vec![
            agg("A", "M", "_m1", Some("prax:cagg:v1\nSELECT 1")),
            agg("B", "M", "_m2", None),
            agg("C", "M", "_m3", Some("hand written")),
        ];
        let cat = TimescaleCatalog::from_rows(rows).unwrap().unwrap();
        assert_eq!(cat.aggregates["A"].sql.as_deref(), Some("SELECT 1"));
        assert_eq!(cat.aggregates["B"].sql, None);
        assert_eq!(cat.aggregates["C"].sql, None);
        assert_eq!(aggregate_record("SELECT 1"), "prax:cagg:v1\nSELECT 1");
    }

    #[test]
    fn test_refresh_job_maps_by_view_or_materialization() {
        let mut rows = base_rows();
        rows.aggregates = vec![agg("A", "M", "_m1", None), agg("B", "M", "_m2", None)];
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
        let mut rows = base_rows();
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

    #[test]
    fn test_timescale_version_supported() {
        assert!(!timescale_version_supported("2.17.2"));
        assert!(timescale_version_supported("2.18.0"));
        assert!(timescale_version_supported("2.30.2"));
        assert!(timescale_version_supported("3.0.0"));
        assert!(!timescale_version_supported("garbage"));
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
        let s = interval_parts_sql("x");
        assert!(s.contains("::int4") && s.contains("::int8"));
    }
}
