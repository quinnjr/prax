//! Row deserialization for ScyllaDB results.

use scylla::frame::response::result::{CqlValue, Row};
use serde::de::DeserializeOwned;

use crate::error::{ScyllaError, ScyllaResult};
use crate::types::ScyllaValue;

/// Trait for types that can be constructed from a ScyllaDB row.
pub trait FromScyllaRow: Sized {
    /// Construct an instance from a row.
    fn from_row(row: &Row) -> ScyllaResult<Self>;
}

/// A helper for extracting values from a row by index.
pub struct RowAccessor<'a> {
    row: &'a Row,
}

impl<'a> RowAccessor<'a> {
    /// Create a new accessor for a row.
    #[must_use]
    pub fn new(row: &'a Row) -> Self {
        Self { row }
    }

    /// Get a value by column index.
    pub fn get<T: FromCqlValue>(&self, index: usize) -> ScyllaResult<T> {
        self.row
            .columns
            .get(index)
            .ok_or_else(|| {
                ScyllaError::deserialization(format!("Column index {} out of bounds", index))
            })?
            .as_ref()
            .map(|v| T::from_cql(v))
            .transpose()?
            .ok_or_else(|| ScyllaError::deserialization(format!("Column {} is null", index)))
    }

    /// Get an optional value by column index.
    pub fn get_opt<T: FromCqlValue>(&self, index: usize) -> ScyllaResult<Option<T>> {
        match self.row.columns.get(index) {
            Some(Some(value)) => Ok(Some(T::from_cql(value)?)),
            Some(None) | None => Ok(None),
        }
    }

    /// Get the number of columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.row.columns.len()
    }

    /// Check if the row has no columns.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.row.columns.is_empty()
    }
}

/// Trait for types that can be extracted from a CQL value.
pub trait FromCqlValue: Sized {
    /// Extract a value from a CQL value.
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self>;
}

impl FromCqlValue for bool {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Boolean(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion("Expected boolean")),
        }
    }
}

impl FromCqlValue for i8 {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::TinyInt(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion("Expected tinyint")),
        }
    }
}

impl FromCqlValue for i16 {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::SmallInt(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion("Expected smallint")),
        }
    }
}

impl FromCqlValue for i32 {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Int(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion("Expected int")),
        }
    }
}

impl FromCqlValue for i64 {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::BigInt(v) => Ok(*v),
            CqlValue::Counter(v) => Ok(v.0),
            _ => Err(ScyllaError::type_conversion("Expected bigint")),
        }
    }
}

impl FromCqlValue for f32 {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Float(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion("Expected float")),
        }
    }
}

impl FromCqlValue for f64 {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Double(v) => Ok(*v),
            CqlValue::Float(v) => Ok(f64::from(*v)),
            _ => Err(ScyllaError::type_conversion("Expected double")),
        }
    }
}

impl FromCqlValue for String {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Text(v) | CqlValue::Ascii(v) => Ok(v.clone()),
            _ => Err(ScyllaError::type_conversion("Expected text")),
        }
    }
}

impl FromCqlValue for Vec<u8> {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Blob(v) => Ok(v.clone()),
            _ => Err(ScyllaError::type_conversion("Expected blob")),
        }
    }
}

impl FromCqlValue for uuid::Uuid {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Uuid(v) => Ok(*v),
            CqlValue::Timeuuid(v) => Ok((*v).into()),
            _ => Err(ScyllaError::type_conversion("Expected uuid")),
        }
    }
}

impl FromCqlValue for chrono::DateTime<chrono::Utc> {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Timestamp(ts) => chrono::DateTime::from_timestamp_millis(ts.0)
                .ok_or_else(|| ScyllaError::type_conversion("Invalid timestamp")),
            _ => Err(ScyllaError::type_conversion("Expected timestamp")),
        }
    }
}

impl FromCqlValue for chrono::NaiveDate {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Date(d) => chrono::NaiveDate::from_num_days_from_ce_opt(d.0 as i32)
                .ok_or_else(|| ScyllaError::type_conversion("Invalid date")),
            _ => Err(ScyllaError::type_conversion("Expected date")),
        }
    }
}

impl FromCqlValue for std::net::IpAddr {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Inet(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion("Expected inet")),
        }
    }
}

impl FromCqlValue for ScyllaValue {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        Ok(value.clone().into())
    }
}

impl<T: FromCqlValue> FromCqlValue for Option<T> {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::Empty => Ok(None),
            _ => Ok(Some(T::from_cql(value)?)),
        }
    }
}

impl<T: FromCqlValue> FromCqlValue for Vec<T> {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        match value {
            CqlValue::List(items) | CqlValue::Set(items) => {
                items.iter().map(|v| T::from_cql(v)).collect()
            }
            _ => Err(ScyllaError::type_conversion("Expected list or set")),
        }
    }
}

impl FromCqlValue for serde_json::Value {
    fn from_cql(value: &CqlValue) -> ScyllaResult<Self> {
        let scylla_value: ScyllaValue = value.clone().into();
        Ok(scylla_value.into())
    }
}

/// Implement `FromScyllaRow` for types that implement `DeserializeOwned`.
///
/// This requires converting the row to JSON first, which may not be efficient
/// for all use cases.
impl<T: DeserializeOwned> FromScyllaRow for T {
    fn from_row(row: &Row) -> ScyllaResult<Self> {
        // Convert row to JSON for serde deserialization
        // This is a simplified approach - a production implementation
        // would use column names from metadata
        let values: Vec<serde_json::Value> = row
            .columns
            .iter()
            .map(|col| {
                col.as_ref()
                    .map(|v| {
                        let sv: ScyllaValue = v.clone().into();
                        sv.into()
                    })
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();

        // Deserialize as array (for tuple structs) or fail
        serde_json::from_value(serde_json::Value::Array(values))
            .map_err(|e| ScyllaError::deserialization(e.to_string()))
    }
}

/// A macro to implement `FromScyllaRow` for a struct with named fields.
///
/// Usage:
/// ```ignore
/// impl_from_row!(User {
///     id: uuid::Uuid,
///     email: String,
///     name: Option<String>,
///     created_at: chrono::DateTime<chrono::Utc>,
/// });
/// ```
#[macro_export]
macro_rules! impl_from_row {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        impl $crate::row::FromScyllaRow for $name {
            fn from_row(row: &scylla::frame::response::result::Row) -> $crate::error::ScyllaResult<Self> {
                let accessor = $crate::row::RowAccessor::new(row);
                let mut idx = 0;
                Ok(Self {
                    $(
                        $field: {
                            let val = accessor.get::<$ty>(idx)?;
                            idx += 1;
                            val
                        },
                    )*
                })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_cql_primitives() {
        assert_eq!(bool::from_cql(&CqlValue::Boolean(true)).unwrap(), true);
        assert_eq!(i32::from_cql(&CqlValue::Int(42)).unwrap(), 42);
        assert_eq!(i64::from_cql(&CqlValue::BigInt(100)).unwrap(), 100);
        assert!((f64::from_cql(&CqlValue::Double(3.14)).unwrap() - 3.14).abs() < f64::EPSILON);
        assert_eq!(
            String::from_cql(&CqlValue::Text("hello".into())).unwrap(),
            "hello"
        );
    }

    #[test]
    fn test_from_cql_optional() {
        let result: Option<i32> = Option::<i32>::from_cql(&CqlValue::Int(42)).unwrap();
        assert_eq!(result, Some(42));

        let result: Option<i32> = Option::<i32>::from_cql(&CqlValue::Empty).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_from_cql_list() {
        let list = CqlValue::List(vec![CqlValue::Int(1), CqlValue::Int(2), CqlValue::Int(3)]);
        let result: Vec<i32> = Vec::<i32>::from_cql(&list).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }
}
