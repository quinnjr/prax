//! Type conversions for Microsoft SQL Server.

use prax_query::filter::FilterValue;

use crate::error::{MssqlError, MssqlResult};

/// Convert a FilterValue to a type that can be used as a SQL Server parameter.
pub fn filter_value_to_sql(value: &FilterValue) -> MssqlResult<Box<dyn tiberius::ToSql>> {
    match value {
        FilterValue::Null => Ok(Box::new(Option::<String>::None)),
        FilterValue::Bool(b) => Ok(Box::new(*b)),
        FilterValue::Int(i) => Ok(Box::new(*i)),
        FilterValue::Float(f) => Ok(Box::new(*f)),
        FilterValue::String(s) => Ok(Box::new(s.clone())),
        FilterValue::Json(j) => {
            // MSSQL stores JSON as NVARCHAR
            Ok(Box::new(j.to_string()))
        }
        FilterValue::List(_) => {
            // Lists need special handling - they should be converted to table-valued parameters
            // or used with IN clauses
            Err(MssqlError::type_conversion(
                "list values should be handled specially",
            ))
        }
    }
}

/// Convert filter values to SQL Server parameters.
pub fn filter_values_to_params(
    values: &[FilterValue],
) -> MssqlResult<Vec<Box<dyn tiberius::ToSql>>> {
    values.iter().map(filter_value_to_sql).collect()
}

/// SQL Server type mapping utilities.
pub mod mssql_types {
    /// Get the SQL Server type for a Rust type name.
    pub fn rust_type_to_mssql(rust_type: &str) -> Option<&'static str> {
        match rust_type {
            "i8" => Some("TINYINT"),
            "i16" => Some("SMALLINT"),
            "i32" => Some("INT"),
            "i64" => Some("BIGINT"),
            "f32" => Some("REAL"),
            "f64" => Some("FLOAT"),
            "bool" => Some("BIT"),
            "String" | "&str" => Some("NVARCHAR(MAX)"),
            "Vec<u8>" | "&[u8]" => Some("VARBINARY(MAX)"),
            "chrono::NaiveDate" => Some("DATE"),
            "chrono::NaiveTime" => Some("TIME"),
            "chrono::NaiveDateTime" => Some("DATETIME2"),
            "chrono::DateTime<chrono::Utc>" => Some("DATETIMEOFFSET"),
            "uuid::Uuid" => Some("UNIQUEIDENTIFIER"),
            "serde_json::Value" => Some("NVARCHAR(MAX)"), // JSON stored as string
            "rust_decimal::Decimal" => Some("DECIMAL(38, 10)"),
            _ => None,
        }
    }

    /// Get the Rust type for a SQL Server type.
    pub fn mssql_type_to_rust(mssql_type: &str) -> &'static str {
        match mssql_type.to_uppercase().as_str() {
            "BIT" => "bool",
            "TINYINT" => "i8",
            "SMALLINT" => "i16",
            "INT" => "i32",
            "BIGINT" => "i64",
            "REAL" => "f32",
            "FLOAT" => "f64",
            "DECIMAL" | "NUMERIC" | "MONEY" | "SMALLMONEY" => "rust_decimal::Decimal",
            "CHAR" | "VARCHAR" | "TEXT" | "NCHAR" | "NVARCHAR" | "NTEXT" => "String",
            "BINARY" | "VARBINARY" | "IMAGE" => "Vec<u8>",
            "DATE" => "chrono::NaiveDate",
            "TIME" => "chrono::NaiveTime",
            "DATETIME" | "DATETIME2" | "SMALLDATETIME" => "chrono::NaiveDateTime",
            "DATETIMEOFFSET" => "chrono::DateTime<chrono::Utc>",
            "UNIQUEIDENTIFIER" => "uuid::Uuid",
            "XML" => "String",
            _ => "unknown",
        }
    }

    /// Get the Prax schema type for a SQL Server type.
    pub fn mssql_type_to_prax(mssql_type: &str) -> &'static str {
        match mssql_type.to_uppercase().as_str() {
            "BIT" => "Boolean",
            "TINYINT" | "SMALLINT" | "INT" => "Int",
            "BIGINT" => "BigInt",
            "REAL" | "FLOAT" => "Float",
            "DECIMAL" | "NUMERIC" | "MONEY" | "SMALLMONEY" => "Decimal",
            "CHAR" | "VARCHAR" | "TEXT" | "NCHAR" | "NVARCHAR" | "NTEXT" => "String",
            "BINARY" | "VARBINARY" | "IMAGE" => "Bytes",
            "DATE" => "Date",
            "TIME" => "Time",
            "DATETIME" | "DATETIME2" | "SMALLDATETIME" => "DateTime",
            "DATETIMEOFFSET" => "DateTime",
            "UNIQUEIDENTIFIER" => "Uuid",
            "XML" => "String",
            _ => "Unknown",
        }
    }

    /// Get the SQL Server type for a Prax schema type.
    pub fn prax_type_to_mssql(prax_type: &str) -> &'static str {
        match prax_type {
            "Boolean" => "BIT",
            "Int" => "INT",
            "BigInt" => "BIGINT",
            "Float" => "FLOAT",
            "Decimal" => "DECIMAL(38, 10)",
            "String" => "NVARCHAR(MAX)",
            "Bytes" => "VARBINARY(MAX)",
            "Date" => "DATE",
            "Time" => "TIME",
            "DateTime" => "DATETIME2",
            "Uuid" => "UNIQUEIDENTIFIER",
            "Json" => "NVARCHAR(MAX)",
            _ => "NVARCHAR(MAX)",
        }
    }
}

/// SQL dialect conversions for MSSQL.
pub mod dialect {
    /// Convert PostgreSQL LIMIT/OFFSET to SQL Server TOP/OFFSET FETCH.
    ///
    /// PostgreSQL: SELECT * FROM t LIMIT 10 OFFSET 20
    /// SQL Server: SELECT * FROM t ORDER BY (SELECT NULL) OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY
    pub fn convert_limit_offset(limit: Option<u64>, offset: Option<u64>) -> String {
        match (limit, offset) {
            (Some(limit), Some(offset)) => {
                format!("OFFSET {} ROWS FETCH NEXT {} ROWS ONLY", offset, limit)
            }
            (Some(limit), None) => {
                format!("OFFSET 0 ROWS FETCH NEXT {} ROWS ONLY", limit)
            }
            (None, Some(offset)) => {
                format!("OFFSET {} ROWS", offset)
            }
            (None, None) => String::new(),
        }
    }

    /// Convert PostgreSQL RETURNING to SQL Server OUTPUT.
    ///
    /// PostgreSQL: INSERT INTO t (...) RETURNING *
    /// SQL Server: INSERT INTO t (...) OUTPUT INSERTED.*
    pub fn convert_returning(columns: &[&str]) -> String {
        if columns.is_empty() || columns == ["*"] {
            "OUTPUT INSERTED.*".to_string()
        } else {
            let cols: Vec<String> = columns.iter().map(|c| format!("INSERTED.{}", c)).collect();
            format!("OUTPUT {}", cols.join(", "))
        }
    }

    /// Convert PostgreSQL boolean literals to SQL Server.
    pub fn convert_bool(value: bool) -> &'static str {
        if value { "1" } else { "0" }
    }

    /// Convert PostgreSQL ILIKE to SQL Server (case-insensitive by default).
    pub fn convert_ilike(column: &str, pattern: &str) -> String {
        format!("{} LIKE {}", column, pattern)
    }

    /// Convert PostgreSQL string concatenation (||) to SQL Server (+).
    pub fn convert_concat(parts: &[&str]) -> String {
        parts.join(" + ")
    }

    /// Convert PostgreSQL COALESCE to SQL Server ISNULL for simple cases.
    pub fn convert_coalesce(expr: &str, default: &str) -> String {
        format!("ISNULL({}, {})", expr, default)
    }

    /// Convert PostgreSQL now() to SQL Server GETUTCDATE().
    pub fn current_timestamp() -> &'static str {
        "GETUTCDATE()"
    }

    /// Convert PostgreSQL uuid_generate_v4() to SQL Server NEWID().
    pub fn generate_uuid() -> &'static str {
        "NEWID()"
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
    fn test_mssql_type_mapping() {
        use mssql_types::*;

        assert_eq!(rust_type_to_mssql("i32"), Some("INT"));
        assert_eq!(rust_type_to_mssql("String"), Some("NVARCHAR(MAX)"));
        assert_eq!(rust_type_to_mssql("bool"), Some("BIT"));

        assert_eq!(mssql_type_to_rust("INT"), "i32");
        assert_eq!(mssql_type_to_rust("NVARCHAR"), "String");
        assert_eq!(mssql_type_to_rust("BIT"), "bool");
    }

    #[test]
    fn test_dialect_conversions() {
        use dialect::*;

        assert_eq!(
            convert_limit_offset(Some(10), Some(20)),
            "OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY"
        );

        assert_eq!(
            convert_limit_offset(Some(10), None),
            "OFFSET 0 ROWS FETCH NEXT 10 ROWS ONLY"
        );

        assert_eq!(convert_returning(&["*"]), "OUTPUT INSERTED.*");
        assert_eq!(
            convert_returning(&["id", "name"]),
            "OUTPUT INSERTED.id, INSERTED.name"
        );

        assert_eq!(convert_bool(true), "1");
        assert_eq!(convert_bool(false), "0");
    }

    #[test]
    fn test_prax_type_mapping() {
        use mssql_types::*;

        assert_eq!(prax_type_to_mssql("Int"), "INT");
        assert_eq!(prax_type_to_mssql("String"), "NVARCHAR(MAX)");
        assert_eq!(prax_type_to_mssql("Boolean"), "BIT");
        assert_eq!(prax_type_to_mssql("DateTime"), "DATETIME2");
        assert_eq!(prax_type_to_mssql("Uuid"), "UNIQUEIDENTIFIER");
    }
}
