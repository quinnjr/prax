//! Type conversion utilities for DuckDB.

use duckdb::types::{ToSqlOutput, Value, ValueRef};
use prax_query::filter::FilterValue;
use serde_json::Value as JsonValue;

/// Convert a FilterValue to a DuckDB Value.
pub fn filter_value_to_duckdb(value: &FilterValue) -> Value {
    match value {
        FilterValue::Null => Value::Null,
        FilterValue::Bool(b) => Value::Boolean(*b),
        FilterValue::Int(i) => Value::BigInt(*i),
        FilterValue::Float(f) => Value::Double(*f),
        FilterValue::String(s) => Value::Text(s.clone()),
        FilterValue::Json(j) => Value::Text(j.to_string()),
        FilterValue::List(list) => {
            // DuckDB supports arrays, but for simplicity convert to JSON
            let json_array: Vec<JsonValue> = list.iter().map(filter_value_to_json).collect();
            Value::Text(serde_json::to_string(&json_array).unwrap_or_default())
        }
    }
}

/// Convert a FilterValue to a JSON value.
pub fn filter_value_to_json(value: &FilterValue) -> JsonValue {
    match value {
        FilterValue::Null => JsonValue::Null,
        FilterValue::Bool(b) => JsonValue::Bool(*b),
        FilterValue::Int(i) => JsonValue::Number((*i).into()),
        FilterValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        FilterValue::String(s) => JsonValue::String(s.clone()),
        FilterValue::Json(j) => j.clone(),
        FilterValue::List(list) => JsonValue::Array(list.iter().map(filter_value_to_json).collect()),
    }
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
        Value::Text(s) => {
            // Try to parse as JSON first
            if let Ok(json) = serde_json::from_str(&s) {
                json
            } else {
                JsonValue::String(s)
            }
        }
        Value::Blob(bytes) => {
            // Encode as hex string (simpler than base64, no extra dependency)
            JsonValue::String(bytes.iter().map(|b| format!("{:02x}", b)).collect())
        }
        Value::Date32(days) => {
            // Days since epoch
            let date = chrono::NaiveDate::from_num_days_from_ce_opt(days + 719163);
            match date {
                Some(d) => JsonValue::String(d.to_string()),
                None => JsonValue::Null,
            }
        }
        Value::Time64(..) => {
            // Time as string
            JsonValue::String(format!("{:?}", value))
        }
        Value::Timestamp(..) => {
            // Timestamp as string
            JsonValue::String(format!("{:?}", value))
        }
        Value::Interval { .. } => {
            // Interval as string
            JsonValue::String(format!("{:?}", value))
        }
        Value::List(list) => {
            JsonValue::Array(list.into_iter().map(duckdb_value_to_json).collect())
        }
        Value::Enum(e) => JsonValue::String(e),
        Value::Struct(fields) => {
            // OrderedMap uses .iter(), not into_iter()
            let obj: serde_json::Map<String, JsonValue> = fields
                .iter()
                .map(|(k, v)| (k.clone(), duckdb_value_to_json(v.clone())))
                .collect();
            JsonValue::Object(obj)
        }
        Value::Array(arr) => {
            JsonValue::Array(arr.into_iter().map(duckdb_value_to_json).collect())
        }
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
        ValueRef::Text(bytes) => {
            let s = String::from_utf8_lossy(bytes).to_string();
            if let Ok(json) = serde_json::from_str(&s) {
                json
            } else {
                JsonValue::String(s)
            }
        }
        ValueRef::Blob(bytes) => {
            // Encode as hex string
            JsonValue::String(bytes.iter().map(|b| format!("{:02x}", b)).collect())
        }
        ValueRef::Date32(days) => {
            let date = chrono::NaiveDate::from_num_days_from_ce_opt(days + 719163);
            match date {
                Some(d) => JsonValue::String(d.to_string()),
                None => JsonValue::Null,
            }
        }
        ValueRef::Time64(..) => JsonValue::String(format!("{:?}", value)),
        ValueRef::Timestamp(..) => JsonValue::String(format!("{:?}", value)),
        ValueRef::Interval { .. } => JsonValue::String(format!("{:?}", value)),
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
        match self.0 {
            FilterValue::Null => Ok(ToSqlOutput::Owned(Value::Null)),
            FilterValue::Bool(b) => Ok(ToSqlOutput::Owned(Value::Boolean(*b))),
            FilterValue::Int(i) => Ok(ToSqlOutput::Owned(Value::BigInt(*i))),
            FilterValue::Float(f) => Ok(ToSqlOutput::Owned(Value::Double(*f))),
            FilterValue::String(s) => Ok(ToSqlOutput::Owned(Value::Text(s.clone()))),
            FilterValue::Json(j) => Ok(ToSqlOutput::Owned(Value::Text(j.to_string()))),
            FilterValue::List(list) => {
                let json_array: Vec<JsonValue> = list.iter().map(filter_value_to_json).collect();
                Ok(ToSqlOutput::Owned(Value::Text(
                    serde_json::to_string(&json_array).unwrap_or_default(),
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_filter_value_to_json() {
        assert_eq!(filter_value_to_json(&FilterValue::Null), JsonValue::Null);
        assert_eq!(
            filter_value_to_json(&FilterValue::Bool(true)),
            JsonValue::Bool(true)
        );
        assert_eq!(
            filter_value_to_json(&FilterValue::Int(42)),
            JsonValue::Number(42.into())
        );
        assert_eq!(
            filter_value_to_json(&FilterValue::String("test".to_string())),
            JsonValue::String("test".to_string())
        );
    }
}
