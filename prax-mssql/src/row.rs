//! Microsoft SQL Server row types and deserialization.

use tiberius::Row;

use crate::error::{MssqlError, MssqlResult};

/// Extension trait for SQL Server rows.
pub trait MssqlRow {
    /// Get a column value by name.
    fn get_value<'a, T>(&'a self, column: &str) -> MssqlResult<T>
    where
        T: tiberius::FromSql<'a>;

    /// Get an optional column value by name.
    fn get_opt<'a, T>(&'a self, column: &str) -> MssqlResult<Option<T>>
    where
        T: tiberius::FromSql<'a>;

    /// Try to get a column value by index.
    fn try_get_by_index<'a, T>(&'a self, index: usize) -> Option<T>
    where
        T: tiberius::FromSql<'a>;
}

impl MssqlRow for Row {
    fn get_value<'a, T>(&'a self, column: &str) -> MssqlResult<T>
    where
        T: tiberius::FromSql<'a>,
    {
        self.try_get(column)
            .map_err(|e| {
                MssqlError::deserialization(format!("failed to get column '{}': {}", column, e))
            })?
            .ok_or_else(|| MssqlError::deserialization(format!("column '{}' is null", column)))
    }

    fn get_opt<'a, T>(&'a self, column: &str) -> MssqlResult<Option<T>>
    where
        T: tiberius::FromSql<'a>,
    {
        self.try_get(column).map_err(|e| {
            MssqlError::deserialization(format!("failed to get column '{}': {}", column, e))
        })
    }

    fn try_get_by_index<'a, T>(&'a self, index: usize) -> Option<T>
    where
        T: tiberius::FromSql<'a>,
    {
        self.get(index)
    }
}

/// Trait for deserializing a SQL Server row into a type.
pub trait FromMssqlRow: Sized {
    /// Deserialize from a SQL Server row.
    fn from_row(row: &Row) -> MssqlResult<Self>;
}

/// Macro to implement FromMssqlRow for simple structs.
///
/// Usage:
/// ```rust,ignore
/// impl_from_row!(User {
///     id: i32,
///     email: String,
///     name: Option<String>,
/// });
/// ```
#[macro_export]
macro_rules! impl_from_mssql_row {
    ($type:ident { $($field:ident : $field_type:ty),* $(,)? }) => {
        impl $crate::row::FromMssqlRow for $type {
            fn from_row(row: &tiberius::Row) -> $crate::error::MssqlResult<Self> {
                use $crate::row::MssqlRow;
                Ok(Self {
                    $(
                        $field: row.get_value(stringify!($field))?,
                    )*
                })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    // Row tests require integration testing with a real SQL Server database
}
