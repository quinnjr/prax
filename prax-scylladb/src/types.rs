//! Type conversions between Prax and ScyllaDB/CQL types.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Timelike, Utc};
use rust_decimal::Decimal;
use scylla::frame::response::result::CqlValue;
#[allow(unused_imports)]
use scylla::serialize::value::SerializeValue;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::error::{ScyllaError, ScyllaResult};
use prax_query::base64::{Engine as _, engine::general_purpose::STANDARD};
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
    /// `TimeUUID`.
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
                "Cannot convert {self:?} to i64"
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
                "Cannot convert {self:?} to f64"
            ))),
        }
    }

    /// Try to convert to string.
    pub fn as_string(&self) -> ScyllaResult<String> {
        match self {
            Self::Text(v) => Ok(v.clone()),
            Self::Uuid(v) | Self::TimeUuid(v) => Ok(v.to_string()),
            _ => Err(ScyllaError::type_conversion(format!(
                "Cannot convert {self:?} to string"
            ))),
        }
    }

    /// Try to convert to bool.
    pub fn as_bool(&self) -> ScyllaResult<bool> {
        match self {
            Self::Boolean(v) => Ok(*v),
            _ => Err(ScyllaError::type_conversion(format!(
                "Cannot convert {self:?} to bool"
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
            FilterValue::List(arr) | FilterValue::Array(arr) => {
                let values: ScyllaResult<Vec<CqlValue>> =
                    arr.iter().map(ToCqlValue::to_cql).collect();
                Ok(CqlValue::List(values?))
            }
            FilterValue::Bytes(b) => Ok(CqlValue::Blob(b.clone())),
            _ => Err(ScyllaError::TypeConversion(
                "unsupported FilterValue variant".into(),
            )),
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
            ScyllaValue::Date(v) => {
                // CQL wire format: `date` is an unsigned 32-bit count of days
                // since 1970-01-01 offset by 2^31, i.e. raw = days_since_unix + 2^31.
                // chrono's `num_days_from_ce()` counts days since 0001-01-01
                // (1970-01-01 is day 719_163 CE), so convert CE -> Unix days
                // with `- 719_163` and then add the 2^31 offset.
                // Ref: Apache Cassandra native protocol spec, `date` type:
                // https://github.com/apache/cassandra/blob/trunk/doc/native_protocol_v4.spec
                Ok(CqlValue::Date(scylla::frame::value::CqlDate(
                    (i64::from(v.num_days_from_ce() - 719_163) + (1i64 << 31)) as u32,
                )))
            }
            ScyllaValue::Time(v) => {
                let nanos = i64::from(v.num_seconds_from_midnight()) * 1_000_000_000
                    + i64::from(v.nanosecond());
                Ok(CqlValue::Time(scylla::frame::value::CqlTime(nanos)))
            }
            ScyllaValue::Timestamp(v) => Ok(CqlValue::Timestamp(
                scylla::frame::value::CqlTimestamp(v.timestamp_millis()),
            )),
            ScyllaValue::Duration(v) => Ok(CqlValue::Duration(*v)),
            ScyllaValue::List(v) => {
                let values: ScyllaResult<Vec<CqlValue>> =
                    v.iter().map(ToCqlValue::to_cql).collect();
                Ok(CqlValue::List(values?))
            }
            ScyllaValue::Set(v) => {
                let values: ScyllaResult<Vec<CqlValue>> =
                    v.iter().map(ToCqlValue::to_cql).collect();
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

/// Convert a CQL decimal (two's-complement big-endian mantissa plus a
/// base-10 scale, i.e. value = mantissa * 10^-scale) to `rust_decimal`.
/// Returns `None` when the value is not representable: a mantissa wider
/// than 128 bits, or a magnitude/scale outside `rust_decimal`'s range.
fn cql_decimal_to_decimal(v: &scylla::frame::value::CqlDecimal) -> Option<Decimal> {
    let (bytes, scale) = v.as_signed_be_bytes_slice_and_exponent();
    if bytes.len() > 16 {
        return None;
    }
    // Sign-extend the big-endian mantissa into a 16-byte buffer.
    let mut buf = if bytes.first().is_some_and(|b| b & 0x80 != 0) {
        [0xFFu8; 16]
    } else {
        [0u8; 16]
    };
    buf[16 - bytes.len()..].copy_from_slice(bytes);
    let mantissa = i128::from_be_bytes(buf);
    if scale >= 0 {
        Decimal::try_from_i128_with_scale(mantissa, u32::try_from(scale).ok()?).ok()
    } else {
        // value = mantissa * 10^(-scale); represent it at scale 0
        let factor = 10i128.checked_pow(u32::try_from(scale.checked_neg()?).ok()?)?;
        Some(Decimal::from(mantissa.checked_mul(factor)?))
    }
}

/// Convert `CqlValue` to `ScyllaValue`.
///
/// Note: values that cannot be represented by `ScyllaValue` — an
/// out-of-range date/time/timestamp, a decimal whose mantissa or scale
/// exceeds `rust_decimal`'s range, or a genuinely unsupported type
/// (tuple, UDT, varint) — decode to [`ScyllaValue::Null`] and are
/// therefore indistinguishable from a genuine CQL `NULL`. These
/// fallbacks are logged at `debug` level so silent data loss can be
/// traced when needed.
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
            CqlValue::Date(v) => {
                // Inverse of the `to_cql` write path above: a raw CQL date is
                // days_since_1970 + 2^31, so CE days = raw - 2^31 + 719_163.
                if let Some(date) = i32::try_from(i64::from(v.0) - (1i64 << 31) + 719_163)
                    .ok()
                    .and_then(NaiveDate::from_num_days_from_ce_opt)
                {
                    ScyllaValue::Date(date)
                } else {
                    tracing::debug!(
                        raw = v.0,
                        "CQL date is out of chrono's representable range; decoding as Null"
                    );
                    ScyllaValue::Null
                }
            }
            CqlValue::Time(v) => {
                // CQL `time` is nanoseconds since midnight.
                let secs = u32::try_from(v.0.div_euclid(1_000_000_000)).ok();
                let nanos = u32::try_from(v.0.rem_euclid(1_000_000_000)).ok();
                if let Some(time) = secs
                    .zip(nanos)
                    .and_then(|(s, n)| NaiveTime::from_num_seconds_from_midnight_opt(s, n))
                {
                    ScyllaValue::Time(time)
                } else {
                    tracing::debug!(
                        raw = v.0,
                        "CQL time is out of chrono's representable range; decoding as Null"
                    );
                    ScyllaValue::Null
                }
            }
            CqlValue::Timestamp(v) => {
                // CQL `timestamp` is milliseconds since the Unix epoch.
                if let Some(timestamp) = DateTime::from_timestamp_millis(v.0) {
                    ScyllaValue::Timestamp(timestamp)
                } else {
                    tracing::debug!(
                        raw = v.0,
                        "CQL timestamp is out of chrono's representable range; \
                             decoding as Null"
                    );
                    ScyllaValue::Null
                }
            }
            CqlValue::Decimal(v) => {
                if let Some(decimal) = cql_decimal_to_decimal(&v) {
                    ScyllaValue::Decimal(decimal)
                } else {
                    tracing::debug!(
                        "CQL decimal is not representable as rust_decimal; decoding as Null"
                    );
                    ScyllaValue::Null
                }
            }
            CqlValue::Duration(v) => ScyllaValue::Duration(v),
            CqlValue::Counter(v) => ScyllaValue::BigInt(v.0),
            // Tuple, UserDefinedType and Varint have no `ScyllaValue`
            // representation yet; keep mapping them to Null rather than panic.
            _ => ScyllaValue::Null,
        }
    }
}

/// Convert `ScyllaValue` to JSON.
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
                .map_or(JsonValue::Null, JsonValue::Number),
            ScyllaValue::Double(v) => {
                serde_json::Number::from_f64(v).map_or(JsonValue::Null, JsonValue::Number)
            }
            ScyllaValue::Decimal(v) => JsonValue::String(v.to_string()),
            ScyllaValue::Text(v) => JsonValue::String(v),
            ScyllaValue::Blob(v) => JsonValue::String(STANDARD.encode(&v)),
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

/// Convenience trait for converting to CQL parameters.
pub trait ToCqlParams {
    /// Convert to a vector of CQL values.
    fn to_cql_params(&self) -> ScyllaResult<Vec<CqlValue>>;
}

impl<T: ToCqlValue> ToCqlParams for Vec<T> {
    fn to_cql_params(&self) -> ScyllaResult<Vec<CqlValue>> {
        self.iter().map(ToCqlValue::to_cql).collect()
    }
}

impl<T: ToCqlValue> ToCqlParams for &[T] {
    fn to_cql_params(&self) -> ScyllaResult<Vec<CqlValue>> {
        self.iter().map(ToCqlValue::to_cql).collect()
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
    fn bytes_and_array_convert_to_cql() {
        assert!(matches!(
            FilterValue::Bytes(vec![0, 255]).to_cql().unwrap(),
            CqlValue::Blob(ref b) if b == &[0u8, 255]
        ));
        let list = FilterValue::Array(vec![FilterValue::Int(1), FilterValue::String("a".into())])
            .to_cql()
            .unwrap();
        assert!(matches!(
            list,
            CqlValue::List(ref items)
                if matches!(items.as_slice(), [CqlValue::BigInt(1), CqlValue::Text(t)] if t == "a")
        ));
    }

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

    #[test]
    fn test_date_wire_encoding() {
        use scylla::frame::value::CqlDate;

        // 1970-01-01 must encode to exactly 2^31.
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let cql = ScyllaValue::Date(epoch).to_cql().unwrap();
        assert!(matches!(cql, CqlValue::Date(CqlDate(d)) if d == 1u32 << 31));

        // A modern date: 2000-01-01 is 10_957 days after the Unix epoch.
        let modern = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        let cql = ScyllaValue::Date(modern).to_cql().unwrap();
        assert!(matches!(cql, CqlValue::Date(CqlDate(d)) if d == (1u32 << 31) + 10_957));

        // Write -> read round trip, including a pre-epoch date.
        for date in [
            epoch,
            modern,
            NaiveDate::from_ymd_opt(1900, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        ] {
            let cql = ScyllaValue::Date(date).to_cql().unwrap();
            assert!(matches!(ScyllaValue::from(cql), ScyllaValue::Date(d) if d == date));
        }
    }

    #[test]
    fn test_cql_value_read_mappings() {
        use scylla::frame::value::{
            Counter, CqlDate, CqlDecimal, CqlDuration, CqlTime, CqlTimestamp,
        };

        // Date: raw = days_since_1970 + 2^31, so 2^31 is the Unix epoch.
        let v = ScyllaValue::from(CqlValue::Date(CqlDate(1 << 31)));
        assert!(
            matches!(v, ScyllaValue::Date(d) if d == NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        );

        // Time: nanoseconds since midnight (01:02:03.0045).
        let v = ScyllaValue::from(CqlValue::Time(CqlTime(3_723_004_500_000)));
        assert!(
            matches!(v, ScyllaValue::Time(t) if t == NaiveTime::from_hms_nano_opt(1, 2, 3, 4_500_000).unwrap())
        );

        // Timestamp: milliseconds since the Unix epoch.
        let v = ScyllaValue::from(CqlValue::Timestamp(CqlTimestamp(1_700_000_000_000)));
        assert!(
            matches!(v, ScyllaValue::Timestamp(ts) if ts.timestamp_millis() == 1_700_000_000_000)
        );

        // Decimal: two's-complement big-endian mantissa + base-10 scale.
        // 12345 * 10^-2 = 123.45
        let v = ScyllaValue::from(CqlValue::Decimal(
            CqlDecimal::from_signed_be_bytes_and_exponent(vec![0x30, 0x39], 2),
        ));
        assert!(matches!(v, ScyllaValue::Decimal(d) if d == Decimal::new(12345, 2)));
        // Negative mantissa: 0xCFC7 = -12345 -> -123.45
        let v = ScyllaValue::from(CqlValue::Decimal(
            CqlDecimal::from_signed_be_bytes_and_exponent(vec![0xCF, 0xC7], 2),
        ));
        assert!(matches!(v, ScyllaValue::Decimal(d) if d == Decimal::new(-12345, 2)));

        // Duration passes straight through.
        let v = ScyllaValue::from(CqlValue::Duration(CqlDuration {
            months: 1,
            days: 2,
            nanoseconds: 3,
        }));
        assert!(
            matches!(v, ScyllaValue::Duration(d) if d.months == 1 && d.days == 2 && d.nanoseconds == 3)
        );

        // Counter is a 64-bit integer.
        let v = ScyllaValue::from(CqlValue::Counter(Counter(42)));
        assert!(matches!(v, ScyllaValue::BigInt(42)));

        // Genuinely unsupported types still map to Null instead of panicking.
        assert!(ScyllaValue::from(CqlValue::Tuple(vec![Some(CqlValue::Int(1))])).is_null());
        assert!(
            ScyllaValue::from(CqlValue::UserDefinedType {
                keyspace: "ks".into(),
                type_name: "udt".into(),
                fields: vec![],
            })
            .is_null()
        );
    }
}
