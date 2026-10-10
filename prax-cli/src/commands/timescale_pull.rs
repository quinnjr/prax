//! Render TimescaleDB catalog state as `.prax` attributes for `prax db pull`.

use prax_migrate::TimescaleCatalog;
use prax_query::introspection::{RenderExtras, camel_case, normalize_type, pascal_case};
use prax_query::sql::DatabaseType;

/// Escape a string for a `.prax` double-quoted literal.
fn escape(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the extra model attributes and view blocks for a Timescale catalog.
///
/// Every `@@timescale.*` attribute is rendered on a single line. Relations in
/// `catalog.unsupported` get no Timescale attributes.
#[must_use]
pub fn render_extras(catalog: &TimescaleCatalog) -> RenderExtras {
    let mut extras = RenderExtras::default();

    for (table, h) in &catalog.hypertables {
        if catalog.unsupported.contains_key(table) {
            continue;
        }
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
            parts.push(format!("orderBy: [{}]", order.join(", ")));
            if let Some(after) = &cs.after {
                parts.push(format!("after: \"{after}\""));
            }
            lines.push(format!("    @@timescale.columnstore({})", parts.join(", ")));
        }
        if let Some(r) = &h.retention {
            lines.push(format!("    @@timescale.retention(dropAfter: \"{r}\")"));
        }
        extras.model_attributes.insert(table.clone(), lines);
    }

    for (view, agg) in &catalog.aggregates {
        // Aggregates never render as plain views, representable or not.
        extras.skip_views.insert(view.clone());
        // `scheduleInterval` is required, so an aggregate without a refresh
        // policy has no representable attribute.
        let Some(policy) = &agg.policy else { continue };
        if catalog.unsupported.contains_key(view) {
            continue;
        }
        let name = pascal_case(view);
        let sql = agg
            .sql
            .as_deref()
            .or_else(|| catalog.aggregate_definitions.get(view).map(String::as_str))
            .unwrap_or_default();
        let inline = sql.contains("\"\"\"");

        let mut block = String::new();
        if agg.sql.is_none() {
            block.push_str("// not created by Prax \u{2014} SQL is Postgres-normalized\n");
        }
        block.push_str(&format!("view {name} {{\n"));
        for col in catalog.aggregate_columns.get(view).into_iter().flatten() {
            let ty = normalize_type(DatabaseType::PostgreSQL, &col.udt_name, None, None, None)
                .to_prax_type();
            let opt = if col.nullable { "?" } else { "" };
            block.push_str(&format!("    {} {ty}{opt}\n", camel_case(&col.name)));
        }
        if name.to_lowercase() != view.to_lowercase() {
            block.push_str(&format!("    @@map(\"{}\")\n", escape(view)));
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
            block.push_str(&format!("    @@sql(\"{}\")\n", escape(sql)));
        }
        block.push('}');
        if !inline {
            block.push_str(&format!(
                "\n\n@@sql(\"{}\", \"\"\"{sql}\"\"\")",
                escape(view)
            ));
        }
        extras.extra_blocks.push(block);
    }

    extras
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::introspect::format_as_prax;
    use crate::config::Config;
    use prax_migrate::timescale::catalog::{
        AggregatePolicy, AggregateState, CatalogColumn, ColumnstoreSettings, ColumnstoreState,
        HypertableState,
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
    fn test_unsupported_hypertable_renders_nothing() {
        let mut c = catalog(Some(AUTHORED));
        c.unsupported.insert("Metric".into(), "why".into());
        assert!(render_extras(&c).model_attributes.is_empty());
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
        let out = format_as_prax(&schema, &config, Some(&catalog(Some(AUTHORED))));
        assert!(out.contains("extensions = [timescaledb]"));
        assert!(out.contains("@@timescale.hypertable(time,"));
        assert!(out.contains("view HourlyMetric {"));
        if let Err(e) = prax_schema::validate_schema(&out) {
            panic!("{e:?}\n{out}");
        }
    }
}
