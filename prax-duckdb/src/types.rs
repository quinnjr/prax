//! Type conversion utilities for DuckDB.

use duckdb::types::{TimeUnit, ToSqlOutput, Value, ValueRef};
use prax_query::filter::FilterValue;
use serde_json::Value as JsonValue;

/// Convert a FilterValue to a DuckDB Value.
#[deny(clippy::wildcard_enum_match_arm)]
pub fn filter_value_to_duckdb(value: &FilterValue) -> Value {
    match value {
        FilterValue::Null => Value::Null,
        FilterValue::Bool(b) => Value::Boolean(*b),
        FilterValue::Int(i) => Value::BigInt(*i),
        FilterValue::Float(f) => Value::Double(*f),
        FilterValue::String(s) => Value::Text(s.clone()),
        FilterValue::Json(j) => Value::Text(j.to_string()),
        FilterValue::Bytes(b) => Value::Blob(b.clone()),
        // Arrays bind as JSON text; SQL converts them (see scalar-lists design).
        FilterValue::List(_) | FilterValue::Array(_) => {
            Value::Text(value.to_json_value().to_string())
        }
        // `FilterValue` is `#[non_exhaustive]`, so a catch-all is required.
        // Every current variant is listed above; when a new variant is added,
        // `#[deny(clippy::wildcard_enum_match_arm)]` makes this arm fail
        // clippy until the variant gets its own arm.
        _ => Value::Text(value.to_json_value().to_string()),
    }
}

/// Split a unit-denominated count into whole seconds and sub-second nanos.
fn units_to_secs_nanos(unit: TimeUnit, value: i64) -> (i64, u32) {
    match unit {
        TimeUnit::Second => (value, 0),
        TimeUnit::Millisecond => (
            value.div_euclid(1_000),
            (value.rem_euclid(1_000) * 1_000_000) as u32,
        ),
        TimeUnit::Microsecond => (
            value.div_euclid(1_000_000),
            (value.rem_euclid(1_000_000) * 1_000) as u32,
        ),
        TimeUnit::Nanosecond => (
            value.div_euclid(1_000_000_000),
            value.rem_euclid(1_000_000_000) as u32,
        ),
    }
}

/// Format a DuckDB timestamp (units since the Unix epoch) as `YYYY-MM-DD HH:MM:SS[.f]`.
fn format_timestamp(unit: TimeUnit, value: i64) -> Option<String> {
    let (secs, nanos) = units_to_secs_nanos(unit, value);
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S%.f").to_string())
}

/// Format a DuckDB time (units since midnight) as `HH:MM:SS[.f]`.
fn format_time64(unit: TimeUnit, value: i64) -> Option<String> {
    let (secs, nanos) = units_to_secs_nanos(unit, value);
    chrono::NaiveTime::from_num_seconds_from_midnight_opt(u32::try_from(secs).ok()?, nanos)
        .map(|t| t.format("%H:%M:%S%.f").to_string())
}

/// Render a DuckDB interval as SQL interval text (e.g. `1 year 2 months 3 days 04:05:06.789`).
fn format_interval(months: i32, days: i32, nanos: i64) -> String {
    fn plural(value: i64, unit: &str) -> String {
        let suffix = if value == 1 || value == -1 { "" } else { "s" };
        format!("{} {}{}", value, unit, suffix)
    }

    let mut parts = Vec::new();
    let total_months = i64::from(months);
    let years = total_months / 12;
    let rem_months = total_months % 12;
    if years != 0 {
        parts.push(plural(years, "year"));
    }
    if rem_months != 0 {
        parts.push(plural(rem_months, "month"));
    }
    if days != 0 {
        parts.push(plural(i64::from(days), "day"));
    }
    if nanos != 0 {
        let negative = nanos < 0;
        let abs = nanos.unsigned_abs();
        let total_secs = abs / 1_000_000_000;
        let sub_nanos = abs % 1_000_000_000;
        let mut time = format!(
            "{}{:02}:{:02}:{:02}",
            if negative { "-" } else { "" },
            total_secs / 3600,
            (total_secs % 3600) / 60,
            total_secs % 60
        );
        if sub_nanos != 0 {
            let frac = format!("{:09}", sub_nanos);
            time.push('.');
            time.push_str(frac.trim_end_matches('0'));
        }
        parts.push(time);
    }
    if parts.is_empty() {
        "00:00:00".to_string()
    } else {
        parts.join(" ")
    }
}

/// Encode bytes as a lowercase hex string without per-byte allocation.
fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Convert a DuckDB Value to a JSON value.
pub fn duckdb_value_to_json(value: Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Boolean(b) => JsonValue::Bool(b),
        Value::TinyInt(i) => JsonValue::Number(i.into()),
        Value::SmallInt(i) => JsonValue::Number(i.into()),
        Value::Int(i) => JsonValue::Number(i.into()),
        Value::BigInt(i) => JsonValue::Number(i.into()),
        Value::HugeInt(i) => {
            // HugeInt is i128, convert to string for JSON
            JsonValue::String(i.to_string())
        }
        Value::UTinyInt(i) => JsonValue::Number(i.into()),
        Value::USmallInt(i) => JsonValue::Number(i.into()),
        Value::UInt(i) => JsonValue::Number(i.into()),
        Value::UBigInt(i) => JsonValue::Number(i.into()),
        Value::Float(f) => serde_json::Number::from_f64(f as f64)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::Double(f) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::Decimal(d) => {
            // Convert Decimal to string to preserve precision
            JsonValue::String(d.to_string())
        }
        // Text is returned as-is; JSON-typed columns are handled by the typed
        // get path, not by guessing at string contents here.
        Value::Text(s) => JsonValue::String(s),
        Value::Blob(bytes) => {
            // Encode as hex string (simpler than base64, no extra dependency)
            JsonValue::String(bytes_to_hex(&bytes))
        }
        Value::Date32(days) => {
            // Days since epoch; checked_add keeps extreme values from
            // overflowing i32.
            let date = days
                .checked_add(719_163)
                .and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
            match date {
                Some(d) => JsonValue::String(d.to_string()),
                None => JsonValue::Null,
            }
        }
        Value::Time64(unit, v) => {
            // Time as HH:MM:SS[.f]
            format_time64(unit, v).map_or(JsonValue::Null, JsonValue::String)
        }
        Value::Timestamp(unit, v) => {
            // Timestamp as ISO-8601 text
            format_timestamp(unit, v).map_or(JsonValue::Null, JsonValue::String)
        }
        Value::Interval {
            months,
            days,
            nanos,
        } => {
            // Interval as SQL interval text
            JsonValue::String(format_interval(months, days, nanos))
        }
        Value::List(list) => JsonValue::Array(list.into_iter().map(duckdb_value_to_json).collect()),
        Value::Enum(e) => JsonValue::String(e),
        Value::Struct(fields) => {
            // OrderedMap uses .iter(), not into_iter()
            let obj: serde_json::Map<String, JsonValue> = fields
                .iter()
                .map(|(k, v)| (k.clone(), duckdb_value_to_json(v.clone())))
                .collect();
            JsonValue::Object(obj)
        }
        Value::Array(arr) => JsonValue::Array(arr.into_iter().map(duckdb_value_to_json).collect()),
        Value::Map(map) => {
            // OrderedMap uses .iter(), not into_iter()
            let obj: serde_json::Map<String, JsonValue> = map
                .iter()
                .map(|(k, v)| (format!("{:?}", k), duckdb_value_to_json(v.clone())))
                .collect();
            JsonValue::Object(obj)
        }
        Value::Union(u) => duckdb_value_to_json(*u),
    }
}

/// Convert a DuckDB ValueRef to a JSON value.
///
/// For complex types (List, Struct, Map, etc.), we convert to owned Value first
/// since the Arrow-based API requires careful index handling.
pub fn duckdb_value_ref_to_json(value: ValueRef<'_>) -> JsonValue {
    match value {
        ValueRef::Null => JsonValue::Null,
        ValueRef::Boolean(b) => JsonValue::Bool(b),
        ValueRef::TinyInt(i) => JsonValue::Number(i.into()),
        ValueRef::SmallInt(i) => JsonValue::Number(i.into()),
        ValueRef::Int(i) => JsonValue::Number(i.into()),
        ValueRef::BigInt(i) => JsonValue::Number(i.into()),
        ValueRef::HugeInt(i) => JsonValue::String(i.to_string()),
        ValueRef::UTinyInt(i) => JsonValue::Number(i.into()),
        ValueRef::USmallInt(i) => JsonValue::Number(i.into()),
        ValueRef::UInt(i) => JsonValue::Number(i.into()),
        ValueRef::UBigInt(i) => JsonValue::Number(i.into()),
        ValueRef::Float(f) => serde_json::Number::from_f64(f as f64)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        ValueRef::Double(f) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        ValueRef::Decimal(d) => JsonValue::String(d.to_string()),
        // Text is returned as-is; JSON-typed columns are handled by the typed
        // get path, not by guessing at string contents here.
        ValueRef::Text(bytes) => JsonValue::String(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => {
            // Encode as hex string
            JsonValue::String(bytes_to_hex(bytes))
        }
        ValueRef::Date32(days) => {
            let date = days
                .checked_add(719_163)
                .and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
            match date {
                Some(d) => JsonValue::String(d.to_string()),
                None => JsonValue::Null,
            }
        }
        ValueRef::Time64(unit, v) => {
            format_time64(unit, v).map_or(JsonValue::Null, JsonValue::String)
        }
        ValueRef::Timestamp(unit, v) => {
            format_timestamp(unit, v).map_or(JsonValue::Null, JsonValue::String)
        }
        ValueRef::Interval {
            months,
            days,
            nanos,
        } => JsonValue::String(format_interval(months, days, nanos)),
        // For complex types, convert to owned Value and then to JSON
        ValueRef::List(..)
        | ValueRef::Enum(..)
        | ValueRef::Struct(..)
        | ValueRef::Array(..)
        | ValueRef::Map(..)
        | ValueRef::Union(..) => {
            // Use to_owned() to convert complex ValueRef types to Value
            duckdb_value_to_json(value.to_owned())
        }
    }
}

/// Wrapper for FilterValue to implement ToSql.
pub struct DuckDbParam<'a>(pub &'a FilterValue);

impl duckdb::ToSql for DuckDbParam<'_> {
    fn to_sql(&self) -> duckdb::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(filter_value_to_duckdb(self.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_array_convert_to_duckdb_values() {
        assert!(matches!(
            filter_value_to_duckdb(&FilterValue::Bytes(vec![0, 255])),
            Value::Blob(ref b) if b == &[0u8, 255]
        ));
        let array = FilterValue::Array(vec![FilterValue::Int(1), FilterValue::String("a".into())]);
        assert!(matches!(
            filter_value_to_duckdb(&array),
            Value::Text(ref t) if t == "[1,\"a\"]"
        ));
    }

    #[test]
    fn test_filter_value_to_duckdb() {
        assert!(matches!(
            filter_value_to_duckdb(&FilterValue::Null),
            Value::Null
        ));
        assert!(matches!(
            filter_value_to_duckdb(&FilterValue::Bool(true)),
            Value::Boolean(true)
        ));
        assert!(matches!(
            filter_value_to_duckdb(&FilterValue::Int(42)),
            Value::BigInt(42)
        ));
    }

    #[test]
    fn test_text_is_not_reparsed_as_json() {
        // Text values that happen to be JSON-parseable must stay strings.
        assert_eq!(
            duckdb_value_to_json(Value::Text("null".to_string())),
            JsonValue::String("null".to_string())
        );
        assert_eq!(
            duckdb_value_to_json(Value::Text("42".to_string())),
            JsonValue::String("42".to_string())
        );
        assert_eq!(
            duckdb_value_ref_to_json(ValueRef::Text(b"null")),
            JsonValue::String("null".to_string())
        );
        assert_eq!(
            duckdb_value_ref_to_json(ValueRef::Text(b"true")),
            JsonValue::String("true".to_string())
        );
    }

    #[test]
    fn test_timestamp_rendering() {
        // 2026-01-01 00:00:00 UTC
        let ts = Value::Timestamp(TimeUnit::Microsecond, 1_767_225_600_000_000);
        assert_eq!(
            duckdb_value_to_json(ts),
            JsonValue::String("2026-01-01 00:00:00".to_string())
        );
        let ts_ref = ValueRef::Timestamp(TimeUnit::Millisecond, 1_767_225_600_123);
        assert_eq!(
            duckdb_value_ref_to_json(ts_ref),
            JsonValue::String("2026-01-01 00:00:00.123".to_string())
        );
    }

    #[test]
    fn test_time64_rendering() {
        // 01:02:03 since midnight
        let time = Value::Time64(TimeUnit::Microsecond, 3_723_000_000);
        assert_eq!(
            duckdb_value_to_json(time),
            JsonValue::String("01:02:03".to_string())
        );
        let time_ref = ValueRef::Time64(TimeUnit::Microsecond, 3_723_456_789);
        assert_eq!(
            duckdb_value_ref_to_json(time_ref),
            JsonValue::String("01:02:03.456789".to_string())
        );
    }
}
