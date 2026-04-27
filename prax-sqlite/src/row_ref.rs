//! Bridge between rusqlite rows and prax_query::row::RowRef.

use std::collections::HashMap;

use prax_query::row::{RowError, RowRef};
use rusqlite::Row;
use rusqlite::types::{Value, ValueRef};

pub struct SqliteRowRef {
    values: HashMap<String, Value>,
}

impl SqliteRowRef {
    pub fn from_rusqlite(row: &Row<'_>) -> Result<Self, RowError> {
        let stmt = row.as_ref();
        let mut values = HashMap::with_capacity(stmt.column_count());
        for i in 0..stmt.column_count() {
            let name = stmt
                .column_name(i)
                .map_err(|e| RowError::TypeConversion {
                    column: i.to_string(),
                    message: e.to_string(),
                })?
                .to_string();
            let v: Value = match row.get_ref(i).map_err(|e| RowError::TypeConversion {
                column: name.clone(),
                message: e.to_string(),
            })? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(i) => Value::Integer(i),
                ValueRef::Real(f) => Value::Real(f),
                ValueRef::Text(b) => Value::Text(String::from_utf8_lossy(b).into_owned()),
                ValueRef::Blob(b) => Value::Blob(b.to_vec()),
            };
            values.insert(name, v);
        }
        Ok(Self { values })
    }

    fn tc(column: &str, msg: impl Into<String>) -> RowError {
        RowError::TypeConversion {
            column: column.into(),
            message: msg.into(),
        }
    }
}

impl RowRef for SqliteRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Integer(i) => i32::try_from(*i).map_err(|_| Self::tc(c, "i64 overflow")),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Integer(i)) => i32::try_from(*i)
                .map(Some)
                .map_err(|_| Self::tc(c, "overflow")),
            Some(_) => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Integer(i) => Ok(*i),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Integer(i)) => Ok(Some(*i)),
            Some(_) => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Real(f)) => Ok(Some(*f)),
            Some(Value::Integer(i)) => Ok(Some(*i as f64)),
            Some(_) => Err(Self::tc(c, "not a number")),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        self.get_i64(c).map(|i| i != 0)
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        self.get_i64_opt(c).map(|o| o.map(|i| i != 0))
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Text(s) => Ok(s.as_str()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not text")),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Text(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(Self::tc(c, "not text")),
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
            _ => Err(Self::tc(c, "not blob")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Blob(b)) => Ok(Some(b.as_slice())),
            Some(Value::Text(s)) => Ok(Some(s.as_bytes())),
            Some(_) => Err(Self::tc(c, "not blob")),
        }
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        let s = self.get_str(c)?;
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| Some(d.with_timezone(&chrono::Utc)))
                .map_err(|e| Self::tc(c, e.to_string())),
        }
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        uuid::Uuid::parse_str(self.get_str(c)?).map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => uuid::Uuid::parse_str(s)
                .map(Some)
                .map_err(|e| Self::tc(c, e.to_string())),
        }
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        serde_json::from_str(self.get_str(c)?).map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => serde_json::from_str(s)
                .map(Some)
                .map_err(|e| Self::tc(c, e.to_string())),
        }
    }
    fn get_decimal(&self, c: &str) -> Result<rust_decimal::Decimal, RowError> {
        self.get_str(c)?
            .parse::<rust_decimal::Decimal>()
            .map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => s
                .parse::<rust_decimal::Decimal>()
                .map(Some)
                .map_err(|e| Self::tc(c, e.to_string())),
        }
    }
}
