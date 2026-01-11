//! Type conversions between Prax and ScyllaDB/CQL types.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Timelike, Utc};
use rust_decimal::Decimal;
use scylla::frame::response::result::CqlValue;
#[allow(unused_imports)]
use scylla::serialize::value::SerializeValue;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::error::{ScyllaError, ScyllaResult};
use prax_query::filter::FilterValue;

/// A wrapper type for CQL values that can be used in queries.
#[derive(Debug, Clone)]
pub enum ScyllaValue {
    /// Null value.
    Null,
    /// Boolean value.
    Boolean(bool),
    /// 8-bit signed integer.
    TinyInt(i8),
    /// 16-bit signed integer.
    SmallInt(i16),
    /// 32-bit signed integer.
    Int(i32),
    /// 64-bit signed integer.
    BigInt(i64),
    /// 32-bit floating point.
    Float(f32),
    /// 64-bit floating point.
    Double(f64),
    /// Decimal number.
    Decimal(Decimal),
    /// Text string.
    Text(String),
    /// Binary blob.
    Blob(Vec<u8>),
    /// UUID.
    Uuid(Uuid),
    /// TimeUUID.
    TimeUuid(Uuid),
    /// Date (without time).
    Date(NaiveDate),
    /// Time (without date).
    Time(NaiveTime),
    /// Timestamp (date and time).
    Timestamp(DateTime<Utc>),
    /// Duration.
    Duration(scylla::frame::value::CqlDuration),
    /// List of values.
    List(Vec<ScyllaValue>),
    /// Set of values.
    Set(Vec<ScyllaValue>),
    /// Map of key-value pairs.
    Map(Vec<(ScyllaValue, ScyllaValue)>),
    /// Inet address.
    Inet(std::net::IpAddr),
}

impl ScyllaValue {
    /// Check if value is null.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Try to convert to i64.
    pub fn as_i64(&self) -> ScyllaResult<i64> {
        match self {
            Self::TinyInt(v) => Ok(i64::from(*v)),
            Self::SmallInt(v) => Ok(i64::from(*v)),
            Self::Int(v) => Ok(i64::from(*v)),
            Self::BigInt(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion(format!(
                "Cannot convert {:?} to i64",
                self
            ))),
        }
    }

    /// Try to convert to f64.
    pub fn as_f64(&self) -> ScyllaResult<f64> {
        match self {
            Self::Float(v) => Ok(f64::from(*v)),
            Self::Double(v) => Ok(*v),
            Self::TinyInt(v) => Ok(f64::from(*v)),
            Self::SmallInt(v) => Ok(f64::from(*v)),
            Self::Int(v) => Ok(f64::from(*v)),
            Self::BigInt(v) => Ok(*v as f64),
            _ => Err(ScyllaError::type_conversion(format!(
                "Cannot convert {:?} to f64",
                self
            ))),
        }
    }

    /// Try to convert to string.
    pub fn as_string(&self) -> ScyllaResult<String> {
        match self {
            Self::Text(v) => Ok(v.clone()),
            Self::Uuid(v) | Self::TimeUuid(v) => Ok(v.to_string()),
            _ => Err(ScyllaError::type_conversion(format!(
                "Cannot convert {:?} to string",
                self
            ))),
        }
    }

    /// Try to convert to bool.
    pub fn as_bool(&self) -> ScyllaResult<bool> {
        match self {
            Self::Boolean(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion(format!(
                "Cannot convert {:?} to bool",
                self
            ))),
        }
    }
}

/// Trait for types that can be converted to CQL values.
pub trait ToCqlValue {
    /// Convert to a CQL-compatible value.
    fn to_cql(&self) -> ScyllaResult<CqlValue>;
}

impl ToCqlValue for FilterValue {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        match self {
            FilterValue::Null => Ok(CqlValue::Empty),
            FilterValue::Bool(v) => Ok(CqlValue::Boolean(*v)),
            FilterValue::Int(v) => Ok(CqlValue::BigInt(*v)),
            FilterValue::Float(v) => Ok(CqlValue::Double(*v)),
            FilterValue::String(v) => Ok(CqlValue::Text(v.clone())),
            FilterValue::Json(json) => {
                // Convert JSON to text representation
                Ok(CqlValue::Text(
                    serde_json::to_string(json).unwrap_or_default(),
                ))
            }
            FilterValue::List(arr) => {
                let values: ScyllaResult<Vec<CqlValue>> = arr.iter().map(|v| v.to_cql()).collect();
                Ok(CqlValue::List(values?))
            }
        }
    }
}

impl ToCqlValue for ScyllaValue {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        match self {
            ScyllaValue::Null => Ok(CqlValue::Empty),
            ScyllaValue::Boolean(v) => Ok(CqlValue::Boolean(*v)),
            ScyllaValue::TinyInt(v) => Ok(CqlValue::TinyInt(*v)),
            ScyllaValue::SmallInt(v) => Ok(CqlValue::SmallInt(*v)),
            ScyllaValue::Int(v) => Ok(CqlValue::Int(*v)),
            ScyllaValue::BigInt(v) => Ok(CqlValue::BigInt(*v)),
            ScyllaValue::Float(v) => Ok(CqlValue::Float(*v)),
            ScyllaValue::Double(v) => Ok(CqlValue::Double(*v)),
            ScyllaValue::Decimal(v) => {
                // Convert Decimal to CqlDecimal
                let mantissa = v.mantissa();
                let scale = v.scale();
                let bytes = mantissa.to_be_bytes().to_vec();
                Ok(CqlValue::Decimal(
                    scylla::frame::value::CqlDecimal::from_signed_be_bytes_and_exponent(
                        bytes,
                        -(scale as i32),
                    ),
                ))
            }
            ScyllaValue::Text(v) => Ok(CqlValue::Text(v.clone())),
            ScyllaValue::Blob(v) => Ok(CqlValue::Blob(v.clone())),
            ScyllaValue::Uuid(v) => Ok(CqlValue::Uuid(*v)),
            ScyllaValue::TimeUuid(v) => Ok(CqlValue::Timeuuid(
                scylla::frame::value::CqlTimeuuid::from(*v),
            )),
            ScyllaValue::Date(v) => Ok(CqlValue::Date(scylla::frame::value::CqlDate(
                v.num_days_from_ce() as u32,
            ))),
            ScyllaValue::Time(v) => {
                let nanos = v.num_seconds_from_midnight() as i64 * 1_000_000_000
                    + i64::from(v.nanosecond());
                Ok(CqlValue::Time(scylla::frame::value::CqlTime(nanos)))
            }
            ScyllaValue::Timestamp(v) => Ok(CqlValue::Timestamp(
                scylla::frame::value::CqlTimestamp(v.timestamp_millis()),
            )),
            ScyllaValue::Duration(v) => Ok(CqlValue::Duration(*v)),
            ScyllaValue::List(v) => {
                let values: ScyllaResult<Vec<CqlValue>> = v.iter().map(|x| x.to_cql()).collect();
                Ok(CqlValue::List(values?))
            }
            ScyllaValue::Set(v) => {
                let values: ScyllaResult<Vec<CqlValue>> = v.iter().map(|x| x.to_cql()).collect();
                Ok(CqlValue::Set(values?))
            }
            ScyllaValue::Map(v) => {
                let pairs: ScyllaResult<Vec<(CqlValue, CqlValue)>> = v
                    .iter()
                    .map(|(k, val)| Ok((k.to_cql()?, val.to_cql()?)))
                    .collect();
                Ok(CqlValue::Map(pairs?))
            }
            ScyllaValue::Inet(v) => Ok(CqlValue::Inet(*v)),
        }
    }
}

/// Convert CqlValue to ScyllaValue.
impl From<CqlValue> for ScyllaValue {
    fn from(value: CqlValue) -> Self {
        match value {
            CqlValue::Empty => ScyllaValue::Null,
            CqlValue::Boolean(v) => ScyllaValue::Boolean(v),
            CqlValue::TinyInt(v) => ScyllaValue::TinyInt(v),
            CqlValue::SmallInt(v) => ScyllaValue::SmallInt(v),
            CqlValue::Int(v) => ScyllaValue::Int(v),
            CqlValue::BigInt(v) => ScyllaValue::BigInt(v),
            CqlValue::Float(v) => ScyllaValue::Float(v),
            CqlValue::Double(v) => ScyllaValue::Double(v),
            CqlValue::Text(v) | CqlValue::Ascii(v) => ScyllaValue::Text(v),
            CqlValue::Blob(v) => ScyllaValue::Blob(v),
            CqlValue::Uuid(v) => ScyllaValue::Uuid(v),
            CqlValue::Timeuuid(v) => ScyllaValue::TimeUuid(v.into()),
            CqlValue::Inet(v) => ScyllaValue::Inet(v),
            CqlValue::List(v) => ScyllaValue::List(v.into_iter().map(Into::into).collect()),
            CqlValue::Set(v) => ScyllaValue::Set(v.into_iter().map(Into::into).collect()),
            CqlValue::Map(v) => ScyllaValue::Map(
                v.into_iter()
                    .map(|(k, val)| (k.into(), val.into()))
                    .collect(),
            ),
            _ => ScyllaValue::Null, // Handle other types as null for now
        }
    }
}

/// Convert ScyllaValue to JSON.
impl From<ScyllaValue> for JsonValue {
    fn from(value: ScyllaValue) -> Self {
        match value {
            ScyllaValue::Null => JsonValue::Null,
            ScyllaValue::Boolean(v) => JsonValue::Bool(v),
            ScyllaValue::TinyInt(v) => JsonValue::Number(v.into()),
            ScyllaValue::SmallInt(v) => JsonValue::Number(v.into()),
            ScyllaValue::Int(v) => JsonValue::Number(v.into()),
            ScyllaValue::BigInt(v) => JsonValue::Number(v.into()),
            ScyllaValue::Float(v) => serde_json::Number::from_f64(f64::from(v))
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            ScyllaValue::Double(v) => serde_json::Number::from_f64(v)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            ScyllaValue::Decimal(v) => JsonValue::String(v.to_string()),
            ScyllaValue::Text(v) => JsonValue::String(v),
            ScyllaValue::Blob(v) => JsonValue::String(base64_encode(&v)),
            ScyllaValue::Uuid(v) | ScyllaValue::TimeUuid(v) => JsonValue::String(v.to_string()),
            ScyllaValue::Date(v) => JsonValue::String(v.to_string()),
            ScyllaValue::Time(v) => JsonValue::String(v.to_string()),
            ScyllaValue::Timestamp(v) => JsonValue::String(v.to_rfc3339()),
            ScyllaValue::Duration(v) => {
                JsonValue::String(format!("{}mo{}d{}ns", v.months, v.days, v.nanoseconds))
            }
            ScyllaValue::List(v) | ScyllaValue::Set(v) => {
                JsonValue::Array(v.into_iter().map(Into::into).collect())
            }
            ScyllaValue::Map(v) => {
                let obj: serde_json::Map<String, JsonValue> = v
                    .into_iter()
                    .filter_map(|(k, val)| {
                        let key = match k {
                            ScyllaValue::Text(s) => Some(s),
                            ScyllaValue::Uuid(u) | ScyllaValue::TimeUuid(u) => Some(u.to_string()),
                            ScyllaValue::Int(i) => Some(i.to_string()),
                            ScyllaValue::BigInt(i) => Some(i.to_string()),
                            _ => None,
                        };
                        key.map(|k| (k, val.into()))
                    })
                    .collect();
                JsonValue::Object(obj)
            }
            ScyllaValue::Inet(v) => JsonValue::String(v.to_string()),
        }
    }
}

/// Simple base64 encoding for blob values.
fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let mut n = 0u32;
        for (i, &byte) in chunk.iter().enumerate() {
            n |= u32::from(byte) << (16 - 8 * i);
        }
        for i in 0..=chunk.len() {
            let idx = ((n >> (18 - 6 * i)) & 0x3f) as usize;
            let c = BASE64_CHARS.as_bytes()[idx] as char;
            result.push(c);
        }
    }
    let padding = (3 - data.len() % 3) % 3;
    for _ in 0..padding {
        result.push('=');
    }
    result
}

const BASE64_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Convenience trait for converting to CQL parameters.
pub trait ToCqlParams {
    /// Convert to a vector of CQL values.
    fn to_cql_params(&self) -> ScyllaResult<Vec<CqlValue>>;
}

impl<T: ToCqlValue> ToCqlParams for Vec<T> {
    fn to_cql_params(&self) -> ScyllaResult<Vec<CqlValue>> {
        self.iter().map(|v| v.to_cql()).collect()
    }
}

impl<T: ToCqlValue> ToCqlParams for &[T] {
    fn to_cql_params(&self) -> ScyllaResult<Vec<CqlValue>> {
        self.iter().map(|v| v.to_cql()).collect()
    }
}

// Implement for common types
impl ToCqlValue for i32 {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::Int(*self))
    }
}

impl ToCqlValue for i64 {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::BigInt(*self))
    }
}

impl ToCqlValue for f64 {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::Double(*self))
    }
}

impl ToCqlValue for bool {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::Boolean(*self))
    }
}

impl ToCqlValue for String {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::Text(self.clone()))
    }
}

impl ToCqlValue for &str {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::Text((*self).to_string()))
    }
}

impl ToCqlValue for Uuid {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        Ok(CqlValue::Uuid(*self))
    }
}

impl<T: ToCqlValue> ToCqlValue for Option<T> {
    fn to_cql(&self) -> ScyllaResult<CqlValue> {
        match self {
            Some(v) => v.to_cql(),
            None => Ok(CqlValue::Empty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_value_to_cql() {
        let int_val = FilterValue::Int(42);
        assert!(matches!(int_val.to_cql().unwrap(), CqlValue::BigInt(42)));

        let str_val = FilterValue::String("hello".into());
        assert!(matches!(
            str_val.to_cql().unwrap(),
            CqlValue::Text(s) if s == "hello"
        ));

        let null_val = FilterValue::Null;
        assert!(matches!(null_val.to_cql().unwrap(), CqlValue::Empty));
    }

    #[test]
    fn test_scylla_value_conversions() {
        let val = ScyllaValue::BigInt(100);
        assert_eq!(val.as_i64().unwrap(), 100);

        let val = ScyllaValue::Double(3.14);
        assert!((val.as_f64().unwrap() - 3.14).abs() < f64::EPSILON);

        let val = ScyllaValue::Text("test".into());
        assert_eq!(val.as_string().unwrap(), "test");

        let val = ScyllaValue::Boolean(true);
        assert!(val.as_bool().unwrap());
    }

    #[test]
    fn test_scylla_value_to_json() {
        let val = ScyllaValue::Int(42);
        let json: JsonValue = val.into();
        assert_eq!(json, JsonValue::Number(42.into()));

        let val = ScyllaValue::Text("hello".into());
        let json: JsonValue = val.into();
        assert_eq!(json, JsonValue::String("hello".into()));

        let val = ScyllaValue::List(vec![ScyllaValue::Int(1), ScyllaValue::Int(2)]);
        let json: JsonValue = val.into();
        assert!(json.is_array());
    }
}
