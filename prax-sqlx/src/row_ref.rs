//! Bridge between SqlxRow and prax_query::row::RowRef.
//!
//! Decodes each column to a string-keyed snapshot so the prax-query
//! `FromRow` pipeline works uniformly across SQLx's three backends
//! (Postgres, MySQL, SQLite). Strings are materialized eagerly so
//! `get_str` can hand back a borrowed slice.

use std::collections::HashMap;

use prax_query::row::{RowError, RowRef};
use sqlx::{Column, Row};

use crate::row::SqlxRow;

enum Value {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
}

/// A driver-agnostic decoded row produced by the sqlx engine. Holds owned
/// values keyed by column name so callers can access them after the row
/// itself has been dropped.
pub struct SqlxRowRef {
    values: HashMap<String, Value>,
}

impl SqlxRowRef {
    /// Decode a raw sqlx row into a driver-agnostic [`SqlxRowRef`].
    pub fn from_sqlx(row: &SqlxRow) -> Result<Self, RowError> {
        let mut values = HashMap::new();
        match row {
            #[cfg(feature = "postgres")]
            SqlxRow::Postgres(r) => {
                for (i, col) in r.columns().iter().enumerate() {
                    let name = col.name().to_string();
                    let v = decode_pg_cell(r, i);
                    values.insert(name, v);
                }
            }
            #[cfg(feature = "mysql")]
            SqlxRow::MySql(r) => {
                for (i, col) in r.columns().iter().enumerate() {
                    let name = col.name().to_string();
                    let v = decode_generic_cell_mysql(r, i);
                    values.insert(name, v);
                }
            }
            #[cfg(feature = "sqlite")]
            SqlxRow::Sqlite(r) => {
                for (i, col) in r.columns().iter().enumerate() {
                    let name = col.name().to_string();
                    let v = decode_generic_cell_sqlite(r, i);
                    values.insert(name, v);
                }
            }
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

/// Probe a Postgres cell in width order (text → bool → i64 → i32 → f64
/// → bytes), falling back to Null for everything we don't recognise.
#[cfg(feature = "postgres")]
fn decode_pg_cell(r: &sqlx::postgres::PgRow, i: usize) -> Value {
    if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
        return Value::Text(s);
    }
    if let Ok(Some(b)) = r.try_get::<Option<bool>, _>(i) {
        return Value::Bool(b);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
        return Value::I64(n);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i32>, _>(i) {
        return Value::I64(n as i64);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i16>, _>(i) {
        return Value::I64(n as i64);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
        return Value::F64(f);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f32>, _>(i) {
        return Value::F64(f as f64);
    }
    if let Ok(Some(b)) = r.try_get::<Option<Vec<u8>>, _>(i) {
        return Value::Bytes(b);
    }
    Value::Null
}

#[cfg(feature = "mysql")]
fn decode_generic_cell_mysql(r: &sqlx::mysql::MySqlRow, i: usize) -> Value {
    if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
        return Value::Text(s);
    }
    if let Ok(Some(b)) = r.try_get::<Option<bool>, _>(i) {
        return Value::Bool(b);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
        return Value::I64(n);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
        return Value::F64(f);
    }
    if let Ok(Some(b)) = r.try_get::<Option<Vec<u8>>, _>(i) {
        return Value::Bytes(b);
    }
    Value::Null
}

#[cfg(feature = "sqlite")]
fn decode_generic_cell_sqlite(r: &sqlx::sqlite::SqliteRow, i: usize) -> Value {
    if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
        return Value::Text(s);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
        return Value::I64(n);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
        return Value::F64(f);
    }
    if let Ok(Some(b)) = r.try_get::<Option<Vec<u8>>, _>(i) {
        return Value::Bytes(b);
    }
    Value::Null
}

impl RowRef for SqlxRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::I64(i) => i32::try_from(*i).map_err(|_| tc(c, "i64 overflow")),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::I64(i)) => i32::try_from(*i)
                .map(Some)
                .map_err(|_| tc(c, "i64 overflow")),
            Some(_) => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::I64(i) => Ok(*i),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not an integer")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::I64(i)) => Ok(Some(*i)),
            Some(_) => Err(tc(c, "not an integer")),
        }
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::F64(f) => Ok(*f),
            Value::I64(i) => Ok(*i as f64),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::F64(f)) => Ok(Some(*f)),
            Some(Value::I64(i)) => Ok(Some(*i as f64)),
            Some(_) => Err(tc(c, "not a number")),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Bool(b) => Ok(*b),
            Value::I64(i) => Ok(*i != 0),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a boolean")),
        }
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(Value::I64(i)) => Ok(Some(*i != 0)),
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
            Value::Bytes(b) => Ok(b.as_slice()),
            Value::Text(s) => Ok(s.as_bytes()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not bytes")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Bytes(b)) => Ok(Some(b.as_slice())),
            Some(Value::Text(s)) => Ok(Some(s.as_bytes())),
            Some(_) => Err(tc(c, "not bytes")),
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
}
