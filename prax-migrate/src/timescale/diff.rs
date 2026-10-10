//! Diff the target schema's TimescaleDB state against the live catalog.
//!
//! Source state comes from the [`TimescaleCatalog`], never from the source
//! schema (spec §4.3). Both sides are compared in table/column-name form; see
//! [`resolve_hypertables`].

use prax_schema::Schema;
use prax_schema::ast::{Model, PgInterval};

use crate::diff::SchemaDiff;
use crate::error::{MigrateResult, MigrationError};
use crate::timescale::catalog::{
    ColumnstoreSettings, ColumnstoreState, HypertableState, TimescaleCatalog,
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
        }
    }
}

/// Diff the target's hypertables against the live catalog.
///
/// `generic` is the schema diff built so far: a table it creates is new and
/// empty (adding its retention policy is not gated), while a table in `source`
/// that it does not create already exists and is converted with
/// `migrate_data`. Without a catalog the database has no Timescale state.
/// Tables are visited in name order, so `changes` is stable.
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] when a change cannot be made in
/// place (spec §4.3): changing a hypertable's time column; adding, removing or
/// changing its space column; adding `space` to an existing table; removing
/// `@@timescale.hypertable` from an existing hypertable; or declaring
/// `@@timescale.*` on a relation the catalog lists as unsupported.
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
                    "Table \"{table}\" is a hypertable and cannot be converted back to a plain \
                     table. Restore @@timescale.hypertable on its model, or create a plain table, \
                     copy the rows into it, and drop the hypertable manually."
                )));
            }
        }
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
    Ok(diff)
}

fn invalid(message: String) -> MigrationError {
    MigrationError::InvalidMigration(message)
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
        assert!(msg.contains("cannot be converted back"), "{msg}");
        assert!(msg.contains("\"Metric\""), "{msg}");
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
}
