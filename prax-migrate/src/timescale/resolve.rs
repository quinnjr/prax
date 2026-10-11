//! Resolve a target schema's Timescale configs into catalog form.
//!
//! Configs hold Prax field, model and view names; the catalog holds table,
//! column and view names. Resolution maps each field through its model's
//! `@map`, keys each hypertable by `Model::table_name()` and each continuous
//! aggregate by `View::view_name()`, so comparisons never churn on `@map`.
//!
//! Configs are parsed strictly: an invalid `@@timescale.*` attribute is an
//! error, never a model or view silently treated as having none.

use std::collections::BTreeMap;

use prax_schema::Schema;
use prax_schema::ast::{ContinuousAggregateConfig, Model, TimescaleModelConfig, View};

use crate::diff::index_column_name;
use crate::error::{MigrateResult, MigrationError};
use crate::timescale::catalog::{
    AggregatePolicy, AggregateState, ColumnstoreSettings, ColumnstoreState, HypertableState,
};

/// Parse `model`'s `@@timescale.*` attributes; `None` when it has none.
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] naming the model when its
/// TimescaleDB attributes are invalid.
pub(crate) fn model_timescale(model: &Model) -> MigrateResult<Option<TimescaleModelConfig>> {
    TimescaleModelConfig::from_model(model).map_err(|e| {
        MigrationError::InvalidMigration(format!(
            "Model \"{}\" has invalid TimescaleDB attributes: {e}. Fix its @@timescale.* \
             attributes before migrating.",
            model.name()
        ))
    })
}

/// Parse `view`'s `@@timescale.continuous` attribute; `None` when it has none.
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] naming the view when its
/// TimescaleDB attributes are invalid.
pub(crate) fn view_continuous_aggregate(
    view: &View,
) -> MigrateResult<Option<ContinuousAggregateConfig>> {
    ContinuousAggregateConfig::from_view(view).map_err(|e| {
        MigrationError::InvalidMigration(format!(
            "View \"{}\" has invalid TimescaleDB attributes: {e}. Fix its \
             @@timescale.continuous attribute before migrating.",
            view.name()
        ))
    })
}

/// Resolve every hypertable model in `schema` into column-name form, keyed by
/// table name.
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] when a model's
/// `@@timescale.*` attributes are invalid.
pub fn resolve_hypertables(schema: &Schema) -> MigrateResult<BTreeMap<String, HypertableState>> {
    let mut resolved = BTreeMap::new();
    for model in schema.models.values() {
        if let Some(config) = model_timescale(model)? {
            resolved.insert(
                model.table_name().to_string(),
                resolve_model(model, &config),
            );
        }
    }
    Ok(resolved)
}

/// Resolve every continuous-aggregate view in `schema` into catalog form,
/// keyed by database view name (`@@map`).
///
/// `source` becomes the database name of the model (`Model::table_name`) or
/// view (`View::view_name`) it names, `sql` the view's verbatim SQL, and
/// `policy` the declared refresh policy.
///
/// # Errors
///
/// Returns [`MigrationError::InvalidMigration`] when a view's
/// `@@timescale.continuous` attribute is invalid.
pub fn resolve_aggregates(schema: &Schema) -> MigrateResult<BTreeMap<String, AggregateState>> {
    let mut resolved = BTreeMap::new();
    for view in schema.views.values() {
        let Some(config) = view_continuous_aggregate(view)? else {
            continue;
        };
        let name = view.view_name().to_string();
        resolved.insert(
            name.clone(),
            AggregateState {
                view: name,
                source: relation_name(schema, &config.source),
                sql: view.resolved_sql(schema).map(str::to_string),
                materialized_only: config.materialized_only,
                policy: Some(AggregatePolicy {
                    start_offset: config.start_offset,
                    end_offset: config.end_offset,
                    schedule_interval: config.schedule_interval,
                }),
            },
        );
    }
    Ok(resolved)
}

/// The database name of the model or view called `name`; `name` itself when
/// neither exists (the validator rejects that).
fn relation_name(schema: &Schema, name: &str) -> String {
    schema
        .models
        .values()
        .find(|m| m.name() == name)
        .map(Model::table_name)
        .or_else(|| {
            schema
                .views
                .values()
                .find(|v| v.name() == name)
                .map(View::view_name)
        })
        .unwrap_or(name)
        .to_string()
}

fn resolve_model(model: &Model, config: &TimescaleModelConfig) -> HypertableState {
    let column = |field: &str| index_column_name(model, field);
    let hypertable = &config.hypertable;
    HypertableState {
        table: model.table_name().to_string(),
        time_column: column(&hypertable.time_field),
        chunk_interval: hypertable.chunk_interval,
        space: hypertable
            .space
            .as_ref()
            .map(|space| (column(&space.field), space.partitions)),
        columnstore: config.columnstore.as_ref().map(|cs| ColumnstoreState {
            settings: ColumnstoreSettings {
                segment_by: cs.segment_by.iter().map(|f| column(f)).collect(),
                order_by: cs
                    .order_by
                    .iter()
                    .map(|o| (column(&o.field), o.descending))
                    .collect(),
            },
            after: cs.after,
        }),
        retention: config.retention.as_ref().map(|r| r.drop_after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(prax: &str) -> Schema {
        prax_schema::parse_schema(prax).unwrap()
    }

    fn invalid_message<T: std::fmt::Debug>(result: MigrateResult<T>) -> String {
        match result {
            Err(MigrationError::InvalidMigration(msg)) => msg,
            other => panic!("expected InvalidMigration, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_hypertables_rejects_malformed_attribute() {
        // `@@timescale.columnstore` without `@@timescale.hypertable` is invalid;
        // it must not resolve as a plain table.
        let prax = "model Metric {\n  time     DateTime\n  deviceId String\n  @@id([deviceId, time])\n  @@timescale.columnstore(segmentBy: [deviceId])\n}\n";
        let msg = invalid_message(resolve_hypertables(&schema(prax)));
        assert!(msg.contains("\"Metric\""), "{msg}");
        assert!(msg.contains("@@timescale.hypertable"), "{msg}");
    }

    #[test]
    fn test_resolve_hypertables_rejects_unknown_argument() {
        let prax = "model Metric {\n  time DateTime\n  @@id([time])\n  @@timescale.hypertable(time, chunkInterval: \"not an interval\")\n}\n";
        let msg = invalid_message(resolve_hypertables(&schema(prax)));
        assert!(msg.contains("\"Metric\""), "{msg}");
        assert!(msg.contains("invalid TimescaleDB attributes"), "{msg}");
    }

    #[test]
    fn test_resolve_aggregates_rejects_malformed_attribute() {
        // `scheduleInterval` is required.
        let prax =
            "view Hourly {\n  bucket DateTime\n  @@timescale.continuous(source: Metric)\n}\n";
        let msg = invalid_message(resolve_aggregates(&schema(prax)));
        assert!(msg.contains("\"Hourly\""), "{msg}");
        assert!(msg.contains("@@timescale.continuous"), "{msg}");
    }

    #[test]
    fn test_resolve_skips_relations_without_timescale_attributes() {
        let prax = "model Metric {\n  time DateTime\n  @@id([time])\n}\n\nview Plain {\n  bucket DateTime\n}\n";
        let schema = schema(prax);
        assert!(resolve_hypertables(&schema).unwrap().is_empty());
        assert!(resolve_aggregates(&schema).unwrap().is_empty());
    }
}
