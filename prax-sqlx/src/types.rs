//! Type conversions for SQLx.

use crate::config::DatabaseBackend;
use prax_query::filter::FilterValue;

/// Convert a FilterValue to the appropriate SQL placeholder string.
pub fn placeholder(backend: DatabaseBackend, index: usize) -> String {
    match backend {
        DatabaseBackend::Postgres => format!("${}", index),
        DatabaseBackend::MySql | DatabaseBackend::Sqlite => "?".to_string(),
    }
}

/// Convert a FilterValue to a string representation for debugging.
pub fn filter_value_to_string(value: &FilterValue) -> String {
    match value {
        FilterValue::String(s) => s.clone(),
        FilterValue::Int(i) => i.to_string(),
        FilterValue::Float(f) => f.to_string(),
        FilterValue::Bool(b) => b.to_string(),
        FilterValue::Null => "NULL".to_string(),
        FilterValue::Json(j) => j.to_string(),
        FilterValue::List(arr) | FilterValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(filter_value_to_string).collect();
            format!("[{}]", items.join(", "))
        }
        FilterValue::Bytes(b) => {
            let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("\\x{hex}")
        }
        _ => value.to_json_value().to_string(),
    }
}

/// Generate SQL for a list of placeholders.
pub fn placeholders(backend: DatabaseBackend, count: usize, start: usize) -> String {
    (start..start + count)
        .map(|i| placeholder(backend, i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Quote an identifier for the given database backend.
pub fn quote_identifier(backend: DatabaseBackend, name: &str) -> String {
    match backend {
        DatabaseBackend::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
        DatabaseBackend::MySql => format!("`{}`", name.replace('`', "``")),
        DatabaseBackend::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// Convert Rust type to SQL type string.
pub fn rust_to_sql_type(backend: DatabaseBackend, rust_type: &str) -> &'static str {
    match backend {
        DatabaseBackend::Postgres => match rust_type {
            "i32" => "INTEGER",
            "i64" => "BIGINT",
            "f32" => "REAL",
            "f64" => "DOUBLE PRECISION",
            "bool" => "BOOLEAN",
            "String" | "&str" => "TEXT",
            "Vec<u8>" | "&[u8]" => "BYTEA",
            "Uuid" => "UUID",
            "DateTime" | "chrono::DateTime" => "TIMESTAMPTZ",
            "NaiveDate" => "DATE",
            "NaiveTime" => "TIME",
            "Decimal" => "DECIMAL",
            "Json" | "serde_json::Value" => "JSONB",
            _ => "TEXT",
        },
        DatabaseBackend::MySql => match rust_type {
            "i32" => "INT",
            "i64" => "BIGINT",
            "f32" => "FLOAT",
            "f64" => "DOUBLE",
            "bool" => "BOOLEAN",
            "String" | "&str" => "TEXT",
            "Vec<u8>" | "&[u8]" => "BLOB",
            "Uuid" => "CHAR(36)",
            "DateTime" | "chrono::DateTime" => "DATETIME",
            "NaiveDate" => "DATE",
            "NaiveTime" => "TIME",
            "Decimal" => "DECIMAL",
            "Json" | "serde_json::Value" => "JSON",
            _ => "TEXT",
        },
        DatabaseBackend::Sqlite => match rust_type {
            "i32" | "i64" => "INTEGER",
            "f32" | "f64" => "REAL",
            "bool" => "INTEGER", // SQLite uses 0/1 for booleans
            "String" | "&str" => "TEXT",
            "Vec<u8>" | "&[u8]" => "BLOB",
            "Uuid" => "TEXT",
            "DateTime" | "chrono::DateTime" => "TEXT", // ISO8601 string
            "NaiveDate" => "TEXT",
            "NaiveTime" => "TEXT",
            "Decimal" => "TEXT",
            "Json" | "serde_json::Value" => "TEXT", // JSON as text
            _ => "TEXT",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_array_format_for_debugging() {
        assert_eq!(
            filter_value_to_string(&FilterValue::Bytes(vec![0, 255])),
            "\\x00ff"
        );
        assert_eq!(
            filter_value_to_string(&FilterValue::Array(vec![
                FilterValue::Int(1),
                FilterValue::String("a".into())
            ])),
            "[1, a]"
        );
    }

    #[test]
    fn test_placeholder() {
        assert_eq!(placeholder(DatabaseBackend::Postgres, 1), "$1");
        assert_eq!(placeholder(DatabaseBackend::Postgres, 5), "$5");
        assert_eq!(placeholder(DatabaseBackend::MySql, 1), "?");
        assert_eq!(placeholder(DatabaseBackend::Sqlite, 1), "?");
    }

    #[test]
    fn test_placeholders() {
        assert_eq!(placeholders(DatabaseBackend::Postgres, 3, 1), "$1, $2, $3");
        assert_eq!(placeholders(DatabaseBackend::MySql, 3, 1), "?, ?, ?");
    }

    #[test]
    fn test_quote_identifier() {
        assert_eq!(
            quote_identifier(DatabaseBackend::Postgres, "users"),
            "\"users\""
        );
        assert_eq!(quote_identifier(DatabaseBackend::MySql, "users"), "`users`");
        assert_eq!(
            quote_identifier(DatabaseBackend::Sqlite, "users"),
            "\"users\""
        );

        // Test escaping
        assert_eq!(
            quote_identifier(DatabaseBackend::Postgres, "user\"name"),
            "\"user\"\"name\""
        );
    }

    #[test]
    fn test_rust_to_sql_type() {
        assert_eq!(
            rust_to_sql_type(DatabaseBackend::Postgres, "i32"),
            "INTEGER"
        );
        assert_eq!(rust_to_sql_type(DatabaseBackend::MySql, "i32"), "INT");
        assert_eq!(rust_to_sql_type(DatabaseBackend::Sqlite, "i32"), "INTEGER");

        assert_eq!(
            rust_to_sql_type(DatabaseBackend::Postgres, "bool"),
            "BOOLEAN"
        );
        assert_eq!(rust_to_sql_type(DatabaseBackend::Sqlite, "bool"), "INTEGER");
    }
}
