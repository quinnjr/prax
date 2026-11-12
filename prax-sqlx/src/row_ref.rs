//! Bridge between SqlxRow and prax_query::row::RowRef.
//!
//! Decodes each column to a string-keyed snapshot so the prax-query
//! `FromRow` pipeline works uniformly across SQLx's three backends
//! (Postgres, MySQL, SQLite). Strings are materialized eagerly so
//! `get_str` can hand back a borrowed slice.
//!
//! ## Type-dispatched decoding
//!
//! Cells are decoded by dispatching on the column's reported type
//! (`Column::type_info()`) rather than trial-decoding a fixed list of
//! Rust types. SQLx's `Decode<String>` only accepts TEXT-family wire
//! types (and the numeric decoders only accept their exact Postgres
//! OIDs), so the old probe chain silently collapsed TIMESTAMP, UUID,
//! JSON, NUMERIC, DATE, and TIME cells into [`Value::Null`] — a
//! non-null `DateTime` read into `Option<DateTime>` came back `None`.
//! With type-dispatch, every recognised type lands in its proper
//! [`Value`] variant, and cells we genuinely cannot decode surface
//! [`RowError::TypeConversion`] instead of a fabricated NULL.
//!
//! The chrono/uuid/json/rust_decimal decoders below rely on the matching
//! sqlx features, which `prax-sqlx`'s Cargo.toml enables unconditionally.

use std::collections::HashMap;

use prax_query::row::{RowError, RowRef};
use sqlx::{Column, Row, TypeInfo};

use crate::row::SqlxRow;

// Crate-private on purpose: variants may be added freely as new column
// types are mapped, without a semver break.
enum Value {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// Postgres TIMESTAMPTZ / MySQL TIMESTAMP — a timezone-aware instant.
    DateTimeUtc(chrono::DateTime<chrono::Utc>),
    /// Postgres TIMESTAMP / MySQL DATETIME / SQLite DATETIME — naive.
    NaiveDateTime(chrono::NaiveDateTime),
    NaiveDate(chrono::NaiveDate),
    NaiveTime(chrono::NaiveTime),
    Uuid(uuid::Uuid),
    Json(serde_json::Value),
    Decimal(rust_decimal::Decimal),
    /// Postgres `TEXT[]` / `VARCHAR[]` / `CHAR[]` / `NAME[]`.
    TextArray(Vec<String>),
}

/// A driver-agnostic decoded row produced by the sqlx engine. Holds owned
/// values keyed by column name so callers can access them after the row
/// itself has been dropped.
pub struct SqlxRowRef {
    values: HashMap<String, Value>,
}

impl SqlxRowRef {
    /// Decode a raw sqlx row into a driver-agnostic [`SqlxRowRef`].
    ///
    /// Returns [`RowError::TypeConversion`] if any column's wire type
    /// cannot be decoded into a [`Value`] — a column is never silently
    /// recorded as NULL unless it actually is NULL.
    pub fn from_sqlx(row: &SqlxRow) -> Result<Self, RowError> {
        let mut values = HashMap::new();
        match row {
            #[cfg(feature = "postgres")]
            SqlxRow::Postgres(r) => {
                for (i, col) in r.columns().iter().enumerate() {
                    let name = col.name().to_string();
                    let v = decode_pg_cell(r, i)?;
                    values.insert(name, v);
                }
            }
            #[cfg(feature = "mysql")]
            SqlxRow::MySql(r) => {
                for (i, col) in r.columns().iter().enumerate() {
                    let name = col.name().to_string();
                    let v = decode_mysql_cell(r, i)?;
                    values.insert(name, v);
                }
            }
            #[cfg(feature = "sqlite")]
            SqlxRow::Sqlite(r) => {
                for (i, col) in r.columns().iter().enumerate() {
                    let name = col.name().to_string();
                    let v = decode_sqlite_cell(r, i)?;
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

/// Decode one cell as `Option<$ty>` and fold it into a [`Value`].
///
/// SQL NULL maps to [`Value::Null`] (the Option-wrapped decode
/// short-circuits before sqlx's type-compatibility check), a decoded
/// value maps through `$wrap`, and a driver-level failure — type
/// mismatch, out-of-range, malformed payload — propagates as
/// [`RowError::TypeConversion`], never as `Null`.
macro_rules! decode_cell {
    ($row:expr, $idx:expr, $column:expr, $ty:ty, $wrap:expr) => {
        match $row.try_get::<Option<$ty>, _>($idx) {
            Ok(None) => Ok(Value::Null),
            Ok(Some(v)) => Ok(($wrap)(v)),
            Err(e) => Err(tc($column, e.to_string())),
        }
    };
}

/// Decode a Postgres cell by dispatching on the column's `PgTypeInfo`.
/// Matched names are sqlx's display names ("INT4", "TIMESTAMPTZ", ...).
#[cfg(feature = "postgres")]
fn decode_pg_cell(r: &sqlx::postgres::PgRow, i: usize) -> Result<Value, RowError> {
    let column = r.columns()[i].name();
    match r.columns()[i].type_info().name() {
        // "CHAR" is BPCHAR (character(n)); sqlx's str decoder also
        // accepts NAME and UNKNOWN.
        "TEXT" | "VARCHAR" | "CHAR" | "NAME" | "UNKNOWN" => {
            decode_cell!(r, i, column, String, Value::Text)
        }
        "BOOL" => decode_cell!(r, i, column, bool, Value::Bool),
        // sqlx's int decoders accept exactly their own OID, so decode
        // each width natively and widen.
        "INT2" => decode_cell!(r, i, column, i16, |v| Value::I64(v as i64)),
        "INT4" => decode_cell!(r, i, column, i32, |v| Value::I64(v as i64)),
        "INT8" => decode_cell!(r, i, column, i64, Value::I64),
        "FLOAT4" => decode_cell!(r, i, column, f32, |v| Value::F64(v as f64)),
        "FLOAT8" => decode_cell!(r, i, column, f64, Value::F64),
        "NUMERIC" => decode_cell!(r, i, column, rust_decimal::Decimal, Value::Decimal),
        "TIMESTAMPTZ" => {
            decode_cell!(
                r,
                i,
                column,
                chrono::DateTime<chrono::Utc>,
                Value::DateTimeUtc
            )
        }
        "TIMESTAMP" => decode_cell!(r, i, column, chrono::NaiveDateTime, Value::NaiveDateTime),
        "DATE" => decode_cell!(r, i, column, chrono::NaiveDate, Value::NaiveDate),
        "TIME" => decode_cell!(r, i, column, chrono::NaiveTime, Value::NaiveTime),
        "UUID" => decode_cell!(r, i, column, uuid::Uuid, Value::Uuid),
        "JSON" | "JSONB" => decode_cell!(r, i, column, serde_json::Value, Value::Json),
        "BYTEA" => decode_cell!(r, i, column, Vec<u8>, Value::Bytes),
        // A NULL element inside the array is a decode error: `Vec<String>`
        // has no slot for it.
        "TEXT[]" | "VARCHAR[]" | "CHAR[]" | "NAME[]" => {
            decode_cell!(r, i, column, Vec<String>, Value::TextArray)
        }
        other => {
            // User-defined enums, domains, arrays, ranges, MONEY, etc.:
            // try a text decode; if the driver rejects the pairing, fail
            // loudly instead of fabricating a NULL.
            match r.try_get::<Option<String>, _>(i) {
                Ok(None) => Ok(Value::Null),
                Ok(Some(s)) => Ok(Value::Text(s)),
                Err(e) => Err(tc(
                    column,
                    format!("unsupported Postgres type {other}: {e}"),
                )),
            }
        }
    }
}

/// Decode a MySQL cell by dispatching on the column's `MySqlTypeInfo`.
/// sqlx derives the name from the wire type plus column flags, e.g.
/// `TINYINT(1)` reports as "BOOLEAN" and unsigned ints carry an
/// " UNSIGNED" suffix.
#[cfg(feature = "mysql")]
fn decode_mysql_cell(r: &sqlx::mysql::MySqlRow, i: usize) -> Result<Value, RowError> {
    let column = r.columns()[i].name();
    let ty = r.columns()[i].type_info().name();
    match ty {
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" => {
            decode_cell!(r, i, column, String, Value::Text)
        }
        // BOOLEAN is TINYINT(1) on the wire; sqlx's bool decoder reads
        // any integer width as `!= 0`.
        "BOOLEAN" => decode_cell!(r, i, column, bool, Value::Bool),
        // Genuine integer columns decode as i64 — NOT bool. (The old
        // probe chain tried bool before i64 and collapsed every MySQL
        // integer into a boolean.)
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => {
            decode_cell!(r, i, column, i64, Value::I64)
        }
        // sqlx only accepts unsigned ints (and YEAR/BIT) as u64; widen
        // with an explicit overflow check.
        _ if ty.ends_with(" UNSIGNED") || ty == "YEAR" || ty == "BIT" => {
            match r.try_get::<Option<u64>, _>(i) {
                Ok(None) => Ok(Value::Null),
                Ok(Some(v)) => i64::try_from(v)
                    .map(Value::I64)
                    .map_err(|_| tc(column, "u64 value overflows i64")),
                Err(e) => Err(tc(column, e.to_string())),
            }
        }
        "FLOAT" | "DOUBLE" => decode_cell!(r, i, column, f64, Value::F64),
        "DECIMAL" => decode_cell!(r, i, column, rust_decimal::Decimal, Value::Decimal),
        // TIMESTAMP decodes as a UTC instant; DATETIME stays naive.
        // (sqlx's NaiveDateTime decoder rejects TIMESTAMP columns.)
        "TIMESTAMP" => {
            decode_cell!(
                r,
                i,
                column,
                chrono::DateTime<chrono::Utc>,
                Value::DateTimeUtc
            )
        }
        "DATETIME" => decode_cell!(r, i, column, chrono::NaiveDateTime, Value::NaiveDateTime),
        "DATE" => decode_cell!(r, i, column, chrono::NaiveDate, Value::NaiveDate),
        "TIME" => decode_cell!(r, i, column, chrono::NaiveTime, Value::NaiveTime),
        "JSON" => decode_cell!(r, i, column, serde_json::Value, Value::Json),
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => {
            decode_cell!(r, i, column, Vec<u8>, Value::Bytes)
        }
        other => match r.try_get::<Option<String>, _>(i) {
            Ok(None) => Ok(Value::Null),
            Ok(Some(s)) => Ok(Value::Text(s)),
            Err(e) => Err(tc(column, format!("unsupported MySQL type {other}: {e}"))),
        },
    }
}

/// Decode a SQLite cell by dispatching on the column's declared-type
/// affinity. SQLite has no native datetime storage: sqlx maps declared
/// DATE/TIME/DATETIME/TIMESTAMP columns onto chrono decoders that read
/// ISO-8601 text or numeric unix timestamps. Declared types sqlx cannot
/// map (e.g. JSON) fall back to the value's runtime storage class, so a
/// JSON column arrives here as "TEXT" and stays usable via
/// `get_json`'s string parse. Columns with no declared type at all
/// (expressions, `SELECT NULL`) report "NULL" and are probed against the
/// runtime storage class.
#[cfg(feature = "sqlite")]
fn decode_sqlite_cell(r: &sqlx::sqlite::SqliteRow, i: usize) -> Result<Value, RowError> {
    let column = r.columns()[i].name();
    match r.columns()[i].type_info().name() {
        "TEXT" => decode_cell!(r, i, column, String, Value::Text),
        "INTEGER" => decode_cell!(r, i, column, i64, Value::I64),
        "REAL" => decode_cell!(r, i, column, f64, Value::F64),
        "BLOB" => decode_cell!(r, i, column, Vec<u8>, Value::Bytes),
        // BOOLEAN columns store 0/1 integers; sqlx's bool decoder
        // accepts them and reads `!= 0`.
        "BOOLEAN" => decode_cell!(r, i, column, bool, Value::Bool),
        "DATETIME" => decode_cell!(r, i, column, chrono::NaiveDateTime, Value::NaiveDateTime),
        "DATE" => decode_cell!(r, i, column, chrono::NaiveDate, Value::NaiveDate),
        "TIME" => decode_cell!(r, i, column, chrono::NaiveTime, Value::NaiveTime),
        _ => {
            // A bytes probe is `Ok(None)` only for a genuine SQL NULL:
            // any non-null value either decodes (text/blob) or fails the
            // compatibility check (int/float).
            if r.try_get::<Option<Vec<u8>>, _>(i)
                .is_ok_and(|v| v.is_none())
            {
                return Ok(Value::Null);
            }
            if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
                return Ok(Value::Text(s));
            }
            if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
                return Ok(Value::I64(n));
            }
            if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
                return Ok(Value::F64(f));
            }
            if let Ok(Some(b)) = r.try_get::<Option<Vec<u8>>, _>(i) {
                return Ok(Value::Bytes(b));
            }
            Err(tc(column, "unsupported SQLite value"))
        }
    }
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
    /// Override the trait default (`get_str` + `to_string`) so
    /// String-typed model fields can read columns decoded into a
    /// non-text variant — most importantly `String @db.Uuid` schema
    /// fields over native UUID columns, mirroring `PgRow::get_string`
    /// in prax-postgres. Datetimes stringify as RFC 3339; decimals and
    /// JSON use their canonical text forms.
    fn get_string(&self, c: &str) -> Result<String, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Text(s) => Ok(s.clone()),
            Value::Uuid(u) => Ok(u.to_string()),
            Value::Decimal(d) => Ok(d.to_string()),
            Value::Json(j) => Ok(j.to_string()),
            Value::DateTimeUtc(d) => Ok(d.to_rfc3339()),
            Value::NaiveDateTime(d) => Ok(d.to_string()),
            Value::NaiveDate(d) => Ok(d.to_string()),
            Value::NaiveTime(t) => Ok(t.to_string()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not text")),
        }
    }
    fn get_string_opt(&self, c: &str) -> Result<Option<String>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Text(s)) => Ok(Some(s.clone())),
            Some(Value::Uuid(u)) => Ok(Some(u.to_string())),
            Some(Value::Decimal(d)) => Ok(Some(d.to_string())),
            Some(Value::Json(j)) => Ok(Some(j.to_string())),
            Some(Value::DateTimeUtc(d)) => Ok(Some(d.to_rfc3339())),
            Some(Value::NaiveDateTime(d)) => Ok(Some(d.to_string())),
            Some(Value::NaiveDate(d)) => Ok(Some(d.to_string())),
            Some(Value::NaiveTime(t)) => Ok(Some(t.to_string())),
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
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::DateTimeUtc(d) => Ok(*d),
            // Naive timestamps (MySQL DATETIME, SQLite DATETIME) are
            // assumed to be UTC, matching driver convention.
            Value::NaiveDateTime(d) => Ok(d.and_utc()),
            Value::Text(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&chrono::Utc))
                .map_err(|e| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a datetime")),
        }
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::DateTimeUtc(d)) => Ok(Some(*d)),
            Some(Value::NaiveDateTime(d)) => Ok(Some(d.and_utc())),
            Some(Value::Text(s)) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| Some(d.with_timezone(&chrono::Utc)))
                .map_err(|e| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not a datetime")),
        }
    }
    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::NaiveDateTime(d) => Ok(*d),
            Value::DateTimeUtc(d) => Ok(d.naive_utc()),
            Value::Text(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.naive_utc())
                .map_err(|e| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a datetime")),
        }
    }
    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::NaiveDateTime(d)) => Ok(Some(*d)),
            Some(Value::DateTimeUtc(d)) => Ok(Some(d.naive_utc())),
            Some(Value::Text(s)) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| Some(d.naive_utc()))
                .map_err(|e| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not a datetime")),
        }
    }
    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::NaiveDate(d) => Ok(*d),
            Value::Text(s) => s
                .parse::<chrono::NaiveDate>()
                .map_err(|e: chrono::ParseError| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a date")),
        }
    }
    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::NaiveDate(d)) => Ok(Some(*d)),
            Some(Value::Text(s)) => s
                .parse::<chrono::NaiveDate>()
                .map(Some)
                .map_err(|e: chrono::ParseError| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not a date")),
        }
    }
    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::NaiveTime(t) => Ok(*t),
            Value::Text(s) => s
                .parse::<chrono::NaiveTime>()
                .map_err(|e: chrono::ParseError| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a time")),
        }
    }
    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::NaiveTime(t)) => Ok(Some(*t)),
            Some(Value::Text(s)) => s
                .parse::<chrono::NaiveTime>()
                .map(Some)
                .map_err(|e: chrono::ParseError| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not a time")),
        }
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Uuid(u) => Ok(*u),
            Value::Text(s) => uuid::Uuid::parse_str(s).map_err(|e| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a uuid")),
        }
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Uuid(u)) => Ok(Some(*u)),
            Some(Value::Text(s)) => uuid::Uuid::parse_str(s)
                .map(Some)
                .map_err(|e| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not a uuid")),
        }
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Json(j) => Ok(j.clone()),
            Value::Text(s) => serde_json::from_str(s).map_err(|e| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not json")),
        }
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Json(j)) => Ok(Some(j.clone())),
            Some(Value::Text(s)) => serde_json::from_str(s)
                .map(Some)
                .map_err(|e| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not json")),
        }
    }
    fn get_decimal(&self, c: &str) -> Result<rust_decimal::Decimal, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::Decimal(d) => Ok(*d),
            Value::I64(i) => Ok(rust_decimal::Decimal::from(*i)),
            Value::Text(s) => s
                .parse::<rust_decimal::Decimal>()
                .map_err(|e: rust_decimal::Error| tc(c, e.to_string())),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a decimal")),
        }
    }
    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(Value::Null) => Ok(None),
            Some(Value::Decimal(d)) => Ok(Some(*d)),
            Some(Value::I64(i)) => Ok(Some(rust_decimal::Decimal::from(*i))),
            Some(Value::Text(s)) => s
                .parse::<rust_decimal::Decimal>()
                .map(Some)
                .map_err(|e: rust_decimal::Error| tc(c, e.to_string())),
            Some(_) => Err(tc(c, "not a decimal")),
        }
    }
    fn get_string_array(&self, c: &str) -> Result<Vec<String>, RowError> {
        match self
            .values
            .get(c)
            .ok_or_else(|| RowError::ColumnNotFound(c.into()))?
        {
            Value::TextArray(items) => Ok(items.clone()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(tc(c, "not a text array")),
        }
    }
    /// Override the trait default (which delegates to `get_str_opt` and
    /// therefore *errors* on any non-text, non-null cell — I64, F64,
    /// Bool, Bytes, ...) with a direct snapshot probe. This is what the
    /// blanket `impl<T: FromColumn> FromColumn for Option<T>` calls to
    /// short-circuit nullable columns, so `Option<i32> = Some(5)` must
    /// get `Ok(false)` here rather than a "not text" failure. Mirrors
    /// the `NullProbe` override on `PgRow` in prax-postgres.
    fn is_null(&self, c: &str) -> Result<bool, RowError> {
        match self.values.get(c) {
            None => Err(RowError::ColumnNotFound(c.into())),
            Some(v) => Ok(matches!(v, Value::Null)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_query::row::FromColumn;

    fn row(pairs: Vec<(&str, Value)>) -> SqlxRowRef {
        SqlxRowRef {
            values: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }

    fn naive_datetime() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
            .unwrap()
            .and_hms_opt(3, 4, 5)
            .unwrap()
    }

    #[test]
    fn is_null_reports_null_variant_as_null() {
        let r = row(vec![("a", Value::Null)]);
        assert!(r.is_null("a").unwrap());
    }

    #[test]
    fn is_null_reports_every_non_null_variant_as_not_null() {
        let r = row(vec![
            ("bool", Value::Bool(true)),
            ("i64", Value::I64(5)),
            ("f64", Value::F64(1.5)),
            ("text", Value::Text("hello".into())),
            ("bytes", Value::Bytes(vec![1, 2, 3])),
            (
                "dt_utc",
                Value::DateTimeUtc(chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()),
            ),
            ("ndt", Value::NaiveDateTime(naive_datetime())),
            (
                "nd",
                Value::NaiveDate(chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()),
            ),
            (
                "nt",
                Value::NaiveTime(chrono::NaiveTime::from_hms_opt(3, 4, 5).unwrap()),
            ),
            ("uuid", Value::Uuid(uuid::Uuid::nil())),
            ("json", Value::Json(serde_json::json!({"a": 1}))),
            ("decimal", Value::Decimal(rust_decimal::Decimal::new(42, 1))),
        ]);
        for col in [
            "bool", "i64", "f64", "text", "bytes", "dt_utc", "ndt", "nd", "nt", "uuid", "json",
            "decimal",
        ] {
            assert!(!r.is_null(col).unwrap(), "{col} should not be null");
        }
    }

    #[test]
    fn is_null_missing_column_is_column_not_found() {
        let r = row(vec![]);
        assert!(matches!(
            r.is_null("missing"),
            Err(RowError::ColumnNotFound(_))
        ));
    }

    /// Regression for the missing `is_null` override: the blanket
    /// `FromColumn for Option<T>` dispatches through `is_null`, which
    /// used to delegate to `get_str_opt` and error on non-text cells,
    /// failing the whole row for `Option<i32> = Some(5)` & friends.
    #[test]
    fn option_from_column_decodes_non_text_cells() {
        let r = row(vec![
            ("n", Value::I64(5)),
            ("f", Value::F64(2.5)),
            ("b", Value::Bool(true)),
            ("null_i", Value::Null),
        ]);
        assert_eq!(Option::<i32>::from_column(&r, "n").unwrap(), Some(5));
        assert_eq!(Option::<f64>::from_column(&r, "f").unwrap(), Some(2.5));
        assert_eq!(Option::<bool>::from_column(&r, "b").unwrap(), Some(true));
        assert_eq!(Option::<i32>::from_column(&r, "null_i").unwrap(), None);
    }

    #[test]
    fn typed_variants_are_readable_through_their_getters() {
        let uuid = uuid::Uuid::nil();
        let ndt = naive_datetime();
        let r = row(vec![
            ("u", Value::Uuid(uuid)),
            ("d", Value::Decimal(rust_decimal::Decimal::new(42, 1))),
            ("j", Value::Json(serde_json::json!([1, 2]))),
            ("t", Value::NaiveDateTime(ndt)),
        ]);
        assert_eq!(r.get_uuid("u").unwrap(), uuid);
        // String-typed model fields over UUID columns (Prisma `String
        // @db.Uuid`) read through `get_string`, like prax-postgres.
        assert_eq!(r.get_string("u").unwrap(), uuid.to_string());
        assert_eq!(
            r.get_decimal("d").unwrap(),
            rust_decimal::Decimal::new(42, 1)
        );
        assert_eq!(r.get_json("j").unwrap(), serde_json::json!([1, 2]));
        assert_eq!(r.get_naive_datetime("t").unwrap(), ndt);
        // The UTC getter assumes naive timestamps are UTC.
        assert_eq!(r.get_datetime_utc("t").unwrap(), ndt.and_utc());
    }

    /// `String[]` scalar-list fields decode through `get_string_array`, and
    /// the blanket `Option<T>` impl maps a NULL array to `None`.
    #[test]
    fn text_array_cells_decode_as_vec_string() {
        let r = row(vec![
            (
                "scopes",
                Value::TextArray(vec!["read".into(), "write".into()]),
            ),
            ("empty", Value::TextArray(vec![])),
            ("null_tags", Value::Null),
            ("name", Value::Text("x".into())),
        ]);
        assert_eq!(
            Vec::<String>::from_column(&r, "scopes").unwrap(),
            vec!["read".to_string(), "write".to_string()]
        );
        assert!(Vec::<String>::from_column(&r, "empty").unwrap().is_empty());
        assert_eq!(
            Option::<Vec<String>>::from_column(&r, "scopes").unwrap(),
            Some(vec!["read".to_string(), "write".to_string()])
        );
        assert_eq!(
            Option::<Vec<String>>::from_column(&r, "null_tags").unwrap(),
            None
        );
        assert!(!r.is_null("scopes").unwrap());
        // A NULL array is not a `Vec<String>`, and a scalar is not an array.
        assert!(matches!(
            Vec::<String>::from_column(&r, "null_tags"),
            Err(RowError::UnexpectedNull(_))
        ));
        assert!(matches!(
            Vec::<String>::from_column(&r, "name"),
            Err(RowError::TypeConversion { .. })
        ));
        assert!(matches!(
            Vec::<String>::from_column(&r, "missing"),
            Err(RowError::ColumnNotFound(_))
        ));
    }

    // NOTE: decoding correctness of `decode_*_cell` against real wire
    // types requires live databases (Postgres/MySQL/SQLite) and is
    // currently untested — prax-sqlx has no live-database integration
    // tests.
}
