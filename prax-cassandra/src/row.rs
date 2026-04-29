//! Row deserialization trait and helpers.

use crate::error::CassandraResult;

/// A CQL row as returned by the cdrs-tokio driver.
///
/// Holds a clone of the underlying cdrs-tokio Row so downstream code
/// can extract typed values by column name via the `ByName` /
/// `IntoRustByName` traits re-exported from `cdrs-tokio`.
#[derive(Debug, Clone)]
pub struct Row {
    pub(crate) inner: cdrs_tokio::types::rows::Row,
}

impl Row {
    /// Wrap an underlying cdrs-tokio row. Clones the row because the
    /// caller typically iterates over a Vec<Row> and we need owned
    /// storage.
    pub fn from_cdrs_row(row: &cdrs_tokio::types::rows::Row) -> CassandraResult<Self> {
        Ok(Self { inner: row.clone() })
    }

    /// Borrow the underlying cdrs-tokio row.
    pub fn as_cdrs(&self) -> &cdrs_tokio::types::rows::Row {
        &self.inner
    }

    /// Check whether a column is present on this row.
    pub fn contains_column(&self, name: &str) -> bool {
        self.inner.contains_column(name)
    }
}

/// Trait for types that can be deserialized from a CQL row.
pub trait FromRow: Sized {
    /// Deserialize a row into this type.
    fn from_row(row: &Row) -> CassandraResult<Self>;
}
