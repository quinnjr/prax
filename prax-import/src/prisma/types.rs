//! Prisma-specific type definitions for parsing.

use serde::{Deserialize, Serialize};

/// Prisma datasource configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrismaDatasource {
    /// The provider name (postgresql, mysql, sqlite, etc.).
    pub provider: String,
    /// The database connection URL.
    pub url: String,
}

/// Prisma model definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrismaModel {
    /// Model name.
    pub name: String,
    /// Model fields.
    pub fields: Vec<PrismaField>,
    /// Model-level attributes (@@id, @@index, etc.).
    pub attributes: Vec<PrismaModelAttribute>,
    /// Documentation comment.
    pub documentation: Option<String>,
}

/// Prisma field definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrismaField {
    /// Field name.
    pub name: String,
    /// Field type.
    pub field_type: PrismaFieldType,
    /// Whether the field is optional.
    pub is_optional: bool,
    /// Whether the field is a list.
    pub is_list: bool,
    /// Field-level attributes (@id, @unique, @default, etc.).
    pub attributes: Vec<PrismaFieldAttribute>,
    /// Documentation comment.
    pub documentation: Option<String>,
}

/// Prisma field type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PrismaFieldType {
    /// String type.
    String,
    /// Boolean type.
    Boolean,
    /// Int type.
    Int,
    /// BigInt type.
    BigInt,
    /// Float type.
    Float,
    /// Decimal type.
    Decimal,
    /// DateTime type.
    DateTime,
    /// Json type.
    Json,
    /// Bytes type.
    Bytes,
    /// Custom type (enum or model reference).
    Custom(String),
}

/// Prisma field attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrismaFieldAttribute {
    /// @id attribute.
    Id,
    /// @unique attribute.
    Unique,
    /// @default attribute.
    Default(PrismaDefaultValue),
    /// @map attribute.
    Map(String),
    /// @relation attribute.
    Relation {
        /// Relation name.
        name: Option<String>,
        /// Fields in this model.
        fields: Option<Vec<String>>,
        /// References in the related model.
        references: Option<Vec<String>>,
        /// On delete action.
        on_delete: Option<String>,
        /// On update action.
        on_update: Option<String>,
    },
    /// @updatedAt attribute.
    UpdatedAt,
}

/// Prisma default value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrismaDefaultValue {
    /// Literal value.
    Literal(String),
    /// Function call (autoincrement(), now(), uuid(), etc.).
    Function(String),
}

/// Prisma model-level attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrismaModelAttribute {
    /// @@id attribute (composite primary key).
    Id(Vec<String>),
    /// @@unique attribute.
    Unique {
        /// Fields in the unique constraint.
        fields: Vec<String>,
        /// Name of the constraint.
        name: Option<String>,
    },
    /// @@index attribute.
    Index {
        /// Fields in the index.
        fields: Vec<String>,
        /// Name of the index.
        name: Option<String>,
    },
    /// @@map attribute.
    Map(String),
}

/// Prisma enum definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrismaEnum {
    /// Enum name.
    pub name: String,
    /// Enum values.
    pub values: Vec<String>,
    /// Documentation comment.
    pub documentation: Option<String>,
}

/// Complete Prisma schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrismaSchema {
    /// Datasource configuration.
    pub datasource: Option<PrismaDatasource>,
    /// Models.
    pub models: Vec<PrismaModel>,
    /// Enums.
    pub enums: Vec<PrismaEnum>,
}
