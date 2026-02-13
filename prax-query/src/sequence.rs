//! Database sequence definitions and operations.
//!
//! This module provides types for defining and manipulating database sequences
//! across different database backends.
//!
//! # Supported Features
//!
//! | Feature                 | PostgreSQL | MySQL | SQLite | MSSQL | MongoDB |
//! |-------------------------|------------|-------|--------|-------|---------|
//! | Sequences               | ✅         | ❌    | ❌     | ✅    | ❌*     |
//! | Custom start/increment  | ✅         | ✅    | ❌     | ✅    | ✅*     |
//! | Sequence manipulation   | ✅         | ❌    | ❌     | ✅    | ✅*     |
//! | Auto-increment pattern  | ✅         | ✅    | ✅     | ✅    | ✅      |
//!
//! > *MongoDB uses counter collections with `findAndModify`
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::sequence::{Sequence, SequenceBuilder};
//!
//! // Define a sequence
//! let seq = Sequence::builder("order_number")
//!     .start(1000)
//!     .increment(1)
//!     .min_value(1)
//!     .max_value(i64::MAX)
//!     .cycle(false)
//!     .cache(20)
//!     .build();
//!
//! // Generate SQL
//! let create_sql = seq.to_postgres_create_sql();
//!
//! // Get next value
//! let next_val_sql = seq.nextval_sql(DatabaseType::PostgreSQL);
//! ```

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

/// A database sequence definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequence {
    /// Sequence name.
    pub name: String,
    /// Schema name (optional).
    pub schema: Option<String>,
    /// Starting value.
    pub start: i64,
    /// Increment value.
    pub increment: i64,
    /// Minimum value (None for no minimum).
    pub min_value: Option<i64>,
    /// Maximum value (None for no maximum).
    pub max_value: Option<i64>,
    /// Whether the sequence cycles when reaching max/min.
    pub cycle: bool,
    /// Number of values to cache (for performance).
    pub cache: Option<i64>,
    /// Whether the sequence is owned by a column.
    pub owned_by: Option<OwnedBy>,
    /// Optional comment/description.
    pub comment: Option<String>,
}

/// Column ownership for a sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedBy {
    /// Table name.
    pub table: String,
    /// Column name.
    pub column: String,
}

impl OwnedBy {
    /// Create a new owned by specification.
    pub fn new(table: impl Into<String>, column: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            column: column.into(),
        }
    }
}

impl Sequence {
    /// Create a new sequence builder.
    pub fn builder(name: impl Into<String>) -> SequenceBuilder {
        SequenceBuilder::new(name)
    }

    /// Get the fully qualified sequence name.
    pub fn qualified_name(&self) -> Cow<'_, str> {
        match &self.schema {
            Some(schema) => Cow::Owned(format!("{}.{}", schema, self.name)),
            None => Cow::Borrowed(&self.name),
        }
    }

    /// Generate PostgreSQL CREATE SEQUENCE SQL.
    pub fn to_postgres_create_sql(&self) -> String {
        let mut sql = String::with_capacity(128);

        sql.push_str("CREATE SEQUENCE ");
        sql.push_str(&self.qualified_name());

        // AS bigint is implicit
        sql.push_str("\n    INCREMENT BY ");
        sql.push_str(&self.increment.to_string());

        if let Some(min) = self.min_value {
            sql.push_str("\n    MINVALUE ");
            sql.push_str(&min.to_string());
        } else {
            sql.push_str("\n    NO MINVALUE");
        }

        if let Some(max) = self.max_value {
            sql.push_str("\n    MAXVALUE ");
            sql.push_str(&max.to_string());
        } else {
            sql.push_str("\n    NO MAXVALUE");
        }

        sql.push_str("\n    START WITH ");
        sql.push_str(&self.start.to_string());

        if let Some(cache) = self.cache {
            sql.push_str("\n    CACHE ");
            sql.push_str(&cache.to_string());
        }

        if self.cycle {
            sql.push_str("\n    CYCLE");
        } else {
            sql.push_str("\n    NO CYCLE");
        }

        if let Some(ref owned) = self.owned_by {
            sql.push_str("\n    OWNED BY ");
            sql.push_str(&owned.table);
            sql.push('.');
            sql.push_str(&owned.column);
        }

        sql.push(';');

        sql
    }

    /// Generate MSSQL CREATE SEQUENCE SQL.
    pub fn to_mssql_create_sql(&self) -> String {
        let mut sql = String::with_capacity(128);

        sql.push_str("CREATE SEQUENCE ");
        sql.push_str(&self.qualified_name());
        sql.push_str(" AS BIGINT");

        sql.push_str("\n    START WITH ");
        sql.push_str(&self.start.to_string());

        sql.push_str("\n    INCREMENT BY ");
        sql.push_str(&self.increment.to_string());

        if let Some(min) = self.min_value {
            sql.push_str("\n    MINVALUE ");
            sql.push_str(&min.to_string());
        } else {
            sql.push_str("\n    NO MINVALUE");
        }

        if let Some(max) = self.max_value {
            sql.push_str("\n    MAXVALUE ");
            sql.push_str(&max.to_string());
        } else {
            sql.push_str("\n    NO MAXVALUE");
        }

        if let Some(cache) = self.cache {
            sql.push_str("\n    CACHE ");
            sql.push_str(&cache.to_string());
        } else {
            sql.push_str("\n    NO CACHE");
        }

        if self.cycle {
            sql.push_str("\n    CYCLE");
        } else {
            sql.push_str("\n    NO CYCLE");
        }

        sql.push(';');

        sql
    }

    /// Generate CREATE SEQUENCE SQL for the specified database type.
    pub fn to_create_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_create_sql()),
            DatabaseType::MSSQL => Ok(self.to_mssql_create_sql()),
            DatabaseType::MySQL => Err(QueryError::unsupported(
                "MySQL does not support sequences. Use AUTO_INCREMENT columns instead.",
            )),
            DatabaseType::SQLite => Err(QueryError::unsupported(
                "SQLite does not support sequences. Use AUTOINCREMENT columns instead.",
            )),
        }
    }

    /// Generate DROP SEQUENCE SQL.
    pub fn to_drop_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!(
                "DROP SEQUENCE IF EXISTS {} CASCADE;",
                self.qualified_name()
            )),
            DatabaseType::MSSQL => Ok(format!(
                "DROP SEQUENCE IF EXISTS {};",
                self.qualified_name()
            )),
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences.",
            )),
        }
    }

    /// Generate ALTER SEQUENCE SQL.
    pub fn to_alter_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => {
                let mut sql = format!("ALTER SEQUENCE {}", self.qualified_name());

                sql.push_str(&format!("\n    INCREMENT BY {}", self.increment));

                if let Some(min) = self.min_value {
                    sql.push_str(&format!("\n    MINVALUE {}", min));
                }

                if let Some(max) = self.max_value {
                    sql.push_str(&format!("\n    MAXVALUE {}", max));
                }

                if let Some(cache) = self.cache {
                    sql.push_str(&format!("\n    CACHE {}", cache));
                }

                if self.cycle {
                    sql.push_str("\n    CYCLE");
                } else {
                    sql.push_str("\n    NO CYCLE");
                }

                sql.push(';');
                Ok(sql)
            }
            DatabaseType::MSSQL => {
                let mut sql = format!("ALTER SEQUENCE {}", self.qualified_name());

                sql.push_str(&format!("\n    INCREMENT BY {}", self.increment));

                if let Some(min) = self.min_value {
                    sql.push_str(&format!("\n    MINVALUE {}", min));
                }

                if let Some(max) = self.max_value {
                    sql.push_str(&format!("\n    MAXVALUE {}", max));
                }

                if let Some(cache) = self.cache {
                    sql.push_str(&format!("\n    CACHE {}", cache));
                } else {
                    sql.push_str("\n    NO CACHE");
                }

                if self.cycle {
                    sql.push_str("\n    CYCLE");
                } else {
                    sql.push_str("\n    NO CYCLE");
                }

                sql.push(';');
                Ok(sql)
            }
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences.",
            )),
        }
    }

    /// Generate SQL to restart the sequence at a specific value.
    pub fn restart_sql(&self, value: i64, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!(
                "ALTER SEQUENCE {} RESTART WITH {};",
                self.qualified_name(),
                value
            )),
            DatabaseType::MSSQL => Ok(format!(
                "ALTER SEQUENCE {} RESTART WITH {};",
                self.qualified_name(),
                value
            )),
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences.",
            )),
        }
    }
}

/// Builder for creating sequences.
#[derive(Debug, Clone)]
pub struct SequenceBuilder {
    name: String,
    schema: Option<String>,
    start: i64,
    increment: i64,
    min_value: Option<i64>,
    max_value: Option<i64>,
    cycle: bool,
    cache: Option<i64>,
    owned_by: Option<OwnedBy>,
    comment: Option<String>,
}

impl SequenceBuilder {
    /// Create a new sequence builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            start: 1,
            increment: 1,
            min_value: None,
            max_value: None,
            cycle: false,
            cache: None,
            owned_by: None,
            comment: None,
        }
    }

    /// Set the schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set the starting value.
    pub fn start(mut self, value: i64) -> Self {
        self.start = value;
        self
    }

    /// Set the increment value.
    pub fn increment(mut self, value: i64) -> Self {
        self.increment = value;
        self
    }

    /// Alias for increment.
    pub fn increment_by(self, value: i64) -> Self {
        self.increment(value)
    }

    /// Set the minimum value.
    pub fn min_value(mut self, value: i64) -> Self {
        self.min_value = Some(value);
        self
    }

    /// Set no minimum value.
    pub fn no_min_value(mut self) -> Self {
        self.min_value = None;
        self
    }

    /// Set the maximum value.
    pub fn max_value(mut self, value: i64) -> Self {
        self.max_value = Some(value);
        self
    }

    /// Set no maximum value.
    pub fn no_max_value(mut self) -> Self {
        self.max_value = None;
        self
    }

    /// Set whether the sequence cycles.
    pub fn cycle(mut self, cycle: bool) -> Self {
        self.cycle = cycle;
        self
    }

    /// Set the cache size.
    pub fn cache(mut self, size: i64) -> Self {
        self.cache = Some(size);
        self
    }

    /// Set no caching.
    pub fn no_cache(mut self) -> Self {
        self.cache = None;
        self
    }

    /// Set the column that owns this sequence.
    pub fn owned_by(mut self, table: impl Into<String>, column: impl Into<String>) -> Self {
        self.owned_by = Some(OwnedBy::new(table, column));
        self
    }

    /// Add a comment/description.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Build the sequence.
    pub fn build(self) -> Sequence {
        Sequence {
            name: self.name,
            schema: self.schema,
            start: self.start,
            increment: self.increment,
            min_value: self.min_value,
            max_value: self.max_value,
            cycle: self.cycle,
            cache: self.cache,
            owned_by: self.owned_by,
            comment: self.comment,
        }
    }
}

/// Sequence operations for retrieving and manipulating sequence values.
pub mod ops {
    use super::*;

    /// Generate SQL for getting the next value from a sequence.
    pub fn nextval(sequence_name: &str, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!("SELECT nextval('{}')", sequence_name)),
            DatabaseType::MSSQL => Ok(format!("SELECT NEXT VALUE FOR {}", sequence_name)),
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences.",
            )),
        }
    }

    /// Generate SQL for getting the current value of a sequence (last retrieved value in session).
    pub fn currval(sequence_name: &str, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!("SELECT currval('{}')", sequence_name)),
            DatabaseType::MSSQL => {
                // MSSQL doesn't have a direct equivalent to currval
                // You need to use a variable or sys.sequences
                Ok(format!(
                    "SELECT current_value FROM sys.sequences WHERE name = '{}'",
                    sequence_name
                ))
            }
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences.",
            )),
        }
    }

    /// Generate SQL for setting a sequence to a specific value.
    pub fn setval(
        sequence_name: &str,
        value: i64,
        is_called: bool,
        db_type: DatabaseType,
    ) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!(
                "SELECT setval('{}', {}, {})",
                sequence_name, value, is_called
            )),
            DatabaseType::MSSQL => {
                // MSSQL uses ALTER SEQUENCE ... RESTART WITH
                Ok(format!(
                    "ALTER SEQUENCE {} RESTART WITH {}",
                    sequence_name, value
                ))
            }
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences.",
            )),
        }
    }

    /// Generate SQL for getting the last inserted ID (auto-increment value).
    ///
    /// This works for databases that don't have sequences but use auto-increment.
    pub fn last_insert_id(db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => "SELECT lastval()".to_string(),
            DatabaseType::MySQL => "SELECT LAST_INSERT_ID()".to_string(),
            DatabaseType::SQLite => "SELECT last_insert_rowid()".to_string(),
            DatabaseType::MSSQL => "SELECT SCOPE_IDENTITY()".to_string(),
        }
    }

    /// Generate SQL expression for using a sequence as a default value.
    pub fn default_nextval(sequence_name: &str, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!("nextval('{}')", sequence_name)),
            DatabaseType::MSSQL => Ok(format!("NEXT VALUE FOR {}", sequence_name)),
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "This database does not support sequences in default expressions.",
            )),
        }
    }
}

/// MongoDB counter-based sequence pattern.
///
/// MongoDB doesn't have native sequences, but you can implement them using
/// a counter collection with `findAndModify`.
pub mod mongodb {
    use serde::{Deserialize, Serialize};

    /// A counter-based sequence for MongoDB.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Counter {
        /// Counter/sequence name.
        pub name: String,
        /// Current value.
        pub value: i64,
        /// Increment amount.
        pub increment: i64,
    }

    impl Counter {
        /// Create a new counter.
        pub fn new(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                value: 0,
                increment: 1,
            }
        }

        /// Set the increment value.
        pub fn with_increment(mut self, increment: i64) -> Self {
            self.increment = increment;
            self
        }

        /// Set the initial value.
        pub fn with_initial_value(mut self, value: i64) -> Self {
            self.value = value;
            self
        }
    }

    /// Builder for MongoDB counter operations.
    #[derive(Debug, Clone)]
    pub struct CounterBuilder {
        /// Collection name for counters.
        pub collection: String,
        /// Counter name.
        pub name: String,
        /// Increment amount.
        pub increment: i64,
    }

    impl CounterBuilder {
        /// Create a new counter builder.
        pub fn new(name: impl Into<String>) -> Self {
            Self {
                collection: "counters".to_string(),
                name: name.into(),
                increment: 1,
            }
        }

        /// Set the collection name for counters.
        pub fn collection(mut self, collection: impl Into<String>) -> Self {
            self.collection = collection.into();
            self
        }

        /// Set the increment amount.
        pub fn increment(mut self, increment: i64) -> Self {
            self.increment = increment;
            self
        }

        /// Get the findAndModify command document for getting the next value.
        ///
        /// Returns a JSON-like structure that can be used with MongoDB driver.
        pub fn next_value_command(&self) -> serde_json::Value {
            serde_json::json!({
                "findAndModify": &self.collection,
                "query": { "_id": &self.name },
                "update": { "$inc": { "seq": self.increment } },
                "new": true,
                "upsert": true
            })
        }

        /// Get the aggregation pipeline stage for incrementing the counter.
        pub fn increment_pipeline(&self) -> Vec<serde_json::Value> {
            vec![
                serde_json::json!({
                    "$match": { "_id": &self.name }
                }),
                serde_json::json!({
                    "$set": { "seq": { "$add": ["$seq", self.increment] } }
                }),
            ]
        }

        /// Get the document for initializing a counter.
        pub fn init_document(&self, start_value: i64) -> serde_json::Value {
            serde_json::json!({
                "_id": &self.name,
                "seq": start_value
            })
        }

        /// Get the update document for resetting a counter.
        pub fn reset_document(&self, value: i64) -> serde_json::Value {
            serde_json::json!({
                "$set": { "seq": value }
            })
        }
    }

    /// Helper function to create a counter builder.
    pub fn counter(name: impl Into<String>) -> CounterBuilder {
        CounterBuilder::new(name)
    }
}

/// Auto-increment column helpers for databases without sequence support.
pub mod auto_increment {
    use super::*;

    /// Generate SQL for creating an auto-increment column.
    pub fn column_definition(
        column_name: &str,
        db_type: DatabaseType,
        start: Option<i64>,
    ) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                // PostgreSQL uses SERIAL or GENERATED ... AS IDENTITY
                if let Some(start_val) = start {
                    format!(
                        "{} BIGINT GENERATED BY DEFAULT AS IDENTITY (START WITH {})",
                        column_name, start_val
                    )
                } else {
                    format!("{} BIGSERIAL", column_name)
                }
            }
            DatabaseType::MySQL => {
                if let Some(start_val) = start {
                    format!(
                        "{} BIGINT AUTO_INCREMENT /* Start: {} */",
                        column_name, start_val
                    )
                } else {
                    format!("{} BIGINT AUTO_INCREMENT", column_name)
                }
            }
            DatabaseType::SQLite => {
                // SQLite uses INTEGER PRIMARY KEY for auto-increment
                format!("{} INTEGER PRIMARY KEY AUTOINCREMENT", column_name)
            }
            DatabaseType::MSSQL => {
                if let Some(start_val) = start {
                    format!("{} BIGINT IDENTITY({}, 1)", column_name, start_val)
                } else {
                    format!("{} BIGINT IDENTITY(1, 1)", column_name)
                }
            }
        }
    }

    /// Generate SQL for setting the auto-increment start value for a table.
    pub fn set_start_value(
        table_name: &str,
        value: i64,
        db_type: DatabaseType,
    ) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => {
                // For IDENTITY columns
                Ok(format!(
                    "ALTER TABLE {} ALTER COLUMN id RESTART WITH {};",
                    table_name, value
                ))
            }
            DatabaseType::MySQL => Ok(format!(
                "ALTER TABLE {} AUTO_INCREMENT = {};",
                table_name, value
            )),
            DatabaseType::SQLite => {
                // SQLite uses sqlite_sequence table
                Ok(format!(
                    "UPDATE sqlite_sequence SET seq = {} WHERE name = '{}';",
                    value - 1,
                    table_name
                ))
            }
            DatabaseType::MSSQL => {
                // MSSQL requires DBCC CHECKIDENT
                Ok(format!(
                    "DBCC CHECKIDENT ('{}', RESEED, {});",
                    table_name,
                    value - 1
                ))
            }
        }
    }

    /// Generate SQL to get the current auto-increment value for a table.
    pub fn get_current_value(table_name: &str, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                format!(
                    "SELECT last_value FROM pg_sequences WHERE sequencename LIKE '%{}%';",
                    table_name
                )
            }
            DatabaseType::MySQL => {
                format!(
                    "SELECT AUTO_INCREMENT FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{}';",
                    table_name
                )
            }
            DatabaseType::SQLite => {
                format!(
                    "SELECT seq FROM sqlite_sequence WHERE name = '{}';",
                    table_name
                )
            }
            DatabaseType::MSSQL => {
                format!("SELECT IDENT_CURRENT('{}');", table_name)
            }
        }
    }
}

/// Pre-built sequence patterns for common use cases.
pub mod patterns {
    use super::*;

    /// Create an order number sequence starting at 1000.
    pub fn order_number(schema: Option<&str>) -> Sequence {
        let mut builder = Sequence::builder("order_number_seq")
            .start(1000)
            .increment(1)
            .min_value(1)
            .cache(20);

        if let Some(s) = schema {
            builder = builder.schema(s);
        }

        builder.build()
    }

    /// Create an invoice number sequence with yearly reset pattern.
    pub fn invoice_number(schema: Option<&str>, year: i32) -> Sequence {
        let mut builder = Sequence::builder(format!("invoice_{}_seq", year))
            .start(1)
            .increment(1)
            .min_value(1)
            .cache(10);

        if let Some(s) = schema {
            builder = builder.schema(s);
        }

        builder.build()
    }

    /// Create a high-performance ID sequence with large cache.
    pub fn high_volume_id(name: &str, schema: Option<&str>) -> Sequence {
        let mut builder = Sequence::builder(name)
            .start(1)
            .increment(1)
            .min_value(1)
            .cache(1000); // Large cache for high-volume inserts

        if let Some(s) = schema {
            builder = builder.schema(s);
        }

        builder.build()
    }

    /// Create a cycling sequence for round-robin distribution.
    pub fn round_robin(name: &str, max: i64, schema: Option<&str>) -> Sequence {
        let mut builder = Sequence::builder(name)
            .start(1)
            .increment(1)
            .min_value(1)
            .max_value(max)
            .cycle(true)
            .cache(10);

        if let Some(s) = schema {
            builder = builder.schema(s);
        }

        builder.build()
    }

    /// Create a negative sequence (counts down).
    pub fn countdown(name: &str, start: i64, schema: Option<&str>) -> Sequence {
        let mut builder = Sequence::builder(name)
            .start(start)
            .increment(-1)
            .min_value(0)
            .no_max_value();

        if let Some(s) = schema {
            builder = builder.schema(s);
        }

        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_builder() {
        let seq = Sequence::builder("order_seq")
            .start(1000)
            .increment(1)
            .min_value(1)
            .max_value(999999)
            .cache(20)
            .build();

        assert_eq!(seq.name, "order_seq");
        assert_eq!(seq.start, 1000);
        assert_eq!(seq.increment, 1);
        assert_eq!(seq.min_value, Some(1));
        assert_eq!(seq.max_value, Some(999999));
        assert_eq!(seq.cache, Some(20));
        assert!(!seq.cycle);
    }

    #[test]
    fn test_postgres_create_sql() {
        let seq = Sequence::builder("order_seq")
            .schema("public")
            .start(1000)
            .increment(1)
            .min_value(1)
            .cache(20)
            .build();

        let sql = seq.to_postgres_create_sql();
        assert!(sql.contains("CREATE SEQUENCE public.order_seq"));
        assert!(sql.contains("INCREMENT BY 1"));
        assert!(sql.contains("MINVALUE 1"));
        assert!(sql.contains("START WITH 1000"));
        assert!(sql.contains("CACHE 20"));
        assert!(sql.contains("NO CYCLE"));
    }

    #[test]
    fn test_mssql_create_sql() {
        let seq = Sequence::builder("order_seq")
            .schema("dbo")
            .start(1000)
            .increment(1)
            .build();

        let sql = seq.to_mssql_create_sql();
        assert!(sql.contains("CREATE SEQUENCE dbo.order_seq"));
        assert!(sql.contains("AS BIGINT"));
        assert!(sql.contains("START WITH 1000"));
        assert!(sql.contains("INCREMENT BY 1"));
    }

    #[test]
    fn test_mysql_not_supported() {
        let seq = Sequence::builder("test").build();
        let result = seq.to_create_sql(DatabaseType::MySQL);
        assert!(result.is_err());
    }

    #[test]
    fn test_sqlite_not_supported() {
        let seq = Sequence::builder("test").build();
        let result = seq.to_create_sql(DatabaseType::SQLite);
        assert!(result.is_err());
    }

    #[test]
    fn test_drop_sql() {
        let seq = Sequence::builder("order_seq").build();

        let pg_drop = seq.to_drop_sql(DatabaseType::PostgreSQL).unwrap();
        assert_eq!(pg_drop, "DROP SEQUENCE IF EXISTS order_seq CASCADE;");

        let mssql_drop = seq.to_drop_sql(DatabaseType::MSSQL).unwrap();
        assert_eq!(mssql_drop, "DROP SEQUENCE IF EXISTS order_seq;");
    }

    #[test]
    fn test_restart_sql() {
        let seq = Sequence::builder("order_seq").build();

        let pg_restart = seq.restart_sql(5000, DatabaseType::PostgreSQL).unwrap();
        assert_eq!(pg_restart, "ALTER SEQUENCE order_seq RESTART WITH 5000;");

        let mssql_restart = seq.restart_sql(5000, DatabaseType::MSSQL).unwrap();
        assert_eq!(mssql_restart, "ALTER SEQUENCE order_seq RESTART WITH 5000;");
    }

    #[test]
    fn test_cycle_sequence() {
        let seq = Sequence::builder("round_robin")
            .start(1)
            .max_value(10)
            .cycle(true)
            .build();

        let sql = seq.to_postgres_create_sql();
        assert!(sql.contains("MAXVALUE 10"));
        assert!(sql.contains("CYCLE"));
        assert!(!sql.contains("NO CYCLE"));
    }

    #[test]
    fn test_owned_by() {
        let seq = Sequence::builder("users_id_seq")
            .owned_by("users", "id")
            .build();

        let sql = seq.to_postgres_create_sql();
        assert!(sql.contains("OWNED BY users.id"));
    }

    mod ops_tests {
        use super::super::ops;
        use super::*;

        #[test]
        fn test_nextval() {
            let pg = ops::nextval("order_seq", DatabaseType::PostgreSQL).unwrap();
            assert_eq!(pg, "SELECT nextval('order_seq')");

            let mssql = ops::nextval("order_seq", DatabaseType::MSSQL).unwrap();
            assert_eq!(mssql, "SELECT NEXT VALUE FOR order_seq");
        }

        #[test]
        fn test_currval() {
            let pg = ops::currval("order_seq", DatabaseType::PostgreSQL).unwrap();
            assert_eq!(pg, "SELECT currval('order_seq')");
        }

        #[test]
        fn test_setval() {
            let pg = ops::setval("order_seq", 1000, true, DatabaseType::PostgreSQL).unwrap();
            assert_eq!(pg, "SELECT setval('order_seq', 1000, true)");

            let mssql = ops::setval("order_seq", 1000, true, DatabaseType::MSSQL).unwrap();
            assert_eq!(mssql, "ALTER SEQUENCE order_seq RESTART WITH 1000");
        }

        #[test]
        fn test_last_insert_id() {
            assert_eq!(
                ops::last_insert_id(DatabaseType::PostgreSQL),
                "SELECT lastval()"
            );
            assert_eq!(
                ops::last_insert_id(DatabaseType::MySQL),
                "SELECT LAST_INSERT_ID()"
            );
            assert_eq!(
                ops::last_insert_id(DatabaseType::SQLite),
                "SELECT last_insert_rowid()"
            );
            assert_eq!(
                ops::last_insert_id(DatabaseType::MSSQL),
                "SELECT SCOPE_IDENTITY()"
            );
        }

        #[test]
        fn test_default_nextval() {
            let pg = ops::default_nextval("order_seq", DatabaseType::PostgreSQL).unwrap();
            assert_eq!(pg, "nextval('order_seq')");

            let mssql = ops::default_nextval("order_seq", DatabaseType::MSSQL).unwrap();
            assert_eq!(mssql, "NEXT VALUE FOR order_seq");
        }
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_counter_builder() {
            let builder = counter("order_number").increment(1);
            let cmd = builder.next_value_command();

            assert_eq!(cmd["findAndModify"], "counters");
            assert_eq!(cmd["query"]["_id"], "order_number");
            assert_eq!(cmd["update"]["$inc"]["seq"], 1);
            assert_eq!(cmd["upsert"], true);
        }

        #[test]
        fn test_custom_collection() {
            let builder = counter("invoice_number").collection("sequences");
            let cmd = builder.next_value_command();

            assert_eq!(cmd["findAndModify"], "sequences");
        }

        #[test]
        fn test_init_document() {
            let builder = counter("order_number");
            let doc = builder.init_document(1000);

            assert_eq!(doc["_id"], "order_number");
            assert_eq!(doc["seq"], 1000);
        }

        #[test]
        fn test_reset_document() {
            let builder = counter("order_number");
            let doc = builder.reset_document(5000);

            assert_eq!(doc["$set"]["seq"], 5000);
        }
    }

    mod auto_increment_tests {
        use super::super::auto_increment;
        use super::*;

        #[test]
        fn test_column_definition() {
            let pg = auto_increment::column_definition("id", DatabaseType::PostgreSQL, None);
            assert_eq!(pg, "id BIGSERIAL");

            let pg_start =
                auto_increment::column_definition("id", DatabaseType::PostgreSQL, Some(1000));
            assert!(pg_start.contains("START WITH 1000"));

            let mysql = auto_increment::column_definition("id", DatabaseType::MySQL, None);
            assert!(mysql.contains("AUTO_INCREMENT"));

            let sqlite = auto_increment::column_definition("id", DatabaseType::SQLite, None);
            assert!(sqlite.contains("INTEGER PRIMARY KEY AUTOINCREMENT"));

            let mssql = auto_increment::column_definition("id", DatabaseType::MSSQL, Some(1000));
            assert!(mssql.contains("IDENTITY(1000, 1)"));
        }

        #[test]
        fn test_set_start_value() {
            let mysql =
                auto_increment::set_start_value("orders", 1000, DatabaseType::MySQL).unwrap();
            assert_eq!(mysql, "ALTER TABLE orders AUTO_INCREMENT = 1000;");

            let mssql =
                auto_increment::set_start_value("orders", 1000, DatabaseType::MSSQL).unwrap();
            assert!(mssql.contains("DBCC CHECKIDENT"));
        }
    }

    mod patterns_tests {
        use super::super::patterns;

        #[test]
        fn test_order_number() {
            let seq = patterns::order_number(Some("sales"));
            assert_eq!(seq.start, 1000);
            assert_eq!(seq.schema, Some("sales".to_string()));
        }

        #[test]
        fn test_round_robin() {
            let seq = patterns::round_robin("worker_queue", 10, None);
            assert!(seq.cycle);
            assert_eq!(seq.max_value, Some(10));
        }

        #[test]
        fn test_countdown() {
            let seq = patterns::countdown("tickets", 100, None);
            assert_eq!(seq.start, 100);
            assert_eq!(seq.increment, -1);
            assert_eq!(seq.min_value, Some(0));
        }
    }
}
