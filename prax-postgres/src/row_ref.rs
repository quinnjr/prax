//! Bridge between tokio_postgres::Row and prax_query::row::RowRef.

use prax_query::row::{RowError, RowRef};
use tokio_postgres::Row;

fn tc<T, E: std::fmt::Display>(column: &str, res: Result<T, E>) -> Result<T, RowError> {
    res.map_err(|e| RowError::TypeConversion {
        column: column.to_string(),
        message: e.to_string(),
    })
}

/// Newtype wrapper to satisfy Rust's orphan rules.
/// Allows implementing prax_query::RowRef (foreign trait to this crate)
/// for tokio_postgres::Row (also foreign).
#[repr(transparent)]
pub struct PgRow(pub Row);

impl std::ops::Deref for PgRow {
    type Target = Row;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Row> for PgRow {
    fn from(row: Row) -> Self {
        PgRow(row)
    }
}

impl RowRef for PgRow {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        tc(c, self.try_get::<_, i32>(c))
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        tc(c, self.try_get::<_, Option<i32>>(c))
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        tc(c, self.try_get::<_, i64>(c))
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        tc(c, self.try_get::<_, Option<i64>>(c))
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        tc(c, self.try_get::<_, f64>(c))
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        tc(c, self.try_get::<_, Option<f64>>(c))
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        tc(c, self.try_get::<_, bool>(c))
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        tc(c, self.try_get::<_, Option<bool>>(c))
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        tc(c, self.try_get::<_, &str>(c))
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        tc(c, self.try_get::<_, Option<&str>>(c))
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        tc(c, self.try_get::<_, &[u8]>(c))
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        tc(c, self.try_get::<_, Option<&[u8]>>(c))
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        tc(c, self.try_get::<_, chrono::DateTime<chrono::Utc>>(c))
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        tc(
            c,
            self.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(c),
        )
    }
    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> {
        tc(c, self.try_get::<_, chrono::NaiveDateTime>(c))
    }
    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        tc(c, self.try_get::<_, Option<chrono::NaiveDateTime>>(c))
    }
    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> {
        tc(c, self.try_get::<_, chrono::NaiveDate>(c))
    }
    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        tc(c, self.try_get::<_, Option<chrono::NaiveDate>>(c))
    }
    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> {
        tc(c, self.try_get::<_, chrono::NaiveTime>(c))
    }
    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        tc(c, self.try_get::<_, Option<chrono::NaiveTime>>(c))
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        tc(c, self.try_get::<_, uuid::Uuid>(c))
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        tc(c, self.try_get::<_, Option<uuid::Uuid>>(c))
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        tc(c, self.try_get::<_, serde_json::Value>(c))
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        tc(c, self.try_get::<_, Option<serde_json::Value>>(c))
    }
    // Note: tokio-postgres 0.7 lacks native rust_decimal::Decimal support via FromSql.
    // The `with-rust_decimal-1` feature mentioned in the plan does not exist in this version.
    // These methods fall back to the default RowRef impl, which returns an unsupported error.
    // A working implementation would require either:
    // - Upgrading to a newer tokio-postgres (if one exists with rust_decimal support)
    // - Using pgnumeric + manual conversion (complex, DST issues)
    // - Accepting NUMERIC columns as TEXT and parsing to Decimal in application code
    // For now, callers must cast NUMERIC to TEXT in SQL if they need Decimal extraction.
}
