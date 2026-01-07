//! SeaORM-specific type definitions for parsing.

use serde::{Deserialize, Serialize};

/// SeaORM entity definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeaOrmEntity {
    /// Entity/model name.
    pub name: String,
    /// Table name (from sea_orm attribute or derived from struct name).
    pub table_name: String,
    /// Entity fields.
    pub fields: Vec<SeaOrmField>,
    /// Relations.
    pub relations: Vec<SeaOrmRelation>,
    /// Documentation comment.
    pub documentation: Option<String>,
}

/// SeaORM field definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeaOrmField {
    /// Field name.
    pub name: String,
    /// Field type.
    pub field_type: SeaOrmFieldType,
    /// Whether the field is optional (Option<T>).
    pub is_optional: bool,
    /// Field-level attributes.
    pub attributes: Vec<SeaOrmFieldAttribute>,
    /// Documentation comment.
    pub documentation: Option<String>,
}

/// SeaORM field type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SeaOrmFieldType {
    /// i32 type.
    I32,
    /// i64 type.
    I64,
    /// f32 type.
    F32,
    /// f64 type.
    F64,
    /// String type.
    String,
    /// bool type.
    Bool,
    /// DateTime<Utc> type.
    DateTime,
    /// Date type.
    Date,
    /// Time type.
    Time,
    /// Decimal type.
    Decimal,
    /// Json type (serde_json::Value).
    Json,
    /// Vec<u8> (bytes).
    Bytes,
    /// Uuid type.
    Uuid,
    /// Custom type (enum or other struct).
    Custom(String),
}

/// SeaORM field attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SeaOrmFieldAttribute {
    /// primary_key attribute.
    PrimaryKey,
    /// auto_increment attribute.
    AutoIncrement,
    /// unique attribute.
    Unique,
    /// indexed attribute.
    Indexed,
    /// nullable attribute (explicit).
    Nullable,
    /// default_value attribute.
    DefaultValue(String),
    /// column_name attribute (different from field name).
    ColumnName(String),
    /// column_type attribute.
    ColumnType(String),
}

/// SeaORM relation definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeaOrmRelation {
    /// Relation name/variant.
    pub name: String,
    /// Relation type.
    pub relation_type: SeaOrmRelationType,
    /// Related entity.
    pub entity: String,
    /// Foreign key column (for belongs_to).
    pub from: Option<Vec<String>>,
    /// Referenced column (for belongs_to).
    pub to: Option<Vec<String>>,
    /// On delete action.
    pub on_delete: Option<String>,
    /// On update action.
    pub on_update: Option<String>,
}

/// SeaORM relation type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SeaOrmRelationType {
    /// has_one relation.
    HasOne,
    /// has_many relation.
    HasMany,
    /// belongs_to relation.
    BelongsTo,
    /// many_to_many relation.
    ManyToMany,
}

/// Complete SeaORM schema (collection of entities).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeaOrmSchema {
    /// Entities.
    pub entities: Vec<SeaOrmEntity>,
}
