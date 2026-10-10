//! TimescaleDB support for the migration engine.

mod aggregates;
pub mod catalog;
pub mod diff;
pub mod quote;
pub mod resolve;
pub mod sql;

pub use catalog::{
    AggregatePolicy, AggregateRow, AggregateState, CAGG_RECORD_MARKER, CatalogColumn, CatalogRows,
    ColumnRow, ColumnstoreSettings, ColumnstoreSettingsRow, ColumnstoreState, DimensionRow,
    ExtensionRow, HypertableRow, HypertableState, IntervalParts, JobRow, TimescaleCatalog, ViewRow,
    aggregate_record, interval_parts_sql, parse_aggregate_record, timescale_version_supported,
};
pub use diff::{TimescaleChange, TimescaleChangeKind, TimescaleDiff, diff_timescale};
pub use quote::{ident_list_literal, name_literal, quote_ident, quote_literal, regclass_literal};
pub use resolve::{resolve_aggregates, resolve_hypertables};
pub use sql::TimescaleSql;
