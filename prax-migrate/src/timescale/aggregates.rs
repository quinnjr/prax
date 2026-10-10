//! Diff continuous aggregates between the target schema and the live catalog
//! (spec §4.8, §5).
//!
//! Aggregates are keyed by database view name. The catalog's `source` edges
//! give the drop order and the descendants dropped with a parent; the target's
//! give the creation order (spec §5.6). Changes are grouped into at most one
//! ungated and one gated [`TimescaleChangeKind::AggregateSet`], so stripping
//! the gated set never leaves half a recreation behind.

use std::collections::{BTreeMap, BTreeSet};

use prax_schema::Schema;

use crate::diff::SchemaDiff;
use crate::error::{MigrateResult, MigrationError};
use crate::timescale::catalog::{AggregateState, CAGG_RECORD_MARKER, TimescaleCatalog};
use crate::timescale::diff::{TimescaleChange, TimescaleChangeKind, TimescaleDiff};
use crate::timescale::resolve::resolve_aggregates;

/// Normalize SQL for comparison: collapse whitespace runs, trim, and drop one
/// trailing `;`. Only comparisons use it; emitted SQL stays verbatim.
pub(crate) fn normalize_sql(sql: &str) -> String {
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.strip_suffix(';') {
        Some(rest) => rest.trim_end().to_string(),
        None => collapsed,
    }
}

/// Order aggregates parents before children (by `source`); ties keep input
/// order.
pub(crate) fn topo_order(aggs: &[AggregateState]) -> Vec<AggregateState> {
    let mut waiting: Vec<&AggregateState> = aggs.iter().collect();
    let mut ordered = Vec::with_capacity(aggs.len());
    while !waiting.is_empty() {
        // The first aggregate whose parent is not still waiting. A cycle (the
        // validator rejects those) falls back to input order.
        let next = waiting
            .iter()
            .position(|a| {
                !waiting
                    .iter()
                    .any(|p| p.view == a.source && p.view != a.view)
            })
            .unwrap_or(0);
        ordered.push(waiting.remove(next).clone());
    }
    ordered
}

/// Append the continuous-aggregate changes to `out`: the ungated
/// `AggregateSet`, the gated one, then policy and `materialized_only` changes
/// in view-name order. Unmarked aggregates add warnings.
///
/// `generic` supplies the column alters and drops that force the aggregates
/// reading a table to be recreated (spec §4.8).
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] when a target aggregate has no
/// SQL, or declares a `source` that differs from the database while its SQL is
/// unchanged.
pub(crate) fn diff_aggregates(
    target: &Schema,
    catalog: Option<&TimescaleCatalog>,
    generic: &SchemaDiff,
    out: &mut TimescaleDiff,
) -> MigrateResult<()> {
    let want = resolve_aggregates(target);
    let no_aggregates = BTreeMap::new();
    let no_views = BTreeMap::new();
    let have = catalog.map_or(&no_aggregates, |c| &c.aggregates);
    let other_views = catalog.map_or(&no_views, |c| &c.other_views);

    // Catalog aggregates that must go (spec §5.5): changed SQL, no longer an
    // aggregate in the target (handed off when the target declares a view of
    // that name), or reading a table whose columns are altered or dropped.
    let mut changed = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut handoff_views = Vec::new();
    for (name, w) in &want {
        let Some(sql) = &w.sql else {
            return Err(MigrationError::InvalidMigration(format!(
                "Continuous aggregate \"{name}\" has no SQL. Add an @@sql attribute to its view \
                 or a top-level @@sql(\"{name}\", \"\"\"...\"\"\") definition."
            )));
        };
        let Some(h) = have.get(name) else { continue };
        // An aggregate without a definition record is never recreated for
        // its SQL: the database's rewritten SQL cannot be compared.
        let Some(recorded) = &h.sql else { continue };
        if normalize_sql(recorded) != normalize_sql(sql) {
            changed.insert(name.clone());
        } else if h.source != w.source {
            return Err(MigrationError::InvalidMigration(format!(
                "Continuous aggregate \"{name}\" declares source \"{}\", but in the database it \
                 reads \"{}\" and its SQL is unchanged. Set `source` in @@timescale.continuous to \
                 the relation its SQL reads, or change the SQL to recreate the aggregate.",
                w.source, h.source
            )));
        }
    }
    for name in have.keys().filter(|n| !want.contains_key(*n)) {
        if target.views.values().any(|v| v.view_name() == name) {
            handoff_views.push(name.clone());
        } else {
            removed.insert(name.clone());
        }
    }
    let mut coupled_roots = BTreeSet::new();
    let mut coupled_columns = BTreeSet::new();
    for alter in &generic.alter_models {
        let readers: Vec<&String> = have
            .values()
            .filter(|a| a.source == alter.table_name)
            .map(|a| &a.view)
            .collect();
        // `drop_fields` holds column names.
        let columns: Vec<&String> = alter
            .alter_fields
            .iter()
            .map(|f| &f.column_name)
            .chain(&alter.drop_fields)
            .collect();
        if readers.is_empty() || columns.is_empty() {
            continue;
        }
        coupled_roots.extend(readers.into_iter().cloned());
        coupled_columns.extend(
            columns
                .into_iter()
                .map(|c| (alter.table_name.clone(), c.clone())),
        );
    }

    // Dropping an aggregate first drops its catalog descendants (§5.6).
    let mut dropped: BTreeSet<String> = changed
        .iter()
        .chain(&removed)
        .chain(&handoff_views)
        .chain(&coupled_roots)
        .cloned()
        .collect();
    loop {
        let descendants: Vec<String> = have
            .values()
            .filter(|a| !dropped.contains(&a.view) && dropped.contains(&a.source))
            .map(|a| a.view.clone())
            .collect();
        if descendants.is_empty() {
            break;
        }
        dropped.extend(descendants);
    }

    // Target aggregates to create, and whether the gated set creates them:
    // dropped ones are recreated there, and replacing a materialized view
    // discards its rows (§4.1).
    let mut creates: BTreeMap<&str, bool> = BTreeMap::new();
    for name in want.keys() {
        if have.contains_key(name) {
            if dropped.contains(name) {
                creates.insert(name, true);
            }
        } else {
            creates.insert(name, other_views.get(name).copied().unwrap_or(false));
        }
    }
    // A create on top of a gated create joins the gated set, so stripping that
    // set never leaves an aggregate without its parent.
    loop {
        let joining: Vec<&str> = creates
            .iter()
            .filter(|&(name, gated)| {
                !gated
                    && want
                        .get(*name)
                        .is_some_and(|w| creates.get(w.source.as_str()) == Some(&true))
            })
            .map(|(name, _)| *name)
            .collect();
        if joining.is_empty() {
            break;
        }
        for name in joining {
            creates.insert(name, true);
        }
    }
    let set_creates = |gated: bool| {
        let picked: Vec<AggregateState> = want
            .values()
            .filter(|a| creates.get(a.view.as_str()) == Some(&gated))
            .cloned()
            .collect();
        topo_order(&picked)
    };
    let set_drop_views = |gated: bool| -> Vec<(String, bool)> {
        creates
            .iter()
            .filter(|&(name, g)| *g == gated && !have.contains_key(*name))
            .filter_map(|(name, _)| other_views.get(*name).map(|m| ((*name).to_string(), *m)))
            .collect()
    };

    let create = set_creates(false);
    let drop_views = set_drop_views(false);
    if !create.is_empty() || !drop_views.is_empty() {
        out.changes.push(TimescaleChange {
            kind: TimescaleChangeKind::AggregateSet {
                drop_aggregates: Vec::new(),
                drop_views,
                create,
                handoff_views: Vec::new(),
                coupled_columns: Vec::new(),
            },
            gated: false,
            note: None,
        });
    }

    let mut drop_aggregates = topo_order(
        &have
            .values()
            .filter(|a| dropped.contains(&a.view))
            .cloned()
            .collect::<Vec<_>>(),
    );
    drop_aggregates.reverse();
    let drop_views = set_drop_views(true);
    if !drop_aggregates.is_empty() || !drop_views.is_empty() {
        let set = GatedSet {
            drop_aggregates,
            drop_views,
            create: set_creates(true),
            handoff_views,
            coupled_columns: coupled_columns.into_iter().collect(),
        };
        let note = set.note(&changed, &removed);
        out.changes.push(TimescaleChange {
            gated: !set.drop_aggregates.is_empty() || set.drop_views.iter().any(|(_, m)| *m),
            note: Some(note),
            kind: TimescaleChangeKind::AggregateSet {
                drop_aggregates: set.drop_aggregates,
                drop_views: set.drop_views,
                create: set.create,
                handoff_views: set.handoff_views,
                coupled_columns: set.coupled_columns,
            },
        });
    }

    // Aggregates that stay in place.
    for (name, w) in &want {
        let Some(h) = have.get(name) else { continue };
        if dropped.contains(name) {
            continue;
        }
        if h.sql.is_none() {
            out.warnings.push(format!(
                "Continuous aggregate \"{name}\" was not created by Prax (its view comment has no \
                 {CAGG_RECORD_MARKER} definition record), so its SQL cannot be compared with the \
                 schema and it is never recreated automatically. To adopt it, remove it from the \
                 schema and migrate with --allow-destructive, which drops it and discards its \
                 materialized data, then restore it."
            ));
        }
        if h.policy != w.policy {
            out.changes.push(TimescaleChange {
                kind: TimescaleChangeKind::ReplaceAggregatePolicy {
                    view: name.clone(),
                    old: h.policy.clone(),
                    new: w.policy.clone(),
                },
                gated: false,
                note: None,
            });
        }
        if h.materialized_only != w.materialized_only {
            out.changes.push(TimescaleChange {
                kind: TimescaleChangeKind::SetMaterializedOnly {
                    view: name.clone(),
                    old: h.materialized_only,
                    new: w.materialized_only,
                },
                gated: false,
                note: None,
            });
        }
    }
    Ok(())
}

/// The contents of the gated `AggregateSet`.
struct GatedSet {
    drop_aggregates: Vec<AggregateState>,
    drop_views: Vec<(String, bool)>,
    create: Vec<AggregateState>,
    handoff_views: Vec<String>,
    coupled_columns: Vec<(String, String)>,
}

impl GatedSet {
    /// Why the set drops what it drops, what is lost, and what is created.
    fn note(&self, changed: &BTreeSet<String>, removed: &BTreeSet<String>) -> String {
        let mut sentences = Vec::new();
        if !changed.is_empty() {
            sentences.push(format!(
                "The SQL of {} changed.",
                counted("continuous aggregate", "continuous aggregates", changed)
            ));
        }
        if !self.coupled_columns.is_empty() {
            let columns: Vec<String> = self
                .coupled_columns
                .iter()
                .map(|(table, column)| format!("\"{table}\".\"{column}\""))
                .collect();
            let one = columns.len() == 1;
            sentences.push(format!(
                "{} {} {} altered or dropped, which Postgres refuses while a view depends on {}.",
                if one { "Column" } else { "Columns" },
                columns.join(", "),
                if one { "is" } else { "are" },
                if one { "it" } else { "them" },
            ));
        }
        if !removed.is_empty() {
            sentences.push(format!(
                "{} {} no longer in the schema.",
                capitalized(&counted(
                    "continuous aggregate",
                    "continuous aggregates",
                    removed
                )),
                if removed.len() == 1 { "is" } else { "are" },
            ));
        }
        if !self.handoff_views.is_empty() {
            sentences.push(format!(
                "{} {} now declared as {}.",
                capitalized(&counted(
                    "continuous aggregate",
                    "continuous aggregates",
                    &self.handoff_views
                )),
                if self.handoff_views.len() == 1 {
                    "is"
                } else {
                    "are"
                },
                if self.handoff_views.len() == 1 {
                    "a view"
                } else {
                    "views"
                },
            ));
        }
        if !self.drop_aggregates.is_empty() {
            let names: Vec<&str> = self
                .drop_aggregates
                .iter()
                .map(|a| a.view.as_str())
                .collect();
            sentences.push(format!(
                "Dropping {} discards {} materialized data, which cannot be recovered where \
                 retention has already dropped the raw chunks.",
                counted("continuous aggregate", "continuous aggregates", &names),
                if names.len() == 1 { "its" } else { "their" },
            ));
        }
        let matviews: Vec<&str> = self
            .drop_views
            .iter()
            .filter(|(_, materialized)| *materialized)
            .map(|(view, _)| view.as_str())
            .collect();
        if !matviews.is_empty() {
            sentences.push(format!(
                "Replacing {} with {} discards {} stored rows.",
                counted("materialized view", "materialized views", &matviews),
                if matviews.len() == 1 {
                    "a continuous aggregate"
                } else {
                    "continuous aggregates"
                },
                if matviews.len() == 1 { "its" } else { "their" },
            ));
        }
        if !self.create.is_empty() {
            let names: Vec<&str> = self.create.iter().map(|a| a.view.as_str()).collect();
            sentences.push(format!(
                "{} {} then created empty from the schema.",
                capitalized(&counted(
                    "continuous aggregate",
                    "continuous aggregates",
                    &names
                )),
                if names.len() == 1 { "is" } else { "are" },
            ));
        }
        if !self.handoff_views.is_empty() {
            sentences.push(format!(
                "{} {} then created from the schema.",
                capitalized(&counted("view", "views", &self.handoff_views)),
                if self.handoff_views.len() == 1 {
                    "is"
                } else {
                    "are"
                },
            ));
        }
        sentences.join(" ")
    }
}

/// `<one> "a"` or `<many> "a", "b"`.
fn counted<S: AsRef<str>>(one: &str, many: &str, names: impl IntoIterator<Item = S>) -> String {
    let quoted: Vec<String> = names
        .into_iter()
        .map(|n| format!("\"{}\"", n.as_ref()))
        .collect();
    let noun = if quoted.len() == 1 { one } else { many };
    format!("{noun} {}", quoted.join(", "))
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use prax_schema::Schema;
    use prax_schema::ast::PgInterval;

    use super::*;
    use crate::diff::SchemaDiffer;
    use crate::error::MigrationError;
    use crate::timescale::catalog::{AggregatePolicy, HypertableState};
    use crate::timescale::diff::{TimescaleChange, TimescaleChangeKind};
    use crate::timescale::resolve_aggregates;

    const DATASOURCE: &str = "datasource db {\n  provider   = \"postgresql\"\n  url        = \"postgres://x\"\n  extensions = [timescaledb]\n}\n";

    /// The §3.1 `HourlyMetric` definition.
    const HOURLY_SQL: &str = "SELECT time_bucket('1 hour', \"time\") AS bucket, \"deviceId\", avg(\"value\") AS \"avgValue\"\nFROM \"Metric\"\nGROUP BY 1, 2";
    const DAILY_SQL: &str = "SELECT time_bucket('1 day', bucket) AS bucket, \"deviceId\", avg(\"avgValue\") AS \"avgValue\"\nFROM \"HourlyMetric\"\nGROUP BY 1, 2";
    const HOURLY: &str = "@@timescale.continuous(source: Metric, scheduleInterval: \"1 hour\", startOffset: \"3 days\", endOffset: \"1 hour\")";
    const DAILY: &str = "@@timescale.continuous(source: HourlyMetric, scheduleInterval: \"1 day\", startOffset: \"30 days\", endOffset: \"1 day\")";

    const fn days(n: i32) -> PgInterval {
        PgInterval::new(0, n, 0)
    }

    const fn hours(n: i64) -> PgInterval {
        PgInterval::new(0, 0, n * 3_600_000_000)
    }

    fn schema(prax: &str) -> Schema {
        prax_schema::parse_schema(prax).unwrap()
    }

    /// A view block carrying `attr`, with its SQL in a top-level `@@sql`.
    fn view(name: &str, attr: &str, sql: &str) -> String {
        format!(
            "view {name} {{\n  bucket   DateTime\n  deviceId String\n  avgValue Float\n\n  {attr}\n}}\n\n@@sql(\"{name}\", \"\"\"\n{sql}\n\"\"\")\n"
        )
    }

    /// The target: hypertable `Metric` (with `value` of `value_type`) and
    /// `views`.
    fn target_with(value_type: &str, views: &[String]) -> String {
        format!(
            "{DATASOURCE}\nmodel Metric {{\n  time     DateTime\n  deviceId String\n  value    {value_type}\n\n  @@id([deviceId, time])\n  @@timescale.hypertable(time, chunkInterval: \"1 day\")\n}}\n\n{}",
            views.join("\n")
        )
    }

    fn target(views: &[String]) -> String {
        target_with("Float", views)
    }

    /// `Metric` as introspected: a plain table with the given extra fields.
    fn source_with(fields: &str) -> String {
        format!(
            "{DATASOURCE}\nmodel Metric {{\n  time     DateTime\n  deviceId String\n  {fields}\n\n  @@id([deviceId, time])\n}}\n"
        )
    }

    fn metric_state() -> HypertableState {
        HypertableState {
            table: "Metric".into(),
            time_column: "time".into(),
            chunk_interval: days(1),
            space: None,
            columnstore: None,
            retention: None,
        }
    }

    /// The catalog: hypertable `Metric` (matching the target), `aggs`, and
    /// non-aggregate views `(name, materialized)`.
    fn catalog(aggs: Vec<AggregateState>, other_views: &[(&str, bool)]) -> TimescaleCatalog {
        TimescaleCatalog {
            version: "2.18.0".into(),
            hypertables: BTreeMap::from([("Metric".to_string(), metric_state())]),
            aggregates: aggs.into_iter().map(|a| (a.view.clone(), a)).collect(),
            aggregate_definitions: BTreeMap::new(),
            aggregate_columns: BTreeMap::new(),
            other_views: other_views
                .iter()
                .map(|(n, m)| ((*n).to_string(), *m))
                .collect(),
            unsupported: BTreeMap::new(),
        }
    }

    /// §3.1 `HourlyMetric` as recorded in the catalog.
    fn hourly() -> AggregateState {
        AggregateState {
            view: "HourlyMetric".into(),
            source: "Metric".into(),
            sql: Some(HOURLY_SQL.into()),
            materialized_only: true,
            policy: Some(AggregatePolicy {
                start_offset: Some(days(3)),
                end_offset: Some(hours(1)),
                schedule_interval: hours(1),
            }),
        }
    }

    /// `DailyMetric`, an aggregate on `HourlyMetric`, as recorded in the
    /// catalog.
    fn daily_on_hourly() -> AggregateState {
        AggregateState {
            view: "DailyMetric".into(),
            source: "HourlyMetric".into(),
            sql: Some(DAILY_SQL.into()),
            materialized_only: true,
            policy: Some(AggregatePolicy {
                start_offset: Some(days(30)),
                end_offset: Some(days(1)),
                schedule_interval: days(1),
            }),
        }
    }

    fn run_from(
        target: &str,
        source: &str,
        catalog: Option<TimescaleCatalog>,
    ) -> MigrateResult<SchemaDiff> {
        let mut differ = SchemaDiffer::new(schema(target)).with_source(schema(source));
        if let Some(catalog) = catalog {
            differ = differ.with_timescale_catalog(catalog);
        }
        differ.diff()
    }

    fn run(target: &str, catalog: Option<TimescaleCatalog>) -> MigrateResult<SchemaDiff> {
        run_from(target, &source_with("value    Float"), catalog)
    }

    /// An `AggregateSet` change, flattened for assertions.
    #[derive(Debug)]
    struct Set<'a> {
        gated: bool,
        note: &'a str,
        drop_aggregates: Vec<&'a str>,
        drop_views: Vec<(&'a str, bool)>,
        create: Vec<&'a AggregateState>,
        handoff_views: Vec<&'a str>,
        coupled_columns: Vec<(&'a str, &'a str)>,
    }

    impl Set<'_> {
        fn create_views(&self) -> Vec<&str> {
            self.create.iter().map(|a| a.view.as_str()).collect()
        }
    }

    fn sets(diff: &TimescaleDiff) -> Vec<Set<'_>> {
        diff.changes
            .iter()
            .filter_map(|c| match &c.kind {
                TimescaleChangeKind::AggregateSet {
                    drop_aggregates,
                    drop_views,
                    create,
                    handoff_views,
                    coupled_columns,
                } => Some(Set {
                    gated: c.gated,
                    note: c.note.as_deref().unwrap_or(""),
                    drop_aggregates: drop_aggregates.iter().map(|a| a.view.as_str()).collect(),
                    drop_views: drop_views.iter().map(|(v, m)| (v.as_str(), *m)).collect(),
                    create: create.iter().collect(),
                    handoff_views: handoff_views.iter().map(String::as_str).collect(),
                    coupled_columns: coupled_columns
                        .iter()
                        .map(|(t, c)| (t.as_str(), c.as_str()))
                        .collect(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The only change, which must be an `AggregateSet`.
    fn only_set(diff: &TimescaleDiff) -> Set<'_> {
        assert_eq!(diff.changes.len(), 1, "changes: {:#?}", diff.changes);
        sets(diff).pop().expect("an AggregateSet")
    }

    fn single(diff: &TimescaleDiff) -> &TimescaleChange {
        assert_eq!(diff.changes.len(), 1, "changes: {:#?}", diff.changes);
        &diff.changes[0]
    }

    #[test]
    fn test_normalize_sql() {
        assert_eq!(
            normalize_sql("  SELECT  1\n\tFROM t ;  \n"),
            "SELECT 1 FROM t"
        );
        assert_eq!(normalize_sql("SELECT 1;;"), "SELECT 1;");
        assert_eq!(normalize_sql("SELECT ';' AS x"), "SELECT ';' AS x");
        assert_eq!(
            normalize_sql(HOURLY_SQL),
            normalize_sql(&format!("\n{HOURLY_SQL};\n"))
        );
    }

    #[test]
    fn test_topo_order_parents_first_ties_in_input_order() {
        let agg = |view: &str, source: &str| AggregateState {
            view: view.into(),
            source: source.into(),
            sql: None,
            materialized_only: true,
            policy: None,
        };
        let ordered = topo_order(&[
            agg("C", "B"),
            agg("X", "Metric"),
            agg("B", "A"),
            agg("A", "Metric"),
        ]);
        let views: Vec<&str> = ordered.iter().map(|a| a.view.as_str()).collect();
        assert_eq!(views, ["X", "A", "B", "C"]);
    }

    #[test]
    fn test_resolve_aggregates_uses_database_names() {
        let prax = format!(
            "{DATASOURCE}\nmodel Metric {{\n  ts DateTime @map(\"time\")\n  @@id([ts])\n  @@map(\"metrics\")\n  @@timescale.hypertable(ts)\n}}\n\nview Hourly {{\n  bucket DateTime\n  @@map(\"hourly_metric\")\n  @@timescale.continuous(source: Metric, scheduleInterval: \"1 hour\")\n}}\n\n@@sql(\"hourly_metric\", \"\"\"\nSELECT 1\n\"\"\")\n\nview Daily {{\n  bucket DateTime\n  @@timescale.continuous(source: Hourly, scheduleInterval: \"1 day\", startOffset: \"3 days\", materializedOnly: false)\n}}\n\n@@sql(\"Daily\", \"\"\"\nSELECT 2\n\"\"\")\n\nview Plain {{\n  bucket DateTime\n}}\n\n@@sql(\"Plain\", \"\"\"\nSELECT 3\n\"\"\")\n"
        );
        let resolved = resolve_aggregates(&schema(&prax));
        assert_eq!(
            resolved.keys().map(String::as_str).collect::<Vec<_>>(),
            ["Daily", "hourly_metric"]
        );

        let hourly = &resolved["hourly_metric"];
        assert_eq!(hourly.view, "hourly_metric");
        assert_eq!(hourly.source, "metrics");
        assert_eq!(
            hourly.sql.as_deref().map(normalize_sql).as_deref(),
            Some("SELECT 1")
        );
        assert!(hourly.materialized_only);
        assert_eq!(
            hourly.policy,
            Some(AggregatePolicy {
                start_offset: None,
                end_offset: None,
                schedule_interval: hours(1),
            })
        );

        let daily = &resolved["Daily"];
        assert_eq!(daily.view, "Daily");
        assert_eq!(daily.source, "hourly_metric");
        assert!(!daily.materialized_only);
        assert_eq!(
            daily.policy,
            Some(AggregatePolicy {
                start_offset: Some(days(3)),
                end_offset: None,
                schedule_interval: days(1),
            })
        );
    }

    #[test]
    fn test_new_aggregate_is_ungated_create() {
        let target = target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]);
        for cat in [None, Some(catalog(vec![], &[]))] {
            let with_catalog = cat.is_some();
            let diff = run(&target, cat).unwrap();
            let all = sets(&diff.timescale);
            assert_eq!(all.len(), 1, "changes: {:#?}", diff.timescale.changes);
            if with_catalog {
                assert_eq!(diff.timescale.changes.len(), 1);
            }
            let set = &all[0];
            assert!(!set.gated);
            assert!(set.note.is_empty(), "{set:?}");
            assert!(set.drop_aggregates.is_empty(), "{set:?}");
            assert!(set.drop_views.is_empty(), "{set:?}");
            assert!(set.handoff_views.is_empty(), "{set:?}");
            assert!(set.coupled_columns.is_empty(), "{set:?}");
            assert_eq!(set.create_views(), ["HourlyMetric"]);

            let created = set.create[0];
            assert_eq!(created.source, "Metric");
            assert_eq!(created.policy, hourly().policy);
            assert!(created.materialized_only);
            assert_eq!(
                created.sql.as_deref().map(normalize_sql),
                Some(normalize_sql(HOURLY_SQL))
            );
            assert!(diff.timescale.gated_notes().is_empty());
            assert!(diff.create_views.is_empty(), "{:?}", diff.create_views);
        }
    }

    #[test]
    fn test_whitespace_only_sql_change_is_empty() {
        let mut have = hourly();
        have.sql = Some(format!(
            "  {} ;\n",
            HOURLY_SQL.replace('\n', "\n\t  ").replace(", ", ",   ")
        ));
        let diff = run(
            &target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]),
            Some(catalog(vec![have], &[])),
        )
        .unwrap();
        assert!(diff.timescale.is_empty(), "{:#?}", diff.timescale.changes);
        assert!(
            diff.timescale.warnings.is_empty(),
            "{:?}",
            diff.timescale.warnings
        );
        assert!(diff.create_views.is_empty() && diff.alter_views.is_empty());
    }

    #[test]
    fn test_sql_change_recreates_with_descendants_gated() {
        let mut old = hourly();
        old.sql = Some(HOURLY_SQL.replace("avg(", "max("));
        let target = target(&[
            view("HourlyMetric", HOURLY, HOURLY_SQL),
            view("DailyMetric", DAILY, DAILY_SQL),
        ]);
        let diff = run(&target, Some(catalog(vec![old, daily_on_hourly()], &[]))).unwrap();
        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.drop_aggregates, ["DailyMetric", "HourlyMetric"]);
        assert_eq!(set.create_views(), ["HourlyMetric", "DailyMetric"]);
        assert!(set.drop_views.is_empty(), "{set:?}");
        assert!(set.handoff_views.is_empty(), "{set:?}");
        assert!(set.coupled_columns.is_empty(), "{set:?}");
        assert_eq!(set.create[1].source, "HourlyMetric");
        assert_eq!(set.create[1].policy, daily_on_hourly().policy);
        for needle in [
            "\"HourlyMetric\"",
            "\"DailyMetric\"",
            "materialized data",
            "discard",
        ] {
            assert!(set.note.contains(needle), "{needle}: {}", set.note);
        }
        assert_eq!(diff.timescale.gated_notes(), vec![set.note.to_string()]);
    }

    #[test]
    fn test_sql_change_drops_descendant_missing_from_target() {
        let mut old = hourly();
        old.sql = Some(HOURLY_SQL.replace("avg(", "max("));
        let diff = run(
            &target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]),
            Some(catalog(vec![old, daily_on_hourly()], &[])),
        )
        .unwrap();
        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.drop_aggregates, ["DailyMetric", "HourlyMetric"]);
        assert_eq!(set.create_views(), ["HourlyMetric"]);
    }

    #[test]
    fn test_unmarked_aggregate_not_recreated_and_warned() {
        let target = target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]);
        let mut have = hourly();
        have.sql = None;
        let diff = run(&target, Some(catalog(vec![have.clone()], &[]))).unwrap();
        assert!(
            diff.timescale.changes.is_empty(),
            "{:#?}",
            diff.timescale.changes
        );
        assert_eq!(
            diff.timescale.warnings.len(),
            1,
            "{:?}",
            diff.timescale.warnings
        );
        let warning = &diff.timescale.warnings[0];
        for needle in [
            "\"HourlyMetric\"",
            "not created by Prax",
            "--allow-destructive",
        ] {
            assert!(warning.contains(needle), "{needle}: {warning}");
        }

        // In-place changes still apply to an unmarked aggregate.
        have.policy = None;
        let diff = run(&target, Some(catalog(vec![have], &[]))).unwrap();
        assert!(matches!(
            &single(&diff.timescale).kind,
            TimescaleChangeKind::ReplaceAggregatePolicy { view, old: None, new: Some(_) }
                if view == "HourlyMetric"
        ));
        assert_eq!(diff.timescale.warnings.len(), 1);
    }

    #[test]
    fn test_policy_change_is_ungated() {
        let target = target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]);
        let mut have = hourly();
        have.policy = Some(AggregatePolicy {
            start_offset: Some(days(7)),
            ..hourly().policy.unwrap()
        });
        let diff = run(&target, Some(catalog(vec![have.clone()], &[]))).unwrap();
        let change = single(&diff.timescale);
        let TimescaleChangeKind::ReplaceAggregatePolicy { view, old, new } = &change.kind else {
            panic!("expected ReplaceAggregatePolicy, got {:?}", change.kind);
        };
        assert_eq!(view, "HourlyMetric");
        assert_eq!(old, &have.policy);
        assert_eq!(new, &hourly().policy);
        assert!(!change.gated);
        assert!(diff.timescale.gated_notes().is_empty());

        // No policy in the database yet.
        have.policy = None;
        let diff = run(&target, Some(catalog(vec![have], &[]))).unwrap();
        let change = single(&diff.timescale);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::ReplaceAggregatePolicy {
                old: None,
                new: Some(_),
                ..
            }
        ));
        assert!(!change.gated);
    }

    #[test]
    fn test_materialized_only_change() {
        let attr = "@@timescale.continuous(source: Metric, scheduleInterval: \"1 hour\", startOffset: \"3 days\", endOffset: \"1 hour\", materializedOnly: false)";
        let diff = run(
            &target(&[view("HourlyMetric", attr, HOURLY_SQL)]),
            Some(catalog(vec![hourly()], &[])),
        )
        .unwrap();
        let change = single(&diff.timescale);
        assert!(matches!(
            &change.kind,
            TimescaleChangeKind::SetMaterializedOnly { view, old: true, new: false }
                if view == "HourlyMetric"
        ));
        assert!(!change.gated);
    }

    #[test]
    fn test_plain_view_to_aggregate_is_ungated() {
        let diff = run(
            &target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]),
            Some(catalog(vec![], &[("HourlyMetric", false)])),
        )
        .unwrap();
        let set = only_set(&diff.timescale);
        assert!(!set.gated);
        assert_eq!(set.drop_views, [("HourlyMetric", false)]);
        assert!(set.drop_aggregates.is_empty(), "{set:?}");
        assert_eq!(set.create_views(), ["HourlyMetric"]);
        assert!(diff.timescale.gated_notes().is_empty());
    }

    #[test]
    fn test_matview_to_aggregate_is_gated() {
        let diff = run(
            &target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]),
            Some(catalog(vec![], &[("HourlyMetric", true)])),
        )
        .unwrap();
        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.drop_views, [("HourlyMetric", true)]);
        assert!(set.drop_aggregates.is_empty(), "{set:?}");
        assert_eq!(set.create_views(), ["HourlyMetric"]);
        for needle in ["\"HourlyMetric\"", "materialized view", "discard"] {
            assert!(set.note.contains(needle), "{needle}: {}", set.note);
        }
        assert_eq!(diff.timescale.gated_notes(), vec![set.note.to_string()]);
    }

    #[test]
    fn test_aggregate_to_plain_view_is_gated_with_handoff() {
        for (attr, materialized) in [("", false), ("@@materialized", true)] {
            let diff = run(
                &target(&[view("HourlyMetric", attr, HOURLY_SQL)]),
                Some(catalog(vec![hourly()], &[])),
            )
            .unwrap();
            let set = only_set(&diff.timescale);
            assert!(set.gated);
            assert_eq!(set.drop_aggregates, ["HourlyMetric"]);
            assert!(set.create.is_empty(), "{set:?}");
            assert_eq!(set.handoff_views, ["HourlyMetric"]);
            for needle in ["\"HourlyMetric\"", "materialized data", "discard"] {
                assert!(set.note.contains(needle), "{needle}: {}", set.note);
            }

            let created: Vec<&str> = diff
                .create_views
                .iter()
                .map(|v| v.view_name.as_str())
                .collect();
            assert_eq!(created, ["HourlyMetric"]);
            assert_eq!(diff.create_views[0].is_materialized, materialized);
            assert!(diff.alter_views.is_empty(), "{:?}", diff.alter_views);
            assert!(diff.drop_views.is_empty(), "{:?}", diff.drop_views);
        }
    }

    #[test]
    fn test_removed_aggregate_is_gated_drop() {
        let diff = run(
            &target(&[]),
            Some(catalog(vec![hourly(), daily_on_hourly()], &[])),
        )
        .unwrap();
        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.drop_aggregates, ["DailyMetric", "HourlyMetric"]);
        assert!(set.create.is_empty(), "{set:?}");
        assert!(set.handoff_views.is_empty(), "{set:?}");
        assert!(set.note.contains("discard"), "{}", set.note);
        assert!(diff.drop_views.is_empty(), "{:?}", diff.drop_views);
    }

    #[test]
    fn test_source_mismatch_with_same_sql_is_invalid() {
        let mut have = hourly();
        have.source = "Reading".into();
        let result = run(
            &target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]),
            Some(catalog(vec![have], &[])),
        );
        let Err(MigrationError::InvalidMigration(msg)) = result else {
            panic!("expected InvalidMigration, got {result:?}");
        };
        for needle in ["source", "\"HourlyMetric\"", "\"Reading\"", "\"Metric\""] {
            assert!(msg.contains(needle), "{needle}: {msg}");
        }
    }

    #[test]
    fn test_source_column_alter_forces_gated_recreation() {
        let target = target(&[
            view("HourlyMetric", HOURLY, HOURLY_SQL),
            view("DailyMetric", DAILY, DAILY_SQL),
        ]);
        let diff = run_from(
            &target,
            &source_with("value    Int"),
            Some(catalog(vec![hourly(), daily_on_hourly()], &[])),
        )
        .unwrap();
        assert_eq!(diff.alter_models.len(), 1);
        assert_eq!(diff.alter_models[0].alter_fields[0].column_name, "value");

        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.drop_aggregates, ["DailyMetric", "HourlyMetric"]);
        assert_eq!(set.create_views(), ["HourlyMetric", "DailyMetric"]);
        assert_eq!(set.coupled_columns, [("Metric", "value")]);
        for needle in ["\"Metric\".\"value\"", "materialized data"] {
            assert!(set.note.contains(needle), "{needle}: {}", set.note);
        }
    }

    #[test]
    fn test_source_column_drop_couples_column_name() {
        let diff = run_from(
            &target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]),
            &source_with("value    Float\n  legacy   Int @map(\"legacy_col\")"),
            Some(catalog(vec![hourly()], &[])),
        )
        .unwrap();
        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.coupled_columns, [("Metric", "legacy_col")]);
        assert_eq!(set.create_views(), ["HourlyMetric"]);
    }

    #[test]
    fn test_column_alter_on_unrelated_table_is_ignored() {
        let mut prax = target(&[view("HourlyMetric", HOURLY, HOURLY_SQL)]);
        prax.push_str("\nmodel Device {\n  id   String @id\n  name String\n}\n");
        let mut source = source_with("value    Float");
        source.push_str("\nmodel Device {\n  id   String @id\n  name Int\n}\n");
        let diff = run_from(&prax, &source, Some(catalog(vec![hourly()], &[]))).unwrap();
        assert_eq!(diff.alter_models.len(), 1);
        assert!(diff.timescale.is_empty(), "{:#?}", diff.timescale.changes);
    }

    #[test]
    fn test_new_aggregate_on_recreated_parent_joins_gated_set() {
        let mut old = hourly();
        old.sql = Some(HOURLY_SQL.replace("avg(", "max("));
        let target = target(&[
            view("DailyMetric", DAILY, DAILY_SQL),
            view("HourlyMetric", HOURLY, HOURLY_SQL),
        ]);
        let diff = run(&target, Some(catalog(vec![old], &[]))).unwrap();
        let set = only_set(&diff.timescale);
        assert!(set.gated);
        assert_eq!(set.drop_aggregates, ["HourlyMetric"]);
        assert_eq!(set.create_views(), ["HourlyMetric", "DailyMetric"]);
    }

    #[test]
    fn test_ungated_set_precedes_gated_set() {
        let mut old = hourly();
        old.sql = Some(HOURLY_SQL.replace("avg(", "max("));
        let target = target(&[
            view("HourlyMetric", HOURLY, HOURLY_SQL),
            view("Report", HOURLY, HOURLY_SQL),
        ]);
        let diff = run(&target, Some(catalog(vec![old], &[("Report", false)]))).unwrap();
        assert_eq!(
            diff.timescale.changes.len(),
            2,
            "{:#?}",
            diff.timescale.changes
        );
        let all = sets(&diff.timescale);
        assert!(!all[0].gated);
        assert_eq!(all[0].drop_views, [("Report", false)]);
        assert_eq!(all[0].create_views(), ["Report"]);
        assert!(all[1].gated);
        assert_eq!(all[1].drop_aggregates, ["HourlyMetric"]);
        assert!(all[1].drop_views.is_empty(), "{:?}", all[1]);
        assert_eq!(all[1].create_views(), ["HourlyMetric"]);
    }

    #[test]
    fn test_aggregate_without_sql_is_invalid() {
        let prax = target(&[format!(
            "view HourlyMetric {{\n  bucket DateTime\n  {HOURLY}\n}}\n"
        )]);
        let result = run(&prax, Some(catalog(vec![], &[])));
        let Err(MigrationError::InvalidMigration(msg)) = result else {
            panic!("expected InvalidMigration, got {result:?}");
        };
        assert!(msg.contains("\"HourlyMetric\""), "{msg}");
        assert!(msg.contains("@@sql"), "{msg}");
    }
}
