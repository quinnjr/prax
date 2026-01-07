//! Diesel-specific type definitions for parsing.

use serde::{Deserialize, Serialize};

/// Diesel table definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DieselTable {
    /// Table name.
    pub name: String,
    /// Primary key columns.
    pub primary_key: Vec<String>,
    /// Table columns.
    pub columns: Vec<DieselColumn>,
}

/// Diesel column definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DieselColumn {
    /// Column name.
    pub name: String,
    /// SQL type.
    pub sql_type: DieselSqlType,
    /// Whether the column is nullable.
    pub is_nullable: bool,
}

/// Diesel SQL type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DieselSqlType {
    /// Integer (Int4).
    Int4,
    /// Big integer (Int8).
    Int8,
    /// Small integer (Int2).
    Int2,
    /// Float (Float4).
    Float4,
    /// Double (Float8).
    Float8,
    /// Numeric/Decimal.
    Numeric,
    /// Text.
    Text,
    /// Varchar.
    Varchar,
    /// Boolean.
    Bool,
    /// Timestamp.
    Timestamp,
    /// Date.
    Date,
    /// Time.
    Time,
    /// Json.
    Json,
    /// Jsonb.
    Jsonb,
    /// Binary data (Bytea).
    Bytea,
    /// UUID.
    Uuid,
    /// Nullable type wrapper.
    Nullable(Box<DieselSqlType>),
    /// Custom type.
    Custom(String),
}

/// Diesel joinable relationship.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DieselJoinable {
    /// Child table name.
    pub child_table: String,
    /// Parent table name.
    pub parent_table: String,
    /// Foreign key column in child table.
    pub foreign_key: String,
}

/// Complete Diesel schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DieselSchema {
    /// Tables.
    pub tables: Vec<DieselTable>,
    /// Joinable relationships.
    pub joinables: Vec<DieselJoinable>,
}
