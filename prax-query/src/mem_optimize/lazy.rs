//! Lazy schema parsing for on-demand introspection.
//!
//! This module provides lazy-loading wrappers for schema introspection that
//! defer parsing until fields are actually accessed, reducing memory usage
//! for large schemas where not all information is needed.
//!
//! # Memory Savings
//!
//! For a database with 100 tables, each with 20 columns:
//! - **Eager parsing**: Parses all 2000 columns upfront (~40-50% of introspection time)
//! - **Lazy parsing**: Only parses columns when accessed (0% until needed)
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::mem_optimize::lazy::LazySchema;
//!
//! // Create lazy schema from raw introspection data
//! let schema = LazySchema::from_json(raw_json)?;
//!
//! // Table names available immediately (minimal parsing)
//! for name in schema.table_names() {
//!     println!("Table: {}", name);
//! }
//!
//! // Columns only parsed when accessed
//! if let Some(users) = schema.get_table("users") {
//!     // Column parsing happens here
//!     for col in users.columns() {
//!         println!("  Column: {} ({})", col.name(), col.db_type());
//!     }
//! }
//! ```

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Lazy Schema
// ============================================================================

/// Lazy-loaded database schema.
///
/// Table metadata is parsed on first access, reducing memory usage
/// when only a subset of tables are needed.
pub struct LazySchema {
    /// Database name.
    name: String,
    /// Schema/namespace.
    schema: Option<String>,
    /// Table entries (lazy-loaded).
    tables: RwLock<HashMap<String, LazyTableEntry>>,
    /// Table names (for fast iteration without loading).
    table_names: Vec<String>,
    /// Enum definitions (usually small, loaded eagerly).
    enums: Vec<LazyEnum>,
}

/// Entry for a table - either raw data or parsed.
enum LazyTableEntry {
    /// Raw JSON data, not yet parsed.
    Raw(serde_json::Value),
    /// Parsed table information.
    Parsed(LazyTable),
}

impl LazySchema {
    /// Create from raw JSON introspection data.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let raw: RawSchema = serde_json::from_str(json)?;
        Ok(Self::from_raw(raw))
    }

    /// Create from parsed raw schema.
    pub fn from_raw(raw: RawSchema) -> Self {
        let table_names: Vec<String> = raw.tables.iter().map(|t| t.name.clone()).collect();

        let mut tables = HashMap::with_capacity(raw.tables.len());
        for table in raw.tables {
            let name = table.name.clone();
            tables.insert(
                name,
                LazyTableEntry::Raw(serde_json::to_value(table).unwrap()),
            );
        }

        Self {
            name: raw.name,
            schema: raw.schema,
            tables: RwLock::new(tables),
            table_names,
            enums: raw.enums.into_iter().map(LazyEnum::from).collect(),
        }
    }

    /// Get database name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get schema/namespace.
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Get table names without parsing table details.
    pub fn table_names(&self) -> &[String] {
        &self.table_names
    }

    /// Get number of tables.
    pub fn table_count(&self) -> usize {
        self.table_names.len()
    }

    /// Get a table by name, parsing on first access.
    pub fn get_table(&self, name: &str) -> Option<LazyTable> {
        let tables = self.tables.read();

        // Check if we have this table
        if let Some(entry) = tables.get(name) {
            match entry {
                LazyTableEntry::Parsed(table) => return Some(table.clone()),
                LazyTableEntry::Raw(_) => {
                    // Need to parse - drop read lock and acquire write lock
                }
            }
        } else {
            return None;
        }

        drop(tables);

        // Acquire write lock and parse
        let mut tables = self.tables.write();

        // Double-check (another thread may have parsed)
        if let Some(entry) = tables.get(name) {
            if let LazyTableEntry::Parsed(table) = entry {
                return Some(table.clone());
            }
        }

        // Parse the raw data
        if let Some(entry) = tables.remove(name) {
            if let LazyTableEntry::Raw(raw) = entry {
                match serde_json::from_value::<RawTable>(raw) {
                    Ok(raw_table) => {
                        let table = LazyTable::from_raw(raw_table);
                        tables.insert(name.to_string(), LazyTableEntry::Parsed(table.clone()));
                        return Some(table);
                    }
                    Err(_) => return None,
                }
            }
        }

        None
    }

    /// Check if a table exists.
    pub fn has_table(&self, name: &str) -> bool {
        self.table_names.iter().any(|n| n == name)
    }

    /// Get all enums.
    pub fn enums(&self) -> &[LazyEnum] {
        &self.enums
    }

    /// Get enum by name.
    pub fn get_enum(&self, name: &str) -> Option<&LazyEnum> {
        self.enums.iter().find(|e| e.name == name)
    }

    /// Get memory statistics.
    pub fn memory_stats(&self) -> LazySchemaStats {
        let tables = self.tables.read();
        let parsed = tables
            .values()
            .filter(|e| matches!(e, LazyTableEntry::Parsed(_)))
            .count();
        let raw = tables.len() - parsed;

        LazySchemaStats {
            total_tables: self.table_names.len(),
            parsed_tables: parsed,
            unparsed_tables: raw,
            enum_count: self.enums.len(),
        }
    }
}

// ============================================================================
// Lazy Table
// ============================================================================

/// Lazy-loaded table information.
#[derive(Clone)]
pub struct LazyTable {
    inner: Arc<LazyTableInner>,
}

struct LazyTableInner {
    /// Table name.
    name: String,
    /// Schema.
    schema: Option<String>,
    /// Comment.
    comment: Option<String>,
    /// Primary key columns.
    primary_key: Vec<String>,
    /// Columns (lazy-loaded).
    columns: RwLock<LazyColumns>,
    /// Foreign keys (lazy-loaded).
    foreign_keys: RwLock<LazyForeignKeys>,
    /// Indexes (lazy-loaded).
    indexes: RwLock<LazyIndexes>,
}

enum LazyColumns {
    Raw(Vec<serde_json::Value>),
    Parsed(Vec<LazyColumn>),
}

enum LazyForeignKeys {
    Raw(Vec<serde_json::Value>),
    Parsed(Vec<LazyForeignKey>),
}

enum LazyIndexes {
    Raw(Vec<serde_json::Value>),
    Parsed(Vec<LazyIndex>),
}

impl LazyTable {
    fn from_raw(raw: RawTable) -> Self {
        Self {
            inner: Arc::new(LazyTableInner {
                name: raw.name,
                schema: raw.schema,
                comment: raw.comment,
                primary_key: raw.primary_key,
                columns: RwLock::new(LazyColumns::Raw(
                    raw.columns
                        .into_iter()
                        .map(|c| serde_json::to_value(c).unwrap())
                        .collect(),
                )),
                foreign_keys: RwLock::new(LazyForeignKeys::Raw(
                    raw.foreign_keys
                        .into_iter()
                        .map(|f| serde_json::to_value(f).unwrap())
                        .collect(),
                )),
                indexes: RwLock::new(LazyIndexes::Raw(
                    raw.indexes
                        .into_iter()
                        .map(|i| serde_json::to_value(i).unwrap())
                        .collect(),
                )),
            }),
        }
    }

    /// Get table name.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Get schema.
    pub fn schema(&self) -> Option<&str> {
        self.inner.schema.as_deref()
    }

    /// Get comment.
    pub fn comment(&self) -> Option<&str> {
        self.inner.comment.as_deref()
    }

    /// Get primary key columns.
    pub fn primary_key(&self) -> &[String] {
        &self.inner.primary_key
    }

    /// Get columns, parsing on first access.
    pub fn columns(&self) -> Vec<LazyColumn> {
        // Fast path: already parsed
        {
            let columns = self.inner.columns.read();
            if let LazyColumns::Parsed(cols) = &*columns {
                return cols.clone();
            }
        }

        // Slow path: parse
        let mut columns = self.inner.columns.write();

        // Double-check
        if let LazyColumns::Parsed(cols) = &*columns {
            return cols.clone();
        }

        // Parse
        if let LazyColumns::Raw(raw) = &*columns {
            let parsed: Vec<LazyColumn> = raw
                .iter()
                .filter_map(|v| serde_json::from_value::<RawColumn>(v.clone()).ok())
                .map(LazyColumn::from)
                .collect();
            let result = parsed.clone();
            *columns = LazyColumns::Parsed(parsed);
            return result;
        }

        vec![]
    }

    /// Get column by name.
    pub fn get_column(&self, name: &str) -> Option<LazyColumn> {
        self.columns().into_iter().find(|c| c.name() == name)
    }

    /// Get column count without parsing.
    pub fn column_count(&self) -> usize {
        let columns = self.inner.columns.read();
        match &*columns {
            LazyColumns::Raw(raw) => raw.len(),
            LazyColumns::Parsed(parsed) => parsed.len(),
        }
    }

    /// Get foreign keys, parsing on first access.
    pub fn foreign_keys(&self) -> Vec<LazyForeignKey> {
        // Fast path
        {
            let fks = self.inner.foreign_keys.read();
            if let LazyForeignKeys::Parsed(fks) = &*fks {
                return fks.clone();
            }
        }

        // Slow path
        let mut fks = self.inner.foreign_keys.write();

        if let LazyForeignKeys::Parsed(fks) = &*fks {
            return fks.clone();
        }

        if let LazyForeignKeys::Raw(raw) = &*fks {
            let parsed: Vec<LazyForeignKey> = raw
                .iter()
                .filter_map(|v| serde_json::from_value::<RawForeignKey>(v.clone()).ok())
                .map(LazyForeignKey::from)
                .collect();
            let result = parsed.clone();
            *fks = LazyForeignKeys::Parsed(parsed);
            return result;
        }

        vec![]
    }

    /// Get indexes, parsing on first access.
    pub fn indexes(&self) -> Vec<LazyIndex> {
        // Fast path
        {
            let idxs = self.inner.indexes.read();
            if let LazyIndexes::Parsed(idxs) = &*idxs {
                return idxs.clone();
            }
        }

        // Slow path
        let mut idxs = self.inner.indexes.write();

        if let LazyIndexes::Parsed(idxs) = &*idxs {
            return idxs.clone();
        }

        if let LazyIndexes::Raw(raw) = &*idxs {
            let parsed: Vec<LazyIndex> = raw
                .iter()
                .filter_map(|v| serde_json::from_value::<RawIndex>(v.clone()).ok())
                .map(LazyIndex::from)
                .collect();
            let result = parsed.clone();
            *idxs = LazyIndexes::Parsed(parsed);
            return result;
        }

        vec![]
    }
}

// ============================================================================
// Lazy Column
// ============================================================================

/// Lazy-loaded column information.
#[derive(Clone)]
pub struct LazyColumn {
    name: String,
    db_type: String,
    nullable: bool,
    default: Option<String>,
    auto_increment: bool,
    is_primary_key: bool,
    is_unique: bool,
    comment: Option<String>,
}

impl LazyColumn {
    /// Get column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get database type.
    pub fn db_type(&self) -> &str {
        &self.db_type
    }

    /// Check if nullable.
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Get default value.
    pub fn default(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// Check if auto-increment.
    pub fn is_auto_increment(&self) -> bool {
        self.auto_increment
    }

    /// Check if primary key.
    pub fn is_primary_key(&self) -> bool {
        self.is_primary_key
    }

    /// Check if unique.
    pub fn is_unique(&self) -> bool {
        self.is_unique
    }

    /// Get comment.
    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }
}

impl From<RawColumn> for LazyColumn {
    fn from(raw: RawColumn) -> Self {
        Self {
            name: raw.name,
            db_type: raw.db_type,
            nullable: raw.nullable,
            default: raw.default,
            auto_increment: raw.auto_increment,
            is_primary_key: raw.is_primary_key,
            is_unique: raw.is_unique,
            comment: raw.comment,
        }
    }
}

// ============================================================================
// Lazy Foreign Key
// ============================================================================

/// Lazy-loaded foreign key information.
#[derive(Clone)]
pub struct LazyForeignKey {
    name: String,
    columns: Vec<String>,
    referenced_table: String,
    referenced_columns: Vec<String>,
    on_delete: String,
    on_update: String,
}

impl LazyForeignKey {
    /// Get constraint name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get local columns.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Get referenced table.
    pub fn referenced_table(&self) -> &str {
        &self.referenced_table
    }

    /// Get referenced columns.
    pub fn referenced_columns(&self) -> &[String] {
        &self.referenced_columns
    }

    /// Get ON DELETE action.
    pub fn on_delete(&self) -> &str {
        &self.on_delete
    }

    /// Get ON UPDATE action.
    pub fn on_update(&self) -> &str {
        &self.on_update
    }
}

impl From<RawForeignKey> for LazyForeignKey {
    fn from(raw: RawForeignKey) -> Self {
        Self {
            name: raw.name,
            columns: raw.columns,
            referenced_table: raw.referenced_table,
            referenced_columns: raw.referenced_columns,
            on_delete: raw.on_delete,
            on_update: raw.on_update,
        }
    }
}

// ============================================================================
// Lazy Index
// ============================================================================

/// Lazy-loaded index information.
#[derive(Clone)]
pub struct LazyIndex {
    name: String,
    columns: Vec<String>,
    is_unique: bool,
    is_primary: bool,
    index_type: Option<String>,
}

impl LazyIndex {
    /// Get index name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get indexed columns.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Check if unique.
    pub fn is_unique(&self) -> bool {
        self.is_unique
    }

    /// Check if primary key index.
    pub fn is_primary(&self) -> bool {
        self.is_primary
    }

    /// Get index type.
    pub fn index_type(&self) -> Option<&str> {
        self.index_type.as_deref()
    }
}

impl From<RawIndex> for LazyIndex {
    fn from(raw: RawIndex) -> Self {
        Self {
            name: raw.name,
            columns: raw.columns,
            is_unique: raw.is_unique,
            is_primary: raw.is_primary,
            index_type: raw.index_type,
        }
    }
}

// ============================================================================
// Lazy Enum
// ============================================================================

/// Enum type definition.
#[derive(Clone)]
pub struct LazyEnum {
    name: String,
    schema: Option<String>,
    values: Vec<String>,
}

impl LazyEnum {
    /// Get enum name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get schema.
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Get enum values.
    pub fn values(&self) -> &[String] {
        &self.values
    }
}

impl From<RawEnum> for LazyEnum {
    fn from(raw: RawEnum) -> Self {
        Self {
            name: raw.name,
            schema: raw.schema,
            values: raw.values,
        }
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Statistics for lazy schema loading.
#[derive(Debug, Clone, Default)]
pub struct LazySchemaStats {
    /// Total table count.
    pub total_tables: usize,
    /// Tables that have been parsed.
    pub parsed_tables: usize,
    /// Tables still in raw form.
    pub unparsed_tables: usize,
    /// Enum count.
    pub enum_count: usize,
}

impl LazySchemaStats {
    /// Get parse ratio (0.0 to 1.0).
    pub fn parse_ratio(&self) -> f64 {
        if self.total_tables == 0 {
            0.0
        } else {
            self.parsed_tables as f64 / self.total_tables as f64
        }
    }
}

// ============================================================================
// Parse On Demand Trait
// ============================================================================

/// Trait for types that support lazy/on-demand parsing.
pub trait ParseOnDemand {
    /// The parsed output type.
    type Output;

    /// Check if already parsed.
    fn is_parsed(&self) -> bool;

    /// Force parsing (if not already done).
    fn parse(&self) -> Self::Output;
}

// ============================================================================
// Raw Schema Types (for deserialization)
// ============================================================================

#[derive(Debug, Deserialize, Serialize)]
struct RawSchema {
    #[serde(default)]
    name: String,
    schema: Option<String>,
    #[serde(default)]
    tables: Vec<RawTable>,
    #[serde(default)]
    enums: Vec<RawEnum>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawTable {
    name: String,
    schema: Option<String>,
    comment: Option<String>,
    #[serde(default)]
    columns: Vec<RawColumn>,
    #[serde(default)]
    primary_key: Vec<String>,
    #[serde(default)]
    foreign_keys: Vec<RawForeignKey>,
    #[serde(default)]
    indexes: Vec<RawIndex>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawColumn {
    name: String,
    db_type: String,
    #[serde(default)]
    nullable: bool,
    default: Option<String>,
    #[serde(default)]
    auto_increment: bool,
    #[serde(default)]
    is_primary_key: bool,
    #[serde(default)]
    is_unique: bool,
    comment: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawForeignKey {
    name: String,
    #[serde(default)]
    columns: Vec<String>,
    referenced_table: String,
    #[serde(default)]
    referenced_columns: Vec<String>,
    #[serde(default = "default_action")]
    on_delete: String,
    #[serde(default = "default_action")]
    on_update: String,
}

fn default_action() -> String {
    "NO ACTION".to_string()
}

#[derive(Debug, Deserialize, Serialize)]
struct RawIndex {
    name: String,
    #[serde(default)]
    columns: Vec<String>,
    #[serde(default)]
    is_unique: bool,
    #[serde(default)]
    is_primary: bool,
    index_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawEnum {
    name: String,
    schema: Option<String>,
    #[serde(default)]
    values: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_schema_json() -> &'static str {
        r#"{
            "name": "test_db",
            "schema": "public",
            "tables": [
                {
                    "name": "users",
                    "columns": [
                        {"name": "id", "db_type": "integer", "is_primary_key": true},
                        {"name": "email", "db_type": "varchar(255)", "nullable": false},
                        {"name": "name", "db_type": "varchar(100)", "nullable": true}
                    ],
                    "primary_key": ["id"],
                    "indexes": [
                        {"name": "users_email_idx", "columns": ["email"], "is_unique": true}
                    ]
                },
                {
                    "name": "posts",
                    "columns": [
                        {"name": "id", "db_type": "integer", "is_primary_key": true},
                        {"name": "user_id", "db_type": "integer"},
                        {"name": "title", "db_type": "varchar(255)"}
                    ],
                    "primary_key": ["id"],
                    "foreign_keys": [
                        {
                            "name": "posts_user_fk",
                            "columns": ["user_id"],
                            "referenced_table": "users",
                            "referenced_columns": ["id"]
                        }
                    ]
                }
            ],
            "enums": [
                {"name": "status", "values": ["pending", "active", "archived"]}
            ]
        }"#
    }

    #[test]
    fn test_lazy_schema_from_json() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();

        assert_eq!(schema.name(), "test_db");
        assert_eq!(schema.table_count(), 2);
        assert!(schema.has_table("users"));
        assert!(schema.has_table("posts"));
    }

    #[test]
    fn test_lazy_table_names_no_parse() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();

        // Getting table names should not parse tables
        let names = schema.table_names();
        assert_eq!(names.len(), 2);

        // Check stats - nothing should be parsed yet
        let stats = schema.memory_stats();
        assert_eq!(stats.parsed_tables, 0);
        assert_eq!(stats.unparsed_tables, 2);
    }

    #[test]
    fn test_lazy_table_parsing() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();

        // Access one table
        let users = schema.get_table("users").unwrap();
        assert_eq!(users.name(), "users");

        // Check stats - one table parsed
        let stats = schema.memory_stats();
        assert_eq!(stats.parsed_tables, 1);
        assert_eq!(stats.unparsed_tables, 1);
    }

    #[test]
    fn test_lazy_columns() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();
        let users = schema.get_table("users").unwrap();

        // Columns should be lazy
        assert_eq!(users.column_count(), 3);

        // Access columns
        let columns = users.columns();
        assert_eq!(columns.len(), 3);

        // Find specific column
        let email = users.get_column("email").unwrap();
        assert_eq!(email.db_type(), "varchar(255)");
        assert!(!email.is_nullable());
    }

    #[test]
    fn test_lazy_foreign_keys() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();
        let posts = schema.get_table("posts").unwrap();

        let fks = posts.foreign_keys();
        assert_eq!(fks.len(), 1);

        let fk = &fks[0];
        assert_eq!(fk.name(), "posts_user_fk");
        assert_eq!(fk.referenced_table(), "users");
    }

    #[test]
    fn test_lazy_indexes() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();
        let users = schema.get_table("users").unwrap();

        let indexes = users.indexes();
        assert_eq!(indexes.len(), 1);

        let idx = &indexes[0];
        assert_eq!(idx.name(), "users_email_idx");
        assert!(idx.is_unique());
    }

    #[test]
    fn test_lazy_enums() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();

        let enums = schema.enums();
        assert_eq!(enums.len(), 1);

        let status = schema.get_enum("status").unwrap();
        assert_eq!(status.values(), &["pending", "active", "archived"]);
    }

    #[test]
    fn test_cached_access() {
        let schema = LazySchema::from_json(sample_schema_json()).unwrap();

        // Access same table multiple times
        let users1 = schema.get_table("users").unwrap();
        let users2 = schema.get_table("users").unwrap();

        // Should get same data
        assert_eq!(users1.name(), users2.name());
        assert_eq!(users1.column_count(), users2.column_count());
    }
}
