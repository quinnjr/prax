//! Resolve a target schema's Timescale configs into catalog form.
//!
//! Configs hold Prax field, model and view names; the catalog holds table,
//! column and view names. Resolution maps each field through its model's
//! `@map`, keys each hypertable by `Model::table_name()` and each continuous
//! aggregate by `View::view_name()`, so comparisons never churn on `@map`.

use std::collections::BTreeMap;

use prax_schema::Schema;
use prax_schema::ast::{Model, TimescaleModelConfig, View};

use crate::diff::index_column_name;
use crate::timescale::catalog::{
    AggregatePolicy, AggregateState, ColumnstoreSettings, ColumnstoreState, HypertableState,
};

/// Resolve every hypertable model in `schema` into column-name form, keyed by
/// table name.
#[must_use]
pub fn resolve_hypertables(schema: &Schema) -> BTreeMap<String, HypertableState> {
    schema
        .models
        .values()
        .filter_map(|model| {
            let config = model.timescale()?;
            Some((
                model.table_name().to_string(),
                resolve_model(model, &config),
            ))
        })
        .collect()
}

/// Resolve every continuous-aggregate view in `schema` into catalog form,
/// keyed by database view name (`@@map`).
///
/// `source` becomes the database name of the model (`Model::table_name`) or
/// view (`View::view_name`) it names, `sql` the view's verbatim SQL, and
/// `policy` the declared refresh policy.
#[must_use]
pub fn resolve_aggregates(schema: &Schema) -> BTreeMap<String, AggregateState> {
    schema
        .views
        .values()
        .filter_map(|view| {
            let config = view.continuous_aggregate()?;
            let name = view.view_name().to_string();
            Some((
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
            ))
        })
        .collect()
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
