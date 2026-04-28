//! Bridge between `tokio_postgres::Row` and `prax_query::row::RowRef`.
//!
//! `RowRef` is defined in `prax-query` and `Row` is defined in `tokio-postgres`;
//! both are foreign to this crate, so Rust's orphan rules forbid a direct
//! `impl RowRef for Row`. We wrap the row in a `#[repr(transparent)]` newtype
//! (`PgRow`) and implement the trait on the wrapper.
//!
//! ## Dual API surface
//!
//! `PgRow` derefs to the wrapped `Row`, so callers can use either API:
//! * The generic `RowRef` interface (`pg_row.get_i32("id")`) — portable across
//!   drivers and what generated `FromRow` impls use.
//! * The native `tokio_postgres::Row` interface (`pg_row.try_get::<_, i32>("id")`)
//!   — full access to the driver's type system for columns `RowRef` does not
//!   model (arrays, range types, etc.).
//!
//! ## `rust_decimal` limitation
//!
//! `tokio-postgres` 0.7 has no `with-rust_decimal-*` feature gate and
//! `rust_decimal::Decimal` therefore has no `FromSql` impl through the driver.
//! `PgRow::get_decimal` and `PgRow::get_decimal_opt` fall back to the trait's
//! default implementations, which return a `RowError::TypeConversion` marked
//! "decimal not supported by this row type". Until we add a bridging
//! `FromSql`/`ToSql` impl (or switch to a driver that exposes the feature),
//! callers that need decimal values should cast the column to text
//! (`amount::text`) and parse in application code.

use prax_query::row::{RowError, RowRef, into_row_error};
use tokio_postgres::Row;

/// Newtype wrapper around `tokio_postgres::Row` that implements
/// `prax_query::row::RowRef`.
///
/// The field is public so callers can move the raw `Row` out with
/// `pg_row.0` when they need ownership (e.g., forwarding to a
/// tokio-postgres API that consumes `Row`). For read-only access, prefer
/// the `Deref` impl — it lets you call every `Row` method without
/// reaching through the tuple field.
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
        into_row_error(c, self.try_get::<_, i32>(c))
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        into_row_error(c, self.try_get::<_, Option<i32>>(c))
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        into_row_error(c, self.try_get::<_, i64>(c))
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        into_row_error(c, self.try_get::<_, Option<i64>>(c))
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        into_row_error(c, self.try_get::<_, f64>(c))
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        into_row_error(c, self.try_get::<_, Option<f64>>(c))
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> {
        into_row_error(c, self.try_get::<_, bool>(c))
    }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> {
        into_row_error(c, self.try_get::<_, Option<bool>>(c))
    }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        into_row_error(c, self.try_get::<_, &str>(c))
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        into_row_error(c, self.try_get::<_, Option<&str>>(c))
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        into_row_error(c, self.try_get::<_, &[u8]>(c))
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        into_row_error(c, self.try_get::<_, Option<&[u8]>>(c))
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        into_row_error(c, self.try_get::<_, chrono::DateTime<chrono::Utc>>(c))
    }
    fn get_datetime_utc_opt(
        &self,
        c: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        into_row_error(
            c,
            self.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(c),
        )
    }
    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> {
        into_row_error(c, self.try_get::<_, chrono::NaiveDateTime>(c))
    }
    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        into_row_error(c, self.try_get::<_, Option<chrono::NaiveDateTime>>(c))
    }
    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> {
        into_row_error(c, self.try_get::<_, chrono::NaiveDate>(c))
    }
    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        into_row_error(c, self.try_get::<_, Option<chrono::NaiveDate>>(c))
    }
    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> {
        into_row_error(c, self.try_get::<_, chrono::NaiveTime>(c))
    }
    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        into_row_error(c, self.try_get::<_, Option<chrono::NaiveTime>>(c))
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        into_row_error(c, self.try_get::<_, uuid::Uuid>(c))
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        into_row_error(c, self.try_get::<_, Option<uuid::Uuid>>(c))
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        into_row_error(c, self.try_get::<_, serde_json::Value>(c))
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        into_row_error(c, self.try_get::<_, Option<serde_json::Value>>(c))
    }
    // `get_decimal` / `get_decimal_opt` intentionally fall back to the trait's
    // default-erroring impl; see the module docs for why.
}
