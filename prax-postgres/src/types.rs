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

impl PgString {
    /// Whether a `PgString` can bind to `ty`. Must match the arms of
    /// `PgString::to_sql` below: the specially-parsed types, user-defined
    /// enums, and everything `String` itself accepts.
    fn binds_to(ty: &Type) -> bool {
        matches!(
            *ty,
            Type::UUID | Type::TIMESTAMPTZ | Type::TIMESTAMP | Type::DATE | Type::TIME
        ) || matches!(ty.kind(), Kind::Enum(_))
            || <String as ToSql>::accepts(ty)
    }
}

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

/// Untyped NULL binding. `FilterValue::Null` must bind successfully against
/// a column of any type (nullable UUID, custom ENUM, TIMESTAMPTZ, INT4,
/// ...), but `Option::<String>::None`'s `accepts()` delegates to
/// `String::accepts()`, which only covers TEXT/VARCHAR/CHAR/NAME/UNKNOWN.
/// tokio-postgres runs that `accepts()` check before `to_sql` is ever
/// called, so a NULL bound against e.g. a nullable UUID column fails with
/// "error serializing parameter N" even though NULL has no type-specific
/// wire representation to get wrong. This wrapper accepts every type and
/// always signals `IsNull::Yes`.
#[derive(Debug)]
struct PgNull;

impl ToSql for PgNull {
    fn to_sql(
        &self,
        _ty: &Type,
        _out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        Ok(IsNull::Yes)
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

/// One element of a `FilterValue::Array` parameter. Delegates to the scalar
/// binders so each element adapts to the array's element type (e.g. a
/// `PgString` into `uuid[]`). `Vec<PgElem>: ToSql` (tokio-postgres) supplies
/// the array wire format and rejects non-array target types.
#[derive(Debug)]
enum PgElem {
    Null(PgNull),
    Bool(bool),
    Int(PgInt),
    Float(f64),
    String(PgString),
    Json(serde_json::Value),
    Bytes(Vec<u8>),
}

impl PgElem {
    fn try_from_value(value: &FilterValue) -> PgResult<Self> {
        Ok(match value {
            FilterValue::Null => Self::Null(PgNull),
            FilterValue::Bool(b) => Self::Bool(*b),
            FilterValue::Int(i) => Self::Int(PgInt(*i)),
            FilterValue::Float(f) => Self::Float(*f),
            FilterValue::String(s) => Self::String(PgString(s.clone())),
            FilterValue::Json(j) => Self::Json(j.clone()),
            FilterValue::Bytes(b) => Self::Bytes(b.clone()),
            FilterValue::List(_) | FilterValue::Array(_) => {
                return Err(PgError::type_conversion(
                    "nested arrays are not supported in FilterValue::Array",
                ));
            }
            _ => {
                return Err(PgError::type_conversion(
                    "unsupported FilterValue variant in FilterValue::Array",
                ));
            }
        })
    }
}

impl ToSql for PgElem {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        // Element-wise `to_sql` is called unchecked by tokio-postgres' array
        // impl, so every arm must verify the element type itself.
        fn check(
            ok: bool,
            kind: &str,
            ty: &Type,
        ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
            if ok {
                Ok(())
            } else {
                Err(format!("cannot bind {kind} array element to postgres type {ty:?}").into())
            }
        }
        match self {
            Self::Null(v) => v.to_sql(ty, out),
            Self::Bool(v) => {
                check(<bool as ToSql>::accepts(ty), "bool", ty)?;
                v.to_sql(ty, out)
            }
            Self::Int(v) => v.to_sql(ty, out),
            Self::Float(v) => {
                check(<f64 as ToSql>::accepts(ty), "float", ty)?;
                v.to_sql(ty, out)
            }
            Self::String(v) => {
                check(PgString::binds_to(ty), "string", ty)?;
                v.to_sql(ty, out)
            }
            Self::Json(v) => {
                check(<serde_json::Value as ToSql>::accepts(ty), "json", ty)?;
                v.to_sql(ty, out)
            }
            Self::Bytes(v) => {
                check(<Vec<u8> as ToSql>::accepts(ty), "bytes", ty)?;
                v.to_sql(ty, out)
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
        FilterValue::Null => Ok(Box::new(PgNull)),
        FilterValue::Bool(b) => Ok(Box::new(*b)),
        FilterValue::Int(i) => Ok(Box::new(PgInt(*i))),
        FilterValue::Float(f) => Ok(Box::new(*f)),
        FilterValue::String(s) => Ok(Box::new(PgString(s.clone()))),
        FilterValue::Json(j) => Ok(Box::new(j.clone())),
        FilterValue::List(_) => {
            // Typed IN filters expand to scalar placeholders upstream
            // (prax-query/src/filter.rs), so this arm only fires for
            // hand-built raw queries via `Sql::bind`.
            Err(PgError::type_conversion(
                "FilterValue::List cannot be bound as one parameter; use Filter::In for IN (...) or FilterValue::Array with = ANY($1)",
            ))
        }
        FilterValue::Bytes(b) => Ok(Box::new(b.clone())),
        FilterValue::Array(items) => {
            let elems = items
                .iter()
                .map(PgElem::try_from_value)
                .collect::<PgResult<Vec<_>>>()?;
            Ok(Box::new(elems))
        }
        _ => Err(PgError::type_conversion("unsupported FilterValue variant")),
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
    fn bytes_binds_as_bytea() {
        let param = filter_value_to_sql(&FilterValue::Bytes(vec![0, 255])).unwrap();
        let mut buf = bytes::BytesMut::new();
        param.to_sql_checked(&Type::BYTEA, &mut buf).unwrap();
        assert_eq!(&buf[..], &[0u8, 255][..]);
    }

    #[test]
    fn nested_list_or_array_in_array_is_rejected() {
        for inner in [FilterValue::List(vec![]), FilterValue::Array(vec![])] {
            let err = filter_value_to_sql(&FilterValue::Array(vec![inner]))
                .expect_err("nested collections must error");
            assert!(matches!(err, PgError::TypeConversion(_)));
            assert!(err.to_string().contains("nested"), "{err}");
        }
    }

    fn bind_array(
        items: Vec<FilterValue>,
        ty: &Type,
    ) -> (
        Result<IsNull, Box<dyn std::error::Error + Sync + Send>>,
        bytes::BytesMut,
    ) {
        let param = filter_value_to_sql(&FilterValue::Array(items)).unwrap();
        let mut buf = bytes::BytesMut::new();
        let res = param.to_sql_checked(ty, &mut buf);
        (res, buf)
    }

    #[test]
    fn array_binds_as_int_array() {
        let items = vec![FilterValue::Int(1), FilterValue::Null];
        let (res, buf) = bind_array(items.clone(), &Type::INT4_ARRAY);
        res.unwrap();
        // Header: ndim=1, has_null=1, elem oid INT4 (23); dim len 2, lbound 1;
        // then elem 1 (len 4, value 1) and a NULL (len -1).
        let mut expected = Vec::new();
        for v in [1i32, 1, 23, 2, 1, 4, 1, -1] {
            expected.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(&buf[..], &expected[..]);
        // Non-array target types are rejected, not panicked on.
        assert!(bind_array(items, &Type::INT4).0.is_err());
    }

    #[test]
    fn array_wrong_element_kind_errors() {
        let cases = [
            (FilterValue::Bool(true), Type::TEXT_ARRAY),
            (FilterValue::Float(1.5), Type::INT4_ARRAY),
            (FilterValue::Bytes(vec![1]), Type::TEXT_ARRAY),
            (
                FilterValue::Json(serde_json::json!({"a": 1})),
                Type::INT4_ARRAY,
            ),
            (FilterValue::String("x".into()), Type::INT4_ARRAY),
            (FilterValue::Int(1), Type::TEXT_ARRAY),
        ];
        for (item, ty) in cases {
            let (res, _) = bind_array(vec![item.clone()], &ty);
            assert!(res.is_err(), "{item:?} into {ty:?} must error");
        }
    }

    /// Bind a one-element array and decode it back with `FromSql`.
    fn assert_roundtrip<T>(item: FilterValue, ty: &Type, expected: T)
    where
        T: for<'a> tokio_postgres::types::FromSql<'a> + PartialEq + std::fmt::Debug,
    {
        let (res, buf) = bind_array(vec![item.clone()], ty);
        res.unwrap_or_else(|e| panic!("{item:?} into {ty:?}: {e}"));
        let decoded = <Vec<T> as tokio_postgres::types::FromSql>::from_sql(ty, &buf)
            .unwrap_or_else(|e| panic!("decoding {item:?} as {ty:?}: {e}"));
        assert_eq!(decoded, vec![expected], "{item:?} into {ty:?}");
    }

    #[test]
    fn array_matching_element_kinds_bind() {
        assert_roundtrip(FilterValue::Bool(true), &Type::BOOL_ARRAY, true);
        assert_roundtrip(FilterValue::Float(1.5), &Type::FLOAT8_ARRAY, 1.5f64);
        assert_roundtrip(
            FilterValue::Bytes(vec![1, 2]),
            &Type::BYTEA_ARRAY,
            vec![1u8, 2],
        );
        assert_roundtrip(
            FilterValue::Json(serde_json::json!({"a": 1})),
            &Type::JSONB_ARRAY,
            serde_json::json!({"a": 1}),
        );
        assert_roundtrip(
            FilterValue::String("x".into()),
            &Type::TEXT_ARRAY,
            "x".to_string(),
        );
        assert_roundtrip(FilterValue::Int(7), &Type::INT8_ARRAY, 7i64);
    }

    #[test]
    fn string_array_elements_adapt_to_element_type() {
        let uuid = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        assert_roundtrip(
            FilterValue::String(uuid.into()),
            &Type::UUID_ARRAY,
            ::uuid::Uuid::from_str(uuid).unwrap(),
        );
        assert_roundtrip(
            FilterValue::String("2024-01-02T03:04:05Z".into()),
            &Type::TIMESTAMPTZ_ARRAY,
            ::chrono::DateTime::parse_from_rfc3339("2024-01-02T03:04:05Z")
                .unwrap()
                .with_timezone(&::chrono::Utc),
        );
        assert_roundtrip(
            FilterValue::String("2024-01-02T03:04:05.5".into()),
            &Type::TIMESTAMP_ARRAY,
            ::chrono::NaiveDateTime::parse_from_str(
                "2024-01-02T03:04:05.5",
                "%Y-%m-%dT%H:%M:%S%.f",
            )
            .unwrap(),
        );
        assert_roundtrip(
            FilterValue::String("2024-01-02".into()),
            &Type::DATE_ARRAY,
            ::chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
        );
        assert_roundtrip(
            FilterValue::String("03:04:05".into()),
            &Type::TIME_ARRAY,
            ::chrono::NaiveTime::from_hms_opt(3, 4, 5).unwrap(),
        );
    }

    #[test]
    fn enum_array_binds_string_elements_verbatim() {
        let mood = Type::new(
            "mood".into(),
            990_001,
            Kind::Enum(vec!["happy".into(), "sad".into()]),
            "public".into(),
        );
        let mood_array = Type::new("_mood".into(), 990_002, Kind::Array(mood), "public".into());
        let (res, buf) = bind_array(vec![FilterValue::String("happy".into())], &mood_array);
        res.unwrap();
        // ndim=1, has_null=0, elem oid, dim len 1, lbound 1, elem len 5, body.
        let mut expected = Vec::new();
        for v in [1i32, 0, 990_001, 1, 1, 5] {
            expected.extend_from_slice(&v.to_be_bytes());
        }
        expected.extend_from_slice(b"happy");
        assert_eq!(&buf[..], &expected[..]);
    }

    #[test]
    fn string_array_invalid_values_error() {
        let cases = [
            ("not-a-uuid", Type::UUID_ARRAY),
            ("2024-13-45", Type::DATE_ARRAY),
            ("nope", Type::DATE_ARRAY),
            ("25:61:61", Type::TIME_ARRAY),
            ("2024-01-02", Type::TIMESTAMPTZ_ARRAY),
            ("garbage", Type::TIMESTAMP_ARRAY),
        ];
        for (s, ty) in cases {
            let (res, _) = bind_array(vec![FilterValue::String(s.into())], &ty);
            assert!(res.is_err(), "{s:?} into {ty:?} must error");
        }
    }

    #[test]
    fn list_bind_error_points_to_alternatives() {
        let err = filter_value_to_sql(&FilterValue::List(vec![]))
            .err()
            .unwrap();
        let msg = err.to_string();
        assert!(
            msg.contains("Filter::In") && msg.contains("ANY($1)"),
            "{msg}"
        );
    }

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
