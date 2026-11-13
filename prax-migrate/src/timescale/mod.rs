//! TimescaleDB support for the migration engine.

mod aggregates;
pub mod catalog;
pub mod diff;
pub(crate) mod quote;
pub(crate) mod resolve;
pub(crate) mod sql;

pub use catalog::{
    AggregatePolicy, AggregateRow, AggregateState, CAGG_RECORD_MARKER, CatalogColumn, CatalogRows,
    ColumnRow, ColumnstoreSettings, ColumnstoreSettingsRow, ColumnstoreState, DimensionRow,
    ExtensionRow, HypertableRow, HypertableState, IntervalParts, JobRow, TIMESCALE_MIN_VERSION,
    TimescaleCatalog, ViewRow, aggregate_record, interval_parts_sql, parse_aggregate_record,
    timescale_version_at_least, timescale_version_supported,
};
#[doc(hidden)]
pub use diff::SET_NOT_NULL_ON_COLUMNSTORE_SINCE;
pub use diff::{
    AggregateSetChange, TimescaleChange, TimescaleChangeKind, TimescaleDiff, diff_timescale,
    opts_into_timescale,
};
pub use quote::{comment_safe, ident_list_literal, quote_ident, quote_literal, regclass_literal};
pub use resolve::{resolve_aggregates, resolve_hypertables};

/// Parse `prax` into a schema, panicking on a parse error.
#[cfg(test)]
pub(crate) fn schema(prax: &str) -> prax_schema::Schema {
    prax_schema::parse_schema(prax).unwrap()
}
