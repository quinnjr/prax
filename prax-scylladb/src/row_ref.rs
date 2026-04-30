//! Bridge between Scylla rows and `prax_query::row::RowRef`.
//!
//! Scylla's native row type is column-indexed (`Row { columns:
//! Vec<Option<CqlValue>> }`), but the prax-query `RowRef` contract is
//! column-name-keyed. Snapshot each row alongside its column-spec
//! metadata from the `QueryResult` so downstream `FromRow` impls can
//! look up values by name just like the other drivers.

use std::collections::HashMap;

use prax_query::row::{RowError, RowRef};
use scylla::frame::response::result::{CqlValue, Row};

pub struct ScyllaRowRef {
    values: HashMap<String, CqlValue>,
}

impl ScyllaRowRef {
    pub fn from_scylla(row: Row, column_names: &[String]) -> Result<Self, RowError> {
        let mut values = HashMap::with_capacity(column_names.len());
        for (i, name) in column_names.iter().enumerate() {
            if let Some(Some(v)) = row.columns.get(i) {
                values.insert(name.clone(), v.clone());
            }
            // NULL columns are dropped from the map; get_*_opt then
            // returns `Ok(None)` while get_* (required) returns
            // `Err(UnexpectedNull)` via the ColumnNotFound path.
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

fn missing(column: &str) -> RowError {
    RowError::ColumnNotFound(column.into())
}

impl RowRef for ScyllaRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::TinyInt(i) => Ok(i32::from(*i)),
            CqlValue::SmallInt(i) => Ok(i32::from(*i)),
            CqlValue::Int(i) => Ok(*i),
            CqlValue::BigInt(i) => i32::try_from(*i).map_err(|_| tc(c, "i64 overflow")),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_i32(c).map(Some)
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::TinyInt(i) => Ok(i64::from(*i)),
            CqlValue::SmallInt(i) => Ok(i64::from(*i)),
            CqlValue::Int(i) => Ok(i64::from(*i)),
            CqlValue::BigInt(i) => Ok(*i),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_i64(c).map(Some)
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::Float(f) => Ok(f64::from(*f)),
            CqlValue::Double(f) => Ok(*f),
            _ => Err(tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_f64(c).map(Some)
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::Boolean(b) => Ok(*b),
            _ => Err(tc(c, "not a boolean")),
        }
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_bool(c).map(Some)
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::Text(s) => Ok(s.as_str()),
            CqlValue::Ascii(s) => Ok(s.as_str()),
            _ => Err(tc(c, "not text")),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_str(c).map(Some)
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::Blob(b) => Ok(b.as_slice()),
            _ => Err(tc(c, "not blob")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_bytes(c).map(Some)
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::Uuid(u) => Ok(*u),
            // Timeuuid has its own wrapper type; extract the inner UUID.
            CqlValue::Timeuuid(t) => Ok(*t.as_ref()),
            _ => Err(tc(c, "not a uuid")),
        }
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_uuid(c).map(Some)
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        match self.values.get(c).ok_or_else(|| missing(c))? {
            CqlValue::Timestamp(ts) => chrono::DateTime::from_timestamp_millis(ts.0)
                .ok_or_else(|| tc(c, "timestamp out of range")),
            _ => Err(tc(c, "not a timestamp")),
        }
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        if !self.values.contains_key(c) {
            return Ok(None);
        }
        self.get_datetime_utc(c).map(Some)
    }
}
