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
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
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
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
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
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
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
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
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
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
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
    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> {
        let s = self.get_str(c)?;
        // Accept RFC3339 (strip tz to naive) OR naive "YYYY-MM-DD HH:MM:SS[.fff]" / "YYYY-MM-DDTHH:MM:SS[.fff]" formats.
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Ok(dt.naive_utc());
        }
        for fmt in [
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M:%S%.f",
            "%Y-%m-%d %H:%M:%S",
        ] {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
                return Ok(dt);
            }
        }
        Err(Self::tc(
            c,
            format!("unrecognized naive datetime format: {s}"),
        ))
    }
    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(_) => self.get_naive_datetime(c).map(Some),
        }
    }
    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> {
        let s = self.get_str(c)?;
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(Some)
                .map_err(|e| Self::tc(c, e.to_string())),
        }
    }
    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> {
        let s = self.get_str(c)?;
        for fmt in ["%H:%M:%S%.f", "%H:%M:%S"] {
            if let Ok(t) = chrono::NaiveTime::parse_from_str(s, fmt) {
                return Ok(t);
            }
        }
        Err(Self::tc(c, format!("unrecognized naive time format: {s}")))
    }
    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        match self.get_str_opt(c)? {
            None => Ok(None),
            Some(_) => self.get_naive_time(c).map(Some),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn materializes_row_from_rusqlite() {
        let conn = Connection::open_in_memory().unwrap();
        let mut stmt = conn.prepare("SELECT 42 AS n, 'hello' AS s").unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let r = SqliteRowRef::from_rusqlite(row).unwrap();
        assert_eq!(r.get_i32("n").unwrap(), 42);
        assert_eq!(r.get_str("s").unwrap(), "hello");
    }

    #[test]
    fn materializes_naive_temporal_values() {
        use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
        let conn = Connection::open_in_memory().unwrap();
        let mut stmt = conn
            .prepare("SELECT '2026-04-27T15:30:45' AS dt, '2026-04-27' AS d, '15:30:45' AS t")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let r = SqliteRowRef::from_rusqlite(row).unwrap();
        assert_eq!(
            r.get_naive_datetime("dt").unwrap(),
            NaiveDateTime::parse_from_str("2026-04-27 15:30:45", "%Y-%m-%d %H:%M:%S").unwrap()
        );
        assert_eq!(
            r.get_naive_date("d").unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()
        );
        assert_eq!(
            r.get_naive_time("t").unwrap(),
            NaiveTime::from_hms_opt(15, 30, 45).unwrap()
        );
    }

    #[test]
    fn opt_methods_distinguish_missing_column_from_null() {
        let conn = Connection::open_in_memory().unwrap();
        let mut stmt = conn
            .prepare("SELECT 42 AS present, NULL AS nulled")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row = rows.next().unwrap().unwrap();
        let r = SqliteRowRef::from_rusqlite(row).unwrap();

        // Present column with a value → Ok(Some(_)).
        assert_eq!(r.get_i32_opt("present").unwrap(), Some(42));

        // Present column whose value is NULL → Ok(None).
        assert_eq!(r.get_i32_opt("nulled").unwrap(), None);

        // Absent column (typo / not in the SELECT list) → Err(ColumnNotFound).
        let err = r.get_i32_opt("missing").unwrap_err();
        assert!(
            matches!(err, RowError::ColumnNotFound(ref col) if col == "missing"),
            "expected ColumnNotFound for absent column, got {err:?}",
        );
    }
}
