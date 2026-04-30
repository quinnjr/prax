//! Database introspection and schema generation.
//!
//! This module provides types for introspecting existing databases and generating
//! Prax schema definitions from the discovered structure.
//!
//! # Database Support
//!
//! | Feature              | PostgreSQL | MySQL | SQLite | MSSQL | MongoDB       |
//! |----------------------|------------|-------|--------|-------|---------------|
//! | Table introspection  | ✅         | ✅    | ✅     | ✅    | ✅ Collection |
//! | Column types         | ✅         | ✅    | ✅     | ✅    | ✅ Inferred   |
//! | Primary keys         | ✅         | ✅    | ✅     | ✅    | ✅ _id        |
//! | Foreign keys         | ✅         | ✅    | ✅     | ✅    | ❌            |
//! | Indexes              | ✅         | ✅    | ✅     | ✅    | ✅            |
//! | Unique constraints   | ✅         | ✅    | ✅     | ✅    | ✅            |
//! | Default values       | ✅         | ✅    | ✅     | ✅    | ❌            |
//! | Enums                | ✅         | ✅    | ❌     | ❌    | ❌            |
//! | Views                | ✅         | ✅    | ✅     | ✅    | ✅            |

use serde::{Deserialize, Serialize};

use crate::sql::DatabaseType;

// ============================================================================
// Introspection Results
// ============================================================================

/// Complete introspection result for a database.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseSchema {
    /// Database name.
    pub name: String,
    /// Schema/namespace (PostgreSQL, MSSQL).
    pub schema: Option<String>,
    /// Tables discovered.
    pub tables: Vec<TableInfo>,
    /// Views discovered.
    pub views: Vec<ViewInfo>,
    /// Enums discovered.
    pub enums: Vec<EnumInfo>,
    /// Sequences discovered.
    pub sequences: Vec<SequenceInfo>,
}

/// Information about a table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableInfo {
    /// Table name.
    pub name: String,
    /// Schema/namespace.
    pub schema: Option<String>,
    /// Table comment/description.
    pub comment: Option<String>,
    /// Columns.
    pub columns: Vec<ColumnInfo>,
    /// Primary key columns.
    pub primary_key: Vec<String>,
    /// Foreign keys.
    pub foreign_keys: Vec<ForeignKeyInfo>,
    /// Indexes.
    pub indexes: Vec<IndexInfo>,
    /// Unique constraints.
    pub unique_constraints: Vec<UniqueConstraint>,
    /// Check constraints.
    pub check_constraints: Vec<CheckConstraint>,
}

/// Information about a column.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Database-specific type name.
    pub db_type: String,
    /// Normalized type for schema generation.
    pub normalized_type: NormalizedType,
    /// Whether the column is nullable.
    pub nullable: bool,
    /// Default value expression.
    pub default: Option<String>,
    /// Whether this is an auto-increment/serial column.
    pub auto_increment: bool,
    /// Whether this is part of primary key.
    pub is_primary_key: bool,
    /// Whether this column has a unique constraint.
    pub is_unique: bool,
    /// Column comment.
    pub comment: Option<String>,
    /// Character maximum length (for varchar, etc.).
    pub max_length: Option<i32>,
    /// Numeric precision.
    pub precision: Option<i32>,
    /// Numeric scale.
    pub scale: Option<i32>,
    /// Enum type name (if applicable).
    pub enum_name: Option<String>,
}

/// Normalized type for cross-database compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizedType {
    /// Integer types.
    Int,
    BigInt,
    SmallInt,
    /// Floating point.
    Float,
    Double,
    /// Fixed precision.
    Decimal {
        precision: Option<i32>,
        scale: Option<i32>,
    },
    /// String types.
    String,
    Text,
    Char {
        length: Option<i32>,
    },
    VarChar {
        length: Option<i32>,
    },
    /// Binary.
    Bytes,
    /// Boolean.
    Boolean,
    /// Date/time.
    DateTime,
    Date,
    Time,
    Timestamp,
    /// JSON.
    Json,
    /// UUID.
    Uuid,
    /// Array of type.
    Array(Box<NormalizedType>),
    /// Enum reference.
    Enum(String),
    /// Unknown/unsupported.
    Unknown(String),
}

impl Default for NormalizedType {
    fn default() -> Self {
        Self::Unknown("unknown".to_string())
    }
}

impl NormalizedType {
    /// Convert to Prax schema type string.
    pub fn to_prax_type(&self) -> String {
        match self {
            Self::Int => "Int".to_string(),
            Self::BigInt => "BigInt".to_string(),
            Self::SmallInt => "Int".to_string(),
            Self::Float => "Float".to_string(),
            Self::Double => "Float".to_string(),
            Self::Decimal { .. } => "Decimal".to_string(),
            Self::String | Self::Text | Self::VarChar { .. } | Self::Char { .. } => {
                "String".to_string()
            }
            Self::Bytes => "Bytes".to_string(),
            Self::Boolean => "Boolean".to_string(),
            Self::DateTime | Self::Timestamp => "DateTime".to_string(),
            Self::Date => "DateTime".to_string(),
            Self::Time => "DateTime".to_string(),
            Self::Json => "Json".to_string(),
            Self::Uuid => "String".to_string(), // Or custom UUID type
            Self::Array(inner) => format!("{}[]", inner.to_prax_type()),
            Self::Enum(name) => name.clone(),
            Self::Unknown(t) => format!("Unsupported<{}>", t),
        }
    }
}

/// Information about a foreign key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForeignKeyInfo {
    /// Constraint name.
    pub name: String,
    /// Local columns.
    pub columns: Vec<String>,
    /// Referenced table.
    pub referenced_table: String,
    /// Referenced schema.
    pub referenced_schema: Option<String>,
    /// Referenced columns.
    pub referenced_columns: Vec<String>,
    /// ON DELETE action.
    pub on_delete: ReferentialAction,
    /// ON UPDATE action.
    pub on_update: ReferentialAction,
}

/// Referential action for foreign keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferentialAction {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    /// Convert to Prax schema string.
    pub fn to_prax(&self) -> &'static str {
        match self {
            Self::NoAction => "NoAction",
            Self::Restrict => "Restrict",
            Self::Cascade => "Cascade",
            Self::SetNull => "SetNull",
            Self::SetDefault => "SetDefault",
        }
    }

    /// Parse from database string.
    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "NO ACTION" | "NOACTION" => Self::NoAction,
            "RESTRICT" => Self::Restrict,
            "CASCADE" => Self::Cascade,
            "SET NULL" | "SETNULL" => Self::SetNull,
            "SET DEFAULT" | "SETDEFAULT" => Self::SetDefault,
            _ => Self::NoAction,
        }
    }
}

/// Information about an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexInfo {
    /// Index name.
    pub name: String,
    /// Columns in the index.
    pub columns: Vec<IndexColumn>,
    /// Whether this is a unique index.
    pub is_unique: bool,
    /// Whether this is a primary key index.
    pub is_primary: bool,
    /// Index type (btree, hash, gin, etc.).
    pub index_type: Option<String>,
    /// Filter condition (partial index).
    pub filter: Option<String>,
}

/// A column in an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexColumn {
    /// Column name.
    pub name: String,
    /// Sort order.
    pub order: SortOrder,
    /// Nulls position.
    pub nulls: NullsOrder,
}

/// Sort order for index columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// Nulls ordering for index columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NullsOrder {
    #[default]
    Last,
    First,
}

/// Unique constraint information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UniqueConstraint {
    /// Constraint name.
    pub name: String,
    /// Columns.
    pub columns: Vec<String>,
}

/// Check constraint information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckConstraint {
    /// Constraint name.
    pub name: String,
    /// Check expression.
    pub expression: String,
}

/// Information about a view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewInfo {
    /// View name.
    pub name: String,
    /// Schema.
    pub schema: Option<String>,
    /// View definition SQL.
    pub definition: Option<String>,
    /// Whether this is a materialized view.
    pub is_materialized: bool,
    /// Columns (inferred from definition).
    pub columns: Vec<ColumnInfo>,
}

/// Information about an enum type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnumInfo {
    /// Enum type name.
    pub name: String,
    /// Schema.
    pub schema: Option<String>,
    /// Enum values.
    pub values: Vec<String>,
}

/// Information about a sequence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SequenceInfo {
    /// Sequence name.
    pub name: String,
    /// Schema.
    pub schema: Option<String>,
    /// Start value.
    pub start: i64,
    /// Increment.
    pub increment: i64,
    /// Minimum value.
    pub min_value: Option<i64>,
    /// Maximum value.
    pub max_value: Option<i64>,
    /// Whether it cycles.
    pub cycle: bool,
}

// ============================================================================
// Introspection Queries
// ============================================================================

/// SQL queries for database introspection.
pub mod queries {
    use super::*;

    /// Get tables query.
    pub fn tables_query(db_type: DatabaseType, schema: Option<&str>) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                let schema_filter = schema.unwrap_or("public");
                format!(
                    "SELECT table_name, obj_description((quote_ident(table_schema) || '.' || quote_ident(table_name))::regclass) as comment \
                     FROM information_schema.tables \
                     WHERE table_schema = '{}' AND table_type = 'BASE TABLE' \
                     ORDER BY table_name",
                    schema_filter
                )
            }
            DatabaseType::MySQL => {
                let schema_filter = schema
                    .map(|s| format!("AND table_schema = '{}'", s))
                    .unwrap_or_default();
                format!(
                    "SELECT table_name, table_comment as comment \
                     FROM information_schema.tables \
                     WHERE table_type = 'BASE TABLE' {} \
                     ORDER BY table_name",
                    schema_filter
                )
            }
            DatabaseType::SQLite => "SELECT name as table_name, NULL as comment \
                 FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name"
                .to_string(),
            DatabaseType::MSSQL => {
                let schema_filter = schema.unwrap_or("dbo");
                format!(
                    "SELECT t.name as table_name, ep.value as comment \
                     FROM sys.tables t \
                     LEFT JOIN sys.extended_properties ep ON ep.major_id = t.object_id AND ep.minor_id = 0 AND ep.name = 'MS_Description' \
                     JOIN sys.schemas s ON t.schema_id = s.schema_id \
                     WHERE s.name = '{}' \
                     ORDER BY t.name",
                    schema_filter
                )
            }
        }
    }

    /// Get columns query.
    pub fn columns_query(db_type: DatabaseType, table: &str, schema: Option<&str>) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                let schema_filter = schema.unwrap_or("public");
                format!(
                    "SELECT \
                        c.column_name, \
                        c.data_type, \
                        c.udt_name, \
                        c.is_nullable = 'YES' as nullable, \
                        c.column_default, \
                        c.character_maximum_length, \
                        c.numeric_precision, \
                        c.numeric_scale, \
                        col_description((quote_ident(c.table_schema) || '.' || quote_ident(c.table_name))::regclass, c.ordinal_position) as comment, \
                        CASE WHEN c.column_default LIKE 'nextval%' THEN true ELSE false END as auto_increment \
                     FROM information_schema.columns c \
                     WHERE c.table_schema = '{}' AND c.table_name = '{}' \
                     ORDER BY c.ordinal_position",
                    schema_filter, table
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "SELECT \
                        column_name, \
                        data_type, \
                        column_type as udt_name, \
                        is_nullable = 'YES' as nullable, \
                        column_default, \
                        character_maximum_length, \
                        numeric_precision, \
                        numeric_scale, \
                        column_comment as comment, \
                        extra LIKE '%auto_increment%' as auto_increment \
                     FROM information_schema.columns \
                     WHERE table_name = '{}' {} \
                     ORDER BY ordinal_position",
                    table,
                    schema
                        .map(|s| format!("AND table_schema = '{}'", s))
                        .unwrap_or_default()
                )
            }
            DatabaseType::SQLite => {
                format!("PRAGMA table_info('{}')", table)
            }
            DatabaseType::MSSQL => {
                let schema_filter = schema.unwrap_or("dbo");
                format!(
                    "SELECT \
                        c.name as column_name, \
                        t.name as data_type, \
                        t.name as udt_name, \
                        c.is_nullable as nullable, \
                        dc.definition as column_default, \
                        c.max_length as character_maximum_length, \
                        c.precision as numeric_precision, \
                        c.scale as numeric_scale, \
                        ep.value as comment, \
                        c.is_identity as auto_increment \
                     FROM sys.columns c \
                     JOIN sys.types t ON c.user_type_id = t.user_type_id \
                     JOIN sys.tables tb ON c.object_id = tb.object_id \
                     JOIN sys.schemas s ON tb.schema_id = s.schema_id \
                     LEFT JOIN sys.default_constraints dc ON c.default_object_id = dc.object_id \
                     LEFT JOIN sys.extended_properties ep ON ep.major_id = c.object_id AND ep.minor_id = c.column_id AND ep.name = 'MS_Description' \
                     WHERE tb.name = '{}' AND s.name = '{}' \
                     ORDER BY c.column_id",
                    table, schema_filter
                )
            }
        }
    }

    /// Get primary keys query.
    pub fn primary_keys_query(db_type: DatabaseType, table: &str, schema: Option<&str>) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                let schema_filter = schema.unwrap_or("public");
                format!(
                    "SELECT a.attname as column_name \
                     FROM pg_index i \
                     JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
                     JOIN pg_class c ON c.oid = i.indrelid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE i.indisprimary AND c.relname = '{}' AND n.nspname = '{}' \
                     ORDER BY array_position(i.indkey, a.attnum)",
                    table, schema_filter
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "SELECT column_name \
                     FROM information_schema.key_column_usage \
                     WHERE constraint_name = 'PRIMARY' AND table_name = '{}' {} \
                     ORDER BY ordinal_position",
                    table,
                    schema
                        .map(|s| format!("AND table_schema = '{}'", s))
                        .unwrap_or_default()
                )
            }
            DatabaseType::SQLite => {
                format!("PRAGMA table_info('{}')", table) // Filter pk column in result
            }
            DatabaseType::MSSQL => {
                let schema_filter = schema.unwrap_or("dbo");
                format!(
                    "SELECT c.name as column_name \
                     FROM sys.indexes i \
                     JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
                     JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id \
                     JOIN sys.tables t ON i.object_id = t.object_id \
                     JOIN sys.schemas s ON t.schema_id = s.schema_id \
                     WHERE i.is_primary_key = 1 AND t.name = '{}' AND s.name = '{}' \
                     ORDER BY ic.key_ordinal",
                    table, schema_filter
                )
            }
        }
    }

    /// Get foreign keys query.
    pub fn foreign_keys_query(db_type: DatabaseType, table: &str, schema: Option<&str>) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                let schema_filter = schema.unwrap_or("public");
                format!(
                    "SELECT \
                        tc.constraint_name, \
                        kcu.column_name, \
                        ccu.table_name as referenced_table, \
                        ccu.table_schema as referenced_schema, \
                        ccu.column_name as referenced_column, \
                        rc.delete_rule, \
                        rc.update_rule \
                     FROM information_schema.table_constraints tc \
                     JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name \
                     JOIN information_schema.constraint_column_usage ccu ON ccu.constraint_name = tc.constraint_name \
                     JOIN information_schema.referential_constraints rc ON rc.constraint_name = tc.constraint_name \
                     WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_name = '{}' AND tc.table_schema = '{}' \
                     ORDER BY tc.constraint_name, kcu.ordinal_position",
                    table, schema_filter
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "SELECT \
                        constraint_name, \
                        column_name, \
                        referenced_table_name as referenced_table, \
                        referenced_table_schema as referenced_schema, \
                        referenced_column_name as referenced_column, \
                        'NO ACTION' as delete_rule, \
                        'NO ACTION' as update_rule \
                     FROM information_schema.key_column_usage \
                     WHERE referenced_table_name IS NOT NULL AND table_name = '{}' {} \
                     ORDER BY constraint_name, ordinal_position",
                    table,
                    schema
                        .map(|s| format!("AND table_schema = '{}'", s))
                        .unwrap_or_default()
                )
            }
            DatabaseType::SQLite => {
                format!("PRAGMA foreign_key_list('{}')", table)
            }
            DatabaseType::MSSQL => {
                let schema_filter = schema.unwrap_or("dbo");
                format!(
                    "SELECT \
                        fk.name as constraint_name, \
                        c.name as column_name, \
                        rt.name as referenced_table, \
                        rs.name as referenced_schema, \
                        rc.name as referenced_column, \
                        fk.delete_referential_action_desc as delete_rule, \
                        fk.update_referential_action_desc as update_rule \
                     FROM sys.foreign_keys fk \
                     JOIN sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id \
                     JOIN sys.columns c ON fkc.parent_object_id = c.object_id AND fkc.parent_column_id = c.column_id \
                     JOIN sys.tables t ON fk.parent_object_id = t.object_id \
                     JOIN sys.schemas s ON t.schema_id = s.schema_id \
                     JOIN sys.tables rt ON fk.referenced_object_id = rt.object_id \
                     JOIN sys.schemas rs ON rt.schema_id = rs.schema_id \
                     JOIN sys.columns rc ON fkc.referenced_object_id = rc.object_id AND fkc.referenced_column_id = rc.column_id \
                     WHERE t.name = '{}' AND s.name = '{}' \
                     ORDER BY fk.name",
                    table, schema_filter
                )
            }
        }
    }

    /// Get indexes query.
    pub fn indexes_query(db_type: DatabaseType, table: &str, schema: Option<&str>) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                let schema_filter = schema.unwrap_or("public");
                format!(
                    "SELECT \
                        i.relname as index_name, \
                        a.attname as column_name, \
                        ix.indisunique as is_unique, \
                        ix.indisprimary as is_primary, \
                        am.amname as index_type, \
                        pg_get_expr(ix.indpred, ix.indrelid) as filter \
                     FROM pg_index ix \
                     JOIN pg_class t ON t.oid = ix.indrelid \
                     JOIN pg_class i ON i.oid = ix.indexrelid \
                     JOIN pg_namespace n ON n.oid = t.relnamespace \
                     JOIN pg_am am ON i.relam = am.oid \
                     JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
                     WHERE t.relname = '{}' AND n.nspname = '{}' \
                     ORDER BY i.relname, array_position(ix.indkey, a.attnum)",
                    table, schema_filter
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "SELECT \
                        index_name, \
                        column_name, \
                        NOT non_unique as is_unique, \
                        index_name = 'PRIMARY' as is_primary, \
                        index_type, \
                        NULL as filter \
                     FROM information_schema.statistics \
                     WHERE table_name = '{}' {} \
                     ORDER BY index_name, seq_in_index",
                    table,
                    schema
                        .map(|s| format!("AND table_schema = '{}'", s))
                        .unwrap_or_default()
                )
            }
            DatabaseType::SQLite => {
                format!("PRAGMA index_list('{}')", table)
            }
            DatabaseType::MSSQL => {
                let schema_filter = schema.unwrap_or("dbo");
                format!(
                    "SELECT \
                        i.name as index_name, \
                        c.name as column_name, \
                        i.is_unique, \
                        i.is_primary_key as is_primary, \
                        i.type_desc as index_type, \
                        i.filter_definition as filter \
                     FROM sys.indexes i \
                     JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
                     JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id \
                     JOIN sys.tables t ON i.object_id = t.object_id \
                     JOIN sys.schemas s ON t.schema_id = s.schema_id \
                     WHERE t.name = '{}' AND s.name = '{}' AND i.name IS NOT NULL \
                     ORDER BY i.name, ic.key_ordinal",
                    table, schema_filter
                )
            }
        }
    }

    /// Get enums query (PostgreSQL only).
    pub fn enums_query(schema: Option<&str>) -> String {
        let schema_filter = schema.unwrap_or("public");
        format!(
            "SELECT t.typname as enum_name, e.enumlabel as enum_value \
             FROM pg_type t \
             JOIN pg_enum e ON t.oid = e.enumtypid \
             JOIN pg_namespace n ON n.oid = t.typnamespace \
             WHERE n.nspname = '{}' \
             ORDER BY t.typname, e.enumsortorder",
            schema_filter
        )
    }

    /// Get views query.
    pub fn views_query(db_type: DatabaseType, schema: Option<&str>) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                let schema_filter = schema.unwrap_or("public");
                format!(
                    "SELECT table_name as view_name, view_definition, false as is_materialized \
                     FROM information_schema.views \
                     WHERE table_schema = '{}' \
                     UNION ALL \
                     SELECT matviewname as view_name, definition as view_definition, true as is_materialized \
                     FROM pg_matviews \
                     WHERE schemaname = '{}' \
                     ORDER BY view_name",
                    schema_filter, schema_filter
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "SELECT table_name as view_name, view_definition, false as is_materialized \
                     FROM information_schema.views \
                     WHERE table_schema = '{}' \
                     ORDER BY view_name",
                    schema.unwrap_or("information_schema")
                )
            }
            DatabaseType::SQLite => {
                "SELECT name as view_name, sql as view_definition, 0 as is_materialized \
                 FROM sqlite_master \
                 WHERE type = 'view' \
                 ORDER BY name"
                    .to_string()
            }
            DatabaseType::MSSQL => {
                let schema_filter = schema.unwrap_or("dbo");
                format!(
                    "SELECT v.name as view_name, m.definition as view_definition, \
                     CASE WHEN i.object_id IS NOT NULL THEN 1 ELSE 0 END as is_materialized \
                     FROM sys.views v \
                     JOIN sys.schemas s ON v.schema_id = s.schema_id \
                     JOIN sys.sql_modules m ON v.object_id = m.object_id \
                     LEFT JOIN sys.indexes i ON v.object_id = i.object_id AND i.index_id = 1 \
                     WHERE s.name = '{}' \
                     ORDER BY v.name",
                    schema_filter
                )
            }
        }
    }
}

// ============================================================================
// Type Mapping
// ============================================================================

/// Map database types to normalized types.
pub fn normalize_type(
    db_type: DatabaseType,
    type_name: &str,
    max_length: Option<i32>,
    precision: Option<i32>,
    scale: Option<i32>,
) -> NormalizedType {
    let type_lower = type_name.to_lowercase();

    match db_type {
        DatabaseType::PostgreSQL => {
            normalize_postgres_type(&type_lower, max_length, precision, scale)
        }
        DatabaseType::MySQL => normalize_mysql_type(&type_lower, max_length, precision, scale),
        DatabaseType::SQLite => normalize_sqlite_type(&type_lower),
        DatabaseType::MSSQL => normalize_mssql_type(&type_lower, max_length, precision, scale),
    }
}

fn normalize_postgres_type(
    type_name: &str,
    _max_length: Option<i32>,
    precision: Option<i32>,
    scale: Option<i32>,
) -> NormalizedType {
    match type_name {
        "int2" | "smallint" | "smallserial" => NormalizedType::SmallInt,
        "int4" | "integer" | "int" | "serial" => NormalizedType::Int,
        "int8" | "bigint" | "bigserial" => NormalizedType::BigInt,
        "real" | "float4" => NormalizedType::Float,
        "double precision" | "float8" => NormalizedType::Double,
        "numeric" | "decimal" => NormalizedType::Decimal { precision, scale },
        "bool" | "boolean" => NormalizedType::Boolean,
        "text" => NormalizedType::Text,
        "varchar" | "character varying" => NormalizedType::VarChar {
            length: _max_length,
        },
        "char" | "character" | "bpchar" => NormalizedType::Char {
            length: _max_length,
        },
        "bytea" => NormalizedType::Bytes,
        "timestamp" | "timestamp without time zone" => NormalizedType::Timestamp,
        "timestamptz" | "timestamp with time zone" => NormalizedType::DateTime,
        "date" => NormalizedType::Date,
        "time" | "time without time zone" | "timetz" | "time with time zone" => {
            NormalizedType::Time
        }
        "json" | "jsonb" => NormalizedType::Json,
        "uuid" => NormalizedType::Uuid,
        t if t.ends_with("[]") => {
            let inner = normalize_postgres_type(&t[..t.len() - 2], None, None, None);
            NormalizedType::Array(Box::new(inner))
        }
        t => NormalizedType::Unknown(t.to_string()),
    }
}

fn normalize_mysql_type(
    type_name: &str,
    max_length: Option<i32>,
    precision: Option<i32>,
    scale: Option<i32>,
) -> NormalizedType {
    match type_name {
        "tinyint" | "smallint" => NormalizedType::SmallInt,
        "int" | "integer" | "mediumint" => NormalizedType::Int,
        "bigint" => NormalizedType::BigInt,
        "float" => NormalizedType::Float,
        "double" | "real" => NormalizedType::Double,
        "decimal" | "numeric" => NormalizedType::Decimal { precision, scale },
        "bit" | "bool" | "boolean" => NormalizedType::Boolean,
        "text" | "mediumtext" | "longtext" => NormalizedType::Text,
        "varchar" => NormalizedType::VarChar { length: max_length },
        "char" => NormalizedType::Char { length: max_length },
        "tinyblob" | "blob" | "mediumblob" | "longblob" | "binary" | "varbinary" => {
            NormalizedType::Bytes
        }
        "datetime" | "timestamp" => NormalizedType::DateTime,
        "date" => NormalizedType::Date,
        "time" => NormalizedType::Time,
        "json" => NormalizedType::Json,
        t if t.starts_with("enum(") => {
            // Extract enum name from table context
            NormalizedType::Enum(t.to_string())
        }
        t => NormalizedType::Unknown(t.to_string()),
    }
}

fn normalize_sqlite_type(type_name: &str) -> NormalizedType {
    // SQLite has dynamic typing, so we map by affinity
    match type_name {
        "integer" | "int" => NormalizedType::Int,
        "real" | "float" | "double" => NormalizedType::Double,
        "text" | "varchar" | "char" | "clob" => NormalizedType::Text,
        "blob" => NormalizedType::Bytes,
        "boolean" | "bool" => NormalizedType::Boolean,
        "datetime" | "timestamp" | "date" | "time" => NormalizedType::DateTime,
        t => NormalizedType::Unknown(t.to_string()),
    }
}

fn normalize_mssql_type(
    type_name: &str,
    max_length: Option<i32>,
    precision: Option<i32>,
    scale: Option<i32>,
) -> NormalizedType {
    match type_name {
        "tinyint" | "smallint" => NormalizedType::SmallInt,
        "int" => NormalizedType::Int,
        "bigint" => NormalizedType::BigInt,
        "real" | "float" => NormalizedType::Float,
        "decimal" | "numeric" | "money" | "smallmoney" => {
            NormalizedType::Decimal { precision, scale }
        }
        "bit" => NormalizedType::Boolean,
        "text" | "ntext" => NormalizedType::Text,
        "varchar" | "nvarchar" => NormalizedType::VarChar { length: max_length },
        "char" | "nchar" => NormalizedType::Char { length: max_length },
        "binary" | "varbinary" | "image" => NormalizedType::Bytes,
        "datetime" | "datetime2" | "datetimeoffset" | "smalldatetime" => NormalizedType::DateTime,
        "date" => NormalizedType::Date,
        "time" => NormalizedType::Time,
        "uniqueidentifier" => NormalizedType::Uuid,
        t => NormalizedType::Unknown(t.to_string()),
    }
}

// ============================================================================
// Schema Generation
// ============================================================================

/// Generate Prax schema from introspection result.
pub fn generate_prax_schema(db: &DatabaseSchema) -> String {
    let mut output = String::new();

    // Header comment
    output.push_str("// Generated by Prax introspection\n");
    output.push_str(&format!("// Database: {}\n\n", db.name));

    // Generate enums
    for enum_info in &db.enums {
        output.push_str(&generate_enum(enum_info));
        output.push('\n');
    }

    // Generate models
    for table in &db.tables {
        output.push_str(&generate_model(table, &db.tables));
        output.push('\n');
    }

    // Generate views
    for view in &db.views {
        output.push_str(&generate_view(view));
        output.push('\n');
    }

    output
}

fn generate_enum(enum_info: &EnumInfo) -> String {
    let mut output = format!("enum {} {{\n", enum_info.name);
    for value in &enum_info.values {
        output.push_str(&format!("    {}\n", value));
    }
    output.push_str("}\n");
    output
}

fn generate_model(table: &TableInfo, all_tables: &[TableInfo]) -> String {
    let mut output = String::new();

    // Comment
    if let Some(ref comment) = table.comment {
        output.push_str(&format!("/// {}\n", comment));
    }

    output.push_str(&format!("model {} {{\n", pascal_case(&table.name)));

    // Fields
    for col in &table.columns {
        output.push_str(&generate_field(col, &table.primary_key));
    }

    // Relations
    for fk in &table.foreign_keys {
        output.push_str(&generate_relation(fk, all_tables));
    }

    // Model attributes
    let attrs = generate_model_attributes(table);
    if !attrs.is_empty() {
        output.push('\n');
        output.push_str(&attrs);
    }

    output.push_str("}\n");
    output
}

fn generate_field(col: &ColumnInfo, primary_key: &[String]) -> String {
    let mut attrs = Vec::new();

    // Check if primary key
    if primary_key.contains(&col.name) {
        attrs.push("@id".to_string());
    }

    // Auto increment
    if col.auto_increment {
        attrs.push("@auto".to_string());
    }

    // Unique
    if col.is_unique && !primary_key.contains(&col.name) {
        attrs.push("@unique".to_string());
    }

    // Default
    if let Some(ref default) = col.default
        && !col.auto_increment
    {
        let default_val = simplify_default(default);
        attrs.push(format!("@default({})", default_val));
    }

    // Map if name differs
    let field_name = camel_case(&col.name);
    if field_name != col.name {
        attrs.push(format!("@map(\"{}\")", col.name));
    }

    // Build type string
    let type_str = col.normalized_type.to_prax_type();
    let optional = if col.nullable { "?" } else { "" };

    let attrs_str = if attrs.is_empty() {
        String::new()
    } else {
        format!(" {}", attrs.join(" "))
    };

    format!("    {} {}{}{}\n", field_name, type_str, optional, attrs_str)
}

fn generate_relation(fk: &ForeignKeyInfo, all_tables: &[TableInfo]) -> String {
    // Find the referenced table
    let _ref_table = all_tables.iter().find(|t| t.name == fk.referenced_table);
    let ref_name = pascal_case(&fk.referenced_table);

    let field_name = if fk.columns.len() == 1 {
        // Derive relation name from FK column (e.g., user_id -> user)
        let col = &fk.columns[0];
        if col.ends_with("_id") {
            camel_case(&col[..col.len() - 3])
        } else {
            camel_case(&fk.referenced_table)
        }
    } else {
        camel_case(&fk.referenced_table)
    };

    let mut attrs = [format!(
        "@relation(fields: [{}], references: [{}]",
        fk.columns
            .iter()
            .map(|c| camel_case(c))
            .collect::<Vec<_>>()
            .join(", "),
        fk.referenced_columns
            .iter()
            .map(|c| camel_case(c))
            .collect::<Vec<_>>()
            .join(", ")
    )];

    // Add referential actions if not default
    if fk.on_delete != ReferentialAction::NoAction {
        attrs[0].push_str(&format!(", onDelete: {}", fk.on_delete.to_prax()));
    }
    if fk.on_update != ReferentialAction::NoAction {
        attrs[0].push_str(&format!(", onUpdate: {}", fk.on_update.to_prax()));
    }

    attrs[0].push(')');

    format!("    {} {} {}\n", field_name, ref_name, attrs.join(" "))
}

fn generate_model_attributes(table: &TableInfo) -> String {
    let mut output = String::new();

    // @@map if table name differs from model name
    let model_name = pascal_case(&table.name);
    if model_name.to_lowercase() != table.name.to_lowercase() {
        output.push_str(&format!("    @@map(\"{}\")\n", table.name));
    }

    // Composite primary key
    if table.primary_key.len() > 1 {
        let fields: Vec<_> = table.primary_key.iter().map(|c| camel_case(c)).collect();
        output.push_str(&format!("    @@id([{}])\n", fields.join(", ")));
    }

    // Indexes
    for idx in &table.indexes {
        if !idx.is_primary {
            let cols: Vec<_> = idx.columns.iter().map(|c| camel_case(&c.name)).collect();
            if idx.is_unique {
                output.push_str(&format!("    @@unique([{}])\n", cols.join(", ")));
            } else {
                output.push_str(&format!("    @@index([{}])\n", cols.join(", ")));
            }
        }
    }

    output
}

fn generate_view(view: &ViewInfo) -> String {
    let mut output = String::new();

    let keyword = if view.is_materialized {
        "materializedView"
    } else {
        "view"
    };
    output.push_str(&format!("{} {} {{\n", keyword, pascal_case(&view.name)));

    for col in &view.columns {
        let type_str = col.normalized_type.to_prax_type();
        let optional = if col.nullable { "?" } else { "" };
        output.push_str(&format!(
            "    {} {}{}\n",
            camel_case(&col.name),
            type_str,
            optional
        ));
    }

    if let Some(ref def) = view.definition {
        output.push_str(&format!("\n    @@sql(\"{}\")\n", def.replace('"', "\\\"")));
    }

    output.push_str("}\n");
    output
}

// ============================================================================
// MongoDB Introspection
// ============================================================================

/// MongoDB collection introspection.
pub mod mongodb {
    use serde_json::Value as JsonValue;

    use super::{ColumnInfo, NormalizedType, TableInfo};

    /// Infer schema from MongoDB documents.
    #[derive(Debug, Clone, Default)]
    pub struct SchemaInferrer {
        /// Field types discovered.
        pub fields: std::collections::HashMap<String, FieldSchema>,
        /// Sample size.
        pub samples: usize,
    }

    /// Inferred field schema.
    #[derive(Debug, Clone, Default)]
    pub struct FieldSchema {
        /// Field name.
        pub name: String,
        /// Types observed.
        pub types: Vec<String>,
        /// Whether field is always present.
        pub required: bool,
        /// Nested fields (for objects).
        pub nested: Option<Box<SchemaInferrer>>,
        /// Array element type.
        pub array_type: Option<String>,
    }

    impl SchemaInferrer {
        /// Create a new inferrer.
        pub fn new() -> Self {
            Self::default()
        }

        /// Add a document sample.
        pub fn add_document(&mut self, doc: &JsonValue) {
            self.samples += 1;

            if let Some(obj) = doc.as_object() {
                for (key, value) in obj {
                    self.infer_field(key, value);
                }
            }
        }

        fn infer_field(&mut self, name: &str, value: &JsonValue) {
            let field = self
                .fields
                .entry(name.to_string())
                .or_insert_with(|| FieldSchema {
                    name: name.to_string(),
                    required: true,
                    ..Default::default()
                });

            let type_name = match value {
                JsonValue::Null => "null",
                JsonValue::Bool(_) => "boolean",
                JsonValue::Number(n) if n.is_i64() => "int",
                JsonValue::Number(n) if n.is_f64() => "double",
                JsonValue::Number(_) => "number",
                JsonValue::String(s) => {
                    // Try to detect special types
                    if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                        "objectId"
                    } else if is_iso_datetime(s) {
                        "date"
                    } else {
                        "string"
                    }
                }
                JsonValue::Array(arr) => {
                    if let Some(first) = arr.first() {
                        let elem_type = match first {
                            JsonValue::Object(_) => "object",
                            JsonValue::String(_) => "string",
                            JsonValue::Number(_) => "number",
                            JsonValue::Bool(_) => "boolean",
                            _ => "mixed",
                        };
                        field.array_type = Some(elem_type.to_string());
                    }
                    "array"
                }
                JsonValue::Object(_) => {
                    // Recurse for nested objects
                    let mut nested = field.nested.take().unwrap_or_default();
                    nested.add_document(value);
                    field.nested = Some(nested);
                    "object"
                }
            };

            if !field.types.contains(&type_name.to_string()) {
                field.types.push(type_name.to_string());
            }
        }

        /// Convert to TableInfo.
        pub fn to_table_info(&self, collection_name: &str) -> TableInfo {
            let mut columns = Vec::new();

            for (name, field) in &self.fields {
                let normalized = infer_normalized_type(field);
                columns.push(ColumnInfo {
                    name: name.clone(),
                    db_type: field.types.join("|"),
                    normalized_type: normalized,
                    nullable: !field.required || field.types.contains(&"null".to_string()),
                    is_primary_key: name == "_id",
                    ..Default::default()
                });
            }

            TableInfo {
                name: collection_name.to_string(),
                columns,
                primary_key: vec!["_id".to_string()],
                ..Default::default()
            }
        }
    }

    fn infer_normalized_type(field: &FieldSchema) -> NormalizedType {
        // Pick most specific type
        if field.types.contains(&"objectId".to_string()) {
            NormalizedType::String // ObjectId maps to String
        } else if field.types.contains(&"date".to_string()) {
            NormalizedType::DateTime
        } else if field.types.contains(&"boolean".to_string()) {
            NormalizedType::Boolean
        } else if field.types.contains(&"int".to_string()) {
            NormalizedType::Int
        } else if field.types.contains(&"double".to_string())
            || field.types.contains(&"number".to_string())
        {
            NormalizedType::Double
        } else if field.types.contains(&"array".to_string()) {
            let inner = match field.array_type.as_deref() {
                Some("string") => NormalizedType::String,
                Some("number") => NormalizedType::Double,
                Some("boolean") => NormalizedType::Boolean,
                _ => NormalizedType::Json,
            };
            NormalizedType::Array(Box::new(inner))
        } else if field.types.contains(&"object".to_string()) {
            NormalizedType::Json
        } else if field.types.contains(&"string".to_string()) {
            NormalizedType::String
        } else {
            NormalizedType::Unknown(field.types.join("|"))
        }
    }

    /// Generate MongoDB collection indexes command.
    pub fn list_indexes_command(collection: &str) -> JsonValue {
        serde_json::json!({
            "listIndexes": collection
        })
    }

    /// Generate MongoDB list collections command.
    pub fn list_collections_command() -> JsonValue {
        serde_json::json!({
            "listCollections": 1
        })
    }

    /// Simple ISO datetime detection without chrono dependency.
    fn is_iso_datetime(s: &str) -> bool {
        // Check for ISO 8601 format: YYYY-MM-DDTHH:MM:SS or similar
        if s.len() < 10 {
            return false;
        }

        let bytes = s.as_bytes();
        // Check YYYY-MM-DD pattern
        bytes.get(4) == Some(&b'-')
            && bytes.get(7) == Some(&b'-')
            && bytes[0..4].iter().all(|b| b.is_ascii_digit())
            && bytes[5..7].iter().all(|b| b.is_ascii_digit())
            && bytes[8..10].iter().all(|b| b.is_ascii_digit())
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn pascal_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

fn camel_case(s: &str) -> String {
    let pascal = pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().chain(chars).collect(),
    }
}

fn simplify_default(default: &str) -> String {
    // Simplify common default expressions
    let d = default.trim();

    if d.eq_ignore_ascii_case("now()") || d.eq_ignore_ascii_case("current_timestamp") {
        return "now()".to_string();
    }

    if d.starts_with("'") && d.ends_with("'") {
        return format!("\"{}\"", &d[1..d.len() - 1]);
    }

    if d.eq_ignore_ascii_case("true") || d.eq_ignore_ascii_case("false") {
        return d.to_lowercase();
    }

    if d.parse::<i64>().is_ok() || d.parse::<f64>().is_ok() {
        return d.to_string();
    }

    format!("dbgenerated(\"{}\")", d.replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pascal_case() {
        assert_eq!(pascal_case("user_profile"), "UserProfile");
        assert_eq!(pascal_case("id"), "Id");
        assert_eq!(pascal_case("created_at"), "CreatedAt");
    }

    #[test]
    fn test_camel_case() {
        assert_eq!(camel_case("user_profile"), "userProfile");
        assert_eq!(camel_case("ID"), "iD");
        assert_eq!(camel_case("created_at"), "createdAt");
    }

    #[test]
    fn test_normalize_postgres_type() {
        assert_eq!(
            normalize_postgres_type("int4", None, None, None),
            NormalizedType::Int
        );
        assert_eq!(
            normalize_postgres_type("bigint", None, None, None),
            NormalizedType::BigInt
        );
        assert_eq!(
            normalize_postgres_type("text", None, None, None),
            NormalizedType::Text
        );
        assert_eq!(
            normalize_postgres_type("timestamptz", None, None, None),
            NormalizedType::DateTime
        );
        assert_eq!(
            normalize_postgres_type("jsonb", None, None, None),
            NormalizedType::Json
        );
        assert_eq!(
            normalize_postgres_type("uuid", None, None, None),
            NormalizedType::Uuid
        );
    }

    #[test]
    fn test_normalize_mysql_type() {
        assert_eq!(
            normalize_mysql_type("int", None, None, None),
            NormalizedType::Int
        );
        assert_eq!(
            normalize_mysql_type("varchar", Some(255), None, None),
            NormalizedType::VarChar { length: Some(255) }
        );
        assert_eq!(
            normalize_mysql_type("datetime", None, None, None),
            NormalizedType::DateTime
        );
    }

    #[test]
    fn test_referential_action() {
        assert_eq!(
            ReferentialAction::from_str("CASCADE"),
            ReferentialAction::Cascade
        );
        assert_eq!(
            ReferentialAction::from_str("SET NULL"),
            ReferentialAction::SetNull
        );
        assert_eq!(
            ReferentialAction::from_str("NO ACTION"),
            ReferentialAction::NoAction
        );
    }

    #[test]
    fn test_generate_simple_model() {
        let table = TableInfo {
            name: "users".to_string(),
            columns: vec![
                ColumnInfo {
                    name: "id".to_string(),
                    normalized_type: NormalizedType::Int,
                    auto_increment: true,
                    ..Default::default()
                },
                ColumnInfo {
                    name: "email".to_string(),
                    normalized_type: NormalizedType::String,
                    is_unique: true,
                    ..Default::default()
                },
                ColumnInfo {
                    name: "created_at".to_string(),
                    normalized_type: NormalizedType::DateTime,
                    nullable: true,
                    default: Some("now()".to_string()),
                    ..Default::default()
                },
            ],
            primary_key: vec!["id".to_string()],
            ..Default::default()
        };

        let schema = generate_model(&table, &[]);
        assert!(schema.contains("model Users"));
        assert!(schema.contains("id Int @id @auto"));
        assert!(schema.contains("email String @unique"));
        assert!(schema.contains("createdAt DateTime?"));
    }

    #[test]
    fn test_simplify_default() {
        assert_eq!(simplify_default("NOW()"), "now()");
        assert_eq!(simplify_default("CURRENT_TIMESTAMP"), "now()");
        assert_eq!(simplify_default("'hello'"), "\"hello\"");
        assert_eq!(simplify_default("42"), "42");
        assert_eq!(simplify_default("true"), "true");
    }

    #[test]
    fn test_queries_tables() {
        let pg = queries::tables_query(DatabaseType::PostgreSQL, Some("public"));
        assert!(pg.contains("information_schema.tables"));
        assert!(pg.contains("public"));

        let mysql = queries::tables_query(DatabaseType::MySQL, None);
        assert!(mysql.contains("information_schema.tables"));

        let sqlite = queries::tables_query(DatabaseType::SQLite, None);
        assert!(sqlite.contains("sqlite_master"));
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_schema_inferrer() {
            let mut inferrer = SchemaInferrer::new();

            inferrer.add_document(&serde_json::json!({
                "_id": "507f1f77bcf86cd799439011",
                "name": "Alice",
                "age": 30,
                "active": true
            }));

            inferrer.add_document(&serde_json::json!({
                "_id": "507f1f77bcf86cd799439012",
                "name": "Bob",
                "age": 25,
                "active": false,
                "email": "bob@example.com"
            }));

            let table = inferrer.to_table_info("users");
            assert_eq!(table.name, "users");
            assert!(table.columns.iter().any(|c| c.name == "_id"));
            assert!(table.columns.iter().any(|c| c.name == "name"));
            assert!(table.columns.iter().any(|c| c.name == "age"));
        }
    }
}
