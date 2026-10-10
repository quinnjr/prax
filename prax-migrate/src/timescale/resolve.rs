//! Resolve a target schema's Timescale configs into catalog form.
//!
//! Configs hold Prax field names; the catalog holds table and column names.
//! Resolution maps each field through its model's `@map` and keys each
//! hypertable by `Model::table_name()`, so comparisons never churn on `@map`.

use std::collections::BTreeMap;

use prax_schema::Schema;
use prax_schema::ast::{Model, TimescaleModelConfig};

use crate::diff::index_column_name;
use crate::timescale::catalog::{ColumnstoreSettings, ColumnstoreState, HypertableState};

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
