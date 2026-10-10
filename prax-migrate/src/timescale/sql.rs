//! Render a [`TimescaleDiff`] as SQL in two phases (spec §4.4–§4.6).
//!
//! The pre-phase runs before the generic Postgres pipeline (removing policies,
//! converting chunks back to rowstore); the post-phase runs after it (creating
//! hypertables, configuring the columnstore, adding policies).

use prax_schema::ast::PgInterval;

use crate::timescale::aggregates::topo_order;
use crate::timescale::catalog::{
    AggregatePolicy, AggregateState, ColumnstoreSettings, aggregate_record,
};
use crate::timescale::diff::{TimescaleChange, TimescaleChangeKind, TimescaleDiff};
use crate::timescale::quote::{
    ident_list_literal, name_literal, quote_ident, quote_literal, regclass_literal,
};

/// SQL for one phase of a Timescale diff.
#[derive(Debug, Default)]
pub struct TimescaleSql {
    /// Statements to apply, in order.
    pub up: Vec<String>,
    /// Statements that undo this phase, in execution order (the reverse of
    /// `up`).
    pub down: Vec<String>,
    /// Warnings: change notes and irreversibility notices.
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Pre,
    Post,
}

/// Statements that run before the generic pipeline.
#[must_use]
pub fn pre(diff: &TimescaleDiff) -> TimescaleSql {
    render(diff, Phase::Pre)
}

/// Statements that run after the generic pipeline.
#[must_use]
pub fn post(diff: &TimescaleDiff) -> TimescaleSql {
    render(diff, Phase::Post)
}

fn render(diff: &TimescaleDiff, phase: Phase) -> TimescaleSql {
    let mut out = TimescaleSql::default();
    // (section, up, down) groups. A stable sort by section keeps change order
    // inside a section; downs are emitted in reverse group order.
    let mut groups: Vec<(u8, Vec<String>, Vec<String>)> = Vec::new();
    for change in &diff.changes {
        let (up, down, extra) = change_sql(change, phase);
        let has_statements = match &change.kind {
            TimescaleChangeKind::AggregateSet {
                drop_aggregates,
                drop_views,
                create,
                ..
            } => match phase {
                Phase::Pre => !drop_aggregates.is_empty() || !drop_views.is_empty(),
                Phase::Post => !create.is_empty(),
            },
            _ => !up.is_empty(),
        };
        if has_statements
            && let Some(note) = &change.note
            && owns_first_statement(&change.kind, phase)
        {
            out.warnings.push(note.clone());
        }
        out.warnings.extend(extra);
        groups.push((section(&change.kind, phase), up, down));
    }
    match phase {
        Phase::Pre => groups.push((1, aggregate_drops_up(diff), aggregate_drops_down(diff))),
        Phase::Post => {
            let (up, down, warnings) = aggregate_creates(diff);
            out.warnings.extend(warnings);
            groups.push((1, up, down));
        }
    }
    groups.sort_by_key(|(section, _, _)| *section);
    for (_, up, _) in &groups {
        out.up.extend(up.iter().cloned());
    }
    out.down = groups
        .into_iter()
        .rev()
        .flat_map(|(_, _, down)| down)
        .collect();
    out
}

/// The section a change's statements belong to within `phase`.
///
/// Pre: 0 policy removals, 1 aggregate drops, 2 columnstore teardown.
/// Post: 0 hypertable DDL and table policies, 1 aggregate creates,
/// 2 aggregate policies, 3 `materialized_only`.
fn section(kind: &TimescaleChangeKind, phase: Phase) -> u8 {
    match (kind, phase) {
        (TimescaleChangeKind::DisableColumnstore { .. }, Phase::Pre) => 2,
        (TimescaleChangeKind::ReplaceAggregatePolicy { .. }, Phase::Post) => 2,
        (TimescaleChangeKind::SetMaterializedOnly { .. }, Phase::Post) => 3,
        (TimescaleChangeKind::AggregateSet { .. }, _) => 1,
        (
            TimescaleChangeKind::DisableColumnstore { .. }
            | TimescaleChangeKind::ReplaceAggregatePolicy { .. }
            | TimescaleChangeKind::SetMaterializedOnly { .. }
            | TimescaleChangeKind::CreateHypertable { .. }
            | TimescaleChangeKind::SetChunkInterval { .. }
            | TimescaleChangeKind::SetPartitions { .. }
            | TimescaleChangeKind::ConfigureColumnstore { .. }
            | TimescaleChangeKind::ReplaceColumnstorePolicy { .. }
            | TimescaleChangeKind::ReplaceRetentionPolicy { .. },
            _,
        ) => 0,
    }
}

/// Every set's dropped aggregates (children first across sets), then views.
fn aggregate_drops_up(diff: &TimescaleDiff) -> Vec<String> {
    let mut up: Vec<String> = dropped_aggregates(diff)
        .iter()
        .map(|a| drop_materialized_view(&a.view))
        .collect();
    for change in &diff.changes {
        if let TimescaleChangeKind::AggregateSet { drop_views, .. } = &change.kind {
            for (view, materialized) in drop_views {
                up.push(if *materialized {
                    drop_materialized_view(view)
                } else {
                    format!("DROP VIEW IF EXISTS {};", quote_ident(view))
                });
            }
        }
    }
    up
}

/// Dropped aggregates in reverse topological order across all sets.
fn dropped_aggregates(diff: &TimescaleDiff) -> Vec<AggregateState> {
    let all: Vec<AggregateState> = diff
        .changes
        .iter()
        .filter_map(|c| match &c.kind {
            TimescaleChangeKind::AggregateSet {
                drop_aggregates, ..
            } => Some(drop_aggregates.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect();
    let mut ordered = topo_order(&all);
    ordered.reverse();
    ordered
}

/// The materialized data of a dropped aggregate cannot be restored.
fn aggregate_drops_down(diff: &TimescaleDiff) -> Vec<String> {
    dropped_aggregates(diff)
        .iter()
        .rev()
        .map(|a| {
            format!(
                "-- irreversible: materialized data of {} was discarded",
                quote_ident(&a.view)
            )
        })
        .collect()
}

/// Create statements for every set's aggregates in topological order, with
/// each create followed by its record comment, policy and backfill note.
fn aggregate_creates(diff: &TimescaleDiff) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut all = Vec::new();
    let mut gated_views: Vec<&str> = Vec::new();
    for change in &diff.changes {
        if let TimescaleChangeKind::AggregateSet { create, .. } = &change.kind {
            all.extend(create.iter().cloned());
            if change.gated {
                gated_views.extend(create.iter().map(|a| a.view.as_str()));
            }
        }
    }
    let ordered = topo_order(&all);
    let (mut up, mut down, mut warnings) = (Vec::new(), Vec::new(), Vec::new());
    for agg in &ordered {
        let Some(sql) = &agg.sql else {
            warnings.push(format!(
                "Continuous aggregate {} has no authored SQL and was not created",
                quote_ident(&agg.view)
            ));
            continue;
        };
        let view = quote_ident(&agg.view);
        let body = sql.trim_end();
        let body = body.strip_suffix(';').unwrap_or(body).trim_end();
        up.push(format!(
            "CREATE MATERIALIZED VIEW {view}\n  WITH (timescaledb.continuous, timescaledb.materialized_only = true) AS\n{body}\nWITH NO DATA;"
        ));
        up.push(format!(
            "COMMENT ON VIEW {view} IS {};",
            quote_literal(&aggregate_record(sql))
        ));
        match &agg.policy {
            Some(policy) => {
                up.push(add_aggregate_policy(&agg.view, policy));
                up.push(backfill_note(&agg.view, policy.start_offset.as_ref()));
            }
            None => up.push(format!(
                "-- {view} is created empty and has no refresh policy; refresh it manually."
            )),
        }
        warnings.push(format!(
            "Continuous aggregate {view} is created WITH NO DATA; see the backfill note in the migration"
        ));
        if !gated_views.contains(&agg.view.as_str()) {
            down.push(drop_materialized_view(&agg.view));
        }
    }
    down.reverse();
    (up, down, warnings)
}

fn backfill_note(name: &str, start_offset: Option<&PgInterval>) -> String {
    let view = quote_ident(name);
    let regclass = regclass_literal(name);
    match start_offset {
        Some(start) => format!(
            "-- {view} is created empty; its refresh policy only covers the last {start}.\n-- To backfill older data, run outside a transaction:\n-- CALL refresh_continuous_aggregate({regclass}, NULL, now() - {});",
            start.to_sql()
        ),
        None => format!("-- {view} is created empty; the first policy run materializes all data."),
    }
}

fn drop_materialized_view(view: &str) -> String {
    format!("DROP MATERIALIZED VIEW IF EXISTS {};", quote_ident(view))
}

fn add_aggregate_policy(view: &str, policy: &AggregatePolicy) -> String {
    let offset = |o: &Option<PgInterval>| {
        o.as_ref()
            .map_or_else(|| "NULL".to_string(), PgInterval::to_sql)
    };
    format!(
        "SELECT add_continuous_aggregate_policy({}, start_offset => {}, end_offset => {}, schedule_interval => {});",
        regclass_literal(view),
        offset(&policy.start_offset),
        offset(&policy.end_offset),
        policy.schedule_interval.to_sql()
    )
}

fn remove_aggregate_policy(view: &str) -> String {
    format!(
        "SELECT remove_continuous_aggregate_policy({}, if_exists => true);",
        regclass_literal(view)
    )
}

fn set_materialized_only(view: &str, value: bool) -> String {
    format!(
        "ALTER MATERIALIZED VIEW {} SET (timescaledb.materialized_only = {value});",
        quote_ident(view)
    )
}

/// Whether `phase` emits the change's first up statement.
fn owns_first_statement(kind: &TimescaleChangeKind, phase: Phase) -> bool {
    match kind {
        TimescaleChangeKind::ReplaceColumnstorePolicy { old, .. }
        | TimescaleChangeKind::ReplaceRetentionPolicy { old, .. } => {
            (phase == Phase::Pre) == old.is_some()
        }
        TimescaleChangeKind::DisableColumnstore { .. } => phase == Phase::Pre,
        TimescaleChangeKind::CreateHypertable { .. }
        | TimescaleChangeKind::SetChunkInterval { .. }
        | TimescaleChangeKind::SetPartitions { .. }
        | TimescaleChangeKind::ConfigureColumnstore { .. } => phase == Phase::Post,
        // Drops run in pre; a set without drops only creates (post).
        TimescaleChangeKind::AggregateSet {
            drop_aggregates,
            drop_views,
            ..
        } => (phase == Phase::Pre) == (!drop_aggregates.is_empty() || !drop_views.is_empty()),
        TimescaleChangeKind::ReplaceAggregatePolicy { old, .. } => {
            (phase == Phase::Pre) == old.is_some()
        }
        TimescaleChangeKind::SetMaterializedOnly { .. } => phase == Phase::Post,
    }
}

fn change_sql(change: &TimescaleChange, phase: Phase) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut up = Vec::new();
    let mut down = Vec::new();
    let mut warnings = Vec::new();
    match &change.kind {
        TimescaleChangeKind::CreateHypertable {
            table,
            time_column,
            chunk_interval,
            space,
            migrate_data,
        } => {
            if phase == Phase::Post {
                let migrate = if *migrate_data {
                    ", migrate_data => true"
                } else {
                    ""
                };
                up.push(format!(
                    "SELECT create_hypertable({}, by_range({}, {}), create_default_indexes => false{});",
                    regclass_literal(table),
                    name_literal(time_column),
                    chunk_interval.to_sql(),
                    migrate
                ));
                if let Some((column, partitions)) = space {
                    up.push(format!(
                        "SELECT add_dimension({}, by_hash({}, {}));",
                        regclass_literal(table),
                        name_literal(column),
                        partitions
                    ));
                }
                if *migrate_data {
                    down.push(format!(
                        "-- irreversible: {} was converted to a hypertable",
                        quote_ident(table)
                    ));
                    warnings.push(format!(
                        "Table {} was converted to a hypertable; the down migration cannot convert it back",
                        quote_ident(table)
                    ));
                }
            }
        }
        TimescaleChangeKind::SetChunkInterval { table, old, new } => {
            if phase == Phase::Post {
                up.push(set_chunk_interval(table, &new.to_sql()));
                down.push(set_chunk_interval(table, &old.to_sql()));
            }
        }
        TimescaleChangeKind::SetPartitions { table, old, new } => {
            if phase == Phase::Post {
                up.push(set_partitions(table, *new));
                down.push(set_partitions(table, *old));
            }
        }
        TimescaleChangeKind::ConfigureColumnstore { table, old, new } => {
            if phase == Phase::Post {
                up.push(configure_columnstore(table, new));
                match old {
                    None => down.extend(disable_columnstore(table)),
                    Some(old) => down.push(configure_columnstore(table, old)),
                }
            }
        }
        TimescaleChangeKind::DisableColumnstore { table, old } => {
            if phase == Phase::Pre {
                up.extend(disable_columnstore(table));
                down.push(configure_columnstore(table, &old.settings));
                if let Some(after) = &old.after {
                    down.push(add_columnstore_policy(table, &after.to_sql()));
                }
            }
        }
        TimescaleChangeKind::ReplaceColumnstorePolicy { table, old, new } => match phase {
            Phase::Pre => {
                if let Some(old) = old {
                    up.push(remove_columnstore_policy(table));
                    down.push(add_columnstore_policy(table, &old.to_sql()));
                }
            }
            Phase::Post => {
                if let Some(new) = new {
                    up.push(add_columnstore_policy(table, &new.to_sql()));
                    down.push(remove_columnstore_policy(table));
                }
            }
        },
        TimescaleChangeKind::ReplaceRetentionPolicy { table, old, new } => match phase {
            Phase::Pre => {
                if let Some(old) = old {
                    up.push(remove_retention_policy(table));
                    down.push(add_retention_policy(table, &old.to_sql()));
                }
            }
            Phase::Post => {
                if let Some(new) = new {
                    up.push(add_retention_policy(table, &new.to_sql()));
                    down.push(remove_retention_policy(table));
                }
            }
        },
        // Rendered across all sets by `render` (drops and creates are ordered
        // globally).
        TimescaleChangeKind::AggregateSet { .. } => {}
        TimescaleChangeKind::ReplaceAggregatePolicy { view, old, new } => match phase {
            Phase::Pre => {
                if let Some(old) = old {
                    up.push(remove_aggregate_policy(view));
                    down.push(add_aggregate_policy(view, old));
                }
            }
            Phase::Post => {
                if let Some(new) = new {
                    up.push(add_aggregate_policy(view, new));
                    down.push(remove_aggregate_policy(view));
                }
            }
        },
        TimescaleChangeKind::SetMaterializedOnly { view, old, new } => {
            if phase == Phase::Post {
                up.push(set_materialized_only(view, *new));
                down.push(set_materialized_only(view, *old));
            }
        }
    }
    (up, down, warnings)
}

fn set_chunk_interval(table: &str, interval: &str) -> String {
    format!(
        "SELECT set_chunk_time_interval({}, {});",
        regclass_literal(table),
        interval
    )
}

fn set_partitions(table: &str, n: i16) -> String {
    format!(
        "SELECT set_number_partitions({}, {});",
        regclass_literal(table),
        n
    )
}

fn configure_columnstore(table: &str, settings: &ColumnstoreSettings) -> String {
    let segment: Vec<(String, bool)> = settings
        .segment_by
        .iter()
        .map(|c| (c.clone(), false))
        .collect();
    format!(
        "ALTER TABLE {} SET (timescaledb.enable_columnstore = true, timescaledb.segmentby = {}, timescaledb.orderby = {});",
        quote_ident(table),
        ident_list_literal(&segment),
        ident_list_literal(&settings.order_by)
    )
}

fn remove_columnstore_policy(table: &str) -> String {
    format!(
        "CALL remove_columnstore_policy({}, if_exists => true);",
        regclass_literal(table)
    )
}

fn add_columnstore_policy(table: &str, after: &str) -> String {
    format!(
        "CALL add_columnstore_policy({}, after => {});",
        regclass_literal(table),
        after
    )
}

fn remove_retention_policy(table: &str) -> String {
    format!(
        "SELECT remove_retention_policy({}, if_exists => true);",
        regclass_literal(table)
    )
}

fn add_retention_policy(table: &str, drop_after: &str) -> String {
    format!(
        "SELECT add_retention_policy({}, drop_after => {});",
        regclass_literal(table),
        drop_after
    )
}

/// Remove the policy, convert every chunk to rowstore, disable the columnstore.
fn disable_columnstore(table: &str) -> Vec<String> {
    vec![
        remove_columnstore_policy(table),
        format!(
            "DO $prax$\nDECLARE chunk regclass;\nBEGIN\n  FOR chunk IN SELECT show_chunks({}) LOOP\n    CALL convert_to_rowstore(chunk, if_columnstore => true);\n  END LOOP;\nEND $prax$;",
            regclass_literal(table)
        ),
        format!(
            "ALTER TABLE {} SET (timescaledb.enable_columnstore = false);",
            quote_ident(table)
        ),
    ]
}

#[cfg(test)]
mod tests {
    use prax_schema::ast::PgInterval;

    use super::*;
    use crate::timescale::catalog::{AggregatePolicy, AggregateState, ColumnstoreState};

    const fn days(n: i32) -> PgInterval {
        PgInterval::new(0, n, 0)
    }

    fn one(kind: TimescaleChangeKind) -> TimescaleDiff {
        TimescaleDiff {
            changes: vec![TimescaleChange {
                kind,
                gated: false,
                note: None,
            }],
            warnings: Vec::new(),
        }
    }

    fn settings(seg: &[&str], ord: &[(&str, bool)]) -> ColumnstoreSettings {
        ColumnstoreSettings {
            segment_by: seg.iter().map(|s| (*s).to_string()).collect(),
            order_by: ord.iter().map(|(c, d)| ((*c).to_string(), *d)).collect(),
        }
    }

    const BLOCK: &str = "DO $prax$\nDECLARE chunk regclass;\nBEGIN\n  FOR chunk IN SELECT show_chunks('\"Metric\"') LOOP\n    CALL convert_to_rowstore(chunk, if_columnstore => true);\n  END LOOP;\nEND $prax$;";
    const RM_CS: &str = "CALL remove_columnstore_policy('\"Metric\"', if_exists => true);";
    const OFF: &str = "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = false);";
    const CS_ON: &str = "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, timescaledb.segmentby = '\"deviceId\"', timescaledb.orderby = '\"time\" DESC');";

    fn create(space: bool, migrate: bool) -> TimescaleDiff {
        one(TimescaleChangeKind::CreateHypertable {
            table: "Metric".into(),
            time_column: "time".into(),
            chunk_interval: days(1),
            space: space.then(|| ("deviceId".to_string(), 4)),
            migrate_data: migrate,
        })
    }

    #[test]
    fn test_create_hypertable_sql() {
        let d = create(true, false);
        assert!(pre(&d).up.is_empty());
        let p = post(&d);
        assert_eq!(
            p.up,
            vec![
                "SELECT create_hypertable('\"Metric\"', by_range('time', INTERVAL '1 day'), create_default_indexes => false);",
                "SELECT add_dimension('\"Metric\"', by_hash('deviceId', 4));",
            ]
        );
        assert!(p.down.is_empty());
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn test_create_hypertable_migrate_data_sql() {
        let p = post(&create(false, true));
        assert_eq!(
            p.up,
            vec![
                "SELECT create_hypertable('\"Metric\"', by_range('time', INTERVAL '1 day'), create_default_indexes => false, migrate_data => true);"
            ]
        );
        assert_eq!(
            p.down,
            vec!["-- irreversible: \"Metric\" was converted to a hypertable"]
        );
        assert_eq!(p.warnings.len(), 1);
    }

    #[test]
    fn test_set_chunk_interval_sql() {
        let p = post(&one(TimescaleChangeKind::SetChunkInterval {
            table: "Metric".into(),
            old: days(1),
            new: days(2),
        }));
        assert_eq!(
            p.up,
            vec!["SELECT set_chunk_time_interval('\"Metric\"', INTERVAL '2 days');"]
        );
        assert_eq!(
            p.down,
            vec!["SELECT set_chunk_time_interval('\"Metric\"', INTERVAL '1 day');"]
        );
    }

    #[test]
    fn test_set_partitions_sql() {
        let p = post(&one(TimescaleChangeKind::SetPartitions {
            table: "Metric".into(),
            old: 4,
            new: 8,
        }));
        assert_eq!(p.up, vec!["SELECT set_number_partitions('\"Metric\"', 8);"]);
        assert_eq!(
            p.down,
            vec!["SELECT set_number_partitions('\"Metric\"', 4);"]
        );
    }

    #[test]
    fn test_configure_columnstore_new_sql() {
        let d = one(TimescaleChangeKind::ConfigureColumnstore {
            table: "Metric".into(),
            old: None,
            new: settings(&["deviceId"], &[("time", true)]),
        });
        let p = post(&d);
        assert_eq!(p.up, vec![CS_ON]);
        assert_eq!(p.down, vec![RM_CS, BLOCK, OFF]);
    }

    #[test]
    fn test_configure_columnstore_empty_segment_sql() {
        let d = one(TimescaleChangeKind::ConfigureColumnstore {
            table: "Metric".into(),
            old: Some(settings(&["deviceId"], &[("time", true)])),
            new: settings(&[], &[("time", false)]),
        });
        let p = post(&d);
        assert_eq!(
            p.up,
            vec![
                "ALTER TABLE \"Metric\" SET (timescaledb.enable_columnstore = true, timescaledb.segmentby = '', timescaledb.orderby = '\"time\"');"
            ]
        );
        assert_eq!(p.down, vec![CS_ON]);
    }

    #[test]
    fn test_disable_columnstore_sql() {
        let d = one(TimescaleChangeKind::DisableColumnstore {
            table: "Metric".into(),
            old: ColumnstoreState {
                settings: settings(&["deviceId"], &[("time", true)]),
                after: Some(days(7)),
            },
        });
        let p = pre(&d);
        assert_eq!(p.up, vec![RM_CS, BLOCK, OFF]);
        assert_eq!(
            p.down,
            vec![
                CS_ON,
                "CALL add_columnstore_policy('\"Metric\"', after => INTERVAL '7 days');"
            ]
        );
        assert!(post(&d).up.is_empty());
    }

    #[test]
    fn test_replace_columnstore_policy_sql() {
        let d = one(TimescaleChangeKind::ReplaceColumnstorePolicy {
            table: "Metric".into(),
            old: Some(days(3)),
            new: Some(days(7)),
        });
        let (pr, po) = (pre(&d), post(&d));
        assert_eq!(pr.up, vec![RM_CS]);
        assert_eq!(
            pr.down,
            vec!["CALL add_columnstore_policy('\"Metric\"', after => INTERVAL '3 days');"]
        );
        assert_eq!(
            po.up,
            vec!["CALL add_columnstore_policy('\"Metric\"', after => INTERVAL '7 days');"]
        );
        assert_eq!(po.down, vec![RM_CS]);
    }

    #[test]
    fn test_replace_retention_policy_sql() {
        let d = one(TimescaleChangeKind::ReplaceRetentionPolicy {
            table: "Metric".into(),
            old: Some(days(30)),
            new: Some(days(90)),
        });
        let (pr, po) = (pre(&d), post(&d));
        let rm = "SELECT remove_retention_policy('\"Metric\"', if_exists => true);";
        assert_eq!(pr.up, vec![rm]);
        assert_eq!(
            pr.down,
            vec!["SELECT add_retention_policy('\"Metric\"', drop_after => INTERVAL '30 days');"]
        );
        assert_eq!(
            po.up,
            vec!["SELECT add_retention_policy('\"Metric\"', drop_after => INTERVAL '90 days');"]
        );
        assert_eq!(po.down, vec![rm]);
    }

    #[test]
    fn test_aggregate_changes_route_to_phases() {
        let set = |drops: bool| TimescaleChangeKind::AggregateSet {
            drop_aggregates: Vec::new(),
            drop_views: if drops {
                vec![("HourlyMetric".to_string(), false)]
            } else {
                Vec::new()
            },
            create: Vec::new(),
            handoff_views: Vec::new(),
            coupled_columns: Vec::new(),
        };
        assert!(owns_first_statement(&set(true), Phase::Pre));
        assert!(!owns_first_statement(&set(true), Phase::Post));
        assert!(owns_first_statement(&set(false), Phase::Post));
        assert!(!owns_first_statement(&set(false), Phase::Pre));

        let policy = |old: bool| TimescaleChangeKind::ReplaceAggregatePolicy {
            view: "HourlyMetric".into(),
            old: old.then(|| crate::timescale::catalog::AggregatePolicy {
                start_offset: None,
                end_offset: None,
                schedule_interval: days(1),
            }),
            new: None,
        };
        assert!(owns_first_statement(&policy(true), Phase::Pre));
        assert!(owns_first_statement(&policy(false), Phase::Post));

        let mo = TimescaleChangeKind::SetMaterializedOnly {
            view: "HourlyMetric".into(),
            old: true,
            new: false,
        };
        assert!(owns_first_statement(&mo, Phase::Post));
        assert!(!owns_first_statement(&mo, Phase::Pre));
    }

    #[test]
    fn test_note_emitted_once_from_owning_phase() {
        let mut d = one(TimescaleChangeKind::ReplaceRetentionPolicy {
            table: "Metric".into(),
            old: Some(days(30)),
            new: Some(days(90)),
        });
        d.changes[0].note = Some("note".into());
        assert_eq!(pre(&d).warnings, vec!["note"]);
        assert!(post(&d).warnings.is_empty());
    }

    fn agg(view: &str, source: &str, sql: &str, policy: bool) -> AggregateState {
        AggregateState {
            view: view.into(),
            source: source.into(),
            sql: Some(sql.into()),
            materialized_only: true,
            policy: policy.then(|| AggregatePolicy {
                start_offset: Some(days(3)),
                end_offset: Some(PgInterval::new(0, 0, 3_600_000_000)),
                schedule_interval: PgInterval::new(0, 0, 3_600_000_000),
            }),
        }
    }

    fn set(
        gated: bool,
        drop_aggregates: Vec<AggregateState>,
        drop_views: Vec<(String, bool)>,
        create: Vec<AggregateState>,
    ) -> TimescaleChange {
        TimescaleChange {
            kind: TimescaleChangeKind::AggregateSet {
                drop_aggregates,
                drop_views,
                create,
                handoff_views: Vec::new(),
                coupled_columns: Vec::new(),
            },
            gated,
            note: None,
        }
    }

    fn diff_of(changes: Vec<TimescaleChange>) -> TimescaleDiff {
        TimescaleDiff {
            changes,
            warnings: Vec::new(),
        }
    }

    const HOURLY_SQL: &str =
        "SELECT time_bucket('1 hour', time) AS bucket FROM \"Metric\" GROUP BY 1";

    #[test]
    fn test_create_aggregate_sql() {
        let d = diff_of(vec![set(
            false,
            vec![],
            vec![],
            vec![agg(
                "HourlyMetric",
                "Metric",
                &format!("{HOURLY_SQL};\n"),
                true,
            )],
        )]);
        let p = post(&d);
        assert_eq!(p.up.len(), 4, "{:?}", p.up);
        assert_eq!(
            p.up[0],
            format!(
                "CREATE MATERIALIZED VIEW \"HourlyMetric\"\n  WITH (timescaledb.continuous, timescaledb.materialized_only = true) AS\n{HOURLY_SQL}\nWITH NO DATA;"
            )
        );
        assert!(p.up[1].starts_with("COMMENT ON VIEW \"HourlyMetric\" IS 'prax:cagg:v1\n"));
        assert_eq!(
            p.up[2],
            "SELECT add_continuous_aggregate_policy('\"HourlyMetric\"', start_offset => INTERVAL '3 days', end_offset => INTERVAL '1 hour', schedule_interval => INTERVAL '1 hour');"
        );
        assert_eq!(
            p.up[3],
            "-- \"HourlyMetric\" is created empty; its refresh policy only covers the last 3 days.\n-- To backfill older data, run outside a transaction:\n-- CALL refresh_continuous_aggregate('\"HourlyMetric\"', NULL, now() - INTERVAL '3 days');"
        );
        assert_eq!(
            p.down,
            vec!["DROP MATERIALIZED VIEW IF EXISTS \"HourlyMetric\";"]
        );
        assert_eq!(
            p.warnings,
            vec![
                "Continuous aggregate \"HourlyMetric\" is created WITH NO DATA; see the backfill note in the migration"
            ]
        );
        assert!(pre(&d).up.is_empty());
    }

    #[test]
    fn test_create_aggregate_null_offsets_and_no_start_note() {
        let mut a = agg("HourlyMetric", "Metric", HOURLY_SQL, true);
        if let Some(p) = a.policy.as_mut() {
            p.start_offset = None;
            p.end_offset = None;
        }
        let p = post(&diff_of(vec![set(false, vec![], vec![], vec![a])]));
        assert_eq!(
            p.up[2],
            "SELECT add_continuous_aggregate_policy('\"HourlyMetric\"', start_offset => NULL, end_offset => NULL, schedule_interval => INTERVAL '1 hour');"
        );
        assert_eq!(
            p.up[3],
            "-- \"HourlyMetric\" is created empty; the first policy run materializes all data."
        );
    }

    #[test]
    fn test_drop_aggregate_and_view_sql() {
        let d = diff_of(vec![set(
            true,
            vec![agg("HourlyMetric", "Metric", HOURLY_SQL, true)],
            vec![("report".into(), false), ("rollup".into(), true)],
            vec![],
        )]);
        let p = pre(&d);
        assert_eq!(
            p.up,
            vec![
                "DROP MATERIALIZED VIEW IF EXISTS \"HourlyMetric\";",
                "DROP VIEW IF EXISTS \"report\";",
                "DROP MATERIALIZED VIEW IF EXISTS \"rollup\";",
            ]
        );
        assert_eq!(
            p.down,
            vec!["-- irreversible: materialized data of \"HourlyMetric\" was discarded"]
        );
        assert!(post(&d).up.is_empty());
    }

    #[test]
    fn test_aggregate_policy_and_materialized_only_sql() {
        let policy = |p: bool| {
            p.then(|| AggregatePolicy {
                start_offset: Some(days(3)),
                end_offset: None,
                schedule_interval: days(1),
            })
        };
        let d = diff_of(vec![
            TimescaleChange {
                kind: TimescaleChangeKind::ReplaceAggregatePolicy {
                    view: "HourlyMetric".into(),
                    old: policy(true),
                    new: policy(true),
                },
                gated: false,
                note: None,
            },
            TimescaleChange {
                kind: TimescaleChangeKind::SetMaterializedOnly {
                    view: "HourlyMetric".into(),
                    old: true,
                    new: false,
                },
                gated: false,
                note: None,
            },
        ]);
        let rm =
            "SELECT remove_continuous_aggregate_policy('\"HourlyMetric\"', if_exists => true);";
        let add = "SELECT add_continuous_aggregate_policy('\"HourlyMetric\"', start_offset => INTERVAL '3 days', end_offset => NULL, schedule_interval => INTERVAL '1 day');";
        let (pr, po) = (pre(&d), post(&d));
        assert_eq!(pr.up, vec![rm]);
        assert_eq!(pr.down, vec![add]);
        assert_eq!(
            po.up,
            vec![
                add,
                "ALTER MATERIALIZED VIEW \"HourlyMetric\" SET (timescaledb.materialized_only = false);"
            ]
        );
        assert_eq!(
            po.down,
            vec![
                "ALTER MATERIALIZED VIEW \"HourlyMetric\" SET (timescaledb.materialized_only = true);",
                rm
            ]
        );
    }

    #[test]
    fn test_aggregate_record_round_trips_hostile_sql() {
        let sql = "SELECT 'it''s' AS \"q\",\n $$x$$ AS d FROM \"Metric\"";
        let p = post(&diff_of(vec![set(
            false,
            vec![],
            vec![],
            vec![agg("HourlyMetric", "Metric", sql, false)],
        )]));
        let comment = &p.up[1];
        let start = comment.find("IS '").unwrap() + 4;
        let literal = comment[start..].strip_suffix("';").unwrap();
        let record = literal.replace("''", "'");
        assert_eq!(
            crate::timescale::catalog::parse_aggregate_record(&record),
            Some(sql)
        );
    }

    #[test]
    fn test_creates_are_topologically_ordered_across_sets() {
        let d = diff_of(vec![
            set(
                false,
                vec![],
                vec![],
                vec![agg("DailyMetric", "HourlyMetric", "SELECT 2", false)],
            ),
            set(
                true,
                vec![],
                vec![],
                vec![agg("HourlyMetric", "Metric", "SELECT 1", false)],
            ),
        ]);
        let p = post(&d);
        let pos = |n: &str| {
            p.up.iter()
                .position(|s| s.starts_with(&format!("CREATE MATERIALIZED VIEW \"{n}\"")))
                .unwrap()
        };
        assert!(pos("HourlyMetric") < pos("DailyMetric"));
        // Only the ungated create is dropped on rollback.
        assert_eq!(
            p.down,
            vec!["DROP MATERIALIZED VIEW IF EXISTS \"DailyMetric\";"]
        );
    }

    #[test]
    fn test_pre_sections_order_policy_removals_aggregate_drops_columnstore() {
        let d = diff_of(vec![
            TimescaleChange {
                kind: TimescaleChangeKind::DisableColumnstore {
                    table: "Metric".into(),
                    old: ColumnstoreState {
                        settings: settings(&["deviceId"], &[("time", true)]),
                        after: None,
                    },
                },
                gated: true,
                note: None,
            },
            set(
                true,
                vec![agg("HourlyMetric", "Metric", HOURLY_SQL, false)],
                vec![],
                vec![],
            ),
            TimescaleChange {
                kind: TimescaleChangeKind::ReplaceRetentionPolicy {
                    table: "Metric".into(),
                    old: Some(days(30)),
                    new: None,
                },
                gated: false,
                note: None,
            },
        ]);
        let up = pre(&d).up;
        let pos = |needle: &str| up.iter().position(|s| s.starts_with(needle)).unwrap();
        let policy = pos("SELECT remove_retention_policy");
        let drop = pos("DROP MATERIALIZED VIEW");
        let block = pos("DO $prax$");
        assert!(policy < drop && drop < block, "{up:?}");
    }
}
