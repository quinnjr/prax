//! Table partitioning and sharding support.
//!
//! This module provides types for defining and managing table partitions
//! across different database backends.
//!
//! # Supported Features
//!
//! | Feature            | PostgreSQL | MySQL | SQLite | MSSQL | MongoDB |
//! |--------------------|------------|-------|--------|-------|---------|
//! | Range Partitioning | ✅         | ✅    | ❌     | ✅    | ✅*     |
//! | List Partitioning  | ✅         | ✅    | ❌     | ✅    | ❌      |
//! | Hash Partitioning  | ✅         | ✅    | ❌     | ✅    | ✅*     |
//! | Partition Pruning  | ✅         | ✅    | ❌     | ✅    | ✅      |
//! | Zone Sharding      | ❌         | ❌    | ❌     | ❌    | ✅      |
//!
//! > *MongoDB uses sharding with range or hashed shard keys
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::partition::{Partition, PartitionType, RangeBound};
//!
//! // Define a range-partitioned table
//! let partition = Partition::builder("orders")
//!     .range_partition()
//!     .column("created_at")
//!     .add_range("orders_2024_q1", RangeBound::from("2024-01-01"), RangeBound::to("2024-04-01"))
//!     .add_range("orders_2024_q2", RangeBound::from("2024-04-01"), RangeBound::to("2024-07-01"))
//!     .build();
//!
//! // Generate SQL
//! let sql = partition.to_postgres_sql()?;
//! ```

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

/// The type of partitioning strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PartitionType {
    /// Partition by value ranges (e.g., date ranges).
    Range,
    /// Partition by specific values (e.g., country codes).
    List,
    /// Partition by hash of column values.
    Hash,
}

impl PartitionType {
    /// Convert to SQL keyword for PostgreSQL.
    pub fn to_postgres_sql(&self) -> &'static str {
        match self {
            Self::Range => "RANGE",
            Self::List => "LIST",
            Self::Hash => "HASH",
        }
    }

    /// Convert to SQL keyword for MySQL.
    pub fn to_mysql_sql(&self) -> &'static str {
        match self {
            Self::Range => "RANGE",
            Self::List => "LIST",
            Self::Hash => "HASH",
        }
    }

    /// Convert to SQL keyword for MSSQL.
    pub fn to_mssql_sql(&self) -> &'static str {
        match self {
            Self::Range => "RANGE",
            Self::List => "LIST",
            Self::Hash => "HASH",
        }
    }
}

/// A bound value for range partitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeBound {
    /// Minimum value (MINVALUE in PostgreSQL).
    MinValue,
    /// Maximum value (MAXVALUE in PostgreSQL).
    MaxValue,
    /// A specific value.
    Value(String),
    /// A date value.
    Date(String),
    /// An integer value.
    Int(i64),
}

impl RangeBound {
    /// Create a specific value bound.
    pub fn value(v: impl Into<String>) -> Self {
        Self::Value(v.into())
    }

    /// Create a date bound.
    pub fn date(d: impl Into<String>) -> Self {
        Self::Date(d.into())
    }

    /// Create an integer bound.
    pub fn int(i: i64) -> Self {
        Self::Int(i)
    }

    /// Convert to SQL expression.
    pub fn to_sql(&self) -> Cow<'static, str> {
        match self {
            Self::MinValue => Cow::Borrowed("MINVALUE"),
            Self::MaxValue => Cow::Borrowed("MAXVALUE"),
            Self::Value(v) => Cow::Owned(format!("'{}'", v)),
            Self::Date(d) => Cow::Owned(format!("'{}'", d)),
            Self::Int(i) => Cow::Owned(i.to_string()),
        }
    }
}

/// A range partition definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangePartitionDef {
    /// Partition name.
    pub name: String,
    /// Lower bound (inclusive).
    pub from: RangeBound,
    /// Upper bound (exclusive).
    pub to: RangeBound,
    /// Tablespace (optional).
    pub tablespace: Option<String>,
}

impl RangePartitionDef {
    /// Create a new range partition definition.
    pub fn new(name: impl Into<String>, from: RangeBound, to: RangeBound) -> Self {
        Self {
            name: name.into(),
            from,
            to,
            tablespace: None,
        }
    }

    /// Set the tablespace.
    pub fn tablespace(mut self, tablespace: impl Into<String>) -> Self {
        self.tablespace = Some(tablespace.into());
        self
    }
}

/// A list partition definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPartitionDef {
    /// Partition name.
    pub name: String,
    /// Values in this partition.
    pub values: Vec<String>,
    /// Tablespace (optional).
    pub tablespace: Option<String>,
}

impl ListPartitionDef {
    /// Create a new list partition definition.
    pub fn new(
        name: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            values: values.into_iter().map(Into::into).collect(),
            tablespace: None,
        }
    }

    /// Set the tablespace.
    pub fn tablespace(mut self, tablespace: impl Into<String>) -> Self {
        self.tablespace = Some(tablespace.into());
        self
    }
}

/// A hash partition definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashPartitionDef {
    /// Partition name.
    pub name: String,
    /// Modulus for hash partitioning.
    pub modulus: u32,
    /// Remainder for this partition.
    pub remainder: u32,
    /// Tablespace (optional).
    pub tablespace: Option<String>,
}

impl HashPartitionDef {
    /// Create a new hash partition definition.
    pub fn new(name: impl Into<String>, modulus: u32, remainder: u32) -> Self {
        Self {
            name: name.into(),
            modulus,
            remainder,
            tablespace: None,
        }
    }

    /// Set the tablespace.
    pub fn tablespace(mut self, tablespace: impl Into<String>) -> Self {
        self.tablespace = Some(tablespace.into());
        self
    }
}

/// Partition definitions based on partition type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartitionDef {
    /// Range partitions.
    Range(Vec<RangePartitionDef>),
    /// List partitions.
    List(Vec<ListPartitionDef>),
    /// Hash partitions.
    Hash(Vec<HashPartitionDef>),
}

/// A table partition specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    /// Table name.
    pub table: String,
    /// Schema name (optional).
    pub schema: Option<String>,
    /// Partition type.
    pub partition_type: PartitionType,
    /// Partition columns.
    pub columns: Vec<String>,
    /// Partition definitions.
    pub partitions: PartitionDef,
    /// Optional comment.
    pub comment: Option<String>,
}

impl Partition {
    /// Create a new partition builder.
    pub fn builder(table: impl Into<String>) -> PartitionBuilder {
        PartitionBuilder::new(table)
    }

    /// Get the fully qualified table name.
    pub fn qualified_table(&self) -> Cow<'_, str> {
        match &self.schema {
            Some(schema) => Cow::Owned(format!("{}.{}", schema, self.table)),
            None => Cow::Borrowed(&self.table),
        }
    }

    /// Generate PostgreSQL CREATE TABLE with partitioning.
    pub fn to_postgres_partition_clause(&self) -> String {
        format!(
            "PARTITION BY {} ({})",
            self.partition_type.to_postgres_sql(),
            self.columns.join(", ")
        )
    }

    /// Generate PostgreSQL CREATE TABLE for a child partition.
    pub fn to_postgres_create_partition(&self, def: &RangePartitionDef) -> String {
        let mut sql = format!(
            "CREATE TABLE {} PARTITION OF {}\n    FOR VALUES FROM ({}) TO ({})",
            def.name,
            self.qualified_table(),
            def.from.to_sql(),
            def.to.to_sql()
        );

        if let Some(ref ts) = def.tablespace {
            sql.push_str(&format!("\n    TABLESPACE {}", ts));
        }

        sql.push(';');
        sql
    }

    /// Generate PostgreSQL CREATE TABLE for a list partition.
    pub fn to_postgres_create_list_partition(&self, def: &ListPartitionDef) -> String {
        let values: Vec<String> = def.values.iter().map(|v| format!("'{}'", v)).collect();

        let mut sql = format!(
            "CREATE TABLE {} PARTITION OF {}\n    FOR VALUES IN ({})",
            def.name,
            self.qualified_table(),
            values.join(", ")
        );

        if let Some(ref ts) = def.tablespace {
            sql.push_str(&format!("\n    TABLESPACE {}", ts));
        }

        sql.push(';');
        sql
    }

    /// Generate PostgreSQL CREATE TABLE for a hash partition.
    pub fn to_postgres_create_hash_partition(&self, def: &HashPartitionDef) -> String {
        let mut sql = format!(
            "CREATE TABLE {} PARTITION OF {}\n    FOR VALUES WITH (MODULUS {}, REMAINDER {})",
            def.name,
            self.qualified_table(),
            def.modulus,
            def.remainder
        );

        if let Some(ref ts) = def.tablespace {
            sql.push_str(&format!("\n    TABLESPACE {}", ts));
        }

        sql.push(';');
        sql
    }

    /// Generate all PostgreSQL partition creation SQL.
    pub fn to_postgres_create_all_partitions(&self) -> Vec<String> {
        match &self.partitions {
            PartitionDef::Range(ranges) => ranges
                .iter()
                .map(|r| self.to_postgres_create_partition(r))
                .collect(),
            PartitionDef::List(lists) => lists
                .iter()
                .map(|l| self.to_postgres_create_list_partition(l))
                .collect(),
            PartitionDef::Hash(hashes) => hashes
                .iter()
                .map(|h| self.to_postgres_create_hash_partition(h))
                .collect(),
        }
    }

    /// Generate MySQL PARTITION BY clause.
    pub fn to_mysql_partition_clause(&self) -> String {
        let columns_expr = if self.columns.len() == 1 {
            self.columns[0].clone()
        } else {
            format!("({})", self.columns.join(", "))
        };

        let mut sql = format!(
            "PARTITION BY {} ({})",
            self.partition_type.to_mysql_sql(),
            columns_expr
        );

        // Add partition definitions inline for MySQL
        match &self.partitions {
            PartitionDef::Range(ranges) => {
                sql.push_str(" (\n");
                for (i, r) in ranges.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(",\n");
                    }
                    sql.push_str(&format!(
                        "    PARTITION {} VALUES LESS THAN ({})",
                        r.name,
                        r.to.to_sql()
                    ));
                }
                sql.push_str("\n)");
            }
            PartitionDef::List(lists) => {
                sql.push_str(" (\n");
                for (i, l) in lists.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(",\n");
                    }
                    let values: Vec<String> = l.values.iter().map(|v| format!("'{}'", v)).collect();
                    sql.push_str(&format!(
                        "    PARTITION {} VALUES IN ({})",
                        l.name,
                        values.join(", ")
                    ));
                }
                sql.push_str("\n)");
            }
            PartitionDef::Hash(hashes) => {
                sql.push_str(&format!(" PARTITIONS {}", hashes.len()));
            }
        }

        sql
    }

    /// Generate MSSQL partition function and scheme.
    pub fn to_mssql_partition_sql(&self) -> QueryResult<Vec<String>> {
        match &self.partitions {
            PartitionDef::Range(ranges) => {
                let mut sqls = Vec::new();

                // Create partition function
                let boundaries: Vec<String> = ranges
                    .iter()
                    .filter(|r| !matches!(r.to, RangeBound::MaxValue))
                    .map(|r| r.to.to_sql().into_owned())
                    .collect();

                let func_name = format!("{}_pf", self.table);
                sqls.push(format!(
                    "CREATE PARTITION FUNCTION {}(datetime2)\nAS RANGE RIGHT FOR VALUES ({});",
                    func_name,
                    boundaries.join(", ")
                ));

                // Create partition scheme
                let scheme_name = format!("{}_ps", self.table);
                let filegroups: Vec<String> =
                    ranges.iter().map(|_| "PRIMARY".to_string()).collect();
                sqls.push(format!(
                    "CREATE PARTITION SCHEME {}\nAS PARTITION {}\nTO ({});",
                    scheme_name,
                    func_name,
                    filegroups.join(", ")
                ));

                Ok(sqls)
            }
            PartitionDef::List(_) => Err(QueryError::unsupported(
                "MSSQL uses partition functions differently for list partitioning. Consider using range partitioning.",
            )),
            PartitionDef::Hash(_) => Err(QueryError::unsupported(
                "MSSQL does not directly support hash partitioning. Use a computed column with range partitioning.",
            )),
        }
    }

    /// Generate SQL for attaching a partition.
    pub fn attach_partition_sql(
        &self,
        partition_name: &str,
        db_type: DatabaseType,
    ) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!(
                "ALTER TABLE {} ATTACH PARTITION {};",
                self.qualified_table(),
                partition_name
            )),
            DatabaseType::MySQL => Err(QueryError::unsupported(
                "MySQL does not support ATTACH PARTITION. Use ALTER TABLE ... REORGANIZE PARTITION.",
            )),
            DatabaseType::SQLite => Err(QueryError::unsupported(
                "SQLite does not support table partitioning.",
            )),
            DatabaseType::MSSQL => Err(QueryError::unsupported(
                "MSSQL uses SWITCH to move partitions. Use partition switching instead.",
            )),
        }
    }

    /// Generate SQL for detaching a partition.
    pub fn detach_partition_sql(
        &self,
        partition_name: &str,
        db_type: DatabaseType,
    ) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!(
                "ALTER TABLE {} DETACH PARTITION {};",
                self.qualified_table(),
                partition_name
            )),
            DatabaseType::MySQL => Err(QueryError::unsupported(
                "MySQL does not support DETACH PARTITION. Drop and recreate the partition.",
            )),
            DatabaseType::SQLite => Err(QueryError::unsupported(
                "SQLite does not support table partitioning.",
            )),
            DatabaseType::MSSQL => Err(QueryError::unsupported(
                "MSSQL uses SWITCH to move partitions. Use partition switching instead.",
            )),
        }
    }

    /// Generate SQL for dropping a partition.
    pub fn drop_partition_sql(
        &self,
        partition_name: &str,
        db_type: DatabaseType,
    ) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(format!("DROP TABLE IF EXISTS {};", partition_name)),
            DatabaseType::MySQL => Ok(format!(
                "ALTER TABLE {} DROP PARTITION {};",
                self.qualified_table(),
                partition_name
            )),
            DatabaseType::SQLite => Err(QueryError::unsupported(
                "SQLite does not support table partitioning.",
            )),
            DatabaseType::MSSQL => Ok(format!(
                "ALTER TABLE {} DROP PARTITION {};",
                self.qualified_table(),
                partition_name
            )),
        }
    }
}

/// Builder for creating partition specifications.
#[derive(Debug, Clone)]
pub struct PartitionBuilder {
    table: String,
    schema: Option<String>,
    partition_type: Option<PartitionType>,
    columns: Vec<String>,
    range_partitions: Vec<RangePartitionDef>,
    list_partitions: Vec<ListPartitionDef>,
    hash_partitions: Vec<HashPartitionDef>,
    comment: Option<String>,
}

impl PartitionBuilder {
    /// Create a new partition builder.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            schema: None,
            partition_type: None,
            columns: Vec::new(),
            range_partitions: Vec::new(),
            list_partitions: Vec::new(),
            hash_partitions: Vec::new(),
            comment: None,
        }
    }

    /// Set the schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set to range partitioning.
    pub fn range_partition(mut self) -> Self {
        self.partition_type = Some(PartitionType::Range);
        self
    }

    /// Set to list partitioning.
    pub fn list_partition(mut self) -> Self {
        self.partition_type = Some(PartitionType::List);
        self
    }

    /// Set to hash partitioning.
    pub fn hash_partition(mut self) -> Self {
        self.partition_type = Some(PartitionType::Hash);
        self
    }

    /// Add a partition column.
    pub fn column(mut self, column: impl Into<String>) -> Self {
        self.columns.push(column.into());
        self
    }

    /// Add multiple partition columns.
    pub fn columns(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.columns.extend(columns.into_iter().map(Into::into));
        self
    }

    /// Add a range partition.
    pub fn add_range(mut self, name: impl Into<String>, from: RangeBound, to: RangeBound) -> Self {
        self.range_partitions
            .push(RangePartitionDef::new(name, from, to));
        self
    }

    /// Add a range partition with tablespace.
    pub fn add_range_with_tablespace(
        mut self,
        name: impl Into<String>,
        from: RangeBound,
        to: RangeBound,
        tablespace: impl Into<String>,
    ) -> Self {
        self.range_partitions
            .push(RangePartitionDef::new(name, from, to).tablespace(tablespace));
        self
    }

    /// Add a list partition.
    pub fn add_list(
        mut self,
        name: impl Into<String>,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.list_partitions
            .push(ListPartitionDef::new(name, values));
        self
    }

    /// Add a hash partition.
    pub fn add_hash(mut self, name: impl Into<String>, modulus: u32, remainder: u32) -> Self {
        self.hash_partitions
            .push(HashPartitionDef::new(name, modulus, remainder));
        self
    }

    /// Add multiple hash partitions automatically.
    pub fn add_hash_partitions(mut self, count: u32, name_prefix: impl Into<String>) -> Self {
        let prefix = name_prefix.into();
        for i in 0..count {
            self.hash_partitions
                .push(HashPartitionDef::new(format!("{}_{}", prefix, i), count, i));
        }
        self
    }

    /// Add a comment.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Build the partition specification.
    pub fn build(self) -> QueryResult<Partition> {
        let partition_type = self.partition_type.ok_or_else(|| {
            QueryError::invalid_input(
                "partition_type",
                "Must specify partition type (range_partition, list_partition, or hash_partition)",
            )
        })?;

        if self.columns.is_empty() {
            return Err(QueryError::invalid_input(
                "columns",
                "Must specify at least one partition column",
            ));
        }

        let partitions = match partition_type {
            PartitionType::Range => {
                if self.range_partitions.is_empty() {
                    return Err(QueryError::invalid_input(
                        "partitions",
                        "Must define at least one range partition with add_range()",
                    ));
                }
                PartitionDef::Range(self.range_partitions)
            }
            PartitionType::List => {
                if self.list_partitions.is_empty() {
                    return Err(QueryError::invalid_input(
                        "partitions",
                        "Must define at least one list partition with add_list()",
                    ));
                }
                PartitionDef::List(self.list_partitions)
            }
            PartitionType::Hash => {
                if self.hash_partitions.is_empty() {
                    return Err(QueryError::invalid_input(
                        "partitions",
                        "Must define at least one hash partition with add_hash() or add_hash_partitions()",
                    ));
                }
                PartitionDef::Hash(self.hash_partitions)
            }
        };

        Ok(Partition {
            table: self.table,
            schema: self.schema,
            partition_type,
            columns: self.columns,
            partitions,
            comment: self.comment,
        })
    }
}

/// Time-based partition generation helpers.
pub mod time_partitions {
    use super::*;

    /// Generate monthly partitions for a date range.
    pub fn monthly_partitions(
        table: &str,
        column: &str,
        start_year: i32,
        start_month: u32,
        count: u32,
    ) -> PartitionBuilder {
        let mut builder = Partition::builder(table).range_partition().column(column);

        let mut year = start_year;
        let mut month = start_month;

        for _ in 0..count {
            let from_date = format!("{:04}-{:02}-01", year, month);

            // Calculate next month
            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            let to_date = format!("{:04}-{:02}-01", next_year, next_month);

            let partition_name = format!("{}_{:04}_{:02}", table, year, month);

            builder = builder.add_range(
                partition_name,
                RangeBound::date(from_date),
                RangeBound::date(to_date),
            );

            year = next_year;
            month = next_month;
        }

        builder
    }

    /// Generate quarterly partitions for a date range.
    pub fn quarterly_partitions(
        table: &str,
        column: &str,
        start_year: i32,
        count: u32,
    ) -> PartitionBuilder {
        let mut builder = Partition::builder(table).range_partition().column(column);

        let mut year = start_year;
        let mut quarter = 1;

        for _ in 0..count {
            let from_month = (quarter - 1) * 3 + 1;
            let from_date = format!("{:04}-{:02}-01", year, from_month);

            let (next_year, next_quarter) = if quarter == 4 {
                (year + 1, 1)
            } else {
                (year, quarter + 1)
            };
            let to_month = (next_quarter - 1) * 3 + 1;
            let to_date = format!("{:04}-{:02}-01", next_year, to_month);

            let partition_name = format!("{}_{}q{}", table, year, quarter);

            builder = builder.add_range(
                partition_name,
                RangeBound::date(from_date),
                RangeBound::date(to_date),
            );

            year = next_year;
            quarter = next_quarter;
        }

        builder
    }

    /// Generate yearly partitions.
    pub fn yearly_partitions(
        table: &str,
        column: &str,
        start_year: i32,
        count: u32,
    ) -> PartitionBuilder {
        let mut builder = Partition::builder(table).range_partition().column(column);

        for i in 0..count {
            let year = start_year + i as i32;
            let from_date = format!("{:04}-01-01", year);
            let to_date = format!("{:04}-01-01", year + 1);
            let partition_name = format!("{}_{}", table, year);

            builder = builder.add_range(
                partition_name,
                RangeBound::date(from_date),
                RangeBound::date(to_date),
            );
        }

        builder
    }
}

/// MongoDB sharding support.
pub mod mongodb {
    use serde::{Deserialize, Serialize};

    /// Shard key type for MongoDB.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum ShardKeyType {
        /// Range-based sharding (good for range queries).
        Range,
        /// Hash-based sharding (better distribution).
        Hashed,
    }

    impl ShardKeyType {
        /// Get the MongoDB index specification value.
        pub fn as_index_value(&self) -> serde_json::Value {
            match self {
                Self::Range => serde_json::json!(1),
                Self::Hashed => serde_json::json!("hashed"),
            }
        }
    }

    /// A shard key definition.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ShardKey {
        /// Fields in the shard key.
        pub fields: Vec<(String, ShardKeyType)>,
        /// Whether the collection is unique on the shard key.
        pub unique: bool,
    }

    impl ShardKey {
        /// Create a new shard key builder.
        pub fn builder() -> ShardKeyBuilder {
            ShardKeyBuilder::new()
        }

        /// Get the shardCollection command.
        pub fn shard_collection_command(
            &self,
            database: &str,
            collection: &str,
        ) -> serde_json::Value {
            let mut key = serde_json::Map::new();
            for (field, key_type) in &self.fields {
                key.insert(field.clone(), key_type.as_index_value());
            }

            serde_json::json!({
                "shardCollection": format!("{}.{}", database, collection),
                "key": key,
                "unique": self.unique
            })
        }

        /// Get the index specification for the shard key.
        pub fn index_spec(&self) -> serde_json::Value {
            let mut spec = serde_json::Map::new();
            for (field, key_type) in &self.fields {
                spec.insert(field.clone(), key_type.as_index_value());
            }
            serde_json::Value::Object(spec)
        }
    }

    /// Builder for shard keys.
    #[derive(Debug, Clone, Default)]
    pub struct ShardKeyBuilder {
        fields: Vec<(String, ShardKeyType)>,
        unique: bool,
    }

    impl ShardKeyBuilder {
        /// Create a new shard key builder.
        pub fn new() -> Self {
            Self::default()
        }

        /// Add a range field to the shard key.
        pub fn range_field(mut self, field: impl Into<String>) -> Self {
            self.fields.push((field.into(), ShardKeyType::Range));
            self
        }

        /// Add a hashed field to the shard key.
        pub fn hashed_field(mut self, field: impl Into<String>) -> Self {
            self.fields.push((field.into(), ShardKeyType::Hashed));
            self
        }

        /// Set whether the shard key should enforce uniqueness.
        pub fn unique(mut self, unique: bool) -> Self {
            self.unique = unique;
            self
        }

        /// Build the shard key.
        pub fn build(self) -> ShardKey {
            ShardKey {
                fields: self.fields,
                unique: self.unique,
            }
        }
    }

    /// Zone (tag-based) sharding configuration.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ShardZone {
        /// Zone name.
        pub name: String,
        /// Minimum shard key value.
        pub min: serde_json::Value,
        /// Maximum shard key value.
        pub max: serde_json::Value,
    }

    impl ShardZone {
        /// Create a new shard zone.
        pub fn new(
            name: impl Into<String>,
            min: serde_json::Value,
            max: serde_json::Value,
        ) -> Self {
            Self {
                name: name.into(),
                min,
                max,
            }
        }

        /// Get the updateZoneKeyRange command.
        pub fn update_zone_key_range_command(&self, namespace: &str) -> serde_json::Value {
            serde_json::json!({
                "updateZoneKeyRange": namespace,
                "min": self.min,
                "max": self.max,
                "zone": self.name
            })
        }

        /// Get the addShardToZone command.
        pub fn add_shard_to_zone_command(&self, shard: &str) -> serde_json::Value {
            serde_json::json!({
                "addShardToZone": shard,
                "zone": self.name
            })
        }
    }

    /// Builder for zone sharding configuration.
    #[derive(Debug, Clone, Default)]
    pub struct ZoneShardingBuilder {
        zones: Vec<ShardZone>,
        shard_assignments: Vec<(String, String)>, // (shard_name, zone_name)
    }

    impl ZoneShardingBuilder {
        /// Create a new zone sharding builder.
        pub fn new() -> Self {
            Self::default()
        }

        /// Add a zone.
        pub fn add_zone(
            mut self,
            name: impl Into<String>,
            min: serde_json::Value,
            max: serde_json::Value,
        ) -> Self {
            self.zones.push(ShardZone::new(name, min, max));
            self
        }

        /// Assign a shard to a zone.
        pub fn assign_shard(mut self, shard: impl Into<String>, zone: impl Into<String>) -> Self {
            self.shard_assignments.push((shard.into(), zone.into()));
            self
        }

        /// Get all configuration commands.
        pub fn build_commands(&self, namespace: &str) -> Vec<serde_json::Value> {
            let mut commands = Vec::new();

            // Add shard to zone assignments
            for (shard, zone) in &self.shard_assignments {
                commands.push(serde_json::json!({
                    "addShardToZone": shard,
                    "zone": zone
                }));
            }

            // Add zone key ranges
            for zone in &self.zones {
                commands.push(zone.update_zone_key_range_command(namespace));
            }

            commands
        }
    }

    /// Helper to create a shard key.
    pub fn shard_key() -> ShardKeyBuilder {
        ShardKeyBuilder::new()
    }

    /// Helper to create zone sharding configuration.
    pub fn zone_sharding() -> ZoneShardingBuilder {
        ZoneShardingBuilder::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_partition_builder() {
        let partition = Partition::builder("orders")
            .schema("sales")
            .range_partition()
            .column("created_at")
            .add_range(
                "orders_2024_q1",
                RangeBound::date("2024-01-01"),
                RangeBound::date("2024-04-01"),
            )
            .add_range(
                "orders_2024_q2",
                RangeBound::date("2024-04-01"),
                RangeBound::date("2024-07-01"),
            )
            .build()
            .unwrap();

        assert_eq!(partition.table, "orders");
        assert_eq!(partition.partition_type, PartitionType::Range);
        assert_eq!(partition.columns, vec!["created_at"]);
    }

    #[test]
    fn test_postgres_partition_clause() {
        let partition = Partition::builder("orders")
            .range_partition()
            .column("created_at")
            .add_range(
                "orders_2024",
                RangeBound::date("2024-01-01"),
                RangeBound::date("2025-01-01"),
            )
            .build()
            .unwrap();

        let clause = partition.to_postgres_partition_clause();
        assert_eq!(clause, "PARTITION BY RANGE (created_at)");
    }

    #[test]
    fn test_postgres_create_partition() {
        let partition = Partition::builder("orders")
            .range_partition()
            .column("created_at")
            .add_range(
                "orders_2024",
                RangeBound::date("2024-01-01"),
                RangeBound::date("2025-01-01"),
            )
            .build()
            .unwrap();

        let sqls = partition.to_postgres_create_all_partitions();
        assert_eq!(sqls.len(), 1);
        assert!(sqls[0].contains("CREATE TABLE orders_2024 PARTITION OF orders"));
        assert!(sqls[0].contains("FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')"));
    }

    #[test]
    fn test_list_partition() {
        let partition = Partition::builder("users")
            .list_partition()
            .column("country")
            .add_list("users_us", ["US", "USA"])
            .add_list("users_eu", ["DE", "FR", "GB", "IT"])
            .build()
            .unwrap();

        assert_eq!(partition.partition_type, PartitionType::List);

        let sqls = partition.to_postgres_create_all_partitions();
        assert_eq!(sqls.len(), 2);
        assert!(sqls[0].contains("FOR VALUES IN"));
    }

    #[test]
    fn test_hash_partition() {
        let partition = Partition::builder("events")
            .hash_partition()
            .column("user_id")
            .add_hash_partitions(4, "events")
            .build()
            .unwrap();

        assert_eq!(partition.partition_type, PartitionType::Hash);

        let sqls = partition.to_postgres_create_all_partitions();
        assert_eq!(sqls.len(), 4);
        assert!(sqls[0].contains("MODULUS 4"));
        assert!(sqls[0].contains("REMAINDER 0"));
    }

    #[test]
    fn test_mysql_partition_clause() {
        let partition = Partition::builder("orders")
            .range_partition()
            .column("created_at")
            .add_range(
                "p2024",
                RangeBound::MinValue,
                RangeBound::date("2025-01-01"),
            )
            .add_range(
                "p_future",
                RangeBound::date("2025-01-01"),
                RangeBound::MaxValue,
            )
            .build()
            .unwrap();

        let clause = partition.to_mysql_partition_clause();
        assert!(clause.contains("PARTITION BY RANGE"));
        assert!(clause.contains("PARTITION p2024"));
        assert!(clause.contains("PARTITION p_future"));
    }

    #[test]
    fn test_detach_partition() {
        let partition = Partition::builder("orders")
            .range_partition()
            .column("created_at")
            .add_range(
                "orders_2024",
                RangeBound::date("2024-01-01"),
                RangeBound::date("2025-01-01"),
            )
            .build()
            .unwrap();

        let sql = partition
            .detach_partition_sql("orders_2024", DatabaseType::PostgreSQL)
            .unwrap();
        assert_eq!(sql, "ALTER TABLE orders DETACH PARTITION orders_2024;");
    }

    #[test]
    fn test_drop_partition() {
        let partition = Partition::builder("orders")
            .range_partition()
            .column("created_at")
            .add_range(
                "orders_2024",
                RangeBound::date("2024-01-01"),
                RangeBound::date("2025-01-01"),
            )
            .build()
            .unwrap();

        let pg_sql = partition
            .drop_partition_sql("orders_2024", DatabaseType::PostgreSQL)
            .unwrap();
        assert_eq!(pg_sql, "DROP TABLE IF EXISTS orders_2024;");

        let mysql_sql = partition
            .drop_partition_sql("orders_2024", DatabaseType::MySQL)
            .unwrap();
        assert!(mysql_sql.contains("DROP PARTITION"));
    }

    #[test]
    fn test_missing_partition_type() {
        let result = Partition::builder("orders")
            .column("created_at")
            .add_range("p1", RangeBound::MinValue, RangeBound::MaxValue)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_missing_columns() {
        let result = Partition::builder("orders")
            .range_partition()
            .add_range("p1", RangeBound::MinValue, RangeBound::MaxValue)
            .build();

        assert!(result.is_err());
    }

    mod time_partition_tests {
        use super::super::time_partitions;

        #[test]
        fn test_monthly_partitions() {
            let builder = time_partitions::monthly_partitions("orders", "created_at", 2024, 1, 3);
            let partition = builder.build().unwrap();

            if let super::PartitionDef::Range(ranges) = &partition.partitions {
                assert_eq!(ranges.len(), 3);
                assert_eq!(ranges[0].name, "orders_2024_01");
                assert_eq!(ranges[1].name, "orders_2024_02");
                assert_eq!(ranges[2].name, "orders_2024_03");
            } else {
                panic!("Expected range partitions");
            }
        }

        #[test]
        fn test_quarterly_partitions() {
            let builder = time_partitions::quarterly_partitions("sales", "order_date", 2024, 4);
            let partition = builder.build().unwrap();

            if let super::PartitionDef::Range(ranges) = &partition.partitions {
                assert_eq!(ranges.len(), 4);
                assert_eq!(ranges[0].name, "sales_2024q1");
                assert_eq!(ranges[3].name, "sales_2024q4");
            } else {
                panic!("Expected range partitions");
            }
        }

        #[test]
        fn test_yearly_partitions() {
            let builder = time_partitions::yearly_partitions("logs", "timestamp", 2020, 5);
            let partition = builder.build().unwrap();

            if let super::PartitionDef::Range(ranges) = &partition.partitions {
                assert_eq!(ranges.len(), 5);
                assert_eq!(ranges[0].name, "logs_2020");
                assert_eq!(ranges[4].name, "logs_2024");
            } else {
                panic!("Expected range partitions");
            }
        }
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_shard_key_builder() {
            let key = shard_key()
                .hashed_field("tenant_id")
                .range_field("created_at")
                .unique(false)
                .build();

            assert_eq!(key.fields.len(), 2);
            assert_eq!(
                key.fields[0],
                ("tenant_id".to_string(), ShardKeyType::Hashed)
            );
            assert_eq!(
                key.fields[1],
                ("created_at".to_string(), ShardKeyType::Range)
            );
        }

        #[test]
        fn test_shard_collection_command() {
            let key = shard_key().hashed_field("user_id").build();

            let cmd = key.shard_collection_command("mydb", "users");
            assert_eq!(cmd["shardCollection"], "mydb.users");
            assert_eq!(cmd["key"]["user_id"], "hashed");
        }

        #[test]
        fn test_zone_sharding() {
            let builder = zone_sharding()
                .add_zone(
                    "US",
                    serde_json::json!({"region": "US"}),
                    serde_json::json!({"region": "US~"}),
                )
                .add_zone(
                    "EU",
                    serde_json::json!({"region": "EU"}),
                    serde_json::json!({"region": "EU~"}),
                )
                .assign_shard("shard0", "US")
                .assign_shard("shard1", "EU");

            let commands = builder.build_commands("mydb.users");
            assert_eq!(commands.len(), 4); // 2 shard assignments + 2 zone ranges
        }

        #[test]
        fn test_shard_key_index_spec() {
            let key = shard_key()
                .range_field("tenant_id")
                .range_field("created_at")
                .build();

            let spec = key.index_spec();
            assert_eq!(spec["tenant_id"], 1);
            assert_eq!(spec["created_at"], 1);
        }
    }
}
