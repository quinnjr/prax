//! Advanced query features.
//!
//! This module provides advanced SQL query capabilities including:
//! - LATERAL joins (correlated subqueries)
//! - DISTINCT ON
//! - RETURNING/OUTPUT clauses
//! - Row locking (FOR UPDATE/SHARE)
//! - TABLESAMPLE
//! - Bulk operations
//!
//! # Database Support
//!
//! | Feature           | PostgreSQL     | MySQL    | SQLite | MSSQL           | MongoDB      |
//! |-------------------|----------------|----------|--------|-----------------|--------------|
//! | LATERAL joins     | ✅             | ✅       | ❌     | ✅ CROSS APPLY  | ✅ $lookup   |
//! | DISTINCT ON       | ✅             | ❌       | ❌     | ❌              | ✅ $first    |
//! | RETURNING/OUTPUT  | ✅             | ❌       | ✅     | ✅ OUTPUT       | ✅           |
//! | FOR UPDATE/SHARE  | ✅             | ✅       | ❌     | ✅ WITH UPDLOCK | ❌           |
//! | TABLESAMPLE       | ✅             | ❌       | ❌     | ✅              | ✅ $sample   |
//! | Bulk operations   | ✅             | ✅       | ✅     | ✅              | ✅ bulkWrite |

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

// ============================================================================
// LATERAL Joins
// ============================================================================

/// A LATERAL join specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateralJoin {
    /// The subquery or function call.
    pub subquery: String,
    /// Alias for the lateral result.
    pub alias: String,
    /// Join type.
    pub join_type: LateralJoinType,
    /// Optional ON condition (for LEFT LATERAL).
    pub condition: Option<String>,
}

/// LATERAL join type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LateralJoinType {
    /// CROSS JOIN LATERAL / CROSS APPLY.
    Cross,
    /// LEFT JOIN LATERAL / OUTER APPLY.
    Left,
}

impl LateralJoin {
    /// Create a new LATERAL join.
    pub fn new(subquery: impl Into<String>, alias: impl Into<String>) -> LateralJoinBuilder {
        LateralJoinBuilder::new(subquery, alias)
    }

    /// Generate PostgreSQL LATERAL join.
    pub fn to_postgres_sql(&self) -> String {
        match self.join_type {
            LateralJoinType::Cross => {
                format!("CROSS JOIN LATERAL ({}) AS {}", self.subquery, self.alias)
            }
            LateralJoinType::Left => {
                let cond = self.condition.as_deref().unwrap_or("TRUE");
                format!(
                    "LEFT JOIN LATERAL ({}) AS {} ON {}",
                    self.subquery, self.alias, cond
                )
            }
        }
    }

    /// Generate MySQL LATERAL join.
    pub fn to_mysql_sql(&self) -> String {
        match self.join_type {
            LateralJoinType::Cross => {
                format!("CROSS JOIN LATERAL ({}) AS {}", self.subquery, self.alias)
            }
            LateralJoinType::Left => {
                let cond = self.condition.as_deref().unwrap_or("TRUE");
                format!(
                    "LEFT JOIN LATERAL ({}) AS {} ON {}",
                    self.subquery, self.alias, cond
                )
            }
        }
    }

    /// Generate MSSQL APPLY join.
    pub fn to_mssql_sql(&self) -> String {
        match self.join_type {
            LateralJoinType::Cross => {
                format!("CROSS APPLY ({}) AS {}", self.subquery, self.alias)
            }
            LateralJoinType::Left => {
                format!("OUTER APPLY ({}) AS {}", self.subquery, self.alias)
            }
        }
    }

    /// Generate SQL for database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_sql()),
            DatabaseType::MySQL => Ok(self.to_mysql_sql()),
            DatabaseType::MSSQL => Ok(self.to_mssql_sql()),
            DatabaseType::SQLite => Err(QueryError::unsupported(
                "LATERAL joins are not supported in SQLite",
            )),
        }
    }
}

/// Builder for LATERAL joins.
#[derive(Debug, Clone)]
pub struct LateralJoinBuilder {
    subquery: String,
    alias: String,
    join_type: LateralJoinType,
    condition: Option<String>,
}

impl LateralJoinBuilder {
    /// Create a new builder.
    pub fn new(subquery: impl Into<String>, alias: impl Into<String>) -> Self {
        Self {
            subquery: subquery.into(),
            alias: alias.into(),
            join_type: LateralJoinType::Cross,
            condition: None,
        }
    }

    /// Make this a LEFT LATERAL join.
    pub fn left(mut self) -> Self {
        self.join_type = LateralJoinType::Left;
        self
    }

    /// Make this a CROSS LATERAL join (default).
    pub fn cross(mut self) -> Self {
        self.join_type = LateralJoinType::Cross;
        self
    }

    /// Set the ON condition.
    pub fn on(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    /// Build the LATERAL join.
    pub fn build(self) -> LateralJoin {
        LateralJoin {
            subquery: self.subquery,
            alias: self.alias,
            join_type: self.join_type,
            condition: self.condition,
        }
    }
}

// ============================================================================
// DISTINCT ON
// ============================================================================

/// DISTINCT ON clause (PostgreSQL specific).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistinctOn {
    /// Columns to distinct on.
    pub columns: Vec<String>,
}

impl DistinctOn {
    /// Create a new DISTINCT ON clause.
    pub fn new<I, S>(columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
        }
    }

    /// Generate PostgreSQL DISTINCT ON clause.
    pub fn to_postgres_sql(&self) -> String {
        format!("DISTINCT ON ({})", self.columns.join(", "))
    }

    /// Generate MySQL workaround using GROUP BY.
    /// Note: This is not exactly equivalent to DISTINCT ON.
    pub fn to_mysql_workaround(&self) -> String {
        format!(
            "-- MySQL workaround: Use GROUP BY {} with appropriate aggregates",
            self.columns.join(", ")
        )
    }
}

/// MongoDB $first aggregation helper for DISTINCT ON behavior.
pub mod mongodb_distinct {
    use serde_json::Value as JsonValue;

    /// Generate $group stage that mimics DISTINCT ON.
    pub fn distinct_on_stage(group_fields: &[&str], first_fields: &[&str]) -> JsonValue {
        let mut group_id = serde_json::Map::new();
        for field in group_fields {
            group_id.insert(field.to_string(), serde_json::json!(format!("${}", field)));
        }

        let mut group_spec = serde_json::Map::new();
        group_spec.insert("_id".to_string(), serde_json::json!(group_id));

        for field in first_fields {
            group_spec.insert(
                field.to_string(),
                serde_json::json!({ "$first": format!("${}", field) }),
            );
        }

        serde_json::json!({ "$group": group_spec })
    }
}

// ============================================================================
// RETURNING / OUTPUT Clause
// ============================================================================

/// RETURNING clause specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Returning {
    /// Columns to return.
    pub columns: Vec<ReturningColumn>,
    /// Operation type (for MSSQL OUTPUT).
    pub operation: ReturnOperation,
}

/// A column in the RETURNING clause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReturningColumn {
    /// All columns (*).
    All,
    /// Specific column name.
    Column(String),
    /// Expression with alias.
    Expression { expr: String, alias: String },
    /// MSSQL INSERTED.column.
    Inserted(String),
    /// MSSQL DELETED.column.
    Deleted(String),
}

/// Operation type for RETURNING/OUTPUT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReturnOperation {
    Insert,
    Update,
    Delete,
}

impl Returning {
    /// Create RETURNING all columns.
    pub fn all(operation: ReturnOperation) -> Self {
        Self {
            columns: vec![ReturningColumn::All],
            operation,
        }
    }

    /// Create RETURNING specific columns.
    pub fn columns<I, S>(operation: ReturnOperation, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            columns: columns
                .into_iter()
                .map(|c| ReturningColumn::Column(c.into()))
                .collect(),
            operation,
        }
    }

    /// Generate PostgreSQL RETURNING clause.
    pub fn to_postgres_sql(&self) -> String {
        let cols = self.format_columns(DatabaseType::PostgreSQL);
        format!("RETURNING {}", cols)
    }

    /// Generate SQLite RETURNING clause.
    pub fn to_sqlite_sql(&self) -> String {
        let cols = self.format_columns(DatabaseType::SQLite);
        format!("RETURNING {}", cols)
    }

    /// Generate MSSQL OUTPUT clause.
    pub fn to_mssql_sql(&self) -> String {
        let cols = self.format_columns(DatabaseType::MSSQL);
        format!("OUTPUT {}", cols)
    }

    /// Format columns for database.
    fn format_columns(&self, db_type: DatabaseType) -> String {
        self.columns
            .iter()
            .map(|col| match col {
                ReturningColumn::All => {
                    if db_type == DatabaseType::MSSQL {
                        match self.operation {
                            ReturnOperation::Insert => "INSERTED.*".to_string(),
                            ReturnOperation::Delete => "DELETED.*".to_string(),
                            ReturnOperation::Update => "INSERTED.*".to_string(),
                        }
                    } else {
                        "*".to_string()
                    }
                }
                ReturningColumn::Column(name) => {
                    if db_type == DatabaseType::MSSQL {
                        match self.operation {
                            ReturnOperation::Insert => format!("INSERTED.{}", name),
                            ReturnOperation::Delete => format!("DELETED.{}", name),
                            ReturnOperation::Update => format!("INSERTED.{}", name),
                        }
                    } else {
                        name.clone()
                    }
                }
                ReturningColumn::Expression { expr, alias } => format!("{} AS {}", expr, alias),
                ReturningColumn::Inserted(name) => format!("INSERTED.{}", name),
                ReturningColumn::Deleted(name) => format!("DELETED.{}", name),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Generate SQL for database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_sql()),
            DatabaseType::SQLite => Ok(self.to_sqlite_sql()),
            DatabaseType::MSSQL => Ok(self.to_mssql_sql()),
            DatabaseType::MySQL => Err(QueryError::unsupported(
                "RETURNING clause is not supported in MySQL. Consider using LAST_INSERT_ID() or separate SELECT.",
            )),
        }
    }
}

// ============================================================================
// Row Locking (FOR UPDATE / FOR SHARE)
// ============================================================================

/// Row locking mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowLock {
    /// Lock strength.
    pub strength: LockStrength,
    /// Tables to lock (optional).
    pub of_tables: Vec<String>,
    /// Wait behavior.
    pub wait: LockWait,
}

/// Lock strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockStrength {
    /// FOR UPDATE - exclusive lock.
    Update,
    /// FOR NO KEY UPDATE - exclusive but allows key reads.
    NoKeyUpdate,
    /// FOR SHARE - shared lock.
    Share,
    /// FOR KEY SHARE - shared key lock.
    KeyShare,
}

/// Lock wait behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockWait {
    /// Wait for lock (default).
    Wait,
    /// Don't wait, error if locked.
    NoWait,
    /// Skip locked rows.
    SkipLocked,
}

impl RowLock {
    /// Create FOR UPDATE lock.
    pub fn for_update() -> RowLockBuilder {
        RowLockBuilder::new(LockStrength::Update)
    }

    /// Create FOR SHARE lock.
    pub fn for_share() -> RowLockBuilder {
        RowLockBuilder::new(LockStrength::Share)
    }

    /// Create FOR NO KEY UPDATE lock.
    pub fn for_no_key_update() -> RowLockBuilder {
        RowLockBuilder::new(LockStrength::NoKeyUpdate)
    }

    /// Create FOR KEY SHARE lock.
    pub fn for_key_share() -> RowLockBuilder {
        RowLockBuilder::new(LockStrength::KeyShare)
    }

    /// Generate PostgreSQL FOR clause.
    pub fn to_postgres_sql(&self) -> String {
        let strength = match self.strength {
            LockStrength::Update => "FOR UPDATE",
            LockStrength::NoKeyUpdate => "FOR NO KEY UPDATE",
            LockStrength::Share => "FOR SHARE",
            LockStrength::KeyShare => "FOR KEY SHARE",
        };

        let mut sql = strength.to_string();

        if !self.of_tables.is_empty() {
            sql.push_str(&format!(" OF {}", self.of_tables.join(", ")));
        }

        match self.wait {
            LockWait::Wait => {}
            LockWait::NoWait => sql.push_str(" NOWAIT"),
            LockWait::SkipLocked => sql.push_str(" SKIP LOCKED"),
        }

        sql
    }

    /// Generate MySQL FOR clause.
    pub fn to_mysql_sql(&self) -> String {
        let strength = match self.strength {
            LockStrength::Update | LockStrength::NoKeyUpdate => "FOR UPDATE",
            LockStrength::Share | LockStrength::KeyShare => "FOR SHARE",
        };

        let mut sql = strength.to_string();

        if !self.of_tables.is_empty() {
            sql.push_str(&format!(" OF {}", self.of_tables.join(", ")));
        }

        match self.wait {
            LockWait::Wait => {}
            LockWait::NoWait => sql.push_str(" NOWAIT"),
            LockWait::SkipLocked => sql.push_str(" SKIP LOCKED"),
        }

        sql
    }

    /// Generate MSSQL table hint.
    pub fn to_mssql_hint(&self) -> String {
        let hint = match self.strength {
            LockStrength::Update | LockStrength::NoKeyUpdate => "UPDLOCK, ROWLOCK",
            LockStrength::Share | LockStrength::KeyShare => "HOLDLOCK, ROWLOCK",
        };

        let wait_hint = match self.wait {
            LockWait::Wait => "",
            LockWait::NoWait => ", NOWAIT",
            LockWait::SkipLocked => ", READPAST",
        };

        format!("WITH ({}{})", hint, wait_hint)
    }

    /// Generate SQL for database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_sql()),
            DatabaseType::MySQL => Ok(self.to_mysql_sql()),
            DatabaseType::MSSQL => Ok(self.to_mssql_hint()),
            DatabaseType::SQLite => Err(QueryError::unsupported(
                "Row locking is not supported in SQLite",
            )),
        }
    }
}

/// Builder for row locks.
#[derive(Debug, Clone)]
pub struct RowLockBuilder {
    strength: LockStrength,
    of_tables: Vec<String>,
    wait: LockWait,
}

impl RowLockBuilder {
    /// Create a new builder.
    pub fn new(strength: LockStrength) -> Self {
        Self {
            strength,
            of_tables: Vec::new(),
            wait: LockWait::Wait,
        }
    }

    /// Lock specific tables.
    pub fn of<I, S>(mut self, tables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.of_tables = tables.into_iter().map(Into::into).collect();
        self
    }

    /// NOWAIT - error immediately if locked.
    pub fn nowait(mut self) -> Self {
        self.wait = LockWait::NoWait;
        self
    }

    /// SKIP LOCKED - skip locked rows.
    pub fn skip_locked(mut self) -> Self {
        self.wait = LockWait::SkipLocked;
        self
    }

    /// Build the row lock.
    pub fn build(self) -> RowLock {
        RowLock {
            strength: self.strength,
            of_tables: self.of_tables,
            wait: self.wait,
        }
    }
}

// ============================================================================
// TABLESAMPLE
// ============================================================================

/// Table sampling configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableSample {
    /// Sampling method.
    pub method: SampleMethod,
    /// Sample size (percentage or rows).
    pub size: SampleSize,
    /// Optional seed for reproducibility.
    pub seed: Option<i64>,
}

/// Sampling method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleMethod {
    /// BERNOULLI - row-level random sampling.
    Bernoulli,
    /// SYSTEM - page-level random sampling (faster, less accurate).
    System,
}

/// Sample size specification.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SampleSize {
    /// Percentage of rows (0-100).
    Percent(f64),
    /// Approximate number of rows.
    Rows(usize),
}

impl TableSample {
    /// Create a percentage sample using BERNOULLI.
    pub fn percent(percent: f64) -> TableSampleBuilder {
        TableSampleBuilder::new(SampleMethod::Bernoulli, SampleSize::Percent(percent))
    }

    /// Create a row count sample.
    pub fn rows(count: usize) -> TableSampleBuilder {
        TableSampleBuilder::new(SampleMethod::System, SampleSize::Rows(count))
    }

    /// Generate PostgreSQL TABLESAMPLE clause.
    pub fn to_postgres_sql(&self) -> String {
        let method = match self.method {
            SampleMethod::Bernoulli => "BERNOULLI",
            SampleMethod::System => "SYSTEM",
        };

        let size = match self.size {
            SampleSize::Percent(p) => format!("{}", p),
            SampleSize::Rows(_) => {
                // PostgreSQL doesn't support row counts directly
                "10".to_string() // Default to 10%
            }
        };

        let mut sql = format!("TABLESAMPLE {} ({})", method, size);

        if let Some(seed) = self.seed {
            sql.push_str(&format!(" REPEATABLE ({})", seed));
        }

        sql
    }

    /// Generate MSSQL TABLESAMPLE clause.
    pub fn to_mssql_sql(&self) -> String {
        let size_clause = match self.size {
            SampleSize::Percent(p) => format!("{} PERCENT", p),
            SampleSize::Rows(n) => format!("{} ROWS", n),
        };

        let mut sql = format!("TABLESAMPLE ({})", size_clause);

        if let Some(seed) = self.seed {
            sql.push_str(&format!(" REPEATABLE ({})", seed));
        }

        sql
    }

    /// Generate SQL for database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_sql()),
            DatabaseType::MSSQL => Ok(self.to_mssql_sql()),
            DatabaseType::MySQL | DatabaseType::SQLite => Err(QueryError::unsupported(
                "TABLESAMPLE is not supported in this database. Use ORDER BY RANDOM() LIMIT instead.",
            )),
        }
    }
}

/// Builder for table sampling.
#[derive(Debug, Clone)]
pub struct TableSampleBuilder {
    method: SampleMethod,
    size: SampleSize,
    seed: Option<i64>,
}

impl TableSampleBuilder {
    /// Create a new builder.
    pub fn new(method: SampleMethod, size: SampleSize) -> Self {
        Self {
            method,
            size,
            seed: None,
        }
    }

    /// Use BERNOULLI sampling.
    pub fn bernoulli(mut self) -> Self {
        self.method = SampleMethod::Bernoulli;
        self
    }

    /// Use SYSTEM sampling.
    pub fn system(mut self) -> Self {
        self.method = SampleMethod::System;
        self
    }

    /// Set seed for reproducibility.
    pub fn seed(mut self, seed: i64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Build the sample configuration.
    pub fn build(self) -> TableSample {
        TableSample {
            method: self.method,
            size: self.size,
            seed: self.seed,
        }
    }
}

/// Random sampling alternatives for unsupported databases.
pub mod random_sample {
    use super::*;

    /// Generate ORDER BY RANDOM() LIMIT for databases without TABLESAMPLE.
    pub fn order_by_random_sql(limit: usize, db_type: DatabaseType) -> String {
        let random_func = match db_type {
            DatabaseType::PostgreSQL => "RANDOM()",
            DatabaseType::MySQL => "RAND()",
            DatabaseType::SQLite => "RANDOM()",
            DatabaseType::MSSQL => "NEWID()",
        };

        format!("ORDER BY {} LIMIT {}", random_func, limit)
    }

    /// Generate WHERE RANDOM() < threshold for row sampling.
    pub fn where_random_sql(threshold: f64, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL | DatabaseType::SQLite => {
                format!("WHERE RANDOM() < {}", threshold)
            }
            DatabaseType::MySQL => format!("WHERE RAND() < {}", threshold),
            DatabaseType::MSSQL => {
                format!(
                    "WHERE ABS(CHECKSUM(NEWID())) % 100 < {}",
                    (threshold * 100.0) as i32
                )
            }
        }
    }
}

// ============================================================================
// Bulk Operations
// ============================================================================

/// Bulk operation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkOperation<T> {
    /// Items to process.
    pub items: Vec<T>,
    /// Batch size.
    pub batch_size: usize,
    /// Whether to continue on error.
    pub ordered: bool,
}

impl<T> BulkOperation<T> {
    /// Create a new bulk operation.
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            batch_size: 1000,
            ordered: true,
        }
    }

    /// Set batch size.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Allow unordered execution (continue on errors).
    pub fn unordered(mut self) -> Self {
        self.ordered = false;
        self
    }

    /// Get batches.
    pub fn batches(&self) -> impl Iterator<Item = &[T]> {
        self.items.chunks(self.batch_size)
    }

    /// Get number of batches.
    pub fn batch_count(&self) -> usize {
        self.items.len().div_ceil(self.batch_size)
    }
}

/// MongoDB bulkWrite operations.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    /// A single bulk write operation.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum BulkWriteOp {
        /// Insert one document.
        InsertOne { document: JsonValue },
        /// Update one document.
        UpdateOne {
            filter: JsonValue,
            update: JsonValue,
            upsert: bool,
        },
        /// Update many documents.
        UpdateMany {
            filter: JsonValue,
            update: JsonValue,
            upsert: bool,
        },
        /// Replace one document.
        ReplaceOne {
            filter: JsonValue,
            replacement: JsonValue,
            upsert: bool,
        },
        /// Delete one document.
        DeleteOne { filter: JsonValue },
        /// Delete many documents.
        DeleteMany { filter: JsonValue },
    }

    impl BulkWriteOp {
        /// Create an insert operation.
        pub fn insert_one(document: JsonValue) -> Self {
            Self::InsertOne { document }
        }

        /// Create an update one operation.
        pub fn update_one(filter: JsonValue, update: JsonValue) -> Self {
            Self::UpdateOne {
                filter,
                update,
                upsert: false,
            }
        }

        /// Create an upsert operation.
        pub fn upsert_one(filter: JsonValue, update: JsonValue) -> Self {
            Self::UpdateOne {
                filter,
                update,
                upsert: true,
            }
        }

        /// Create a delete one operation.
        pub fn delete_one(filter: JsonValue) -> Self {
            Self::DeleteOne { filter }
        }

        /// Convert to MongoDB format.
        pub fn to_command(&self) -> JsonValue {
            match self {
                Self::InsertOne { document } => {
                    serde_json::json!({ "insertOne": { "document": document } })
                }
                Self::UpdateOne {
                    filter,
                    update,
                    upsert,
                } => {
                    serde_json::json!({
                        "updateOne": {
                            "filter": filter,
                            "update": update,
                            "upsert": upsert
                        }
                    })
                }
                Self::UpdateMany {
                    filter,
                    update,
                    upsert,
                } => {
                    serde_json::json!({
                        "updateMany": {
                            "filter": filter,
                            "update": update,
                            "upsert": upsert
                        }
                    })
                }
                Self::ReplaceOne {
                    filter,
                    replacement,
                    upsert,
                } => {
                    serde_json::json!({
                        "replaceOne": {
                            "filter": filter,
                            "replacement": replacement,
                            "upsert": upsert
                        }
                    })
                }
                Self::DeleteOne { filter } => {
                    serde_json::json!({ "deleteOne": { "filter": filter } })
                }
                Self::DeleteMany { filter } => {
                    serde_json::json!({ "deleteMany": { "filter": filter } })
                }
            }
        }
    }

    /// Bulk write builder.
    #[derive(Debug, Clone, Default)]
    pub struct BulkWriteBuilder {
        operations: Vec<BulkWriteOp>,
        ordered: bool,
        bypass_validation: bool,
    }

    impl BulkWriteBuilder {
        /// Create a new builder.
        pub fn new() -> Self {
            Self {
                operations: Vec::new(),
                ordered: true,
                bypass_validation: false,
            }
        }

        /// Add an operation.
        pub fn add(mut self, op: BulkWriteOp) -> Self {
            self.operations.push(op);
            self
        }

        /// Add multiple operations.
        pub fn add_many<I>(mut self, ops: I) -> Self
        where
            I: IntoIterator<Item = BulkWriteOp>,
        {
            self.operations.extend(ops);
            self
        }

        /// Insert one document.
        pub fn insert_one(self, document: JsonValue) -> Self {
            self.add(BulkWriteOp::insert_one(document))
        }

        /// Update one document.
        pub fn update_one(self, filter: JsonValue, update: JsonValue) -> Self {
            self.add(BulkWriteOp::update_one(filter, update))
        }

        /// Upsert one document.
        pub fn upsert_one(self, filter: JsonValue, update: JsonValue) -> Self {
            self.add(BulkWriteOp::upsert_one(filter, update))
        }

        /// Delete one document.
        pub fn delete_one(self, filter: JsonValue) -> Self {
            self.add(BulkWriteOp::delete_one(filter))
        }

        /// Set unordered execution.
        pub fn unordered(mut self) -> Self {
            self.ordered = false;
            self
        }

        /// Bypass document validation.
        pub fn bypass_validation(mut self) -> Self {
            self.bypass_validation = true;
            self
        }

        /// Build the bulkWrite command.
        pub fn build(&self, collection: &str) -> JsonValue {
            let ops: Vec<JsonValue> = self.operations.iter().map(|op| op.to_command()).collect();

            serde_json::json!({
                "bulkWrite": collection,
                "operations": ops,
                "ordered": self.ordered,
                "bypassDocumentValidation": self.bypass_validation
            })
        }
    }

    /// MongoDB $sample aggregation stage.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Sample {
        /// Number of documents to sample.
        pub size: usize,
    }

    impl Sample {
        /// Create a new sample stage.
        pub fn new(size: usize) -> Self {
            Self { size }
        }

        /// Convert to aggregation stage.
        pub fn to_stage(&self) -> JsonValue {
            serde_json::json!({ "$sample": { "size": self.size } })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lateral_join_postgres() {
        let lateral = LateralJoin::new(
            "SELECT * FROM orders WHERE orders.user_id = users.id LIMIT 3",
            "recent_orders",
        )
        .build();

        let sql = lateral.to_postgres_sql();
        assert!(sql.contains("CROSS JOIN LATERAL"));
        assert!(sql.contains("AS recent_orders"));
    }

    #[test]
    fn test_lateral_join_mssql() {
        let lateral = LateralJoin::new(
            "SELECT TOP 3 * FROM orders WHERE orders.user_id = users.id",
            "recent_orders",
        )
        .left()
        .build();

        let sql = lateral.to_mssql_sql();
        assert!(sql.contains("OUTER APPLY"));
    }

    #[test]
    fn test_distinct_on() {
        let distinct = DistinctOn::new(["department", "date"]);
        let sql = distinct.to_postgres_sql();

        assert!(sql.contains("DISTINCT ON (department, date)"));
    }

    #[test]
    fn test_returning_postgres() {
        let ret = Returning::all(ReturnOperation::Insert);
        let sql = ret.to_postgres_sql();

        assert_eq!(sql, "RETURNING *");
    }

    #[test]
    fn test_returning_mssql() {
        let ret = Returning::columns(ReturnOperation::Insert, ["id", "name"]);
        let sql = ret.to_mssql_sql();

        assert!(sql.contains("OUTPUT INSERTED.id, INSERTED.name"));
    }

    #[test]
    fn test_for_update() {
        let lock = RowLock::for_update().nowait().build();
        let sql = lock.to_postgres_sql();

        assert!(sql.contains("FOR UPDATE"));
        assert!(sql.contains("NOWAIT"));
    }

    #[test]
    fn test_for_share_skip_locked() {
        let lock = RowLock::for_share().skip_locked().build();
        let sql = lock.to_postgres_sql();

        assert!(sql.contains("FOR SHARE"));
        assert!(sql.contains("SKIP LOCKED"));
    }

    #[test]
    fn test_row_lock_mssql() {
        let lock = RowLock::for_update().nowait().build();
        let sql = lock.to_mssql_hint();

        assert!(sql.contains("UPDLOCK"));
        assert!(sql.contains("NOWAIT"));
    }

    #[test]
    fn test_tablesample_postgres() {
        let sample = TableSample::percent(10.0).seed(42).build();
        let sql = sample.to_postgres_sql();

        assert!(sql.contains("TABLESAMPLE BERNOULLI (10)"));
        assert!(sql.contains("REPEATABLE (42)"));
    }

    #[test]
    fn test_tablesample_mssql() {
        let sample = TableSample::rows(1000).build();
        let sql = sample.to_mssql_sql();

        assert!(sql.contains("TABLESAMPLE (1000 ROWS)"));
    }

    #[test]
    fn test_bulk_operation_batches() {
        let bulk: BulkOperation<i32> = BulkOperation::new(vec![1, 2, 3, 4, 5]).batch_size(2);

        assert_eq!(bulk.batch_count(), 3);
        let batches: Vec<_> = bulk.batches().collect();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0], &[1, 2]);
        assert_eq!(batches[1], &[3, 4]);
        assert_eq!(batches[2], &[5]);
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_bulk_write_builder() {
            let bulk = BulkWriteBuilder::new()
                .insert_one(serde_json::json!({ "name": "Alice" }))
                .update_one(
                    serde_json::json!({ "_id": 1 }),
                    serde_json::json!({ "$set": { "status": "active" } }),
                )
                .delete_one(serde_json::json!({ "_id": 2 }))
                .unordered()
                .build("users");

            assert_eq!(bulk["bulkWrite"], "users");
            assert_eq!(bulk["ordered"], false);
            assert!(bulk["operations"].is_array());
            assert_eq!(bulk["operations"].as_array().unwrap().len(), 3);
        }

        #[test]
        fn test_sample_stage() {
            let sample = Sample::new(100);
            let stage = sample.to_stage();

            assert_eq!(stage["$sample"]["size"], 100);
        }

        #[test]
        fn test_bulk_write_upsert() {
            let op = BulkWriteOp::upsert_one(
                serde_json::json!({ "email": "test@example.com" }),
                serde_json::json!({ "$set": { "name": "Test" } }),
            );

            let cmd = op.to_command();
            assert!(cmd["updateOne"]["upsert"].as_bool().unwrap());
        }
    }

    mod distinct_on_tests {
        use super::super::mongodb_distinct::*;

        #[test]
        fn test_distinct_on_stage() {
            let stage = distinct_on_stage(&["department"], &["name", "salary"]);

            assert!(stage["$group"]["_id"]["department"].is_string());
            assert!(stage["$group"]["name"]["$first"].is_string());
            assert!(stage["$group"]["salary"]["$first"].is_string());
        }
    }
}
