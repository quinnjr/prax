//! Type conversion utilities for MySQL.

use mysql_async::Value;
use serde_json::Value as JsonValue;

use prax_query::filter::FilterValue;

/// Convert a FilterValue to a MySQL Value.
pub fn filter_value_to_mysql(value: &FilterValue) -> Value {
    match value {
        FilterValue::Null => Value::NULL,
        FilterValue::Bool(b) => Value::from(*b),
        FilterValue::Int(i) => Value::from(*i),
        FilterValue::Float(f) => Value::from(*f),
        FilterValue::String(s) => Value::from(s.as_str()),
        FilterValue::Json(j) => Value::from(j.to_string()),
        FilterValue::Bytes(b) => Value::Bytes(b.clone()),
        // Lists and arrays serialise to JSON text.
        FilterValue::List(_) | FilterValue::Array(_) => {
            Value::from(value.to_json_value().to_string())
        }
        _ => Value::from(value.to_json_value().to_string()),
    }
}

/// Convert a MySQL Value to a JSON Value.
pub fn from_mysql_value(value: Value) -> JsonValue {
    match value {
        Value::NULL => JsonValue::Null,
        Value::Bytes(bytes) => {
            // Try to parse as UTF-8 string first
            match String::from_utf8(bytes.clone()) {
                Ok(s) => {
                    // Only parse as JSON when the text looks like a JSON object
                    // or array; scalars like "123"/"null"/"true" stay strings.
                    if s.starts_with('{') || s.starts_with('[') {
                        serde_json::from_str(&s).unwrap_or(JsonValue::String(s))
                    } else {
                        JsonValue::String(s)
                    }
                }
                Err(_) => {
                    // Binary data, encode as base64
                    JsonValue::String(base64_encode(&bytes))
                }
            }
        }
        Value::Int(i) => JsonValue::Number(i.into()),
        Value::UInt(u) => JsonValue::Number(u.into()),
        Value::Float(f) => serde_json::Number::from_f64(f64::from(f))
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::Double(d) => serde_json::Number::from_f64(d)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::Date(year, month, day, hour, minute, second, micro) => {
            let datetime = format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}",
                year, month, day, hour, minute, second, micro
            );
            JsonValue::String(datetime)
        }
        Value::Time(is_neg, days, hours, minutes, seconds, micro) => {
            let sign = if is_neg { "-" } else { "" };
            let time = format!(
                "{}{}:{:02}:{:02}.{:06}",
                sign,
                days * 24 + u32::from(hours),
                minutes,
                seconds,
                micro
            );
            JsonValue::String(time)
        }
    }
}

/// Simple base64 encoding for binary data.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied().unwrap_or(0);
        let b2 = data.get(i + 2).copied().unwrap_or(0);

        result.push(ALPHABET[(b0 >> 2) as usize] as char);
        result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if i + 1 < data.len() {
            result.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < data.len() {
            result.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_array_convert_to_mysql_values() {
        assert_eq!(
            filter_value_to_mysql(&FilterValue::Bytes(vec![0, 255])),
            Value::Bytes(vec![0, 255])
        );
        assert_eq!(
            filter_value_to_mysql(&FilterValue::Array(vec![
                FilterValue::Int(1),
                FilterValue::String("a".into())
            ])),
            Value::from("[1,\"a\"]")
        );
    }

    #[test]
    fn test_filter_value_to_mysql_null() {
        let result = filter_value_to_mysql(&FilterValue::Null);
        assert!(matches!(result, Value::NULL));
    }

    #[test]
    fn test_filter_value_to_mysql_bool() {
        let result = filter_value_to_mysql(&FilterValue::Bool(true));
        // mysql_async converts bool to Int
        assert!(matches!(result, Value::Int(1)));
    }

    #[test]
    fn test_filter_value_to_mysql_int() {
        let result = filter_value_to_mysql(&FilterValue::Int(42));
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_filter_value_to_mysql_float() {
        let result = filter_value_to_mysql(&FilterValue::Float(3.14));
        assert!(matches!(result, Value::Double(_)));
    }

    #[test]
    fn test_filter_value_to_mysql_string() {
        let result = filter_value_to_mysql(&FilterValue::String("hello".to_string()));
        assert!(matches!(result, Value::Bytes(_)));
    }

    #[test]
    fn test_from_mysql_value_null() {
        let result = from_mysql_value(Value::NULL);
        assert_eq!(result, JsonValue::Null);
    }

    #[test]
    fn test_from_mysql_value_int() {
        let result = from_mysql_value(Value::Int(42));
        assert_eq!(result, JsonValue::Number(42.into()));
    }

    #[test]
    fn test_from_mysql_value_uint() {
        let result = from_mysql_value(Value::UInt(100));
        assert_eq!(result, JsonValue::Number(100u64.into()));
    }

    #[test]
    fn test_from_mysql_value_string() {
        let result = from_mysql_value(Value::Bytes(b"hello".to_vec()));
        assert_eq!(result, JsonValue::String("hello".to_string()));
    }

    #[test]
    fn test_from_mysql_value_json_scalars_stay_strings() {
        // JSON-looking scalars must not be re-typed; they are legitimate text.
        for text in ["123", "null", "true"] {
            let result = from_mysql_value(Value::Bytes(text.as_bytes().to_vec()));
            assert_eq!(result, JsonValue::String(text.to_string()));
        }
    }

    #[test]
    fn test_from_mysql_value_json_object() {
        let result = from_mysql_value(Value::Bytes(br#"{"a":1}"#.to_vec()));
        assert_eq!(result, serde_json::json!({"a": 1}));

        let result = from_mysql_value(Value::Bytes(b"[1,2]".to_vec()));
        assert_eq!(result, serde_json::json!([1, 2]));
    }

    #[test]
    fn test_base64_encode() {
        let result = base64_encode(b"Hello");
        assert_eq!(result, "SGVsbG8=");
    }
}
