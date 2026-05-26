//! Type conversions for PostgreSQL.

use std::str::FromStr;

use prax_query::filter::FilterValue;
use tokio_postgres::types::{IsNull, Kind, ToSql, Type};

use crate::error::{PgError, PgResult};

/// Polymorphic integer binding. `FilterValue::Int` always carries an i64
/// (the widest scalar variant), but Postgres strictly validates client
/// bindings against column types: binding an i64 to an `INT4` column
/// fails with `WrongType { postgres: Int4, rust: "i64" }`. This wrapper
/// inspects the target column type at bind time and narrows to i16 /
/// i32 / i64 with a bounds check before forwarding to tokio-postgres'
/// own impls.
#[derive(Debug)]
struct PgInt(i64);

impl ToSql for PgInt {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match *ty {
            Type::INT2 => {
                let v: i16 = self
                    .0
                    .try_into()
                    .map_err(|_| format!("value {} overflows INT2", self.0))?;
                v.to_sql(ty, out)
            }
            Type::INT4 => {
                let v: i32 = self
                    .0
                    .try_into()
                    .map_err(|_| format!("value {} overflows INT4", self.0))?;
                v.to_sql(ty, out)
            }
            Type::INT8 => self.0.to_sql(ty, out),
            _ => Err(format!("cannot bind integer to postgres type {ty:?}").into()),
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::INT2 | Type::INT4 | Type::INT8)
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Polymorphic string binding. `FilterValue::String` always carries a
/// Rust `String`, but Postgres rejects a String bound to a UUID column
/// (`WrongType { postgres: Uuid, rust: "alloc::string::String" }`).
/// This wrapper inspects the target column type at bind time and
/// converts as appropriate; for plain TEXT/VARCHAR/CHAR/NAME and other
/// types it forwards the String directly.
#[derive(Debug)]
struct PgString(String);

impl ToSql for PgString {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match *ty {
            Type::UUID => {
                let parsed = ::uuid::Uuid::from_str(&self.0).map_err(|e| {
                    format!("FilterValue::String('{}') is not a valid UUID: {e}", self.0)
                })?;
                parsed.to_sql(ty, out)
            }
            // chrono types round-trip through `FilterValue::String`
            // (see `prax-query/src/filter.rs`). Postgres rejects a String
            // bound to TIMESTAMPTZ/TIMESTAMP/DATE/TIME with WrongType,
            // so re-parse and forward through tokio-postgres' chrono
            // FromSql/ToSql impls.
            Type::TIMESTAMPTZ => {
                let parsed: ::chrono::DateTime<::chrono::Utc> =
                    ::chrono::DateTime::parse_from_rfc3339(&self.0)
                        .map_err(|e| {
                            format!(
                                "FilterValue::String('{}') is not a valid RFC3339 \
                                 datetime for TIMESTAMPTZ: {e}",
                                self.0
                            )
                        })?
                        .with_timezone(&::chrono::Utc);
                parsed.to_sql(ty, out)
            }
            Type::TIMESTAMP => {
                let parsed =
                    ::chrono::NaiveDateTime::parse_from_str(&self.0, "%Y-%m-%dT%H:%M:%S%.f")
                        .map_err(|e| {
                            format!(
                                "FilterValue::String('{}') is not a valid \
                         ISO-8601 naive datetime for TIMESTAMP: {e}",
                                self.0
                            )
                        })?;
                parsed.to_sql(ty, out)
            }
            Type::DATE => {
                let parsed =
                    ::chrono::NaiveDate::parse_from_str(&self.0, "%Y-%m-%d").map_err(|e| {
                        format!(
                            "FilterValue::String('{}') is not a valid \
                             YYYY-MM-DD date for DATE: {e}",
                            self.0
                        )
                    })?;
                parsed.to_sql(ty, out)
            }
            Type::TIME => {
                let parsed =
                    ::chrono::NaiveTime::parse_from_str(&self.0, "%H:%M:%S%.f").map_err(|e| {
                        format!(
                            "FilterValue::String('{}') is not a valid \
                                 HH:MM:SS time for TIME: {e}",
                            self.0
                        )
                    })?;
                parsed.to_sql(ty, out)
            }
            _ => {
                // User-defined `ENUM` columns reach this arm; their
                // wire format is just utf-8 bytes, so write the
                // string body directly. Plain TEXT/VARCHAR/CHAR/NAME
                // also takes this path through `String: ToSql`.
                if matches!(ty.kind(), Kind::Enum(_)) {
                    out.extend_from_slice(self.0.as_bytes());
                    Ok(IsNull::No)
                } else {
                    self.0.to_sql(ty, out)
                }
            }
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Convert a FilterValue to a type that can be used as a PostgreSQL parameter.
pub fn filter_value_to_sql(value: &FilterValue) -> PgResult<Box<dyn ToSql + Sync + Send>> {
    match value {
        FilterValue::Null => Ok(Box::new(Option::<String>::None)),
        FilterValue::Bool(b) => Ok(Box::new(*b)),
        FilterValue::Int(i) => Ok(Box::new(PgInt(*i))),
        FilterValue::Float(f) => Ok(Box::new(*f)),
        FilterValue::String(s) => Ok(Box::new(PgString(s.clone()))),
        FilterValue::Json(j) => Ok(Box::new(j.clone())),
        FilterValue::List(_) => {
            // Lists need special handling - they should be converted to arrays
            // For now, return an error and handle lists specially in the engine
            Err(PgError::type_conversion(
                "list values should be handled specially",
            ))
        }
    }
}

/// Convert filter values to PostgreSQL parameters.
pub fn filter_values_to_params(
    values: &[FilterValue],
) -> PgResult<Vec<Box<dyn ToSql + Sync + Send>>> {
    values.iter().map(filter_value_to_sql).collect()
}

/// PostgreSQL type mapping utilities.
pub mod pg_types {
    use super::*;

    /// Get the PostgreSQL type for a Rust type name.
    pub fn rust_type_to_pg(rust_type: &str) -> Option<Type> {
        match rust_type {
            "i16" => Some(Type::INT2),
            "i32" => Some(Type::INT4),
            "i64" => Some(Type::INT8),
            "f32" => Some(Type::FLOAT4),
            "f64" => Some(Type::FLOAT8),
            "bool" => Some(Type::BOOL),
            "String" | "&str" => Some(Type::TEXT),
            "Vec<u8>" | "&[u8]" => Some(Type::BYTEA),
            "chrono::NaiveDate" => Some(Type::DATE),
            "chrono::NaiveTime" => Some(Type::TIME),
            "chrono::NaiveDateTime" => Some(Type::TIMESTAMP),
            "chrono::DateTime<chrono::Utc>" => Some(Type::TIMESTAMPTZ),
            "uuid::Uuid" => Some(Type::UUID),
            "serde_json::Value" => Some(Type::JSONB),
            _ => None,
        }
    }

    /// Get the Rust type for a PostgreSQL type.
    pub fn pg_type_to_rust(pg_type: &Type) -> &'static str {
        match *pg_type {
            Type::BOOL => "bool",
            Type::INT2 => "i16",
            Type::INT4 => "i32",
            Type::INT8 => "i64",
            Type::FLOAT4 => "f32",
            Type::FLOAT8 => "f64",
            Type::TEXT | Type::VARCHAR | Type::CHAR | Type::NAME => "String",
            Type::BYTEA => "Vec<u8>",
            Type::DATE => "chrono::NaiveDate",
            Type::TIME => "chrono::NaiveTime",
            Type::TIMESTAMP => "chrono::NaiveDateTime",
            Type::TIMESTAMPTZ => "chrono::DateTime<chrono::Utc>",
            Type::UUID => "uuid::Uuid",
            Type::JSON | Type::JSONB => "serde_json::Value",
            _ => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_value_to_sql() {
        let result = filter_value_to_sql(&FilterValue::Int(42));
        assert!(result.is_ok());

        let result = filter_value_to_sql(&FilterValue::String("test".to_string()));
        assert!(result.is_ok());

        let result = filter_value_to_sql(&FilterValue::Bool(true));
        assert!(result.is_ok());
    }

    #[test]
    fn test_pg_type_mapping() {
        use pg_types::*;

        assert_eq!(rust_type_to_pg("i32"), Some(Type::INT4));
        assert_eq!(rust_type_to_pg("String"), Some(Type::TEXT));
        assert_eq!(rust_type_to_pg("bool"), Some(Type::BOOL));

        assert_eq!(pg_type_to_rust(&Type::INT4), "i32");
        assert_eq!(pg_type_to_rust(&Type::TEXT), "String");
    }
}
