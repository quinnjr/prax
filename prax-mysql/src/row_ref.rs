//! Bridge between mysql_async rows and prax_query::row::RowRef.
//!
//! `mysql_async::Row` exposes per-column `Value`s. `MysqlRowRef` pulls every
//! column off the row at construction time and stores them in an owned
//! `HashMap<String, Value>`. String values are pre-decoded into a separate
//! HashMap at construction time so `RowRef::get_str` can safely return `&str`
//! without interior mutability or unsafe pointer casts.

use std::collections::HashMap;

use mysql_async::{Row, Value};
use prax_query::row::{RowError, RowRef};

/// Owned snapshot of a MySQL row keyed by column name.
pub struct MysqlRowRef {
    values: HashMap<String, Value>,
    decoded: HashMap<String, String>,
}

impl MysqlRowRef {
    /// Materialize a `mysql_async::Row` into a `RowRef`-capable snapshot.
    pub fn from_row(row: Row) -> Result<Self, RowError> {
        let columns = row.columns_ref().to_vec();
        let mut values = HashMap::with_capacity(columns.len());
        let mut decoded = HashMap::with_capacity(columns.len());
        for (i, col) in columns.iter().enumerate() {
            let name = col.name_str().to_string();
            let v: Option<Value> = row.get(i);
            let v = v.unwrap_or(Value::NULL);

            // Decode to string form for every non-NULL value so get_str works
            // on any column. Preserves existing behavior where get_str accepts
            // integers, floats, dates, etc. and returns their string form.
            let text = match &v {
                Value::Bytes(b) => std::str::from_utf8(b).map(|s| s.to_string()).ok(),
                Value::NULL => None,
                Value::Int(i) => Some(i.to_string()),
                Value::UInt(u) => Some(u.to_string()),
                Value::Float(f) => Some(f.to_string()),
                Value::Double(d) => Some(d.to_string()),
                Value::Date(y, mo, d, h, mi, s, us) => Some(format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
                    y, mo, d, h, mi, s, us
                )),
                Value::Time(neg, days, h, m, s, us) => {
                    let sign = if *neg { "-" } else { "" };
                    Some(format!(
                        "{}{}:{:02}:{:02}.{:06}",
                        sign,
                        days * 24 + (*h as u32),
                        m,
                        s,
                        us
                    ))
                }
            };
            if let Some(t) = text {
                decoded.insert(name.clone(), t);
            }
            values.insert(name, v);
        }
        Ok(Self { values, decoded })
    }

    fn get(&self, c: &str) -> Result<&Value, RowError> {
        self.values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))
    }

    fn tc(c: &str, msg: impl Into<String>) -> RowError {
        RowError::TypeConversion {
            column: c.into(),
            message: msg.into(),
        }
    }
}

impl RowRef for MysqlRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self.get(c)? {
            Value::Int(i) => i32::try_from(*i).map_err(|_| Self::tc(c, "i64 overflow")),
            Value::UInt(u) => i32::try_from(*u).map_err(|_| Self::tc(c, "u64 overflow")),
            Value::Bytes(b) => {
                // MySQL can return numbers as text in certain contexts
                let s = std::str::from_utf8(b).map_err(|e| Self::tc(c, e.to_string()))?;
                s.parse::<i32>().map_err(|e| Self::tc(c, e.to_string()))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not int")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_i32(c).map(Some),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self.get(c)? {
            Value::Int(i) => Ok(*i),
            Value::UInt(u) => i64::try_from(*u).map_err(|_| Self::tc(c, "u64 overflow")),
            Value::Bytes(b) => {
                let s = std::str::from_utf8(b).map_err(|e| Self::tc(c, e.to_string()))?;
                s.parse::<i64>().map_err(|e| Self::tc(c, e.to_string()))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not int")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_i64(c).map(Some),
        }
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self.get(c)? {
            Value::Double(d) => Ok(*d),
            Value::Float(f) => Ok(f64::from(*f)),
            Value::Int(i) => Ok(*i as f64),
            Value::UInt(u) => Ok(*u as f64),
            Value::Bytes(b) => {
                let s = std::str::from_utf8(b).map_err(|e| Self::tc(c, e.to_string()))?;
                s.parse::<f64>().map_err(|e| Self::tc(c, e.to_string()))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not float")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_f64(c).map(Some),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        self.get_i64(c).map(|i| i != 0)
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        self.get_i64_opt(c).map(|o| o.map(|i| i != 0))
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match (self.values.get(c), self.decoded.get(c)) {
            (None, _) => Err(RowError::ColumnNotFound(c.into())),
            (Some(Value::NULL), _) => Err(RowError::UnexpectedNull(c.into())),
            (_, Some(s)) => Ok(s.as_str()),
            (Some(v), None) => Err(Self::tc(c, format!("not text: {v:?}"))),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match (self.values.get(c), self.decoded.get(c)) {
            (None, _) => Err(RowError::ColumnNotFound(c.into())),
            (Some(Value::NULL), _) => Ok(None),
            (_, Some(s)) => Ok(Some(s.as_str())),
            (Some(v), None) => Err(Self::tc(c, format!("not text: {v:?}"))),
        }
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self.get(c)? {
            Value::Bytes(b) => Ok(b.as_slice()),
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not bytes")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            Value::Bytes(b) => Ok(Some(b.as_slice())),
            _ => Err(Self::tc(c, "not bytes")),
        }
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        match self.get(c)? {
            Value::Date(y, mo, d, h, mi, s, micro) => {
                let naive = chrono::NaiveDate::from_ymd_opt(*y as i32, *mo as u32, *d as u32)
                    .and_then(|dt| dt.and_hms_micro_opt(*h as u32, *mi as u32, *s as u32, *micro))
                    .ok_or_else(|| Self::tc(c, "invalid datetime"))?;
                Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    naive,
                    chrono::Utc,
                ))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not datetime")),
        }
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_datetime_utc(c).map(Some),
        }
    }
    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> {
        match self.get(c)? {
            Value::Date(y, mo, d, h, mi, s, micro) => {
                chrono::NaiveDate::from_ymd_opt(*y as i32, *mo as u32, *d as u32)
                    .and_then(|dt| dt.and_hms_micro_opt(*h as u32, *mi as u32, *s as u32, *micro))
                    .ok_or_else(|| Self::tc(c, "invalid naive datetime"))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not datetime")),
        }
    }
    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_naive_datetime(c).map(Some),
        }
    }
    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> {
        match self.get(c)? {
            Value::Date(y, mo, d, _, _, _, _) => {
                chrono::NaiveDate::from_ymd_opt(*y as i32, *mo as u32, *d as u32)
                    .ok_or_else(|| Self::tc(c, "invalid naive date"))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not date")),
        }
    }
    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_naive_date(c).map(Some),
        }
    }
    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> {
        match self.get(c)? {
            Value::Time(false, 0, h, m, s, micro) => {
                chrono::NaiveTime::from_hms_micro_opt(*h as u32, *m as u32, *s as u32, *micro)
                    .ok_or_else(|| Self::tc(c, "invalid naive time"))
            }
            Value::Time(_, _, _, _, _, _) => Err(Self::tc(
                c,
                "negative or >24h TIME has no NaiveTime mapping",
            )),
            Value::Date(_, _, _, h, m, s, micro) => {
                chrono::NaiveTime::from_hms_micro_opt(*h as u32, *m as u32, *s as u32, *micro)
                    .ok_or_else(|| Self::tc(c, "invalid naive time"))
            }
            Value::NULL => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not time")),
        }
    }
    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        match self.get(c)? {
            Value::NULL => Ok(None),
            _ => self.get_naive_time(c).map(Some),
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
