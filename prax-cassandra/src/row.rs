//! Row deserialization trait and helpers.

use crate::error::CassandraResult;

/// A CQL row as returned by the cdrs-tokio driver.
///
/// Holds a clone of the underlying cdrs-tokio Row so downstream code
/// can extract typed values by column name via [`ByName`] /
/// [`IntoRustByName`] (re-exported from `cdrs-tokio`).
#[derive(Debug, Clone)]
pub struct Row {
    pub(crate) inner: Option<cdrs_tokio::types::rows::Row>,
}

impl Default for Row {
    fn default() -> Self {
        Self { inner: None }
    }
}

impl Row {
    /// Create an empty row (used in tests and fixtures).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Wrap an underlying cdrs-tokio row. Clones the row because the
    /// caller typically iterates over a Vec<Row> and we need owned
    /// storage.
    pub fn from_cdrs_row(row: &cdrs_tokio::types::rows::Row) -> CassandraResult<Self> {
        Ok(Self {
            inner: Some(row.clone()),
        })
    }

    /// Borrow the underlying cdrs-tokio row, if present.
    pub fn as_cdrs(&self) -> Option<&cdrs_tokio::types::rows::Row> {
        self.inner.as_ref()
    }

    /// Check whether a column is present on this row.
    pub fn contains_column(&self, name: &str) -> bool {
        self.inner
            .as_ref()
            .map(|r| r.contains_column(name))
            .unwrap_or(false)
    }
}

/// Trait for types that can be deserialized from a CQL row.
pub trait FromRow: Sized {
    /// Deserialize a row into this type.
    fn from_row(row: &Row) -> CassandraResult<Self>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_row() {
        let row = Row::empty();
        assert!(row.as_cdrs().is_none());
        assert!(!row.contains_column("anything"));
    }
}
