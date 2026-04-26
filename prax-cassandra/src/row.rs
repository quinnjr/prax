//! Row deserialization trait and helpers.

use crate::error::CassandraResult;

/// A CQL row as returned by the cdrs-tokio driver.
///
/// This is a thin newtype so prax-cassandra can evolve its row
/// representation independently of the underlying driver.
#[derive(Debug, Default, Clone)]
pub struct Row {
    /// Column name → raw CQL-encoded bytes.
    pub(crate) columns: Vec<(String, Vec<u8>)>,
}

impl Row {
    /// Create an empty row (used in tests and fixtures).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Return the raw bytes for a named column, if present.
    pub fn column_bytes(&self, name: &str) -> Option<&[u8]> {
        self.columns
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_slice())
    }

    /// Number of columns in this row.
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    /// Returns true if this row has no columns.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
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
        assert!(row.is_empty());
        assert_eq!(row.len(), 0);
    }

    #[test]
    fn test_column_bytes_lookup() {
        let row = Row {
            columns: vec![
                ("id".into(), vec![1, 2, 3]),
                ("name".into(), b"alice".to_vec()),
            ],
        };
        assert_eq!(row.column_bytes("id"), Some(&[1u8, 2, 3][..]));
        assert_eq!(row.column_bytes("name"), Some(&b"alice"[..]));
        assert!(row.column_bytes("missing").is_none());
    }
}
