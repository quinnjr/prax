//! Render TimescaleDB catalog state as `.prax` attributes for `prax db pull`.

use std::collections::{BTreeMap, BTreeSet};

use prax_migrate::TimescaleCatalog;
use prax_migrate::timescale::{
    HypertableState, TIMESCALE_MIN_VERSION, comment_safe, timescale_version_supported,
};
use prax_query::introspection::{
    RenderExtras, camel_case, escape_map_value, normalize_type, pascal_case,
};
use prax_query::sql::DatabaseType;

/// Keep `catalog` for rendering when the installed TimescaleDB is a version
/// Prax supports.
///
/// # Errors
///
/// Returns the warning to show instead when the installed TimescaleDB is
/// older than [`TIMESCALE_MIN_VERSION`]: such a catalog carries only the
/// version, so none of its hypertables or continuous aggregates can be pulled.
pub fn supported_catalog(catalog: TimescaleCatalog) -> Result<TimescaleCatalog, String> {
    if timescale_version_supported(&catalog.version) {
        return Ok(catalog);
    }
    let (major, minor) = TIMESCALE_MIN_VERSION;
    Err(format!(
        "TimescaleDB {} is older than {major}.{minor}, the minimum Prax supports; \
         hypertables and continuous aggregates were not pulled.",
        comment_safe(&catalog.version)
    ))
}

/// Whether `ident` is a valid `.prax` identifier (`[A-Za-z_][A-Za-z0-9_]*`).
fn is_prax_ident(ident: &str) -> bool {
    let mut chars = ident.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The first of `names` that cannot be rendered as `convert(name)`: it holds a
/// control character (which could end the line it is rendered on), or its
/// converted form is not a `.prax` identifier.
fn first_unrepresentable<'a>(
    names: impl IntoIterator<Item = &'a str>,
    convert: fn(&str) -> String,
) -> Option<&'a str> {
    names
        .into_iter()
        .find(|name| name.chars().any(char::is_control) || !is_prax_ident(&convert(name)))
}

/// The reason given for a name that cannot be rendered.
fn unrepresentable(name: &str) -> String {
    format!("name \"{name}\" cannot be represented in a .prax schema")
}

/// A `// prax:` line reporting that the `kind` named `name` was not pulled,
/// and why. Both may come from the database, so they are made safe for a
/// one-line comment.
fn not_pulled(kind: &str, name: &str, reason: &str) -> String {
    format!(
        "// prax: {kind} \"{}\" was not pulled: {}.",
        comment_safe(name),
        comment_safe(reason)
    )
}

/// A catalog reason collapsed onto one line.
fn one_line(reason: &str) -> String {
    reason.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build the extra model attributes and view blocks for a Timescale catalog.
///
/// Every `@@timescale.*` attribute is rendered on a single line. A hypertable
/// Prax cannot manage, or whose table or referenced columns cannot be named in
/// a `.prax` schema, is rendered as a plain model plus a `// prax:` comment
/// naming the reason. Likewise a continuous aggregate that is unsupported, has
/// no refresh policy or SQL definition, carries an unrepresentable name, or is
/// built on a relation that was not pulled gets a `// prax:` comment instead
/// of a view block.
#[must_use]
pub fn render_extras(catalog: &TimescaleCatalog) -> RenderExtras {
    let mut extras = RenderExtras::default();

    // Unsupported hypertables are left out of `catalog.hypertables`.
    for (table, reason) in &catalog.unsupported {
        if !catalog.unsupported_aggregates.contains(table) {
            extras
                .extra_blocks
                .push(not_pulled("hypertable", table, &one_line(reason)));
        }
    }
    for (table, h) in &catalog.hypertables {
        if catalog.unsupported.contains_key(table) {
            continue; // reported above
        }
        if let Some(name) = unrepresentable_hypertable_name(table, h) {
            extras
                .extra_blocks
                .push(not_pulled("hypertable", table, &unrepresentable(name)));
            continue;
        }
        extras
            .model_attributes
            .insert(table.clone(), render_hypertable_attrs(h));
    }

    let views: BTreeSet<&str> = catalog
        .aggregates
        .keys()
        .chain(&catalog.unsupported_aggregates)
        .map(String::as_str)
        .collect();
    let mut rendered: BTreeMap<&str, Result<String, String>> = views
        .iter()
        .map(|view| (*view, render_aggregate(catalog, view)))
        .collect();
    // An aggregate is only valid over a source that is pulled too: a model
    // carrying `@@timescale.hypertable` or a rendered aggregate. Repeat until
    // no rendered aggregate is left over a source that was not pulled.
    loop {
        let orphaned: Vec<(&str, &str)> = rendered
            .iter()
            .filter(|(_, block)| block.is_ok())
            .filter_map(|(view, _)| {
                let source = catalog.aggregates.get(*view)?.source.as_str();
                let pulled = extras.model_attributes.contains_key(source)
                    || rendered.get(source).is_some_and(Result::is_ok);
                (!pulled).then_some((*view, source))
            })
            .collect();
        if orphaned.is_empty() {
            break;
        }
        for (view, source) in orphaned {
            let reason = format!("it is built on \"{source}\", which was not pulled");
            let comment = not_pulled("continuous aggregate", view, &reason);
            rendered.insert(view, Err(comment));
        }
    }
    for view in views {
        // Aggregates never render as plain views, representable or not.
        extras.skip_views.insert(view.to_string());
        if let Some(Ok(block) | Err(block)) = rendered.remove(view) {
            extras.extra_blocks.push(block);
        }
    }

    extras
}

/// The first name of hypertable `table` that cannot be rendered: the table as
/// a model name, or a column its attributes reference as a field name.
fn unrepresentable_hypertable_name<'a>(table: &'a str, h: &'a HypertableState) -> Option<&'a str> {
    let columns = std::iter::once(h.time_column.as_str())
        .chain(h.space.as_ref().map(|(col, _)| col.as_str()))
        .chain(h.columnstore.iter().flat_map(|cs| {
            cs.settings
                .segment_by
                .iter()
                .map(String::as_str)
                .chain(cs.settings.order_by.iter().map(|(col, _)| col.as_str()))
        }));
    first_unrepresentable([table], pascal_case)
        .or_else(|| first_unrepresentable(columns, camel_case))
}

/// The `@@timescale.*` model attribute lines for hypertable `h`.
fn render_hypertable_attrs(h: &HypertableState) -> Vec<String> {
    let mut lines = Vec::new();
    let mut hyper = format!(
        "    @@timescale.hypertable({}, chunkInterval: \"{}\"",
        camel_case(&h.time_column),
        h.chunk_interval
    );
    if let Some((col, n)) = &h.space {
        hyper.push_str(&format!(", space: hash({}, {})", camel_case(col), n));
    }
    hyper.push(')');
    lines.push(hyper);

    if let Some(cs) = &h.columnstore {
        let mut parts = Vec::new();
        if !cs.settings.segment_by.is_empty() {
            let cols: Vec<_> = cs
                .settings
                .segment_by
                .iter()
                .map(|c| camel_case(c))
                .collect();
            parts.push(format!("segmentBy: [{}]", cols.join(", ")));
        }
        let order: Vec<_> = cs
            .settings
            .order_by
            .iter()
            .map(|(c, desc)| {
                if *desc {
                    format!("desc({})", camel_case(c))
                } else {
                    camel_case(c)
                }
            })
            .collect();
        // `orderBy: []` does not parse; omitted, it defaults to the time
        // column descending.
        if !order.is_empty() {
            parts.push(format!("orderBy: [{}]", order.join(", ")));
        }
        if let Some(after) = &cs.after {
            parts.push(format!("after: \"{after}\""));
        }
        lines.push(format!("    @@timescale.columnstore({})", parts.join(", ")));
    }
    if let Some(r) = &h.retention {
        lines.push(format!("    @@timescale.retention(dropAfter: \"{r}\")"));
    }
    lines
}

/// The view block for continuous aggregate `view`, followed by its top-level
/// `@@sql` unless the SQL has to be inlined.
///
/// # Errors
///
/// Returns the `// prax:` comment line to emit instead when the aggregate is
/// unsupported, carries a name that cannot be represented in a `.prax`
/// schema, has no refresh policy, or has no SQL definition.
fn render_aggregate(catalog: &TimescaleCatalog, view: &str) -> Result<String, String> {
    let skip = |reason: &str| not_pulled("continuous aggregate", view, reason);
    let Some(agg) = catalog.aggregates.get(view) else {
        let reason = catalog
            .unsupported
            .get(view)
            .map(String::as_str)
            .map_or_else(|| "Prax cannot manage it".to_string(), one_line);
        return Err(skip(&reason));
    };
    let columns = catalog
        .aggregate_columns
        .get(view)
        .map_or(&[][..], Vec::as_slice);
    if let Some(name) = first_unrepresentable([view, agg.source.as_str()], pascal_case)
        .or_else(|| first_unrepresentable(columns.iter().map(|c| c.name.as_str()), camel_case))
    {
        return Err(skip(&unrepresentable(name)));
    }
    // `scheduleInterval` is required, so an aggregate without a refresh
    // policy has no representable attribute.
    let Some(policy) = &agg.policy else {
        return Err(skip(
            "it has no refresh policy, and @@timescale.continuous requires scheduleInterval",
        ));
    };
    let non_empty = |sql: &&str| !sql.trim().is_empty();
    let recorded = agg.sql.as_deref().filter(non_empty);
    let Some(sql) = recorded.or_else(|| {
        catalog
            .aggregate_definitions
            .get(view)
            .map(String::as_str)
            .filter(non_empty)
    }) else {
        return Err(skip("no SQL definition"));
    };

    let name = pascal_case(view);
    let inline = sql.contains("\"\"\"") || sql.starts_with('"') || sql.ends_with('"');

    let mut block = String::new();
    if recorded.is_none() {
        block.push_str("// not created by Prax \u{2014} SQL is Postgres-normalized\n");
    }
    block.push_str(&format!("view {name} {{\n"));
    for col in columns {
        let ty = normalize_type(DatabaseType::PostgreSQL, &col.udt_name, None, None, None)
            .to_prax_type();
        let opt = if col.nullable { "?" } else { "" };
        block.push_str(&format!("    {} {ty}{opt}\n", camel_case(&col.name)));
    }
    // Compared exactly: a case-only difference (`hourly` vs `Hourly`) still
    // needs `@@map`, or the top-level `@@sql("hourly", ...)` would not
    // resolve to this view.
    if name != view {
        block.push_str(&format!("    @@map(\"{}\")\n", escape_map_value(view)));
    }
    let mut parts = vec![format!("source: {}", pascal_case(&agg.source))];
    parts.push(format!(
        "scheduleInterval: \"{}\"",
        policy.schedule_interval
    ));
    if let Some(s) = &policy.start_offset {
        parts.push(format!("startOffset: \"{s}\""));
    }
    if let Some(e) = &policy.end_offset {
        parts.push(format!("endOffset: \"{e}\""));
    }
    // The attribute defaults to materialized-only, so the live
    // (real-time) state must be stated explicitly.
    if !agg.materialized_only {
        parts.push("materializedOnly: false".to_string());
    }
    block.push_str(&format!(
        "    @@timescale.continuous({})\n",
        parts.join(", ")
    ));
    if inline {
        block.push_str(&format!("    @@sql(\"{}\")\n", escape_map_value(sql)));
    }
    block.push('}');
    if !inline {
        block.push_str(&format!(
            "\n\n@@sql(\"{}\", \"\"\"{sql}\"\"\")",
            escape_map_value(view)
        ));
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::introspect::format_as_prax_with;
    use crate::config::Config;
    use prax_migrate::timescale::catalog::{
        AggregatePolicy, AggregateState, CatalogColumn, ColumnstoreSettings, ColumnstoreState,
    };
    use prax_query::introspection::{ColumnInfo, DatabaseSchema, TableInfo};
    use prax_schema::ast::PgInterval;

    fn days(d: i32) -> PgInterval {
        PgInterval::new(0, d, 0)
    }

    fn hours(h: i64) -> PgInterval {
        PgInterval::new(0, 0, h * 3_600_000_000)
    }

    fn col(name: &str, udt: &str) -> CatalogColumn {
        CatalogColumn {
            name: name.into(),
            udt_name: udt.into(),
            nullable: false,
        }
    }

    fn catalog(sql: Option<&str>) -> TimescaleCatalog {
        let mut c = TimescaleCatalog {
            version: "2.18.0".into(),
            hypertables: Default::default(),
            aggregates: Default::default(),
            aggregate_definitions: Default::default(),
            aggregate_columns: Default::default(),
            other_views: Default::default(),
            unsupported: Default::default(),
            unsupported_aggregates: Default::default(),
        };
        c.hypertables.insert(
            "Metric".into(),
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
            },
        );
        c.aggregates.insert(
            "HourlyMetric".into(),
            AggregateState {
                view: "HourlyMetric".into(),
                source: "Metric".into(),
                sql: sql.map(str::to_string),
                materialized_only: true,
                policy: Some(AggregatePolicy {
                    start_offset: Some(days(3)),
                    end_offset: Some(hours(1)),
                    schedule_interval: hours(1),
                }),
            },
        );
        c.aggregate_definitions.insert(
            "HourlyMetric".into(),
            "SELECT time_bucket('1 hour', \"time\") AS bucket FROM \"Metric\" GROUP BY 1".into(),
        );
        c.aggregate_columns.insert(
            "HourlyMetric".into(),
            vec![
                col("bucket", "timestamptz"),
                col("deviceId", "text"),
                col("avgValue", "float8"),
            ],
        );
        c
    }

    const AUTHORED: &str = "SELECT time_bucket('1 hour', \"time\") AS bucket, \"deviceId\", avg(value) AS \"avgValue\" FROM \"Metric\" GROUP BY 1, 2";

    #[test]
    fn test_render_hypertable_lines() {
        let extras = render_extras(&catalog(Some(AUTHORED)));
        assert_eq!(
            extras.model_attributes["Metric"],
            vec![
                r#"    @@timescale.hypertable(time, chunkInterval: "1 day", space: hash(deviceId, 4))"#,
                r#"    @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "7 days")"#,
                r#"    @@timescale.retention(dropAfter: "90 days")"#,
            ]
        );
        assert!(extras.skip_views.contains("HourlyMetric"));
    }

    #[test]
    fn test_empty_order_by_is_omitted_and_validates() {
        let mut c = catalog(Some(AUTHORED));
        c.hypertables
            .get_mut("Metric")
            .and_then(|h| h.columnstore.as_mut())
            .unwrap()
            .settings
            .order_by
            .clear();
        let extras = render_extras(&c);
        assert_eq!(
            extras.model_attributes["Metric"][1],
            r#"    @@timescale.columnstore(segmentBy: [deviceId], after: "7 days")"#
        );
        let out = render_schema(&c);
        if let Err(e) = prax_schema::validate_schema(&out) {
            panic!("{e:?}\n{out}");
        }
    }

    #[test]
    fn test_render_aggregate_block() {
        let extras = render_extras(&catalog(Some(AUTHORED)));
        let b = &extras.extra_blocks[0];
        assert!(b.contains("view HourlyMetric {"));
        assert!(b.contains("    bucket DateTime"));
        assert!(b.contains(
            r#"@@timescale.continuous(source: Metric, scheduleInterval: "1 hour", startOffset: "3 days", endOffset: "1 hour")"#
        ));
        assert!(b.contains(r#"@@sql("HourlyMetric", """"#));
        assert!(!b.contains("not created by Prax"));
    }

    #[test]
    fn test_render_unmarked_aggregate_uses_view_definition() {
        let extras = render_extras(&catalog(None));
        let b = &extras.extra_blocks[0];
        assert!(b.starts_with("// not created by Prax"));
        assert!(b.contains("SELECT time_bucket('1 hour'"));
    }

    #[test]
    fn test_render_sql_with_triple_quotes_is_inline() {
        let extras = render_extras(&catalog(Some("SELECT '\"\"\"'")));
        let b = &extras.extra_blocks[0];
        assert!(b.contains(r#"    @@sql("SELECT '\"\"\"'")"#));
        assert!(!b.contains("\"\"\"\")"));
    }

    #[test]
    fn test_aggregate_without_policy_is_reported() {
        let mut c = catalog(Some(AUTHORED));
        c.aggregates.get_mut("HourlyMetric").unwrap().policy = None;
        let extras = render_extras(&c);
        assert_eq!(
            extras.extra_blocks,
            vec![
                r#"// prax: continuous aggregate "HourlyMetric" was not pulled: it has no refresh policy, and @@timescale.continuous requires scheduleInterval."#
            ]
        );
        assert!(extras.skip_views.contains("HourlyMetric"));
    }

    #[test]
    fn test_unsupported_aggregate_is_reported() {
        // As `TimescaleCatalog::from_rows` leaves it: in `unsupported` and
        // `unsupported_aggregates`, not in `aggregates`.
        let mut c = catalog(Some(AUTHORED));
        c.aggregates.clear();
        c.aggregate_definitions.clear();
        c.aggregate_columns.clear();
        c.unsupported.insert(
            "HourlyMetric".into(),
            "it is built on \"Gone\", which is not a hypertable in this schema".into(),
        );
        c.unsupported_aggregates.insert("HourlyMetric".into());
        let extras = render_extras(&c);
        assert_eq!(
            extras.extra_blocks,
            vec![
                r#"// prax: continuous aggregate "HourlyMetric" was not pulled: it is built on "Gone", which is not a hypertable in this schema."#
            ]
        );
        assert!(extras.skip_views.contains("HourlyMetric"));
        // The pulled schema still validates.
        let out = render_schema(&c);
        assert!(!out.contains("view HourlyMetric"), "{out}");
        if let Err(e) = prax_schema::validate_schema(&out) {
            panic!("{e:?}\n{out}");
        }
    }

    #[test]
    fn test_sql_ending_in_quote_is_inline_and_validates() {
        let sql = "SELECT time_bucket('1 hour', \"time\") AS bucket FROM \"Metric\" GROUP BY 1 ORDER BY \"bucket\"";
        let extras = render_extras(&catalog(Some(sql)));
        assert!(extras.extra_blocks[0].contains("    @@sql(\""));
        assert!(!extras.extra_blocks[0].contains("\"\"\""));
        let out = render_schema(&catalog(Some(sql)));
        let schema = prax_schema::validate_schema(&out).unwrap_or_else(|e| panic!("{e:?}\n{out}"));
        let view = schema.views.get("HourlyMetric").unwrap();
        assert_eq!(view.resolved_sql(&schema), Some(sql));
    }

    #[test]
    fn test_unsupported_hypertable_is_reported() {
        // As `TimescaleCatalog::from_rows` leaves it: in `unsupported`, not
        // in `hypertables`. Its aggregate is unsupported with it.
        let mut c = catalog(Some(AUTHORED));
        c.hypertables.clear();
        c.aggregates.clear();
        c.aggregate_definitions.clear();
        c.aggregate_columns.clear();
        c.unsupported.insert(
            "Metric".into(),
            "integer time dimensions are\n  not supported".into(),
        );
        let extras = render_extras(&c);
        assert!(extras.model_attributes.is_empty());
        assert_eq!(
            extras.extra_blocks,
            vec![
                r#"// prax: hypertable "Metric" was not pulled: integer time dimensions are not supported."#
            ]
        );
        let out = render_schema(&c);
        if let Err(e) = prax_schema::validate_schema(&out) {
            panic!("{e:?}\n{out}");
        }
    }

    #[test]
    fn test_hypertable_also_listed_unsupported_renders_no_attributes() {
        let mut c = catalog(Some(AUTHORED));
        c.unsupported.insert("Metric".into(), "why".into());
        let extras = render_extras(&c);
        assert!(extras.model_attributes.is_empty());
        assert_eq!(
            extras.extra_blocks[0],
            r#"// prax: hypertable "Metric" was not pulled: why."#
        );
        // Its aggregate has no hypertable source left to stand on.
        assert!(
            extras.extra_blocks.contains(
                &r#"// prax: continuous aggregate "HourlyMetric" was not pulled: it is built on "Metric", which was not pulled."#
                    .to_string()
            ),
            "{:?}",
            extras.extra_blocks
        );
    }

    /// The `catalog` fixture with lowercase relation names, as most
    /// hand-written databases have: hypertable `metrics`, aggregate `hourly`.
    fn lowercase_catalog() -> TimescaleCatalog {
        let mut c = catalog(Some(
            "SELECT time_bucket('1 hour', \"time\") AS bucket, \"deviceId\", avg(value) AS \"avgValue\" FROM metrics GROUP BY 1, 2",
        ));
        let mut h = c.hypertables.remove("Metric").unwrap();
        h.table = "metrics".into();
        c.hypertables.insert("metrics".into(), h);
        let mut a = c.aggregates.remove("HourlyMetric").unwrap();
        a.view = "hourly".into();
        a.source = "metrics".into();
        c.aggregates.insert("hourly".into(), a);
        let cols = c.aggregate_columns.remove("HourlyMetric").unwrap();
        c.aggregate_columns.insert("hourly".into(), cols);
        c.aggregate_definitions.clear();
        c
    }

    #[test]
    fn test_lowercase_names_get_map_and_validate() {
        let c = lowercase_catalog();
        let extras = render_extras(&c);
        let b = &extras.extra_blocks[0];
        assert!(b.contains("view Hourly {"), "{b}");
        assert!(b.contains("    @@map(\"hourly\")\n"), "{b}");
        assert!(b.contains("source: Metrics,"), "{b}");
        assert!(b.contains(r#"@@sql("hourly", """"#), "{b}");

        let out = render_schema_for("metrics", &c);
        assert!(out.contains("model Metrics {"), "{out}");
        assert!(out.contains("@@map(\"metrics\")"), "{out}");
        assert!(out.contains("@@map(\"hourly\")"), "{out}");
        let schema = prax_schema::validate_schema(&out).unwrap_or_else(|e| panic!("{e:?}\n{out}"));
        let view = schema.views.get("Hourly").unwrap();
        assert!(view.resolved_sql(&schema).is_some(), "{out}");
    }

    #[test]
    fn test_unrepresentable_hypertable_names_are_reported() {
        for bad in ["device\nid", "device-id", "t\u{e9}l\u{e9}m\u{e9}trie", ""] {
            let mut c = catalog(Some(AUTHORED));
            c.hypertables
                .get_mut("Metric")
                .and_then(|h| h.columnstore.as_mut())
                .unwrap()
                .settings
                .segment_by = vec![bad.to_string()];
            let extras = render_extras(&c);
            assert!(extras.model_attributes.is_empty(), "{bad:?}");
            let comment = &extras.extra_blocks[0];
            assert!(
                comment.starts_with(r#"// prax: hypertable "Metric" was not pulled: name ""#)
                    && comment.ends_with(r#"" cannot be represented in a .prax schema."#),
                "{comment:?}"
            );
            assert!(!comment.contains('\n'), "{comment:?}");
        }
    }

    #[test]
    fn test_unrepresentable_hypertable_table_is_reported() {
        let mut c = catalog(Some(AUTHORED));
        let h = c.hypertables.remove("Metric").unwrap();
        c.hypertables.insert("Metric\n@@ignore".into(), h);
        let extras = render_extras(&c);
        assert!(extras.model_attributes.is_empty());
        assert_eq!(
            extras.extra_blocks[0],
            r#"// prax: hypertable "Metric @@ignore" was not pulled: name "Metric @@ignore" cannot be represented in a .prax schema."#
        );
    }

    #[test]
    fn test_unrepresentable_aggregate_names_are_reported() {
        // An unrepresentable view name or column name each block the view.
        let mut view_name = catalog(Some(AUTHORED));
        let a = view_name.aggregates.remove("HourlyMetric").unwrap();
        view_name
            .aggregates
            .insert("hourly\n@@sql(\"x\")".into(), a);

        let mut column_name = catalog(Some(AUTHORED));
        column_name
            .aggregate_columns
            .get_mut("HourlyMetric")
            .unwrap()
            .push(col("avg value", "float8"));

        for (c, bad) in [
            (view_name, "hourly @@sql(\"x\")"),
            (column_name, "avg value"),
        ] {
            let extras = render_extras(&c);
            let comment = extras.extra_blocks.last().unwrap();
            assert!(
                comment.ends_with(&format!(
                    "was not pulled: name \"{bad}\" cannot be represented in a .prax schema."
                )),
                "{comment:?}"
            );
            assert!(!comment.contains('\n'), "{comment:?}");
            assert!(!comment.contains("view "), "{comment:?}");
        }
    }

    #[test]
    fn test_aggregate_without_sql_is_reported() {
        let mut c = catalog(None);
        c.aggregate_definitions.clear();
        let extras = render_extras(&c);
        assert_eq!(
            extras.extra_blocks,
            vec![
                r#"// prax: continuous aggregate "HourlyMetric" was not pulled: no SQL definition."#
            ]
        );
        assert!(extras.skip_views.contains("HourlyMetric"));
    }

    #[test]
    fn test_unsupported_aggregate_name_is_comment_safe() {
        let mut c = catalog(Some(AUTHORED));
        c.aggregates.clear();
        c.aggregate_definitions.clear();
        c.aggregate_columns.clear();
        c.unsupported
            .insert("x\nview Evil {".into(), "reason\r\nmodel Evil {}".into());
        c.unsupported_aggregates.insert("x\nview Evil {".into());
        let extras = render_extras(&c);
        assert_eq!(
            extras.extra_blocks,
            vec![
                r#"// prax: continuous aggregate "x view Evil {" was not pulled: reason model Evil {}."#
            ]
        );
    }

    #[test]
    fn test_old_timescale_is_not_pulled() {
        let mut old = catalog(Some(AUTHORED));
        old.version = "2.17.2".into();
        let message = supported_catalog(old).unwrap_err();
        let (major, minor) = TIMESCALE_MIN_VERSION;
        assert!(
            message.contains("2.17.2") && message.contains(&format!("{major}.{minor}")),
            "{message}"
        );
        assert!(message.contains("were not pulled"), "{message}");

        let current = catalog(Some(AUTHORED));
        assert_eq!(supported_catalog(current.clone()), Ok(current));
    }

    fn column(name: &str, db_type: &str, pk: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            db_type: db_type.into(),
            normalized_type: prax_query::introspection::normalize_type(
                DatabaseType::PostgreSQL,
                db_type,
                None,
                None,
                None,
            ),
            nullable: false,
            is_primary_key: pk,
            ..Default::default()
        }
    }

    fn render_schema(cat: &TimescaleCatalog) -> String {
        render_schema_for("Metric", cat)
    }

    /// Render a database with one `table` (time, deviceId, value) and `cat`.
    fn render_schema_for(table: &str, cat: &TimescaleCatalog) -> String {
        let schema = DatabaseSchema {
            name: "db".into(),
            tables: vec![TableInfo {
                name: table.into(),
                columns: vec![
                    column("time", "timestamptz", true),
                    column("deviceId", "text", true),
                    column("value", "float8", false),
                ],
                primary_key: vec!["deviceId".into(), "time".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut config = Config::default();
        config.database.provider = "postgresql".into();
        format_as_prax_with(&schema, &config, Some(cat))
    }

    #[test]
    fn test_rendered_schema_validates() {
        let schema = DatabaseSchema {
            name: "db".into(),
            tables: vec![TableInfo {
                name: "Metric".into(),
                columns: vec![
                    column("time", "timestamptz", true),
                    column("deviceId", "text", true),
                    column("value", "float8", false),
                ],
                primary_key: vec!["deviceId".into(), "time".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut config = Config::default();
        config.database.provider = "postgresql".into();
        let out = format_as_prax_with(&schema, &config, Some(&catalog(Some(AUTHORED))));
        assert!(out.contains("extensions = [timescaledb]"));
        assert!(out.contains("@@timescale.hypertable(time,"));
        assert!(out.contains("view HourlyMetric {"));
        if let Err(e) = prax_schema::validate_schema(&out) {
            panic!("{e:?}\n{out}");
        }
    }
}
