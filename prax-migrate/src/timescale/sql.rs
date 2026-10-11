//! Render a [`TimescaleDiff`] as SQL in two phases around the generic
//! Postgres pipeline.
//!
//! The pre-phase runs before the generic pipeline: it removes policies, drops
//! continuous aggregates (and the views they replace) and converts chunks
//! back to rowstore, so the pipeline can alter or drop the tables they depend
//! on. The post-phase runs after it, once the tables exist: it creates
//! hypertables, configures the columnstore, adds policies and creates
//! aggregates. Names interpolated into `--` comment lines go through
//! [`comment_safe`], so no name can end the comment.

use prax_schema::ast::PgInterval;

use crate::timescale::aggregates::topo_order;
use crate::timescale::catalog::{
    AggregatePolicy, AggregateState, ColumnstoreSettings, aggregate_record,
};
use crate::timescale::diff::{TimescaleChange, TimescaleChangeKind, TimescaleDiff};
use crate::timescale::quote::{
    comment_safe, ident_list_literal, quote_ident, quote_literal, regclass_literal,
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

/// Where a group of statements runs within its phase, in execution order.
///
/// The pre-phase uses `PolicyRemovals`, `Aggregates`, `ColumnstoreTeardown`;
/// the post-phase uses `Hypertables`, `Aggregates`, `AggregatePolicies`,
/// `MaterializedOnly`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    /// Pre: remove table and aggregate policies before what they act on
    /// changes.
    PolicyRemovals,
    /// Post: create hypertables, configure the columnstore and add table
    /// policies.
    Hypertables,
    /// Drop (pre) or create (post) continuous aggregates, ordered across all
    /// sets.
    Aggregates,
    /// Pre: convert chunks back to rowstore once no aggregate reads the
    /// table.
    ColumnstoreTeardown,
    /// Post: add aggregate refresh policies once the aggregates exist.
    AggregatePolicies,
    /// Post: change `materialized_only` last.
    MaterializedOnly,
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
    let mut groups: Vec<(Section, Vec<String>, Vec<String>)> = Vec::new();
    for change in &diff.changes {
        let sql = change_sql(change, phase);
        if sql.owns_note
            && let Some(note) = &change.note
        {
            out.warnings.push(note.clone());
        }
        out.warnings.extend(sql.warnings);
        groups.push((sql.section, sql.up, sql.down));
    }
    match phase {
        Phase::Pre => groups.push((
            Section::Aggregates,
            aggregate_drops_up(diff),
            aggregate_drops_down(diff),
        )),
        Phase::Post => {
            let (up, down, warnings) = aggregate_creates(diff);
            out.warnings.extend(warnings);
            groups.push((Section::Aggregates, up, down));
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

/// One change's SQL within one phase.
struct ChangeSql {
    section: Section,
    up: Vec<String>,
    down: Vec<String>,
    warnings: Vec<String>,
    /// Whether this phase emits the change's note. The note goes with the
    /// change's first statement: the pre-phase owns it when it emits anything
    /// for the change, the post-phase otherwise.
    owns_note: bool,
}

/// A change's statements for one phase.
#[derive(Default)]
struct Part {
    up: Vec<String>,
    down: Vec<String>,
    warnings: Vec<String>,
}

impl Part {
    fn new(up: Vec<String>, down: Vec<String>) -> Self {
        Self {
            up,
            down,
            warnings: Vec::new(),
        }
    }
}

fn change_sql(change: &TimescaleChange, phase: Phase) -> ChangeSql {
    let (pre, post) = phase_parts(&change.kind);
    // Aggregate sets render their statements across all sets
    // (`aggregate_drops_up`, `aggregate_creates`), so whether a phase emits
    // anything for one comes from the set itself.
    let (pre_emits, post_emits) = match &change.kind {
        TimescaleChangeKind::AggregateSet(set) => (
            !set.drop_aggregates.is_empty() || !set.drop_views.is_empty(),
            !set.create.is_empty(),
        ),
        _ => (!pre.up.is_empty(), !post.up.is_empty()),
    };
    let (part, owns_note) = match phase {
        Phase::Pre => (pre, pre_emits),
        Phase::Post => (post, post_emits && !pre_emits),
    };
    ChangeSql {
        section: section(&change.kind, phase),
        up: part.up,
        down: part.down,
        warnings: part.warnings,
        owns_note,
    }
}

/// The section a change's statements belong to within `phase`.
fn section(kind: &TimescaleChangeKind, phase: Phase) -> Section {
    match (kind, phase) {
        (TimescaleChangeKind::AggregateSet(_), _) => Section::Aggregates,
        (TimescaleChangeKind::DisableColumnstore { .. }, Phase::Pre) => {
            Section::ColumnstoreTeardown
        }
        (TimescaleChangeKind::ReplaceAggregatePolicy { .. }, Phase::Post) => {
            Section::AggregatePolicies
        }
        (TimescaleChangeKind::SetMaterializedOnly { .. }, Phase::Post) => Section::MaterializedOnly,
        (_, Phase::Pre) => Section::PolicyRemovals,
        (_, Phase::Post) => Section::Hypertables,
    }
}

/// A change's `(pre, post)` statements.
fn phase_parts(kind: &TimescaleChangeKind) -> (Part, Part) {
    match kind {
        TimescaleChangeKind::CreateHypertable {
            table,
            time_column,
            chunk_interval,
            space,
            migrate_data,
        } => (
            Part::default(),
            create_hypertable(
                table,
                time_column,
                chunk_interval,
                space.as_ref(),
                *migrate_data,
            ),
        ),
        TimescaleChangeKind::SetChunkInterval { table, old, new } => (
            Part::default(),
            Part::new(
                vec![set_chunk_interval(table, &new.to_sql())],
                vec![set_chunk_interval(table, &old.to_sql())],
            ),
        ),
        TimescaleChangeKind::SetPartitions { table, old, new } => (
            Part::default(),
            Part::new(
                vec![set_partitions(table, *new)],
                vec![set_partitions(table, *old)],
            ),
        ),
        TimescaleChangeKind::ConfigureColumnstore { table, old, new } => {
            let down = match old {
                None => disable_columnstore(table),
                Some(old) => vec![configure_columnstore(table, old)],
            };
            (
                Part::default(),
                Part::new(vec![configure_columnstore(table, new)], down),
            )
        }
        TimescaleChangeKind::DisableColumnstore { table, old } => {
            let mut down = vec![configure_columnstore(table, &old.settings)];
            if let Some(after) = &old.after {
                down.push(add_columnstore_policy(table, &after.to_sql()));
            }
            (Part::new(disable_columnstore(table), down), Part::default())
        }
        TimescaleChangeKind::ReplaceColumnstorePolicy { table, old, new } => replace_policy(
            old.as_ref(),
            new.as_ref(),
            || remove_columnstore_policy(table),
            |after| add_columnstore_policy(table, &after.to_sql()),
        ),
        TimescaleChangeKind::ReplaceRetentionPolicy { table, old, new } => replace_policy(
            old.as_ref(),
            new.as_ref(),
            || remove_retention_policy(table),
            |drop_after| add_retention_policy(table, &drop_after.to_sql()),
        ),
        // Rendered across all sets by `render` (drops and creates are ordered
        // globally).
        TimescaleChangeKind::AggregateSet(_) => (Part::default(), Part::default()),
        TimescaleChangeKind::ReplaceAggregatePolicy { view, old, new } => replace_policy(
            old.as_ref(),
            new.as_ref(),
            || remove_aggregate_policy(view),
            |policy| add_aggregate_policy(view, policy),
        ),
        TimescaleChangeKind::SetMaterializedOnly { view, old, new } => (
            Part::default(),
            Part::new(
                vec![set_materialized_only(view, *new)],
                vec![set_materialized_only(view, *old)],
            ),
        ),
    }
}

/// A policy replacement: the pre-phase removes the old policy (its down adds
/// it back), the post-phase adds the new one (its down removes it). Either
/// side is empty when there is no policy on it.
fn replace_policy<T>(
    old: Option<&T>,
    new: Option<&T>,
    remove: impl Fn() -> String,
    add: impl Fn(&T) -> String,
) -> (Part, Part) {
    let pre = old.map_or_else(Part::default, |old| {
        Part::new(vec![remove()], vec![add(old)])
    });
    let post = new.map_or_else(Part::default, |new| {
        Part::new(vec![add(new)], vec![remove()])
    });
    (pre, post)
}

fn create_hypertable(
    table: &str,
    time_column: &str,
    chunk_interval: &PgInterval,
    space: Option<&(String, i16)>,
    migrate_data: bool,
) -> Part {
    let migrate = if migrate_data {
        ", migrate_data => true"
    } else {
        ""
    };
    let mut part = Part::default();
    part.up.push(format!(
        "SELECT create_hypertable({}, by_range({}, {}), create_default_indexes => false{});",
        regclass_literal(table),
        quote_literal(time_column),
        chunk_interval.to_sql(),
        migrate
    ));
    if let Some((column, partitions)) = space {
        part.up.push(format!(
            "SELECT add_dimension({}, by_hash({}, {}));",
            regclass_literal(table),
            quote_literal(column),
            partitions
        ));
    }
    if migrate_data {
        part.down.push(format!(
            "-- irreversible: {} was converted to a hypertable",
            comment_safe(&quote_ident(table))
        ));
        part.warnings.push(format!(
            "Table {} was converted to a hypertable; the down migration cannot convert it back",
            quote_ident(table)
        ));
    }
    part
}

/// The drop statement for each view replaced by an aggregate, across all
/// sets: `(view, materialized)`.
fn dropped_views(diff: &TimescaleDiff) -> impl Iterator<Item = &(String, bool)> {
    diff.changes
        .iter()
        .filter_map(|c| match &c.kind {
            TimescaleChangeKind::AggregateSet(set) => Some(&set.drop_views),
            _ => None,
        })
        .flatten()
}

/// Every set's replaced views, then its dropped aggregates (children first
/// across sets). The views go first: a plain view may read an aggregate being
/// dropped, and Postgres refuses to drop an aggregate a view depends on; no
/// aggregate can read a plain view.
fn aggregate_drops_up(diff: &TimescaleDiff) -> Vec<String> {
    let views = dropped_views(diff).map(|(view, materialized)| {
        if *materialized {
            drop_materialized_view(view)
        } else {
            format!("DROP VIEW IF EXISTS {};", quote_ident(view))
        }
    });
    let aggregates = dropped_aggregates(diff)
        .into_iter()
        .map(|a| drop_materialized_view(&a.view));
    views.chain(aggregates).collect()
}

/// Dropped aggregates in reverse topological order across all sets.
fn dropped_aggregates(diff: &TimescaleDiff) -> Vec<AggregateState> {
    let all: Vec<AggregateState> = diff
        .changes
        .iter()
        .filter_map(|c| match &c.kind {
            TimescaleChangeKind::AggregateSet(set) => Some(set.drop_aggregates.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect();
    let mut ordered = topo_order(&all);
    ordered.reverse();
    ordered
}

/// Neither a dropped aggregate's materialized data nor a dropped view can be
/// restored: one marker per drop, in the reverse of `aggregate_drops_up`.
fn aggregate_drops_down(diff: &TimescaleDiff) -> Vec<String> {
    let views: Vec<String> = dropped_views(diff)
        .map(|(view, materialized)| {
            let view = comment_safe(&quote_ident(view));
            if *materialized {
                format!(
                    "-- irreversible: materialized view {view} was dropped and its rows were \
                     discarded"
                )
            } else {
                format!("-- irreversible: view {view} was dropped")
            }
        })
        .collect();
    dropped_aggregates(diff)
        .iter()
        .rev()
        .map(|a| {
            format!(
                "-- irreversible: materialized data of {} was discarded",
                comment_safe(&quote_ident(&a.view))
            )
        })
        .chain(views.into_iter().rev())
        .collect()
}

/// Create statements for every set's aggregates in topological order, with
/// each create followed by its record comment, policy and backfill note.
fn aggregate_creates(diff: &TimescaleDiff) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut all = Vec::new();
    let mut gated_views: Vec<&str> = Vec::new();
    for change in &diff.changes {
        if let TimescaleChangeKind::AggregateSet(set) = &change.kind {
            all.extend(set.create.iter().cloned());
            if change.gated {
                gated_views.extend(set.create.iter().map(|a| a.view.as_str()));
            }
        }
    }
    let ordered = topo_order(&all);
    let (mut up, mut down, mut warnings) = (Vec::new(), Vec::new(), Vec::new());
    for agg in &ordered {
        let Some(sql) = &agg.sql else {
            // Unreachable in practice (the diff rejects a target aggregate
            // without SQL), but the generator cannot return an error and the
            // old aggregate may already be dropped: abort the migration
            // visibly rather than silently skipping the create.
            warnings.push(format!(
                "Continuous aggregate {} has no authored SQL and cannot be created",
                quote_ident(&agg.view)
            ));
            let message = format!(
                "Prax: continuous aggregate {} has no SQL definition; cannot recreate it",
                quote_ident(&agg.view)
            );
            up.push(format!(
                "DO $prax$ BEGIN RAISE EXCEPTION {}; END $prax$;",
                quote_literal(&message)
            ));
            continue;
        };
        let view = quote_ident(&agg.view);
        let body = sql.trim_end();
        let body = body.strip_suffix(';').unwrap_or(body).trim_end();
        up.push(format!(
            "CREATE MATERIALIZED VIEW {view}\n  WITH (timescaledb.continuous, timescaledb.materialized_only = {}) AS\n{body}\nWITH NO DATA;",
            agg.materialized_only
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
                "-- {} is created empty and has no refresh policy; refresh it manually.",
                comment_safe(&view)
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
    let view = comment_safe(&quote_ident(name));
    let regclass = comment_safe(&regclass_literal(name));
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
    use crate::timescale::diff::AggregateSetChange;

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

    /// Whether `phase` emits the note of a change of `kind`.
    fn owns_note(kind: TimescaleChangeKind, phase: Phase) -> bool {
        let change = TimescaleChange {
            kind,
            gated: false,
            note: None,
        };
        change_sql(&change, phase).owns_note
    }

    #[test]
    fn test_aggregate_changes_route_to_phases() {
        let created = agg("HourlyMetric", "Metric", HOURLY_SQL, false);
        let set = |drops: bool| {
            TimescaleChangeKind::AggregateSet(AggregateSetChange {
                drop_views: if drops {
                    vec![("HourlyMetric".to_string(), false)]
                } else {
                    Vec::new()
                },
                create: vec![created.clone()],
                ..AggregateSetChange::default()
            })
        };
        assert!(owns_note(set(true), Phase::Pre));
        assert!(!owns_note(set(true), Phase::Post));
        assert!(owns_note(set(false), Phase::Post));
        assert!(!owns_note(set(false), Phase::Pre));

        let policy = || crate::timescale::catalog::AggregatePolicy {
            start_offset: None,
            end_offset: None,
            schedule_interval: days(1),
        };
        let replace = |old: bool| TimescaleChangeKind::ReplaceAggregatePolicy {
            view: "HourlyMetric".into(),
            old: old.then(policy),
            new: Some(policy()),
        };
        assert!(owns_note(replace(true), Phase::Pre));
        assert!(!owns_note(replace(true), Phase::Post));
        assert!(owns_note(replace(false), Phase::Post));
        assert!(!owns_note(replace(false), Phase::Pre));

        let mo = || TimescaleChangeKind::SetMaterializedOnly {
            view: "HourlyMetric".into(),
            old: true,
            new: false,
        };
        assert!(owns_note(mo(), Phase::Post));
        assert!(!owns_note(mo(), Phase::Pre));
    }

    #[test]
    fn test_sections_order_within_each_phase() {
        assert!(Section::PolicyRemovals < Section::Aggregates);
        assert!(Section::Aggregates < Section::ColumnstoreTeardown);
        assert!(Section::Hypertables < Section::Aggregates);
        assert!(Section::Aggregates < Section::AggregatePolicies);
        assert!(Section::AggregatePolicies < Section::MaterializedOnly);
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
            kind: TimescaleChangeKind::AggregateSet(AggregateSetChange {
                drop_aggregates,
                drop_views,
                create,
                ..AggregateSetChange::default()
            }),
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
    fn test_create_aggregate_uses_declared_materialized_only() {
        let mut real_time = agg("HourlyMetric", "Metric", HOURLY_SQL, true);
        real_time.materialized_only = false;
        let p = post(&diff_of(vec![set(false, vec![], vec![], vec![real_time])]));
        assert_eq!(
            p.up[0],
            format!(
                "CREATE MATERIALIZED VIEW \"HourlyMetric\"\n  WITH (timescaledb.continuous, timescaledb.materialized_only = false) AS\n{HOURLY_SQL}\nWITH NO DATA;"
            )
        );
    }

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
        // Replaced views first: a view may read an aggregate being dropped.
        assert_eq!(
            p.up,
            vec![
                "DROP VIEW IF EXISTS \"report\";",
                "DROP MATERIALIZED VIEW IF EXISTS \"rollup\";",
                "DROP MATERIALIZED VIEW IF EXISTS \"HourlyMetric\";",
            ]
        );
        assert_eq!(
            p.down,
            vec![
                "-- irreversible: materialized data of \"HourlyMetric\" was discarded",
                "-- irreversible: materialized view \"rollup\" was dropped and its rows were discarded",
                "-- irreversible: view \"report\" was dropped",
            ]
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

    /// No comment line may be ended early by a name: every line of every
    /// comment statement starts with `--`.
    fn assert_comments_single_line(statements: &[String]) {
        for statement in statements.iter().filter(|s| s.starts_with("--")) {
            for line in statement.lines() {
                assert!(line.starts_with("--"), "{statement:?}");
            }
            assert!(
                !statement.contains(['\r', '\u{2028}', '\u{2029}', '\u{0085}']),
                "{statement:?}"
            );
        }
    }

    #[test]
    fn test_comment_lines_neutralize_hostile_names() {
        let evil = "Evil\nDROP TABLE \"Metric\"; --\r\u{2028}x";
        // No refresh policy, then a policy with a start offset (backfill note).
        for policy in [false, true] {
            let p = post(&diff_of(vec![set(
                false,
                vec![],
                vec![],
                vec![agg(evil, "Metric", HOURLY_SQL, policy)],
            )]));
            assert_comments_single_line(&p.up);
            assert!(
                p.up.iter().any(|s| s.starts_with("-- \"Evil DROP TABLE")),
                "{:?}",
                p.up
            );
        }

        let p = pre(&diff_of(vec![set(
            true,
            vec![agg(evil, "Metric", HOURLY_SQL, false)],
            vec![(evil.to_string(), false), (format!("{evil}2"), true)],
            vec![],
        )]));
        assert_eq!(p.down.len(), 3, "{:?}", p.down);
        assert_comments_single_line(&p.down);

        let p = post(&one(TimescaleChangeKind::CreateHypertable {
            table: evil.into(),
            time_column: "time".into(),
            chunk_interval: days(1),
            space: None,
            migrate_data: true,
        }));
        assert_comments_single_line(&p.down);
    }

    #[test]
    fn test_backslash_names_use_escape_string_literals() {
        let p = post(&one(TimescaleChangeKind::SetPartitions {
            table: "a\\b".into(),
            old: 4,
            new: 8,
        }));
        assert_eq!(
            p.up,
            vec!["SELECT set_number_partitions(E'\"a\\\\b\"', 8);"]
        );
    }

    #[test]
    fn test_create_aggregate_without_sql_aborts() {
        let mut a = agg("HourlyMetric", "Metric", "SELECT 1", false);
        a.sql = None;
        let p = post(&diff_of(vec![set(false, vec![], vec![], vec![a])]));
        assert_eq!(
            p.up,
            vec![
                "DO $prax$ BEGIN RAISE EXCEPTION 'Prax: continuous aggregate \"HourlyMetric\" has no SQL definition; cannot recreate it'; END $prax$;"
            ]
        );
        assert_eq!(p.warnings.len(), 1);
    }
}
