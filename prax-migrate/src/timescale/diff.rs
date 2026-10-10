//! Diff the target schema's TimescaleDB state against the live catalog.
//!
//! Source state comes from the [`TimescaleCatalog`], never from the source
//! schema (spec §4.3). Both sides are compared in table/column-name form; see
//! [`resolve_hypertables`].

use prax_schema::Schema;
use prax_schema::ast::{Model, PgInterval};

use crate::diff::SchemaDiff;
use crate::error::{MigrateResult, MigrationError};
use crate::timescale::aggregates::diff_aggregates;
use crate::timescale::catalog::{
    AggregatePolicy, AggregateState, ColumnstoreSettings, ColumnstoreState, HypertableState,
    TimescaleCatalog, timescale_version_at_least,
};
use crate::timescale::resolve::resolve_hypertables;

/// TimescaleDB changes between the live database and the target schema.
#[derive(Debug, Clone, Default)]
pub struct TimescaleDiff {
    /// Changes grouped by table, tables in name order.
    pub changes: Vec<TimescaleChange>,
    /// Warnings not attached to a single change.
    pub warnings: Vec<String>,
}

impl TimescaleDiff {
    /// Whether there are no changes. Warnings alone do not make a diff
    /// non-empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// One description per gated change: its note, or `<Kind> on "<relation>"`
    /// when it has none.
    #[must_use]
    pub fn gated_notes(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|c| c.gated)
            .map(TimescaleChange::description)
            .collect()
    }
}

/// One TimescaleDB change.
#[derive(Debug, Clone)]
pub struct TimescaleChange {
    /// What changes.
    pub kind: TimescaleChangeKind,
    /// Whether the change needs `--allow-destructive` (spec §4.1).
    pub gated: bool,
    /// User-facing explanation, emitted as a migration warning.
    pub note: Option<String>,
}

impl TimescaleChange {
    /// The note, or `<Kind> on "<relation>"` when the change has none.
    pub(crate) fn description(&self) -> String {
        self.note.clone().unwrap_or_else(|| {
            let (kind, relation) = self.kind.label();
            format!("{kind} on \"{relation}\"")
        })
    }
}

/// The kinds of TimescaleDB change. Each carries the old value for down SQL.
#[derive(Debug, Clone)]
pub enum TimescaleChangeKind {
    /// Turn a table into a hypertable (and add its space dimension).
    CreateHypertable {
        /// Table name.
        table: String,
        /// Time dimension column.
        time_column: String,
        /// Chunk interval.
        chunk_interval: PgInterval,
        /// Hash space dimension `(column, partitions)`.
        space: Option<(String, i16)>,
        /// Whether the table already exists and its rows must be moved into
        /// chunks (`migrate_data => true`).
        migrate_data: bool,
    },
    /// Change the chunk interval used for new chunks.
    SetChunkInterval {
        /// Table name.
        table: String,
        /// Previous interval.
        old: PgInterval,
        /// New interval.
        new: PgInterval,
    },
    /// Change the number of space partitions used for new chunks.
    SetPartitions {
        /// Table name.
        table: String,
        /// Previous partition count.
        old: i16,
        /// New partition count.
        new: i16,
    },
    /// Enable the columnstore or change its settings.
    ConfigureColumnstore {
        /// Table name.
        table: String,
        /// Previous settings; `None` when the columnstore was disabled.
        old: Option<ColumnstoreSettings>,
        /// New settings.
        new: ColumnstoreSettings,
    },
    /// Remove the columnstore policy, convert every chunk back to rowstore and
    /// disable the columnstore.
    DisableColumnstore {
        /// Table name.
        table: String,
        /// Previous columnstore configuration, including its policy.
        old: ColumnstoreState,
    },
    /// Add, change or remove the columnstore policy.
    ReplaceColumnstorePolicy {
        /// Table name.
        table: String,
        /// Previous `after` interval; `None` when there was no policy.
        old: Option<PgInterval>,
        /// New `after` interval; `None` removes the policy.
        new: Option<PgInterval>,
    },
    /// Add, change or remove the retention policy.
    ReplaceRetentionPolicy {
        /// Table name.
        table: String,
        /// Previous `drop_after`; `None` when there was no policy.
        old: Option<PgInterval>,
        /// New `drop_after`; `None` removes the policy.
        new: Option<PgInterval>,
    },
    /// Drop continuous aggregates and the views they replace, then create
    /// aggregates (spec §5.5, §5.6). A diff holds at most one ungated set
    /// (creates and plain-view drops) and one gated set.
    AggregateSet {
        /// Aggregates to drop, children before parents.
        drop_aggregates: Vec<AggregateState>,
        /// Non-aggregate views to drop before an aggregate of the same name is
        /// created; the flag is `true` for a materialized view.
        drop_views: Vec<(String, bool)>,
        /// Aggregates to create, parents before children.
        create: Vec<AggregateState>,
        /// Dropped aggregates the target declares as plain or materialized
        /// views; the generic pipeline creates those views
        /// (`SchemaDiff::create_views`).
        handoff_views: Vec<String>,
        /// `(table, column)` type changes and drops in the generic diff that
        /// force this recreation (spec §4.8).
        coupled_columns: Vec<(String, String)>,
    },
    /// Add, change or remove a continuous aggregate's refresh policy.
    ReplaceAggregatePolicy {
        /// View name.
        view: String,
        /// Previous policy; `None` when there was none.
        old: Option<AggregatePolicy>,
        /// New policy; `None` removes the policy.
        new: Option<AggregatePolicy>,
    },
    /// Change a continuous aggregate's `timescaledb.materialized_only`.
    SetMaterializedOnly {
        /// View name.
        view: String,
        /// Previous value.
        old: bool,
        /// New value.
        new: bool,
    },
}

impl TimescaleChangeKind {
    /// The variant name and the relation it applies to.
    fn label(&self) -> (&'static str, &str) {
        match self {
            Self::CreateHypertable { table, .. } => ("CreateHypertable", table),
            Self::SetChunkInterval { table, .. } => ("SetChunkInterval", table),
            Self::SetPartitions { table, .. } => ("SetPartitions", table),
            Self::ConfigureColumnstore { table, .. } => ("ConfigureColumnstore", table),
            Self::DisableColumnstore { table, .. } => ("DisableColumnstore", table),
            Self::ReplaceColumnstorePolicy { table, .. } => ("ReplaceColumnstorePolicy", table),
            Self::ReplaceRetentionPolicy { table, .. } => ("ReplaceRetentionPolicy", table),
            Self::AggregateSet {
                drop_aggregates,
                drop_views,
                create,
                ..
            } => (
                "AggregateSet",
                create
                    .first()
                    .or_else(|| drop_aggregates.first())
                    .map(|a| a.view.as_str())
                    .or_else(|| drop_views.first().map(|(v, _)| v.as_str()))
                    .unwrap_or_default(),
            ),
            Self::ReplaceAggregatePolicy { view, .. } => ("ReplaceAggregatePolicy", view),
            Self::SetMaterializedOnly { view, .. } => ("SetMaterializedOnly", view),
        }
    }
}

/// Diff the target's hypertables and continuous aggregates against the live
/// catalog.
///
/// `generic` is the schema diff built so far: a table it creates is new and
/// empty (adding its retention policy is not gated), while a table in `source`
/// that it does not create already exists and is converted with
/// `migrate_data`. Its column type changes and drops force the recreation of
/// the aggregates reading that table (spec §4.8). Without a catalog the database
/// has no Timescale state. Hypertable changes come first, tables in name
/// order, then aggregate changes, so `changes` is stable.
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] when a change cannot be made in
/// place (spec §4.3): changing a hypertable's time column; adding, removing or
/// changing its space column; adding `space` to an existing table; a hypertable
/// whose model does not declare `@@timescale.hypertable`; declaring
/// `@@timescale.*` on a relation the catalog lists as unsupported; declaring a
/// continuous aggregate `source` that differs from the database while its SQL
/// is unchanged; declaring a continuous aggregate without SQL; or, on a
/// hypertable whose columnstore is enabled, changing a column's type, dropping
/// a `segmentby`/`orderby` column, or (before TimescaleDB 2.30) making a column
/// `NOT NULL` (spec §8 item 10).
pub fn diff_timescale(
    target: &Schema,
    source: Option<&Schema>,
    catalog: Option<&TimescaleCatalog>,
    generic: &SchemaDiff,
) -> MigrateResult<TimescaleDiff> {
    let target_model = |table: &str| target.models.values().find(|m| m.table_name() == table);

    if let Some(catalog) = catalog {
        for (relation, reason) in &catalog.unsupported {
            if target_model(relation).is_some_and(declares_timescale) {
                return Err(invalid(format!(
                    "Table \"{relation}\" declares @@timescale.* attributes, but its TimescaleDB \
                     setup cannot be managed by Prax: {reason}. Remove the @@timescale.* \
                     attributes from its model to leave it unmanaged, or reconfigure the \
                     hypertable manually."
                )));
            }
        }
        for table in catalog.hypertables.keys() {
            if target_model(table).is_some_and(|m| m.timescale().is_none()) {
                return Err(invalid(format!(
                    "Table \"{table}\" is a hypertable in the database, but its model does not \
                     declare @@timescale.hypertable. Declare @@timescale.hypertable on the model, \
                     or remove the table from the schema; converting a hypertable back to a \
                     plain table is not supported (to do it manually, create a plain table, copy \
                     the rows into it, and drop the hypertable)."
                )));
            }
        }
        check_columnstore_column_alters(catalog, generic)?;
    }

    let mut diff = TimescaleDiff::default();
    for (table, want) in resolve_hypertables(target) {
        match catalog.and_then(|c| c.hypertables.get(&table)) {
            Some(have) => alter_hypertable(have, &want, &mut diff.changes)?,
            None => {
                let created = generic.create_models.iter().any(|m| m.table_name == table);
                let exists = !created
                    && source.is_some_and(|s| s.models.values().any(|m| m.table_name() == table));
                create_hypertable(&want, created, exists, &mut diff.changes)?;
            }
        }
    }
    diff_aggregates(target, catalog, generic, &mut diff)?;
    Ok(diff)
}

/// Whether `target` opts into TimescaleDB management: `timescaledb` is in its
/// datasource `extensions` (aliases canonicalized), or a model or view declares
/// an `@@timescale.*` attribute.
///
/// Only an opted-in target should be diffed against the database's Timescale
/// state (see [`crate::SchemaDiffer::with_timescale_catalog`]); `prax migrate`
/// does not even read that state otherwise, so hypertables and continuous
/// aggregates managed by hand never block migrations of a schema that does not
/// use TimescaleDB attributes.
#[must_use]
pub fn opts_into_timescale(target: &Schema) -> bool {
    let extension = target.datasource.as_ref().is_some_and(|ds| {
        ds.extensions
            .iter()
            .any(|e| prax_schema::ast::canonical_extension_name(&e.name) == "timescaledb")
    });
    let declared = |attrs: &[prax_schema::ast::Attribute]| {
        attrs
            .iter()
            .any(|a| a.name.as_str().starts_with("timescale."))
    };
    extension
        || target.models.values().any(|m| declared(&m.attributes))
        || target.views.values().any(|v| declared(&v.attributes))
}

fn invalid(message: String) -> MigrationError {
    MigrationError::InvalidMigration(message)
}

/// The first TimescaleDB version observed to accept `SET NOT NULL` on a
/// hypertable with columnstore chunks; 2.18 rejects it (spec §8 item 10).
/// Versions in between were not probed and are treated as rejecting it.
const SET_NOT_NULL_ON_COLUMNSTORE_SINCE: (u32, u32) = (2, 30);

/// How to make a column change TimescaleDB rejects while the columnstore is
/// enabled.
const DISABLE_COLUMNSTORE_FIRST: &str = "Disable the columnstore in a separate migration first \
     (remove @@timescale.columnstore from the model and migrate with --allow-destructive, which \
     converts every chunk back to rowstore), then make this change, then restore \
     @@timescale.columnstore.";

/// Refuse the column changes TimescaleDB rejects on a hypertable whose
/// columnstore is enabled (spec §4.8, §8 item 10): changing a column's type,
/// dropping a `segmentby` or `orderby` column, and (before 2.30) `SET NOT NULL`.
fn check_columnstore_column_alters(
    catalog: &TimescaleCatalog,
    generic: &SchemaDiff,
) -> MigrateResult<()> {
    let (major, minor) = SET_NOT_NULL_ON_COLUMNSTORE_SINCE;
    let set_not_null_accepted = timescale_version_at_least(&catalog.version, major, minor);
    for alter in &generic.alter_models {
        let table = &alter.table_name;
        let Some(columnstore) = catalog
            .hypertables
            .get(table)
            .and_then(|h| h.columnstore.as_ref())
        else {
            continue;
        };
        for field in &alter.alter_fields {
            let column = &field.column_name;
            if field.new_type.is_some() {
                return Err(invalid(format!(
                    "Cannot change the type of column \"{column}\" of hypertable \"{table}\" \
                     while its columnstore is enabled: TimescaleDB rejects ALTER COLUMN ... TYPE \
                     on a hypertable with columnstore chunks. {DISABLE_COLUMNSTORE_FIRST}"
                )));
            }
            if field.new_nullable == Some(false) && !set_not_null_accepted {
                return Err(invalid(format!(
                    "Cannot make column \"{column}\" of hypertable \"{table}\" required while \
                     its columnstore is enabled: TimescaleDB {} rejects SET NOT NULL on a \
                     hypertable with columnstore chunks (TimescaleDB {major}.{minor} and newer \
                     accept it). {DISABLE_COLUMNSTORE_FIRST}",
                    catalog.version
                )));
            }
        }
        let settings = &columnstore.settings;
        for column in &alter.drop_fields {
            if settings.segment_by.contains(column)
                || settings.order_by.iter().any(|(c, _)| c == column)
            {
                return Err(invalid(format!(
                    "Cannot drop column \"{column}\" from hypertable \"{table}\": it is a \
                     columnstore segmentBy or orderBy column, and TimescaleDB rejects dropping \
                     those while the columnstore is enabled (columnstore chunks keep using them \
                     even after the settings change). {DISABLE_COLUMNSTORE_FIRST}"
                )));
            }
        }
    }
    Ok(())
}

fn declares_timescale(model: &Model) -> bool {
    model
        .attributes
        .iter()
        .any(|a| a.name.as_str().starts_with("timescale."))
}

fn change(kind: TimescaleChangeKind, gated: bool, note: Option<String>) -> TimescaleChange {
    TimescaleChange { kind, gated, note }
}

/// A table without a hypertable in the catalog. `created`: the table is created
/// in this migration, so it is empty. `exists`: it already exists and holds
/// rows to convert.
fn create_hypertable(
    want: &HypertableState,
    created: bool,
    exists: bool,
    out: &mut Vec<TimescaleChange>,
) -> MigrateResult<()> {
    let table = &want.table;
    if exists && let Some((column, _)) = &want.space {
        return Err(invalid(format!(
            "Cannot add a space dimension on \"{column}\" to existing table \"{table}\": \
             TimescaleDB's add_dimension only works on an empty hypertable. Remove `space` from \
             @@timescale.hypertable, or create a new hypertable with the space dimension, copy \
             the rows into it, and swap the tables manually."
        )));
    }
    let note = exists.then(|| {
        format!(
            "Converting existing table \"{table}\" to a hypertable rewrites its rows into chunks; \
             it holds locks while it runs, may be slow on large tables, and cannot be undone by \
             the down migration."
        )
    });
    out.push(change(
        TimescaleChangeKind::CreateHypertable {
            table: table.clone(),
            time_column: want.time_column.clone(),
            chunk_interval: want.chunk_interval,
            space: want.space.clone(),
            migrate_data: exists,
        },
        false,
        note,
    ));
    columnstore_changes(table, None, want.columnstore.as_ref(), out);
    match want.retention {
        // A table created in this migration holds no data the policy could drop.
        Some(new) if created => out.push(change(
            TimescaleChangeKind::ReplaceRetentionPolicy {
                table: table.clone(),
                old: None,
                new: Some(new),
            },
            false,
            None,
        )),
        _ => retention_change(table, None, want.retention, out),
    }
    Ok(())
}

/// A hypertable in both the catalog and the target.
fn alter_hypertable(
    have: &HypertableState,
    want: &HypertableState,
    out: &mut Vec<TimescaleChange>,
) -> MigrateResult<()> {
    let table = &want.table;
    if have.time_column != want.time_column {
        return Err(invalid(format!(
            "Cannot change the time column of hypertable \"{table}\" from \"{}\" to \"{}\": \
             TimescaleDB cannot repartition an existing hypertable. Create a new hypertable on \
             \"{}\", copy the rows into it, and swap the tables manually.",
            have.time_column, want.time_column, want.time_column
        )));
    }
    let partitions = match (&have.space, &want.space) {
        (None, None) => None,
        (None, Some((column, _))) => {
            return Err(invalid(format!(
                "Cannot add a space dimension on \"{column}\" to existing hypertable \"{table}\": \
                 TimescaleDB's add_dimension only works on an empty hypertable. Remove `space` \
                 from @@timescale.hypertable, or create a new hypertable with the space \
                 dimension, copy the rows into it, and swap the tables manually."
            )));
        }
        (Some((column, _)), None) => {
            return Err(invalid(format!(
                "Cannot remove the space dimension on \"{column}\" from hypertable \"{table}\": \
                 TimescaleDB cannot drop a dimension. Restore `space` on the field mapped to \
                 \"{column}\", or create a new hypertable without it, copy the rows into it, and \
                 swap the tables manually."
            )));
        }
        (Some((old_column, old)), Some((new_column, new))) => {
            if old_column != new_column {
                return Err(invalid(format!(
                    "Cannot change the space dimension of hypertable \"{table}\" from \
                     \"{old_column}\" to \"{new_column}\": TimescaleDB cannot repartition an \
                     existing hypertable. Restore `space` on the field mapped to \
                     \"{old_column}\", or create a new hypertable, copy the rows into it, and \
                     swap the tables manually."
                )));
            }
            (old != new).then_some((*old, *new))
        }
    };

    if chunk_width_differs(have.chunk_interval, want.chunk_interval) {
        let (old, new) = (have.chunk_interval, want.chunk_interval);
        out.push(change(
            TimescaleChangeKind::SetChunkInterval {
                table: table.clone(),
                old,
                new,
            },
            false,
            Some(format!(
                "Changing the chunk interval of \"{table}\" from {old} to {new} applies to new \
                 chunks only; existing chunks keep their current size."
            )),
        ));
    }
    if let Some((old, new)) = partitions {
        out.push(change(
            TimescaleChangeKind::SetPartitions {
                table: table.clone(),
                old,
                new,
            },
            false,
            Some(format!(
                "Changing the number of space partitions of \"{table}\" from {old} to {new} \
                 applies to new chunks only; existing chunks keep their current partitioning."
            )),
        ));
    }
    columnstore_changes(
        table,
        have.columnstore.as_ref(),
        want.columnstore.as_ref(),
        out,
    );
    retention_change(table, have.retention, want.retention, out);
    Ok(())
}

/// Timescale stores chunk width as fixed microseconds, so `1 day` equals
/// `24 hours` here.
fn chunk_width_differs(old: PgInterval, new: PgInterval) -> bool {
    match (old.fixed_micros(), new.fixed_micros()) {
        (Some(old), Some(new)) => old != new,
        _ => old != new,
    }
}

fn columnstore_changes(
    table: &str,
    old: Option<&ColumnstoreState>,
    new: Option<&ColumnstoreState>,
    out: &mut Vec<TimescaleChange>,
) {
    match (old, new) {
        (None, None) => {}
        (None, Some(new)) => {
            out.push(change(
                TimescaleChangeKind::ConfigureColumnstore {
                    table: table.to_string(),
                    old: None,
                    new: new.settings.clone(),
                },
                false,
                None,
            ));
            if new.after.is_some() {
                out.push(change(
                    TimescaleChangeKind::ReplaceColumnstorePolicy {
                        table: table.to_string(),
                        old: None,
                        new: new.after,
                    },
                    false,
                    None,
                ));
            }
        }
        (Some(old), None) => out.push(change(
            TimescaleChangeKind::DisableColumnstore {
                table: table.to_string(),
                old: old.clone(),
            },
            true,
            Some(format!(
                "Disabling the columnstore on \"{table}\" converts every columnstore chunk back \
                 to rowstore, which can take a long time and needs extra disk space while it \
                 runs."
            )),
        )),
        (Some(old), Some(new)) => {
            if old.settings != new.settings {
                out.push(change(
                    TimescaleChangeKind::ConfigureColumnstore {
                        table: table.to_string(),
                        old: Some(old.settings.clone()),
                        new: new.settings.clone(),
                    },
                    false,
                    Some(format!(
                        "Changing the columnstore settings of \"{table}\" affects only chunks \
                         not yet converted to the columnstore; converted chunks keep their \
                         previous segmentby and orderby."
                    )),
                ));
            }
            if old.after != new.after {
                out.push(change(
                    TimescaleChangeKind::ReplaceColumnstorePolicy {
                        table: table.to_string(),
                        old: old.after,
                        new: new.after,
                    },
                    false,
                    None,
                ));
            }
        }
    }
}

/// Adding retention to an existing table or shortening `dropAfter` is gated
/// (tables created in this migration are handled by `create_hypertable`).
/// Shortening compares conservatively (`new.lower < old.upper`), so
/// month-based intervals that might be shorter count as shorter.
fn retention_change(
    table: &str,
    old: Option<PgInterval>,
    new: Option<PgInterval>,
    out: &mut Vec<TimescaleChange>,
) {
    if old == new {
        return;
    }
    let gated_note = match (old, new) {
        (None, Some(new)) => Some(format!(
            "Adding a retention policy to \"{table}\" automatically drops chunks older than \
             {new}."
        )),
        (Some(old), Some(new)) if new.lower_micros() < old.upper_micros() => Some(format!(
            "Shortening the retention policy of \"{table}\" from {old} to {new} automatically \
             drops chunks older than {new}."
        )),
        _ => None,
    };
    out.push(change(
        TimescaleChangeKind::ReplaceRetentionPolicy {
            table: table.to_string(),
            old,
            new,
        },
        gated_note.is_some(),
        gated_note,
    ));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use prax_schema::Schema;
    use prax_schema::ast::PgInterval;

    use super::*;
    use crate::diff::{SchemaDiff, SchemaDiffer};
    use crate::error::{MigrateResult, MigrationError};
    use crate::timescale::catalog::{
        ColumnstoreSettings, ColumnstoreState, HypertableState, TimescaleCatalog,
    };

    const DATASOURCE: &str = "datasource db {\n  provider   = \"postgresql\"\n  url        = \"postgres://x\"\n  extensions = [timescaledb]\n}\n";

    const HYPERTABLE: &str =
        "@@timescale.hypertable(time, chunkInterval: \"1 day\", space: hash(deviceId, 4))";
    const COLUMNSTORE: &str =
        "@@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: \"7 days\")";
    const RETENTION: &str = "@@timescale.retention(dropAfter: \"90 days\")";

    const fn days(n: i32) -> PgInterval {
        PgInterval::new(0, n, 0)
    }

    fn schema(prax: &str) -> Schema {
        prax_schema::parse_schema(prax).unwrap()
    }

    /// The §3.1 `Metric` model carrying the given model attributes.
    fn metric(attrs: &[&str]) -> String {
        format!(
            "{DATASOURCE}\nmodel Metric {{\n  time     DateTime\n  deviceId String\n  value    Float\n\n  @@id([deviceId, time])\n  {}\n}}\n",
            attrs.join("\n  ")
        )
    }

    /// The §3.1 model with every Timescale attribute.
    fn full_metric() -> String {
        metric(&[HYPERTABLE, COLUMNSTORE, RETENTION])
    }

    /// `Metric` as introspected: a plain table.
    fn plain_metric() -> String {
        metric(&[])
    }

    fn metric_state() -> HypertableState {
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
    }

    fn catalog(h: Vec<HypertableState>) -> TimescaleCatalog {
        TimescaleCatalog {
            version: "2.18.0".into(),
            hypertables: h.into_iter().map(|s| (s.table.clone(), s)).collect(),
            aggregates: BTreeMap::new(),
            aggregate_definitions: BTreeMap::new(),
            aggregate_columns: BTreeMap::new(),
            other_views: BTreeMap::new(),
            unsupported: BTreeMap::new(),
        }
    }

    fn run(
        target: &str,
        source: Option<&str>,
        catalog: Option<TimescaleCatalog>,
    ) -> MigrateResult<TimescaleDiff> {
        let mut differ = SchemaDiffer::new(schema(target));
        if let Some(source) = source {
            differ = differ.with_source(schema(source));
        }
        if let Some(catalog) = catalog {
            differ = differ.with_timescale_catalog(catalog);
        }
        differ.diff().map(|d| d.timescale)
    }

    fn invalid(result: MigrateResult<TimescaleDiff>) -> String {
        match result {
            Err(MigrationError::InvalidMigration(msg)) => msg,
            other => panic!("expected InvalidMigration, got {other:?}"),
        }
    }

    fn single(diff: &TimescaleDiff) -> &TimescaleChange {
        assert_eq!(diff.changes.len(), 1, "changes: {:#?}", diff.changes);
        &diff.changes[0]
    }

    fn note(change: &TimescaleChange) -> &str {
        change.note.as_deref().unwrap_or("")
    }

    #[test]
    fn test_new_table_creates_hypertable_without_migrate_data() {
        let diff = run(&full_metric(), None, None).unwrap();
        assert_eq!(diff.changes.len(), 4, "changes: {:#?}", diff.changes);

        let create = &diff.changes[0];
        let TimescaleChangeKind::CreateHypertable {
            table,
            time_column,
            chunk_interval,
            space,
            migrate_data,
        } = &create.kind
        else {
            panic!("expected CreateHypertable, got {:?}", create.kind);
        };
        assert_eq!(table, "Metric");
        assert_eq!(time_column, "time");
        assert_eq!(*chunk_interval, days(1));
        assert_eq!(space, &Some(("deviceId".to_string(), 4)));
        assert!(!migrate_data);
        assert!(!create.gated);
        assert!(create.note.is_none());

        let TimescaleChangeKind::ConfigureColumnstore { table, old, new } = &diff.changes[1].kind
        else {
            panic!(
                "expected ConfigureColumnstore, got {:?}",
                diff.changes[1].kind
            );
        };
        assert_eq!(table, "Metric");
        assert!(old.is_none());
        assert_eq!(new.segment_by, vec!["deviceId".to_string()]);
        assert_eq!(new.order_by, vec![("time".to_string(), true)]);
        assert!(!diff.changes[1].gated);

        assert!(matches!(
            &diff.changes[2].kind,
            TimescaleChangeKind::ReplaceColumnstorePolicy { table, old: None, new: Some(n) }
                if table == "Metric" && *n == days(7)
        ));
        assert!(!diff.changes[2].gated);

        assert!(matches!(
            &diff.changes[3].kind,
            TimescaleChangeKind::ReplaceRetentionPolicy { table, old: None, new: Some(n) }
                if table == "Metric" && *n == days(90)
        ));
        assert!(
            !diff.changes[3].gated,
            "retention on a table created in this migration is ungated"
        );
        assert!(diff.gated_notes().is_empty());
    }

    #[test]
    fn test_existing_table_converts_with_migrate_data() {
        let target = metric(&["@@timescale.hypertable(time, chunkInterval: \"1 day\")"]);
        let diff = run(&target, Some(&plain_metric()), None).unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::CreateHypertable { table, space: None, migrate_data: true, .. }
                if table == "Metric"
        ));
        assert!(!change.gated);
        assert!(note(change).contains("rewrites its rows"), "{change:?}");
        assert!(note(change).contains("\"Metric\""), "{change:?}");
    }

    #[test]
    fn test_existing_table_conversion_with_retention_is_gated() {
        let target = metric(&[
            "@@timescale.hypertable(time, chunkInterval: \"1 day\")",
            RETENTION,
        ]);
        let diff = run(&target, Some(&plain_metric()), None).unwrap();
        assert_eq!(diff.changes.len(), 2, "changes: {:#?}", diff.changes);
        assert!(matches!(
            &diff.changes[0].kind,
            TimescaleChangeKind::CreateHypertable {
                migrate_data: true,
                ..
            }
        ));
        let retention = &diff.changes[1];
        assert!(matches!(
            &retention.kind,
            TimescaleChangeKind::ReplaceRetentionPolicy { table, old: None, new: Some(n) }
                if table == "Metric" && *n == days(90)
        ));
        assert!(retention.gated);
        assert!(
            note(retention).contains("automatically drops chunks older than 90 days"),
            "{retention:?}"
        );
        assert_eq!(diff.gated_notes(), vec![note(retention).to_string()]);
    }

    #[test]
    fn test_retention_stays_gated_when_table_is_neither_created_nor_in_source() {
        // An inconsistent `generic` (table not created, no source schema): the
        // table is treated as new for `migrate_data`, but retention stays gated.
        let diff =
            diff_timescale(&schema(&full_metric()), None, None, &SchemaDiff::default()).unwrap();
        assert!(matches!(
            &diff.changes[0].kind,
            TimescaleChangeKind::CreateHypertable {
                migrate_data: false,
                ..
            }
        ));
        let retention = diff.changes.last().unwrap();
        assert!(matches!(
            &retention.kind,
            TimescaleChangeKind::ReplaceRetentionPolicy {
                old: None,
                new: Some(_),
                ..
            }
        ));
        assert!(retention.gated);
    }

    #[test]
    fn test_space_on_existing_table_is_invalid() {
        let msg = invalid(run(&metric(&[HYPERTABLE]), Some(&plain_metric()), None));
        assert!(msg.contains("space"), "{msg}");
        assert!(msg.contains("empty"), "{msg}");
        assert!(msg.contains("Metric"), "{msg}");
    }

    #[test]
    fn test_time_column_change_is_invalid() {
        let mut state = metric_state();
        state.time_column = "recordedAt".into();
        let msg = invalid(run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![state])),
        ));
        assert!(msg.contains("time column"), "{msg}");
        assert!(msg.contains("recordedAt"), "{msg}");
        assert!(msg.contains("\"time\""), "{msg}");
    }

    #[test]
    fn test_space_change_on_existing_hypertable_is_invalid() {
        // Added.
        let mut state = metric_state();
        state.space = None;
        let msg = invalid(run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![state])),
        ));
        assert!(msg.contains("space"), "{msg}");
        assert!(msg.contains("empty"), "{msg}");

        // Removed.
        let target = metric(&[
            "@@timescale.hypertable(time, chunkInterval: \"1 day\")",
            COLUMNSTORE,
            RETENTION,
        ]);
        let msg = invalid(run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        ));
        assert!(msg.contains("space"), "{msg}");
        assert!(msg.contains("remove"), "{msg}");

        // Column changed.
        let mut state = metric_state();
        state.space = Some(("value".into(), 4));
        let msg = invalid(run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![state])),
        ));
        assert!(msg.contains("\"value\""), "{msg}");
        assert!(msg.contains("\"deviceId\""), "{msg}");
    }

    #[test]
    fn test_removing_hypertable_from_existing_table_is_invalid() {
        let msg = invalid(run(
            &plain_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        ));
        assert!(
            msg.contains(
                "is a hypertable in the database, but its model does not declare \
                 @@timescale.hypertable"
            ),
            "{msg}"
        );
        assert!(msg.contains("remove the table from the schema"), "{msg}");
        assert!(msg.contains("\"Metric\""), "{msg}");
    }

    #[test]
    fn test_opts_into_timescale() {
        // `timescaledb` in the datasource extensions.
        assert!(opts_into_timescale(&schema(&plain_metric())));
        // An `@@timescale.*` attribute on a model or a view, even without the
        // extension (validation rejects that, but it still opts in).
        assert!(opts_into_timescale(&schema(
            "model M {\n  time DateTime\n  @@id([time])\n  @@timescale.hypertable(time)\n}\n"
        )));
        assert!(opts_into_timescale(&schema(
            "view V {\n  bucket DateTime\n  @@timescale.continuous(source: M, scheduleInterval: \"1 hour\")\n}\n"
        )));
        // Neither.
        assert!(!opts_into_timescale(&schema(
            "model M {\n  time DateTime\n  @@id([time])\n}\n"
        )));
        assert!(!opts_into_timescale(&schema(
            "datasource db {\n  provider   = \"postgresql\"\n  url        = \"postgres://x\"\n  extensions = [pgvector]\n}\nmodel M {\n  time DateTime\n  @@id([time])\n}\n"
        )));
    }

    #[test]
    fn test_chunk_interval_equivalent_fixed_width_is_unchanged() {
        let mut state = metric_state();
        state.chunk_interval = PgInterval::new(0, 0, 86_400_000_000);
        let diff = run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![state])),
        )
        .unwrap();
        assert!(diff.is_empty(), "changes: {:#?}", diff.changes);
    }

    #[test]
    fn test_chunk_interval_change() {
        let target = metric(&[
            "@@timescale.hypertable(time, chunkInterval: \"2 days\", space: hash(deviceId, 4))",
            COLUMNSTORE,
            RETENTION,
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::SetChunkInterval { table, old, new }
                if table == "Metric" && *old == days(1) && *new == days(2)
        ));
        assert!(!change.gated);
        assert!(note(change).contains("new chunks only"), "{change:?}");
    }

    #[test]
    fn test_partition_count_change() {
        let target = metric(&[
            "@@timescale.hypertable(time, chunkInterval: \"1 day\", space: hash(deviceId, 8))",
            COLUMNSTORE,
            RETENTION,
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::SetPartitions { table, old: 4, new: 8 } if table == "Metric"
        ));
        assert!(!change.gated);
        assert!(note(change).contains("new chunks only"), "{change:?}");
    }

    #[test]
    fn test_columnstore_enable() {
        let mut state = metric_state();
        state.columnstore = None;
        let diff = run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![state])),
        )
        .unwrap();
        assert_eq!(diff.changes.len(), 2, "changes: {:#?}", diff.changes);
        let TimescaleChangeKind::ConfigureColumnstore { table, old, new } = &diff.changes[0].kind
        else {
            panic!(
                "expected ConfigureColumnstore, got {:?}",
                diff.changes[0].kind
            );
        };
        assert_eq!(table, "Metric");
        assert!(old.is_none());
        assert_eq!(new, &metric_state().columnstore.unwrap().settings);
        assert!(matches!(
            &diff.changes[1].kind,
            TimescaleChangeKind::ReplaceColumnstorePolicy { table, old: None, new: Some(n) }
                if table == "Metric" && *n == days(7)
        ));
        assert!(diff.changes.iter().all(|c| !c.gated));
    }

    #[test]
    fn test_columnstore_settings_change() {
        let target = metric(&[
            HYPERTABLE,
            "@@timescale.columnstore(orderBy: [desc(time)], after: \"7 days\")",
            RETENTION,
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        let TimescaleChangeKind::ConfigureColumnstore { table, old, new } = &change.kind else {
            panic!("expected ConfigureColumnstore, got {:?}", change.kind);
        };
        assert_eq!(table, "Metric");
        assert_eq!(
            old.as_ref(),
            Some(&metric_state().columnstore.unwrap().settings)
        );
        assert!(new.segment_by.is_empty());
        assert_eq!(new.order_by, vec![("time".to_string(), true)]);
        assert!(!change.gated);
        assert!(
            note(change).contains("chunks not yet converted"),
            "{change:?}"
        );
    }

    #[test]
    fn test_columnstore_policy_change() {
        let target = metric(&[
            HYPERTABLE,
            "@@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: \"14 days\")",
            RETENTION,
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::ReplaceColumnstorePolicy { table, old: Some(o), new: Some(n) }
                if table == "Metric" && *o == days(7) && *n == days(14)
        ));
        assert!(!change.gated);
    }

    #[test]
    fn test_columnstore_disable_is_gated() {
        let diff = run(
            &metric(&[HYPERTABLE, RETENTION]),
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        let TimescaleChangeKind::DisableColumnstore { table, old } = &change.kind else {
            panic!("expected DisableColumnstore, got {:?}", change.kind);
        };
        assert_eq!(table, "Metric");
        assert_eq!(Some(old), metric_state().columnstore.as_ref());
        assert!(change.gated);
        assert!(note(change).contains("rowstore"), "{change:?}");
        assert_eq!(diff.gated_notes(), vec![note(change).to_string()]);
    }

    #[test]
    fn test_retention_add_is_gated() {
        let mut state = metric_state();
        state.retention = None;
        let diff = run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![state])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::ReplaceRetentionPolicy { table, old: None, new: Some(n) }
                if table == "Metric" && *n == days(90)
        ));
        assert!(change.gated);
        assert!(
            note(change).contains("automatically drops chunks older than 90 days"),
            "{change:?}"
        );
    }

    #[test]
    fn test_retention_shorten_is_gated() {
        let target = metric(&[
            HYPERTABLE,
            COLUMNSTORE,
            "@@timescale.retention(dropAfter: \"30 days\")",
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::ReplaceRetentionPolicy { table, old: Some(o), new: Some(n) }
                if table == "Metric" && *o == days(90) && *n == days(30)
        ));
        assert!(change.gated);
        assert!(
            note(change).contains("automatically drops chunks older than 30 days"),
            "{change:?}"
        );
    }

    #[test]
    fn test_retention_lengthen_is_ungated() {
        let target = metric(&[
            HYPERTABLE,
            COLUMNSTORE,
            "@@timescale.retention(dropAfter: \"180 days\")",
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::ReplaceRetentionPolicy { old: Some(o), new: Some(n), .. }
                if *o == days(90) && *n == days(180)
        ));
        assert!(!change.gated);
        assert!(diff.gated_notes().is_empty());
    }

    #[test]
    fn test_retention_shorten_uses_conservative_month_bounds() {
        // 3 months may be as short as 84 days, which is less than 90 days.
        let target = metric(&[
            HYPERTABLE,
            COLUMNSTORE,
            "@@timescale.retention(dropAfter: \"3 months\")",
        ]);
        let diff = run(
            &target,
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        assert!(single(&diff).gated);
    }

    #[test]
    fn test_retention_remove_is_ungated() {
        let diff = run(
            &metric(&[HYPERTABLE, COLUMNSTORE]),
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        let change = single(&diff);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::ReplaceRetentionPolicy { table, old: Some(o), new: None }
                if table == "Metric" && *o == days(90)
        ));
        assert!(!change.gated);
    }

    #[test]
    fn test_map_resolution_no_churn() {
        let target = format!(
            "{DATASOURCE}\nmodel Metric {{\n  ts       DateTime @map(\"time\")\n  deviceId String   @map(\"device_id\")\n  value    Float\n\n  @@id([deviceId, ts])\n  @@map(\"metrics\")\n  @@timescale.hypertable(ts, chunkInterval: \"1 day\", space: hash(deviceId, 4))\n  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(ts)], after: \"7 days\")\n  {RETENTION}\n}}\n"
        );
        let source = format!(
            "{DATASOURCE}\nmodel Metrics {{\n  time      DateTime\n  device_id String\n  value     Float\n\n  @@id([device_id, time])\n  @@map(\"metrics\")\n}}\n"
        );
        let mut state = metric_state();
        state.table = "metrics".into();
        state.space = Some(("device_id".into(), 4));
        state.columnstore = Some(ColumnstoreState {
            settings: ColumnstoreSettings {
                segment_by: vec!["device_id".into()],
                order_by: vec![("time".into(), true)],
            },
            after: Some(days(7)),
        });
        let diff = run(&target, Some(&source), Some(catalog(vec![state]))).unwrap();
        assert!(diff.is_empty(), "changes: {:#?}", diff.changes);
    }

    #[test]
    fn test_resolve_hypertables_maps_fields_to_columns() {
        let target = format!(
            "{DATASOURCE}\nmodel Metric {{\n  ts       DateTime @map(\"time\")\n  deviceId String   @map(\"device_id\")\n\n  @@map(\"metrics\")\n  @@timescale.hypertable(ts, space: hash(deviceId, 2))\n}}\n"
        );
        let resolved = crate::timescale::resolve_hypertables(&schema(&target));
        let state = &resolved["metrics"];
        assert_eq!(state.table, "metrics");
        assert_eq!(state.time_column, "time");
        assert_eq!(state.chunk_interval, days(7));
        assert_eq!(state.space, Some(("device_id".to_string(), 2)));
        assert!(state.columnstore.is_none());
        assert!(state.retention.is_none());
    }

    #[test]
    fn test_matching_catalog_is_empty() {
        let diff = run(
            &full_metric(),
            Some(&plain_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        assert!(diff.is_empty(), "changes: {:#?}", diff.changes);
    }

    #[test]
    fn test_dropping_hypertable_model_emits_no_timescale_changes() {
        let target = format!("{DATASOURCE}\nmodel Device {{\n  id String @id\n}}\n");
        let diff = run(
            &target,
            Some(&full_metric()),
            Some(catalog(vec![metric_state()])),
        )
        .unwrap();
        assert!(diff.changes.is_empty(), "changes: {:#?}", diff.changes);
    }

    #[test]
    fn test_target_on_unsupported_relation_is_invalid() {
        let mut cat = catalog(vec![]);
        cat.unsupported
            .insert("Metric".into(), "integer time dimension".into());
        let msg = invalid(run(&full_metric(), Some(&plain_metric()), Some(cat)));
        assert!(msg.contains("integer time dimension"), "{msg}");
        assert!(msg.contains("\"Metric\""), "{msg}");
    }

    #[test]
    fn test_unsupported_relation_without_timescale_is_ignored() {
        let mut cat = catalog(vec![]);
        cat.unsupported
            .insert("Metric".into(), "integer time dimension".into());
        let diff = run(&plain_metric(), Some(&plain_metric()), Some(cat)).unwrap();
        assert!(diff.changes.is_empty(), "changes: {:#?}", diff.changes);
    }

    #[test]
    fn test_changes_are_ordered_by_table() {
        let target = format!(
            "{DATASOURCE}\nmodel Zeta {{\n  time DateTime\n  @@id([time])\n  @@timescale.hypertable(time)\n}}\nmodel Alpha {{\n  time DateTime\n  @@id([time])\n  @@timescale.hypertable(time)\n}}\n"
        );
        let diff = run(&target, None, None).unwrap();
        let tables: Vec<&str> = diff
            .changes
            .iter()
            .map(|c| match &c.kind {
                TimescaleChangeKind::CreateHypertable { table, .. } => table.as_str(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(tables, vec!["Alpha", "Zeta"]);
    }

    #[test]
    fn test_schema_diff_is_empty_includes_timescale() {
        let diff = SchemaDiff {
            timescale: TimescaleDiff {
                changes: vec![TimescaleChange {
                    kind: TimescaleChangeKind::SetPartitions {
                        table: "Metric".into(),
                        old: 4,
                        new: 8,
                    },
                    gated: false,
                    note: None,
                }],
                warnings: Vec::new(),
            },
            ..Default::default()
        };
        assert!(!diff.timescale.is_empty());
        assert!(!diff.is_empty());
        assert_ne!(diff.summary(), "No changes");
    }

    /// `Metric` with the given extra fields and model attributes.
    fn metric_fields(fields: &str, attrs: &[&str]) -> String {
        format!(
            "{DATASOURCE}\nmodel Metric {{\n  time     DateTime\n  deviceId String\n{fields}\n\n  @@id([deviceId, time])\n  {}\n}}\n",
            attrs.join("\n  ")
        )
    }

    /// Diff a field change on hypertable `Metric` in `state` at `version`.
    fn columnstore_alter(
        source_fields: &str,
        target_fields: &str,
        state: HypertableState,
        version: &str,
    ) -> MigrateResult<TimescaleDiff> {
        let mut catalog = catalog(vec![state]);
        catalog.version = version.into();
        run(
            &metric_fields(target_fields, &[HYPERTABLE, COLUMNSTORE, RETENTION]),
            Some(&metric_fields(source_fields, &[])),
            Some(catalog),
        )
    }

    fn with_settings(segment_by: &[&str], order_by: &[(&str, bool)]) -> HypertableState {
        let mut state = metric_state();
        if let Some(cs) = state.columnstore.as_mut() {
            cs.settings = ColumnstoreSettings {
                segment_by: segment_by.iter().map(|c| (*c).to_string()).collect(),
                order_by: order_by
                    .iter()
                    .map(|(c, d)| ((*c).to_string(), *d))
                    .collect(),
            };
        }
        state
    }

    #[test]
    fn test_type_change_on_columnstore_hypertable_is_invalid() {
        for version in ["2.18.2", "2.30.2"] {
            let msg = invalid(columnstore_alter(
                "  value    Float",
                "  value    Int",
                metric_state(),
                version,
            ));
            assert!(
                msg.contains("type of column \"value\" of hypertable \"Metric\""),
                "{msg}"
            );
            assert!(msg.contains("--allow-destructive"), "{msg}");
        }
    }

    #[test]
    fn test_type_change_without_columnstore_is_allowed() {
        let mut state = metric_state();
        state.columnstore = None;
        assert!(columnstore_alter("  value    Float", "  value    Int", state, "2.18.2").is_ok());
    }

    #[test]
    fn test_dropping_segmentby_or_orderby_column_is_invalid() {
        let state = with_settings(&["region"], &[("time", true)]);
        let msg = invalid(columnstore_alter(
            "  value    Float\n  region   String?",
            "  value    Float",
            state,
            "2.30.2",
        ));
        assert!(
            msg.contains("drop column \"region\" from hypertable \"Metric\""),
            "{msg}"
        );

        let state = with_settings(&["deviceId"], &[("seq", false), ("time", true)]);
        let msg = invalid(columnstore_alter(
            "  value    Float\n  seq      Int?",
            "  value    Float",
            state,
            "2.30.2",
        ));
        assert!(msg.contains("drop column \"seq\""), "{msg}");
    }

    #[test]
    fn test_dropping_other_column_on_columnstore_hypertable_is_allowed() {
        assert!(
            columnstore_alter(
                "  value    Float\n  note     String?",
                "  value    Float",
                metric_state(),
                "2.18.2",
            )
            .is_ok()
        );
    }

    #[test]
    fn test_set_not_null_on_columnstore_hypertable_depends_on_version() {
        let msg = invalid(columnstore_alter(
            "  value    Float?",
            "  value    Float",
            metric_state(),
            "2.18.2",
        ));
        assert!(
            msg.contains("\"value\" of hypertable \"Metric\" required") && msg.contains("2.18.2"),
            "{msg}"
        );
        assert!(
            columnstore_alter(
                "  value    Float?",
                "  value    Float",
                metric_state(),
                "2.30.2"
            )
            .is_ok()
        );
        // DROP NOT NULL is accepted on every version.
        assert!(
            columnstore_alter(
                "  value    Float",
                "  value    Float?",
                metric_state(),
                "2.18.2"
            )
            .is_ok()
        );
    }
}
