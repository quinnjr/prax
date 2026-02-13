//! Zero-copy row deserialization traits and utilities.
//!
//! This module provides traits for efficient row deserialization that minimizes
//! memory allocations by borrowing data directly from the database row.
//!
//! # Zero-Copy Deserialization
//!
//! The `FromRowRef` trait enables zero-copy deserialization for types that can
//! borrow string data directly from the row:
//!
//! ```rust,ignore
//! use prax_query::row::{FromRowRef, RowRef};
//!
//! struct UserRef<'a> {
//!     id: i32,
//!     email: &'a str,  // Borrowed from row
//!     name: Option<&'a str>,
//! }
//!
//! impl<'a> FromRowRef<'a> for UserRef<'a> {
//!     fn from_row_ref(row: &'a impl RowRef) -> Result<Self, RowError> {
//!         Ok(Self {
//!             id: row.get("id")?,
//!             email: row.get_str("email")?,
//!             name: row.get_str_opt("name")?,
//!         })
//!     }
//! }
//! ```
//!
//! # Performance
//!
//! Zero-copy deserialization can significantly reduce allocations:
//! - String fields borrow directly from row buffer (no allocation)
//! - Integer/float fields are copied (no difference)
//! - Optional fields return `Option<&'a str>` instead of `Option<String>`

use std::borrow::Cow;
use std::fmt;

/// Error type for row deserialization.
#[derive(Debug, Clone)]
pub enum RowError {
    /// Column not found.
    ColumnNotFound(String),
    /// Type conversion error.
    TypeConversion { column: String, message: String },
    /// Null value in non-nullable column.
    UnexpectedNull(String),
}

impl fmt::Display for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ColumnNotFound(col) => write!(f, "column '{}' not found", col),
            Self::TypeConversion { column, message } => {
                write!(f, "type conversion error for '{}': {}", column, message)
            }
            Self::UnexpectedNull(col) => write!(f, "unexpected null in column '{}'", col),
        }
    }
}

impl std::error::Error for RowError {}

/// A database row that supports zero-copy access.
///
/// This trait is implemented by database-specific row types to enable
/// efficient data extraction without unnecessary copying.
pub trait RowRef {
    /// Get an integer column value.
    fn get_i32(&self, column: &str) -> Result<i32, RowError>;

    /// Get an optional integer column value.
    fn get_i32_opt(&self, column: &str) -> Result<Option<i32>, RowError>;

    /// Get a 64-bit integer column value.
    fn get_i64(&self, column: &str) -> Result<i64, RowError>;

    /// Get an optional 64-bit integer column value.
    fn get_i64_opt(&self, column: &str) -> Result<Option<i64>, RowError>;

    /// Get a float column value.
    fn get_f64(&self, column: &str) -> Result<f64, RowError>;

    /// Get an optional float column value.
    fn get_f64_opt(&self, column: &str) -> Result<Option<f64>, RowError>;

    /// Get a boolean column value.
    fn get_bool(&self, column: &str) -> Result<bool, RowError>;

    /// Get an optional boolean column value.
    fn get_bool_opt(&self, column: &str) -> Result<Option<bool>, RowError>;

    /// Get a string column value as a borrowed reference (zero-copy).
    ///
    /// This is the key method for zero-copy deserialization. The returned
    /// string slice borrows directly from the row's internal buffer.
    fn get_str(&self, column: &str) -> Result<&str, RowError>;

    /// Get an optional string column value as a borrowed reference.
    fn get_str_opt(&self, column: &str) -> Result<Option<&str>, RowError>;

    /// Get a string column value as owned (for cases where ownership is needed).
    fn get_string(&self, column: &str) -> Result<String, RowError> {
        self.get_str(column).map(|s| s.to_string())
    }

    /// Get an optional string as owned.
    fn get_string_opt(&self, column: &str) -> Result<Option<String>, RowError> {
        self.get_str_opt(column)
            .map(|opt| opt.map(|s| s.to_string()))
    }

    /// Get a bytes column value as a borrowed reference (zero-copy).
    fn get_bytes(&self, column: &str) -> Result<&[u8], RowError>;

    /// Get optional bytes as borrowed reference.
    fn get_bytes_opt(&self, column: &str) -> Result<Option<&[u8]>, RowError>;

    /// Get column value as a Cow, borrowing when possible.
    fn get_cow_str(&self, column: &str) -> Result<Cow<'_, str>, RowError> {
        self.get_str(column).map(Cow::Borrowed)
    }
}

/// Trait for types that can be deserialized from a row reference (zero-copy).
///
/// This trait uses lifetimes to enable borrowing string data directly
/// from the row, avoiding allocations.
pub trait FromRowRef<'a>: Sized {
    /// Deserialize from a row reference.
    fn from_row_ref(row: &'a impl RowRef) -> Result<Self, RowError>;
}

/// Trait for types that can be deserialized from a row (owning).
///
/// This is the traditional deserialization trait that takes ownership
/// of all data.
pub trait FromRow: Sized {
    /// Deserialize from a row.
    fn from_row(row: &impl RowRef) -> Result<Self, RowError>;
}

// Blanket implementation: any FromRow can be used with any row
impl<T: FromRow> FromRowRef<'_> for T {
    fn from_row_ref(row: &impl RowRef) -> Result<Self, RowError> {
        T::from_row(row)
    }
}

/// A row iterator that yields zero-copy deserialized values.
pub struct RowRefIter<'a, R: RowRef, T: FromRowRef<'a>> {
    rows: std::slice::Iter<'a, R>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, R: RowRef, T: FromRowRef<'a>> RowRefIter<'a, R, T> {
    /// Create a new row iterator.
    pub fn new(rows: &'a [R]) -> Self {
        Self {
            rows: rows.iter(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a, R: RowRef, T: FromRowRef<'a>> Iterator for RowRefIter<'a, R, T> {
    type Item = Result<T, RowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(|row| T::from_row_ref(row))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}

impl<'a, R: RowRef, T: FromRowRef<'a>> ExactSizeIterator for RowRefIter<'a, R, T> {}

/// A collected result that can either borrow or own data.
///
/// This is useful for caching query results while still supporting
/// zero-copy deserialization for fresh queries.
#[derive(Debug, Clone)]
pub enum RowData<'a> {
    /// Borrowed string data.
    Borrowed(&'a str),
    /// Owned string data.
    Owned(String),
}

impl<'a> RowData<'a> {
    /// Get the string value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(s) => s,
            Self::Owned(s) => s,
        }
    }

    /// Convert to owned data.
    pub fn into_owned(self) -> String {
        match self {
            Self::Borrowed(s) => s.to_string(),
            Self::Owned(s) => s,
        }
    }

    /// Create borrowed data.
    pub const fn borrowed(s: &'a str) -> Self {
        Self::Borrowed(s)
    }

    /// Create owned data.
    pub fn owned(s: impl Into<String>) -> Self {
        Self::Owned(s.into())
    }
}

impl<'a> From<&'a str> for RowData<'a> {
    fn from(s: &'a str) -> Self {
        Self::Borrowed(s)
    }
}

impl From<String> for RowData<'static> {
    fn from(s: String) -> Self {
        Self::Owned(s)
    }
}

impl<'a> AsRef<str> for RowData<'a> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Macro to implement FromRow for simple structs.
///
/// This generates efficient deserialization code that minimizes allocations.
///
/// # Example
///
/// ```rust,ignore
/// use prax_query::impl_from_row;
///
/// struct User {
///     id: i32,
///     email: String,
///     name: Option<String>,
/// }
///
/// impl_from_row!(User {
///     id: i32,
///     email: String,
///     name: Option<String>,
/// });
/// ```
#[macro_export]
macro_rules! impl_from_row {
    ($type:ident { $($field:ident : i32),* $(,)? }) => {
        impl $crate::row::FromRow for $type {
            fn from_row(row: &impl $crate::row::RowRef) -> Result<Self, $crate::row::RowError> {
                Ok(Self {
                    $(
                        $field: row.get_i32(stringify!($field))?,
                    )*
                })
            }
        }
    };
    ($type:ident { $($field:ident : $field_type:ty),* $(,)? }) => {
        impl $crate::row::FromRow for $type {
            fn from_row(row: &impl $crate::row::RowRef) -> Result<Self, $crate::row::RowError> {
                Ok(Self {
                    $(
                        $field: $crate::row::_get_typed_value::<$field_type>(row, stringify!($field))?,
                    )*
                })
            }
        }
    };
}

/// Helper function for the impl_from_row macro.
#[doc(hidden)]
pub fn _get_typed_value<T: FromColumn>(row: &impl RowRef, column: &str) -> Result<T, RowError> {
    T::from_column(row, column)
}

/// Trait for types that can be extracted from a column.
pub trait FromColumn: Sized {
    /// Extract value from a row column.
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError>;
}

impl FromColumn for i32 {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_i32(column)
    }
}

impl FromColumn for i64 {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_i64(column)
    }
}

impl FromColumn for f64 {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_f64(column)
    }
}

impl FromColumn for bool {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_bool(column)
    }
}

impl FromColumn for String {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_string(column)
    }
}

impl FromColumn for Option<i32> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_i32_opt(column)
    }
}

impl FromColumn for Option<i64> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_i64_opt(column)
    }
}

impl FromColumn for Option<f64> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_f64_opt(column)
    }
}

impl FromColumn for Option<bool> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_bool_opt(column)
    }
}

impl FromColumn for Option<String> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_string_opt(column)
    }
}

impl FromColumn for Vec<u8> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_bytes(column).map(|b| b.to_vec())
    }
}

impl FromColumn for Option<Vec<u8>> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> {
        row.get_bytes_opt(column).map(|opt| opt.map(|b| b.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock row for testing
    struct MockRow {
        data: std::collections::HashMap<String, String>,
    }

    impl RowRef for MockRow {
        fn get_i32(&self, column: &str) -> Result<i32, RowError> {
            self.data
                .get(column)
                .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))?
                .parse()
                .map_err(|e| RowError::TypeConversion {
                    column: column.to_string(),
                    message: format!("{}", e),
                })
        }

        fn get_i32_opt(&self, column: &str) -> Result<Option<i32>, RowError> {
            match self.data.get(column) {
                Some(v) if v == "NULL" => Ok(None),
                Some(v) => v.parse().map(Some).map_err(|e| RowError::TypeConversion {
                    column: column.to_string(),
                    message: format!("{}", e),
                }),
                None => Ok(None),
            }
        }

        fn get_i64(&self, column: &str) -> Result<i64, RowError> {
            self.data
                .get(column)
                .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))?
                .parse()
                .map_err(|e| RowError::TypeConversion {
                    column: column.to_string(),
                    message: format!("{}", e),
                })
        }

        fn get_i64_opt(&self, column: &str) -> Result<Option<i64>, RowError> {
            match self.data.get(column) {
                Some(v) if v == "NULL" => Ok(None),
                Some(v) => v.parse().map(Some).map_err(|e| RowError::TypeConversion {
                    column: column.to_string(),
                    message: format!("{}", e),
                }),
                None => Ok(None),
            }
        }

        fn get_f64(&self, column: &str) -> Result<f64, RowError> {
            self.data
                .get(column)
                .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))?
                .parse()
                .map_err(|e| RowError::TypeConversion {
                    column: column.to_string(),
                    message: format!("{}", e),
                })
        }

        fn get_f64_opt(&self, column: &str) -> Result<Option<f64>, RowError> {
            match self.data.get(column) {
                Some(v) if v == "NULL" => Ok(None),
                Some(v) => v.parse().map(Some).map_err(|e| RowError::TypeConversion {
                    column: column.to_string(),
                    message: format!("{}", e),
                }),
                None => Ok(None),
            }
        }

        fn get_bool(&self, column: &str) -> Result<bool, RowError> {
            let v = self
                .data
                .get(column)
                .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))?;
            match v.as_str() {
                "true" | "t" | "1" => Ok(true),
                "false" | "f" | "0" => Ok(false),
                _ => Err(RowError::TypeConversion {
                    column: column.to_string(),
                    message: "invalid boolean".to_string(),
                }),
            }
        }

        fn get_bool_opt(&self, column: &str) -> Result<Option<bool>, RowError> {
            match self.data.get(column) {
                Some(v) if v == "NULL" => Ok(None),
                Some(v) => match v.as_str() {
                    "true" | "t" | "1" => Ok(Some(true)),
                    "false" | "f" | "0" => Ok(Some(false)),
                    _ => Err(RowError::TypeConversion {
                        column: column.to_string(),
                        message: "invalid boolean".to_string(),
                    }),
                },
                None => Ok(None),
            }
        }

        fn get_str(&self, column: &str) -> Result<&str, RowError> {
            self.data
                .get(column)
                .map(|s| s.as_str())
                .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))
        }

        fn get_str_opt(&self, column: &str) -> Result<Option<&str>, RowError> {
            match self.data.get(column) {
                Some(v) if v == "NULL" => Ok(None),
                Some(v) => Ok(Some(v.as_str())),
                None => Ok(None),
            }
        }

        fn get_bytes(&self, column: &str) -> Result<&[u8], RowError> {
            self.data
                .get(column)
                .map(|s| s.as_bytes())
                .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))
        }

        fn get_bytes_opt(&self, column: &str) -> Result<Option<&[u8]>, RowError> {
            match self.data.get(column) {
                Some(v) if v == "NULL" => Ok(None),
                Some(v) => Ok(Some(v.as_bytes())),
                None => Ok(None),
            }
        }
    }

    #[test]
    fn test_row_ref_get_i32() {
        let mut data = std::collections::HashMap::new();
        data.insert("id".to_string(), "42".to_string());
        let row = MockRow { data };

        assert_eq!(row.get_i32("id").unwrap(), 42);
    }

    #[test]
    fn test_row_ref_get_str_zero_copy() {
        let mut data = std::collections::HashMap::new();
        data.insert("email".to_string(), "test@example.com".to_string());
        let row = MockRow { data };

        let email = row.get_str("email").unwrap();
        assert_eq!(email, "test@example.com");
        // Note: In a real implementation, this would be zero-copy
        // borrowing directly from the row's buffer
    }

    #[test]
    fn test_row_data() {
        let borrowed: RowData = RowData::borrowed("hello");
        assert_eq!(borrowed.as_str(), "hello");

        let owned: RowData = RowData::owned("world".to_string());
        assert_eq!(owned.as_str(), "world");
    }
}
