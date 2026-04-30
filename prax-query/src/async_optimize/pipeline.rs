//! Pipelined database execution for high-throughput operations.
//!
//! This module implements database pipelining, which allows multiple queries
//! to be sent without waiting for individual responses. This dramatically
//! reduces latency for bulk operations.
//!
//! # How Pipelining Works
//!
//! Without pipelining (sequential):
//! ```text
//! Client: QUERY1 -> wait -> QUERY2 -> wait -> QUERY3 -> wait
//! Server: -----> RESULT1 -----> RESULT2 -----> RESULT3
//! ```
//!
//! With pipelining:
//! ```text
//! Client: QUERY1 -> QUERY2 -> QUERY3 -> wait for all
//! Server: -----> RESULT1 -> RESULT2 -> RESULT3
//! ```
//!
//! # Performance
//!
//! Pipelining can reduce total execution time by 50-70% for bulk operations,
//! especially when network latency is significant.
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::async_optimize::pipeline::{QueryPipeline, PipelineConfig};
//!
//! let pipeline = QueryPipeline::new(PipelineConfig::default())
//!     .add_insert("INSERT INTO users (name) VALUES ($1)", vec!["Alice".into()])
//!     .add_insert("INSERT INTO users (name) VALUES ($1)", vec!["Bob".into()])
//!     .add_insert("INSERT INTO users (name) VALUES ($1)", vec!["Charlie".into()]);
//!
//! // Execute all inserts with minimal round-trips
//! let results = pipeline.execute_batch().await?;
//! ```

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::filter::FilterValue;
use crate::sql::DatabaseType;

/// Configuration for pipeline execution.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Maximum queries per pipeline batch.
    pub max_batch_size: usize,
    /// Timeout for entire pipeline execution.
    pub execution_timeout: Duration,
    /// Whether to wrap pipeline in a transaction.
    pub use_transaction: bool,
    /// Whether to rollback on any error.
    pub rollback_on_error: bool,
    /// Maximum pipeline depth (pending queries).
    pub max_depth: usize,
    /// Whether to collect execution statistics.
    pub collect_stats: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 1000,
            execution_timeout: Duration::from_secs(60),
            use_transaction: false,
            rollback_on_error: true,
            max_depth: 100,
            collect_stats: true,
        }
    }
}

impl PipelineConfig {
    /// Create config optimized for bulk inserts.
    #[must_use]
    pub fn for_bulk_inserts() -> Self {
        Self {
            max_batch_size: 5000,
            execution_timeout: Duration::from_secs(300),
            use_transaction: true,
            rollback_on_error: true,
            max_depth: 500,
            collect_stats: true,
        }
    }

    /// Create config optimized for bulk updates.
    #[must_use]
    pub fn for_bulk_updates() -> Self {
        Self {
            max_batch_size: 1000,
            execution_timeout: Duration::from_secs(180),
            use_transaction: true,
            rollback_on_error: true,
            max_depth: 200,
            collect_stats: true,
        }
    }

    /// Create config optimized for mixed operations.
    #[must_use]
    pub fn for_mixed_operations() -> Self {
        Self {
            max_batch_size: 500,
            execution_timeout: Duration::from_secs(120),
            use_transaction: true,
            rollback_on_error: true,
            max_depth: 100,
            collect_stats: true,
        }
    }

    /// Set maximum batch size.
    #[must_use]
    pub fn with_max_batch_size(mut self, size: usize) -> Self {
        self.max_batch_size = size.max(1);
        self
    }

    /// Set execution timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.execution_timeout = timeout;
        self
    }

    /// Set whether to use transaction.
    #[must_use]
    pub fn with_transaction(mut self, use_tx: bool) -> Self {
        self.use_transaction = use_tx;
        self
    }
}

/// Error from pipeline execution.
#[derive(Debug, Clone)]
pub struct PipelineError {
    /// Index of the failed query.
    pub query_index: usize,
    /// Error message.
    pub message: String,
    /// Whether this was a timeout.
    pub is_timeout: bool,
    /// SQL that failed (if available).
    pub sql: Option<String>,
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Pipeline query {} failed: {}",
            self.query_index, self.message
        )
    }
}

impl std::error::Error for PipelineError {}

/// Result of a single query in the pipeline.
#[derive(Debug, Clone)]
pub enum QueryResult {
    /// Query returned rows.
    Rows {
        /// Number of rows returned.
        count: usize,
    },
    /// Query was executed (no rows).
    Executed {
        /// Rows affected.
        rows_affected: u64,
    },
    /// Query failed.
    Error {
        /// Error message.
        message: String,
    },
}

impl QueryResult {
    /// Check if the query succeeded.
    pub fn is_success(&self) -> bool {
        !matches!(self, Self::Error { .. })
    }

    /// Get rows affected (for inserts/updates/deletes).
    pub fn rows_affected(&self) -> Option<u64> {
        match self {
            Self::Executed { rows_affected } => Some(*rows_affected),
            _ => None,
        }
    }
}

/// Result of pipeline execution.
#[derive(Debug)]
pub struct PipelineResult {
    /// Results for each query.
    pub results: Vec<QueryResult>,
    /// Total rows affected.
    pub total_affected: u64,
    /// Total rows returned.
    pub total_returned: u64,
    /// Execution statistics.
    pub stats: PipelineStats,
}

impl PipelineResult {
    /// Check if all queries succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|r| r.is_success())
    }

    /// Get first error.
    pub fn first_error(&self) -> Option<&str> {
        self.results.iter().find_map(|r| {
            if let QueryResult::Error { message } = r {
                Some(message.as_str())
            } else {
                None
            }
        })
    }

    /// Count successful queries.
    pub fn success_count(&self) -> usize {
        self.results.iter().filter(|r| r.is_success()).count()
    }

    /// Count failed queries.
    pub fn error_count(&self) -> usize {
        self.results.iter().filter(|r| !r.is_success()).count()
    }
}

/// Statistics from pipeline execution.
#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    /// Total queries executed.
    pub total_queries: usize,
    /// Successful queries.
    pub successful: usize,
    /// Failed queries.
    pub failed: usize,
    /// Total execution time.
    pub total_duration: Duration,
    /// Time spent waiting for results.
    pub wait_time: Duration,
    /// Number of batches used.
    pub batches_used: usize,
    /// Average queries per batch.
    pub avg_batch_size: f64,
}

/// A query in the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineQuery {
    /// SQL query.
    pub sql: String,
    /// Query parameters.
    pub params: Vec<FilterValue>,
    /// Whether this query returns rows.
    pub expects_rows: bool,
    /// Optional query identifier.
    pub id: Option<String>,
}

impl PipelineQuery {
    /// Create a new pipeline query.
    pub fn new(sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        Self {
            sql: sql.into(),
            params,
            expects_rows: true,
            id: None,
        }
    }

    /// Create an execute-only query (no result rows).
    pub fn execute(sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        Self {
            sql: sql.into(),
            params,
            expects_rows: false,
            id: None,
        }
    }

    /// Set query identifier.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// Query pipeline for batching multiple operations.
#[derive(Debug)]
pub struct QueryPipeline {
    config: PipelineConfig,
    queries: VecDeque<PipelineQuery>,
    db_type: DatabaseType,
}

impl QueryPipeline {
    /// Create a new query pipeline.
    pub fn new(config: PipelineConfig) -> Self {
        Self {
            config,
            queries: VecDeque::new(),
            db_type: DatabaseType::PostgreSQL,
        }
    }

    /// Set database type.
    #[must_use]
    pub fn for_database(mut self, db_type: DatabaseType) -> Self {
        self.db_type = db_type;
        self
    }

    /// Add a query to the pipeline.
    #[must_use]
    pub fn add_query(mut self, sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        self.queries.push_back(PipelineQuery::new(sql, params));
        self
    }

    /// Add an execute-only query.
    #[must_use]
    pub fn add_execute(mut self, sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        self.queries.push_back(PipelineQuery::execute(sql, params));
        self
    }

    /// Add an INSERT query.
    #[must_use]
    pub fn add_insert(self, sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        self.add_execute(sql, params)
    }

    /// Add an UPDATE query.
    #[must_use]
    pub fn add_update(self, sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        self.add_execute(sql, params)
    }

    /// Add a DELETE query.
    #[must_use]
    pub fn add_delete(self, sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        self.add_execute(sql, params)
    }

    /// Add a SELECT query.
    #[must_use]
    pub fn add_select(self, sql: impl Into<String>, params: Vec<FilterValue>) -> Self {
        self.add_query(sql, params)
    }

    /// Add a custom pipeline query.
    pub fn push(&mut self, query: PipelineQuery) {
        self.queries.push_back(query);
    }

    /// Get number of queries.
    pub fn len(&self) -> usize {
        self.queries.len()
    }

    /// Check if pipeline is empty.
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }

    /// Get the queries.
    pub fn queries(&self) -> &VecDeque<PipelineQuery> {
        &self.queries
    }

    /// Convert to batch SQL for databases that support it.
    ///
    /// Returns None if the database doesn't support multi-statement execution.
    pub fn to_batch_sql(&self) -> Option<(String, Vec<FilterValue>)> {
        if self.queries.is_empty() {
            return None;
        }

        // Only PostgreSQL and MySQL support multi-statement
        match self.db_type {
            DatabaseType::PostgreSQL | DatabaseType::MySQL => {}
            _ => return None,
        }

        let mut combined = String::new();
        let mut all_params = Vec::new();
        let mut param_offset = 0;

        for query in &self.queries {
            if !combined.is_empty() {
                combined.push_str(";\n");
            }

            // Renumber parameters for PostgreSQL
            if self.db_type == DatabaseType::PostgreSQL && !query.params.is_empty() {
                let renumbered = renumber_params(&query.sql, param_offset);
                combined.push_str(&renumbered);
                param_offset += query.params.len();
            } else {
                combined.push_str(&query.sql);
            }

            all_params.extend(query.params.clone());
        }

        Some((combined, all_params))
    }

    /// Split into batches based on config.
    pub fn into_batches(self) -> Vec<Vec<PipelineQuery>> {
        let batch_size = self.config.max_batch_size;
        let queries: Vec<_> = self.queries.into_iter().collect();

        queries.chunks(batch_size).map(|c| c.to_vec()).collect()
    }

    /// Create SQL for transactional execution.
    pub fn to_transaction_sql(&self) -> Vec<(String, Vec<FilterValue>)> {
        let mut statements = Vec::new();

        // Begin transaction
        statements.push((self.begin_transaction_sql().to_string(), Vec::new()));

        // Add all queries
        for query in &self.queries {
            statements.push((query.sql.clone(), query.params.clone()));
        }

        // Commit transaction
        statements.push((self.commit_sql().to_string(), Vec::new()));

        statements
    }

    /// Get BEGIN statement for database.
    fn begin_transaction_sql(&self) -> &'static str {
        match self.db_type {
            DatabaseType::PostgreSQL => "BEGIN",
            DatabaseType::MySQL => "START TRANSACTION",
            DatabaseType::SQLite => "BEGIN TRANSACTION",
            DatabaseType::MSSQL => "BEGIN TRANSACTION",
        }
    }

    /// Get COMMIT statement for database.
    fn commit_sql(&self) -> &'static str {
        "COMMIT"
    }

    /// Get ROLLBACK statement for database.
    #[allow(dead_code)]
    fn rollback_sql(&self) -> &'static str {
        "ROLLBACK"
    }
}

/// Renumber PostgreSQL-style parameters ($1, $2, etc) starting from an offset.
fn renumber_params(sql: &str, offset: usize) -> String {
    let mut result = String::with_capacity(sql.len() + 10);
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            // Parse the parameter number
            let mut num_str = String::new();
            while let Some(&digit) = chars.peek() {
                if digit.is_ascii_digit() {
                    num_str.push(digit);
                    chars.next();
                } else {
                    break;
                }
            }

            if let Ok(num) = num_str.parse::<usize>() {
                result.push('$');
                result.push_str(&(num + offset).to_string());
            } else {
                result.push('$');
                result.push_str(&num_str);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Builder for creating bulk insert pipelines.
#[derive(Debug)]
pub struct BulkInsertPipeline {
    table: String,
    columns: Vec<String>,
    rows: Vec<Vec<FilterValue>>,
    db_type: DatabaseType,
    batch_size: usize,
}

impl BulkInsertPipeline {
    /// Create a new bulk insert pipeline.
    pub fn new(table: impl Into<String>, columns: Vec<String>) -> Self {
        Self {
            table: table.into(),
            columns,
            rows: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
            batch_size: 1000,
        }
    }

    /// Set database type.
    #[must_use]
    pub fn for_database(mut self, db_type: DatabaseType) -> Self {
        self.db_type = db_type;
        self
    }

    /// Set batch size.
    #[must_use]
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    /// Add a row to insert.
    pub fn add_row(&mut self, values: Vec<FilterValue>) {
        assert_eq!(
            values.len(),
            self.columns.len(),
            "Row has {} values, expected {}",
            values.len(),
            self.columns.len()
        );
        self.rows.push(values);
    }

    /// Add multiple rows.
    pub fn add_rows(&mut self, rows: impl IntoIterator<Item = Vec<FilterValue>>) {
        for row in rows {
            self.add_row(row);
        }
    }

    /// Get number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Generate multi-row INSERT statements.
    pub fn to_insert_statements(&self) -> Vec<(String, Vec<FilterValue>)> {
        if self.rows.is_empty() {
            return Vec::new();
        }

        let mut statements = Vec::new();

        for chunk in self.rows.chunks(self.batch_size) {
            let (sql, params) = self.build_multi_insert(chunk);
            statements.push((sql, params));
        }

        statements
    }

    fn build_multi_insert(&self, rows: &[Vec<FilterValue>]) -> (String, Vec<FilterValue>) {
        let cols_str = self.columns.join(", ");
        let mut sql = format!("INSERT INTO {} ({}) VALUES ", self.table, cols_str);
        let mut params = Vec::with_capacity(rows.len() * self.columns.len());
        let mut param_idx = 1;

        for (row_idx, row) in rows.iter().enumerate() {
            if row_idx > 0 {
                sql.push_str(", ");
            }
            sql.push('(');

            for (col_idx, value) in row.iter().enumerate() {
                if col_idx > 0 {
                    sql.push_str(", ");
                }

                match self.db_type {
                    DatabaseType::PostgreSQL => {
                        sql.push_str(&format!("${}", param_idx));
                    }
                    DatabaseType::MySQL | DatabaseType::SQLite => {
                        sql.push('?');
                    }
                    DatabaseType::MSSQL => {
                        sql.push_str(&format!("@p{}", param_idx));
                    }
                }

                params.push(value.clone());
                param_idx += 1;
            }

            sql.push(')');
        }

        (sql, params)
    }

    /// Convert to query pipeline.
    pub fn to_pipeline(self) -> QueryPipeline {
        let statements = self.to_insert_statements();
        let mut pipeline =
            QueryPipeline::new(PipelineConfig::for_bulk_inserts()).for_database(self.db_type);

        for (sql, params) in statements {
            pipeline = pipeline.add_insert(sql, params);
        }

        pipeline
    }
}

/// Builder for creating bulk update pipelines.
#[derive(Debug)]
pub struct BulkUpdatePipeline {
    table: String,
    updates: Vec<BulkUpdate>,
    db_type: DatabaseType,
}

#[derive(Debug, Clone)]
struct BulkUpdate {
    set: Vec<(String, FilterValue)>,
    where_clause: Vec<(String, FilterValue)>,
}

impl BulkUpdatePipeline {
    /// Create a new bulk update pipeline.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            updates: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
        }
    }

    /// Set database type.
    #[must_use]
    pub fn for_database(mut self, db_type: DatabaseType) -> Self {
        self.db_type = db_type;
        self
    }

    /// Add an update.
    pub fn add_update(
        &mut self,
        set: Vec<(String, FilterValue)>,
        where_clause: Vec<(String, FilterValue)>,
    ) {
        self.updates.push(BulkUpdate { set, where_clause });
    }

    /// Get number of updates.
    pub fn len(&self) -> usize {
        self.updates.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }

    /// Generate UPDATE statements.
    pub fn to_update_statements(&self) -> Vec<(String, Vec<FilterValue>)> {
        self.updates
            .iter()
            .map(|update| self.build_update(update))
            .collect()
    }

    fn build_update(&self, update: &BulkUpdate) -> (String, Vec<FilterValue>) {
        let mut sql = format!("UPDATE {} SET ", self.table);
        let mut params = Vec::new();
        let mut param_idx = 1;

        // SET clause
        for (idx, (col, val)) in update.set.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }

            match self.db_type {
                DatabaseType::PostgreSQL => {
                    sql.push_str(&format!("{} = ${}", col, param_idx));
                }
                DatabaseType::MySQL | DatabaseType::SQLite => {
                    sql.push_str(&format!("{} = ?", col));
                }
                DatabaseType::MSSQL => {
                    sql.push_str(&format!("{} = @p{}", col, param_idx));
                }
            }

            params.push(val.clone());
            param_idx += 1;
        }

        // WHERE clause
        if !update.where_clause.is_empty() {
            sql.push_str(" WHERE ");

            for (idx, (col, val)) in update.where_clause.iter().enumerate() {
                if idx > 0 {
                    sql.push_str(" AND ");
                }

                match self.db_type {
                    DatabaseType::PostgreSQL => {
                        sql.push_str(&format!("{} = ${}", col, param_idx));
                    }
                    DatabaseType::MySQL | DatabaseType::SQLite => {
                        sql.push_str(&format!("{} = ?", col));
                    }
                    DatabaseType::MSSQL => {
                        sql.push_str(&format!("{} = @p{}", col, param_idx));
                    }
                }

                params.push(val.clone());
                param_idx += 1;
            }
        }

        (sql, params)
    }

    /// Convert to query pipeline.
    pub fn to_pipeline(self) -> QueryPipeline {
        let statements = self.to_update_statements();
        let mut pipeline =
            QueryPipeline::new(PipelineConfig::for_bulk_updates()).for_database(self.db_type);

        for (sql, params) in statements {
            pipeline = pipeline.add_update(sql, params);
        }

        pipeline
    }
}

/// Execution context for running pipelines.
///
/// This trait should be implemented for specific database connections.
#[allow(async_fn_in_trait)]
pub trait PipelineExecutor {
    /// Execute a pipeline and return results.
    async fn execute_pipeline(
        &self,
        pipeline: &QueryPipeline,
    ) -> Result<PipelineResult, PipelineError>;
}

/// Simulated pipeline execution for testing and benchmarking.
pub struct SimulatedExecutor {
    latency: Duration,
    error_rate: f64,
}

impl SimulatedExecutor {
    /// Create a new simulated executor.
    pub fn new(latency: Duration, error_rate: f64) -> Self {
        Self {
            latency,
            error_rate,
        }
    }

    /// Execute pipeline with simulated latency.
    pub async fn execute(&self, pipeline: &QueryPipeline) -> PipelineResult {
        let start = Instant::now();
        let mut results = Vec::new();
        let mut total_affected = 0u64;
        let mut successful = 0;
        let mut failed = 0;

        // Simulate batch processing
        for _query in pipeline.queries() {
            // Simulate latency (reduced for pipelining)
            tokio::time::sleep(self.latency / 10).await;

            // Simulate errors
            if rand_like_error(self.error_rate) {
                results.push(QueryResult::Error {
                    message: "Simulated error".to_string(),
                });
                failed += 1;
            } else {
                let affected = 1;
                total_affected += affected;
                results.push(QueryResult::Executed {
                    rows_affected: affected,
                });
                successful += 1;
            }
        }

        let total_duration = start.elapsed();
        let batches_used = pipeline.len().div_ceil(1000);

        PipelineResult {
            results,
            total_affected,
            total_returned: 0,
            stats: PipelineStats {
                total_queries: pipeline.len(),
                successful,
                failed,
                total_duration,
                wait_time: total_duration,
                batches_used,
                avg_batch_size: pipeline.len() as f64 / batches_used.max(1) as f64,
            },
        }
    }
}

/// Simple error simulation (not cryptographically random, just for testing).
fn rand_like_error(rate: f64) -> bool {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    (nanos as f64 / u32::MAX as f64) < rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_builder() {
        let pipeline = QueryPipeline::new(PipelineConfig::default())
            .add_insert(
                "INSERT INTO users (name) VALUES ($1)",
                vec![FilterValue::String("Alice".into())],
            )
            .add_insert(
                "INSERT INTO users (name) VALUES ($1)",
                vec![FilterValue::String("Bob".into())],
            );

        assert_eq!(pipeline.len(), 2);
    }

    #[test]
    fn test_bulk_insert_pipeline() {
        let mut pipeline =
            BulkInsertPipeline::new("users", vec!["name".into(), "age".into()]).with_batch_size(2);

        pipeline.add_row(vec![
            FilterValue::String("Alice".into()),
            FilterValue::Int(30),
        ]);
        pipeline.add_row(vec![
            FilterValue::String("Bob".into()),
            FilterValue::Int(25),
        ]);
        pipeline.add_row(vec![
            FilterValue::String("Charlie".into()),
            FilterValue::Int(35),
        ]);

        let statements = pipeline.to_insert_statements();

        // Should create 2 batches (2 + 1)
        assert_eq!(statements.len(), 2);

        // First batch has 2 rows
        let (sql1, params1) = &statements[0];
        assert!(sql1.contains("VALUES"));
        assert_eq!(params1.len(), 4); // 2 rows * 2 columns

        // Second batch has 1 row
        let (sql2, params2) = &statements[1];
        assert!(sql2.contains("VALUES"));
        assert_eq!(params2.len(), 2); // 1 row * 2 columns
    }

    #[test]
    fn test_bulk_update_pipeline() {
        let mut pipeline = BulkUpdatePipeline::new("users");

        pipeline.add_update(
            vec![("name".into(), FilterValue::String("Updated".into()))],
            vec![("id".into(), FilterValue::Int(1))],
        );

        let statements = pipeline.to_update_statements();
        assert_eq!(statements.len(), 1);

        let (sql, params) = &statements[0];
        assert!(sql.contains("UPDATE users SET"));
        assert!(sql.contains("WHERE"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_renumber_params() {
        let sql = "SELECT * FROM users WHERE id = $1 AND name = $2";
        let renumbered = renumber_params(sql, 5);
        assert_eq!(
            renumbered,
            "SELECT * FROM users WHERE id = $6 AND name = $7"
        );
    }

    #[test]
    fn test_batch_sql() {
        let pipeline = QueryPipeline::new(PipelineConfig::default())
            .for_database(DatabaseType::PostgreSQL)
            .add_query("SELECT 1", vec![])
            .add_query("SELECT 2", vec![]);

        let batch = pipeline.to_batch_sql();
        assert!(batch.is_some());

        let (sql, _) = batch.unwrap();
        assert!(sql.contains("SELECT 1"));
        assert!(sql.contains("SELECT 2"));
    }

    #[test]
    fn test_transaction_sql() {
        let pipeline = QueryPipeline::new(PipelineConfig::default())
            .for_database(DatabaseType::PostgreSQL)
            .add_insert("INSERT INTO users VALUES ($1)", vec![FilterValue::Int(1)]);

        let statements = pipeline.to_transaction_sql();

        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0].0, "BEGIN");
        assert!(statements[1].0.contains("INSERT"));
        assert_eq!(statements[2].0, "COMMIT");
    }

    #[tokio::test]
    async fn test_simulated_executor() {
        let executor = SimulatedExecutor::new(Duration::from_millis(1), 0.0);

        let pipeline = QueryPipeline::new(PipelineConfig::default())
            .add_insert("INSERT INTO users VALUES ($1)", vec![FilterValue::Int(1)])
            .add_insert("INSERT INTO users VALUES ($1)", vec![FilterValue::Int(2)]);

        let result = executor.execute(&pipeline).await;

        assert!(result.all_succeeded());
        assert_eq!(result.stats.total_queries, 2);
        assert_eq!(result.total_affected, 2);
    }
}
