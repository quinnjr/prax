//! TimescaleDB support for the migration engine.

pub mod catalog;

pub use catalog::{
    AggregatePolicy, AggregateRow, AggregateState, CAGG_RECORD_MARKER, CatalogColumn, CatalogRows,
    ColumnRow, ColumnstoreSettings, ColumnstoreSettingsRow, ColumnstoreState, DimensionRow,
    ExtensionRow, HypertableRow, HypertableState, IntervalParts, JobRow, TimescaleCatalog, ViewRow,
    aggregate_record, interval_parts_sql, parse_aggregate_record, timescale_version_supported,
};
