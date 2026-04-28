//! Bridge between tiberius rows and prax_query::row::RowRef.
//!
//! tiberius `Row`s contain owned `ColumnData` values. `MssqlRowRef`
//! materializes every column into an owned `HashMap<String, ColumnValue>`
//! at construction time so the generic RowRef trait (which hands out `&str`
//! and `&[u8]`) can be satisfied for the life of `&self`.

use std::collections::HashMap;

use prax_query::row::{RowError, RowRef};
use tiberius::Row;

/// Owned column value snapshot.
#[derive(Debug, Clone)]
pub(crate) enum ColumnValue {
    /// SQL NULL.
    Null,
    /// Boolean value (SQL BIT).
    Bool(bool),
    /// 8-bit unsigned integer (SQL TINYINT).
    U8(u8),
    /// 16-bit signed integer (SQL SMALLINT).
    I16(i16),
    /// 32-bit signed integer (SQL INT).
    I32(i32),
    /// 64-bit signed integer (SQL BIGINT).
    I64(i64),
    /// 32-bit floating point (SQL REAL / FLOAT(24)).
    F32(f32),
    /// 64-bit floating point (SQL FLOAT / FLOAT(53)).
    F64(f64),
    /// UTF-16 string (SQL NVARCHAR / NCHAR / VARCHAR / CHAR).
    String(String),
    /// Binary data (SQL VARBINARY / BINARY).
    Bytes(Vec<u8>),
    /// UUID (SQL UNIQUEIDENTIFIER).
    Uuid(uuid::Uuid),
    /// Decimal / numeric value (stored as string for now).
    Decimal(String),
    /// Temporal values.
    NaiveDateTime(chrono::NaiveDateTime),
    NaiveDate(chrono::NaiveDate),
    NaiveTime(chrono::NaiveTime),
    DateTimeUtc(chrono::DateTime<chrono::Utc>),
}

/// Bridge between tiberius Row and prax_query::row::RowRef.
pub struct MssqlRowRef {
    values: HashMap<String, ColumnValue>,
}

impl MssqlRowRef {
    /// Build a materialized snapshot from a tiberius Row.
    ///
    /// Uses tiberius's FromSql conversions to extract typed values.
    pub fn from_row(row: &Row) -> Result<Self, RowError> {
        let mut values = HashMap::with_capacity(row.len());
        for (idx, col) in row.columns().iter().enumerate() {
            let name = col.name().to_string();
            let val = if let Ok(Some(s)) = row.try_get::<&str, _>(idx) {
                ColumnValue::String(s.to_string())
            } else if let Ok(Some(v)) = row.try_get::<i32, _>(idx) {
                ColumnValue::I32(v)
            } else if let Ok(Some(v)) = row.try_get::<i64, _>(idx) {
                ColumnValue::I64(v)
            } else if let Ok(Some(v)) = row.try_get::<i16, _>(idx) {
                ColumnValue::I16(v)
            } else if let Ok(Some(v)) = row.try_get::<u8, _>(idx) {
                ColumnValue::U8(v)
            } else if let Ok(Some(v)) = row.try_get::<f64, _>(idx) {
                ColumnValue::F64(v)
            } else if let Ok(Some(v)) = row.try_get::<f32, _>(idx) {
                ColumnValue::F32(v)
            } else if let Ok(Some(v)) = row.try_get::<bool, _>(idx) {
                ColumnValue::Bool(v)
            } else if let Ok(Some(v)) = row.try_get::<uuid::Uuid, _>(idx) {
                ColumnValue::Uuid(v)
            } else if let Ok(Some(v)) = row.try_get::<&[u8], _>(idx) {
                ColumnValue::Bytes(v.to_vec())
            } else if let Ok(Some(v)) = row.try_get::<chrono::NaiveDateTime, _>(idx) {
                ColumnValue::NaiveDateTime(v)
            } else if let Ok(Some(v)) = row.try_get::<chrono::NaiveDate, _>(idx) {
                ColumnValue::NaiveDate(v)
            } else if let Ok(Some(v)) = row.try_get::<chrono::NaiveTime, _>(idx) {
                ColumnValue::NaiveTime(v)
            } else if let Ok(Some(v)) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(idx) {
                ColumnValue::DateTimeUtc(v)
            } else if let Ok(None) = row.try_get::<i32, _>(idx) {
                ColumnValue::Null
            } else {
                return Err(Self::tc(
                    &name,
                    format!("unsupported or ambiguous tiberius type at column {}", idx),
                ));
            };
            values.insert(name, val);
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

impl RowRef for MssqlRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::I32(v) => Ok(*v),
            ColumnValue::I16(v) => Ok(*v as i32),
            ColumnValue::U8(v) => Ok(*v as i32),
            _ => Err(Self::tc(c, "not an i32")),
        }
    }

    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::I32(v)) => Ok(Some(*v)),
            Some(ColumnValue::I16(v)) => Ok(Some(*v as i32)),
            Some(ColumnValue::U8(v)) => Ok(Some(*v as i32)),
            Some(_) => Err(Self::tc(c, "not an i32")),
        }
    }

    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::I64(v) => Ok(*v),
            ColumnValue::I32(v) => Ok(*v as i64),
            ColumnValue::I16(v) => Ok(*v as i64),
            ColumnValue::U8(v) => Ok(*v as i64),
            _ => Err(Self::tc(c, "not an i64")),
        }
    }

    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::I64(v)) => Ok(Some(*v)),
            Some(ColumnValue::I32(v)) => Ok(Some(*v as i64)),
            Some(ColumnValue::I16(v)) => Ok(Some(*v as i64)),
            Some(ColumnValue::U8(v)) => Ok(Some(*v as i64)),
            Some(_) => Err(Self::tc(c, "not an i64")),
        }
    }

    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::F64(v) => Ok(*v),
            ColumnValue::F32(v) => Ok(*v as f64),
            ColumnValue::I64(v) => Ok(*v as f64),
            ColumnValue::I32(v) => Ok(*v as f64),
            ColumnValue::I16(v) => Ok(*v as f64),
            ColumnValue::U8(v) => Ok(*v as f64),
            _ => Err(Self::tc(c, "not a number")),
        }
    }

    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::F64(v)) => Ok(Some(*v)),
            Some(ColumnValue::F32(v)) => Ok(Some(*v as f64)),
            Some(ColumnValue::I64(v)) => Ok(Some(*v as f64)),
            Some(ColumnValue::I32(v)) => Ok(Some(*v as f64)),
            Some(ColumnValue::I16(v)) => Ok(Some(*v as f64)),
            Some(ColumnValue::U8(v)) => Ok(Some(*v as f64)),
            Some(_) => Err(Self::tc(c, "not a number")),
        }
    }

    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::Bool(v) => Ok(*v),
            _ => Err(Self::tc(c, "not a bool")),
        }
    }

    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::Bool(v)) => Ok(Some(*v)),
            Some(_) => Err(Self::tc(c, "not a bool")),
        }
    }

    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::String(s) => Ok(s.as_str()),
            _ => Err(Self::tc(c, "not text")),
        }
    }

    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(Self::tc(c, "not text")),
        }
    }

    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::Bytes(b) => Ok(b.as_slice()),
            ColumnValue::String(s) => Ok(s.as_bytes()),
            _ => Err(Self::tc(c, "not bytes")),
        }
    }

    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::Bytes(b)) => Ok(Some(b.as_slice())),
            Some(ColumnValue::String(s)) => Ok(Some(s.as_bytes())),
            Some(_) => Err(Self::tc(c, "not bytes")),
        }
    }

    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::DateTimeUtc(dt) => Ok(*dt),
            _ => Err(Self::tc(c, "not a DateTimeOffset")),
        }
    }

    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::DateTimeUtc(dt)) => Ok(Some(*dt)),
            Some(_) => Err(Self::tc(c, "not a DateTimeOffset")),
        }
    }

    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::NaiveDateTime(dt) => Ok(*dt),
            _ => Err(Self::tc(c, "not a naive datetime")),
        }
    }

    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::NaiveDateTime(dt)) => Ok(Some(*dt)),
            Some(_) => Err(Self::tc(c, "not a naive datetime")),
        }
    }

    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::NaiveDate(d) => Ok(*d),
            _ => Err(Self::tc(c, "not a date")),
        }
    }

    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::NaiveDate(d)) => Ok(Some(*d)),
            Some(_) => Err(Self::tc(c, "not a date")),
        }
    }

    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::NaiveTime(t) => Ok(*t),
            _ => Err(Self::tc(c, "not a time")),
        }
    }

    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::NaiveTime(t)) => Ok(Some(*t)),
            Some(_) => Err(Self::tc(c, "not a time")),
        }
    }

    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::Uuid(u) => Ok(*u),
            _ => Err(Self::tc(c, "not a uuid")),
        }
    }

    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::Uuid(u)) => Ok(Some(*u)),
            Some(_) => Err(Self::tc(c, "not a uuid")),
        }
    }

    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        let s = self.get_str(c)?;
        serde_json::from_str(s).map_err(|e| Self::tc(c, e.to_string()))
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
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            ColumnValue::Null => Err(RowError::UnexpectedNull(c.into())),
            ColumnValue::Decimal(s) => s
                .parse::<rust_decimal::Decimal>()
                .map_err(|e| Self::tc(c, e.to_string())),
            _ => Err(Self::tc(c, "not a decimal")),
        }
    }

    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(ColumnValue::Null) => Ok(None),
            Some(ColumnValue::Decimal(s)) => s
                .parse::<rust_decimal::Decimal>()
                .map(Some)
                .map_err(|e| Self::tc(c, e.to_string())),
            Some(_) => Err(Self::tc(c, "not a decimal")),
        }
    }
}

#[cfg(test)]
mod tests {
    // Row tests require integration testing with a real SQL Server database.
}
