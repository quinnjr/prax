//! Render a [`TimescaleDiff`] as SQL in two phases (spec §4.4–§4.6).
//!
//! The pre-phase runs before the generic Postgres pipeline (removing policies,
//! converting chunks back to rowstore); the post-phase runs after it (creating
//! hypertables, configuring the columnstore, adding policies).

use crate::timescale::catalog::ColumnstoreSettings;
use crate::timescale::diff::{TimescaleChange, TimescaleChangeKind, TimescaleDiff};
use crate::timescale::quote::{ident_list_literal, name_literal, quote_ident, regclass_literal};

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
    // Down statements per change, each group in its own execution order; the
    // groups are emitted in reverse change order.
    let mut down_groups: Vec<Vec<String>> = Vec::new();
    for change in &diff.changes {
        let (up, down, extra) = change_sql(change, phase);
        if !up.is_empty()
            && let Some(note) = &change.note
            && owns_first_statement(&change.kind, phase)
        {
            out.warnings.push(note.clone());
        }
        out.warnings.extend(extra);
        out.up.extend(up);
        down_groups.push(down);
    }
    out.down = down_groups.into_iter().rev().flatten().collect();
    out
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
    use crate::timescale::catalog::ColumnstoreState;

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
}
