//! Bridge between duckdb rows and prax_query::row::RowRef.
//!
//! Snapshots every column value out of a `duckdb::Row` into an owned
//! `duckdb::types::Value` so callers can read fields by name without
//! holding a borrow on the underlying statement.

use std::collections::HashMap;

use duckdb::Row;
use duckdb::types::{Value, ValueRef};
use prax_query::row::{RowError, RowRef};

pub struct DuckDbRowRef {
    values: HashMap<String, Value>,
}

impl DuckDbRowRef {
    pub fn from_duckdb(row: &Row<'_>, column_names: &[String]) -> Result<Self, RowError> {
        let mut values = HashMap::with_capacity(column_names.len());
        for (i, name) in column_names.iter().enumerate() {
            let v: Value = match row.get_ref(i).map_err(|e| tc(name, e.to_string()))? {
                ValueRef::Null => Value::Null,
                ValueRef::Boolean(b) => Value::Boolean(b),
                ValueRef::TinyInt(i) => Value::TinyInt(i),
                ValueRef::SmallInt(i) => Value::SmallInt(i),
                ValueRef::Int(i) => Value::Int(i),
                ValueRef::BigInt(i) => Value::BigInt(i),
                ValueRef::UTinyInt(i) => Value::UTinyInt(i),
                ValueRef::USmallInt(i) => Value::USmallInt(i),
                ValueRef::UInt(i) => Value::UInt(i),
                ValueRef::UBigInt(i) => Value::UBigInt(i),
                ValueRef::Float(f) => Value::Float(f),
                ValueRef::Double(f) => Value::Double(f),
                ValueRef::Text(bytes) => Value::Text(String::from_utf8_lossy(bytes).into_owned()),
                ValueRef::Blob(bytes) => Value::Blob(bytes.to_vec()),
                other => other.to_owned(),
            };
            values.insert(name.clone(), v);
        }
        Ok(Self { values })
    }
}

fn tc(column: &str, msg: impl Into<String>) -> RowError {
    RowError::TypeConversion {
        column: column.into(),
        message: msg.into(),
    }
}

/// Read an integer cell, coercing across DuckDB's width-specific variants.
/// Returns `UnexpectedNull` for NULL, `ColumnNotFound` for an absent column.
fn as_i64(v: Option<&Value>, column: &str) -> Result<i64, RowError> {
    match v.ok_or_else(|| RowError::ColumnNotFound(column.into()))? {
        Value::TinyInt(i) => Ok(*i as i64),
        Value::SmallInt(i) => Ok(*i as i64),
        Value::Int(i) => Ok(*i as i64),
        Value::BigInt(i) => Ok(*i),
        Value::UTinyInt(i) => Ok(*i as i64),
        Value::USmallInt(i) => Ok(*i as i64),
        Value::UInt(i) => Ok(*i as i64),
        Value::UBigInt(i) => i64::try_from(*i).map_err(|_| tc(column, "u64 exceeds i64::MAX")),
        Value::Null => Err(RowError::UnexpectedNull(column.into())),
        _ => Err(tc(column, "not an integer")),
    }
}

fn as_i64_opt(v: Option<&Value>, column: &str) -> Result<Option<i64>, RowError> {
    match v {
        None => Err(RowError::ColumnNotFound(column.into())),
        Some(Value::Null) => Ok(None),
        Some(other) => as_i64(Some(other), column).map(Some),
    }
}

impl RowRef for DuckDbRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        let i = as_i64(self.values.get(c), c)?;
        i32::try_from(i).map_err(|_| tc(c, "i64 overflow"))
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match as_i64_opt(self.values.get(c), c)? {
            None => Ok(None),
            Some(i) => i32::try_from(i).map(Some).map_err(|_| tc(c, "overflow")),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        as_i64(self.values.get(c), c)
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        as_i64_opt(self.values.get(c), c)
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Double(f) => Ok(*f),
            Value::Float(f) => Ok(*f as f64),
            Value::TinyInt(i) => Ok(*i as f64),
            Value::SmallInt(i) => Ok(*i as f64),
            Value::Int(i) => Ok(*i as f64),
            Value::BigInt(i) => Ok(*i as f64),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(_) => self.get_f64(c).map(Some),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Boolean(b) => Ok(*b),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a boolean")),
        }
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Boolean(b)) => Ok(Some(*b)),
            Some(_) => Err(tc(c, "not a boolean")),
        }
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Text(s) => Ok(s.as_str()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not text")),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Text(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(tc(c, "not text")),
        }
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Blob(b) => Ok(b.as_slice()),
            Value::Text(s) => Ok(s.as_bytes()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not blob")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Blob(b)) => Ok(Some(b.as_slice())),
            Some(Value::Text(s)) => Ok(Some(s.as_bytes())),
            Some(_) => Err(tc(c, "not blob")),
        }
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        let s = self.get_str(c)?;
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .map_err(|e| tc(c, e.to_string()))
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| Some(d.with_timezone(&chrono::Utc)))
                .map_err(|e| tc(c, e.to_string())),
        }
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        uuid::Uuid::parse_str(self.get_str(c)?).map_err(|e| tc(c, e.to_string()))
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => uuid::Uuid::parse_str(s)
                .map(Some)
                .map_err(|e| tc(c, e.to_string())),
        }
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        serde_json::from_str(self.get_str(c)?).map_err(|e| tc(c, e.to_string()))
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => serde_json::from_str(s)
                .map(Some)
                .map_err(|e| tc(c, e.to_string())),
        }
    }
    fn get_decimal(&self, c: &str) -> Result<rust_decimal::Decimal, RowError> {
        self.get_str(c)?
            .parse::<rust_decimal::Decimal>()
            .map_err(|e| tc(c, e.to_string()))
    }
    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => s
                .parse::<rust_decimal::Decimal>()
                .map(Some)
                .map_err(|e| tc(c, e.to_string())),
        }
    }
}
