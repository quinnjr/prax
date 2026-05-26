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

use std::error::Error as StdError;

use prax_query::row::{RowError, RowRef, into_row_error};
use tokio_postgres::Row;
use tokio_postgres::types::{FromSql, Kind, Type};

/// `FromSql` shim that accepts any column type and decodes its raw
/// bytes as UTF-8.
///
/// Used to read postgres `ENUM` columns into a Rust `String`, since
/// `&str: FromSql` only `accepts` TEXT/VARCHAR/BPCHAR/NAME/UNKNOWN
/// (plus a few citext-shaped names). User-defined enums encode as
/// raw UTF-8 on the wire — the same shape `text_from_sql` would
/// produce — so this just runs the bytes through `str::from_utf8`.
struct AnyText(String);

impl<'a> FromSql<'a> for AnyText {
    fn from_sql(_ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn StdError + Sync + Send>> {
        Ok(AnyText(std::str::from_utf8(raw)?.to_owned()))
    }
    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// `FromSql` shim used exclusively for null-probe logic.
///
/// `NullProbe` accepts every Postgres wire type and discards the bytes
/// entirely — we only care whether the column is NULL, not what the
/// value is. This avoids the UTF-8 conversion failure that `AnyText`
/// would incur on binary-encoded types (UUID, INTEGER, BYTEA, etc.)
/// when probing a non-null column.
///
/// `Option<NullProbe>` deserialises to `None` on NULL and `Some(NullProbe)`
/// on any non-null value regardless of the column's OID, making it the
/// correct type for `RowRef::is_null` overrides on drivers where the
/// default `get_str_opt` fallback would reject non-text columns.
struct NullProbe;

impl<'a> FromSql<'a> for NullProbe {
    fn from_sql(_ty: &Type, _raw: &'a [u8]) -> Result<Self, Box<dyn StdError + Sync + Send>> {
        Ok(NullProbe)
    }
    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// Newtype wrapper around `tokio_postgres::Row` that implements
/// `prax_query::row::RowRef`.
///
/// The inner row is private; use [`PgRow::into_inner`] when ownership is
/// needed (e.g., forwarding to a tokio-postgres API that consumes `Row`).
/// For read-only access, the `Deref<Target = Row>` impl lets you call
/// any `Row` method directly.
#[repr(transparent)]
pub struct PgRow(Row);

impl PgRow {
    /// Move the wrapped `tokio_postgres::Row` out of this wrapper.
    pub fn into_inner(self) -> Row {
        self.0
    }
}

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
    /// Override the trait default (which is `get_str + to_string`) so
    /// we can decode columns whose Postgres-side type is *not* TEXT but
    /// whose Rust-side codegen has emitted `pub field: String`:
    ///
    /// * `UUID` columns — emitted as `String` for Prisma `String @db.Uuid`
    ///   fields. We decode via `uuid::Uuid::from_sql` and stringify.
    /// * User-defined `ENUM` columns — also emitted as `String` via
    ///   `FromColumn`'s `get_string` call (see codegen for `enum`
    ///   variants). We decode via `AnyText` since `&str: FromSql` does
    ///   not accept enum types.
    ///
    /// Without this override, the row decode fails with `"error
    /// deserializing column N"` and the whole query bubbles up a
    /// `[P6003] type conversion error`.
    ///
    /// A matching override on `get_string_opt` covers nullable variants.
    fn get_string(&self, c: &str) -> Result<String, RowError> {
        let columns = self.0.columns();
        if let Some(col) = columns.iter().find(|col| col.name() == c) {
            let ty = col.type_();
            if *ty == Type::UUID {
                return into_row_error(c, self.try_get::<_, ::uuid::Uuid>(c))
                    .map(|u| u.to_string());
            }
            if matches!(ty.kind(), Kind::Enum(_)) {
                return into_row_error(c, self.try_get::<_, AnyText>(c)).map(|t| t.0);
            }
        }
        into_row_error(c, self.try_get::<_, &str>(c)).map(|s| s.to_string())
    }
    fn get_string_opt(&self, c: &str) -> Result<Option<String>, RowError> {
        let columns = self.0.columns();
        if let Some(col) = columns.iter().find(|col| col.name() == c) {
            let ty = col.type_();
            if *ty == Type::UUID {
                return into_row_error(c, self.try_get::<_, Option<::uuid::Uuid>>(c))
                    .map(|opt| opt.map(|u| u.to_string()));
            }
            if matches!(ty.kind(), Kind::Enum(_)) {
                return into_row_error(c, self.try_get::<_, Option<AnyText>>(c))
                    .map(|opt| opt.map(|t| t.0));
            }
        }
        into_row_error(c, self.try_get::<_, Option<&str>>(c)).map(|opt| opt.map(|s| s.to_string()))
    }
    /// Override the trait default (which falls back to `get_str_opt`, and
    /// therefore fails for non-TEXT column types like INTEGER, UUID, etc.)
    /// with a type-agnostic null probe.
    ///
    /// `NullProbe: FromSql` accepts every Postgres wire type and discards
    /// the payload entirely, so `Option<NullProbe>` deserialises to `None`
    /// on NULL and `Some(NullProbe)` on any non-null value — regardless of
    /// the column's OID — without attempting any byte interpretation. This
    /// is exactly what the blanket `impl<T: FromColumn> FromColumn for
    /// Option<T>` needs to short-circuit nullable columns of any type.
    fn is_null(&self, c: &str) -> Result<bool, RowError> {
        into_row_error(c, self.try_get::<_, Option<NullProbe>>(c)).map(|opt| opt.is_none())
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
    fn get_decimal(&self, c: &str) -> Result<rust_decimal::Decimal, RowError> {
        Err(RowError::TypeConversion {
            column: c.to_string(),
            message: "decimal columns require tokio-postgres with \
                      `with-rust_decimal-*` feature, which this workspace does not \
                      currently enable. Cast NUMERIC columns to TEXT in your SQL \
                      (e.g. amount::text) and decode as String, or upgrade \
                      tokio-postgres."
                .to_string(),
        })
    }
    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        // NULL can't be distinguished from the underlying "unsupported" state
        // at this layer; surface the same actionable message.
        Err(RowError::TypeConversion {
            column: c.to_string(),
            message: "decimal columns require tokio-postgres with \
                      `with-rust_decimal-*` feature, which this workspace does not \
                      currently enable. Cast NUMERIC columns to TEXT in your SQL \
                      (e.g. amount::text) and decode as String, or upgrade \
                      tokio-postgres."
                .to_string(),
        })
    }
}
