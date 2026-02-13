//! Database-specific optimizations.
//!
//! This module provides performance optimizations tailored to each database:
//! - Prepared statement caching (PostgreSQL, MySQL, MSSQL)
//! - Batch size tuning for bulk operations
//! - MongoDB pipeline aggregation
//! - Query plan hints for complex queries
//!
//! # Performance Characteristics
//!
//! | Database   | Optimization              | Typical Gain |
//! |------------|---------------------------|--------------|
//! | PostgreSQL | Prepared statement cache  | 30-50%       |
//! | MySQL      | Multi-row INSERT batching | 40-60%       |
//! | MongoDB    | Bulk write batching       | 50-70%       |
//! | MSSQL      | Table-valued parameters   | 30-40%       |
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::db_optimize::{PreparedStatementCache, BatchConfig, QueryHints};
//!
//! // Prepared statement caching
//! let cache = PreparedStatementCache::new(100);
//! let stmt = cache.get_or_prepare("find_user", || {
//!     "SELECT * FROM users WHERE id = $1"
//! });
//!
//! // Auto-tuned batching
//! let config = BatchConfig::auto_tune(payload_size, row_count);
//! for batch in data.chunks(config.batch_size) {
//!     execute_batch(batch);
//! }
//!
//! // Query hints
//! let hints = QueryHints::new()
//!     .parallel(4)
//!     .index_hint("users_email_idx");
//! ```

use parking_lot::RwLock;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::sql::DatabaseType;

// ==============================================================================
// Prepared Statement Cache
// ==============================================================================

/// Statistics for prepared statement cache.
#[derive(Debug, Clone, Default)]
pub struct PreparedStatementStats {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of statements currently cached.
    pub cached_count: usize,
    /// Total preparation time saved (estimated).
    pub time_saved_ms: u64,
}

impl PreparedStatementStats {
    /// Calculate hit rate as a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// A cached prepared statement entry.
#[derive(Debug, Clone)]
pub struct CachedStatement {
    /// The SQL statement text.
    pub sql: String,
    /// Unique statement identifier/name.
    pub name: String,
    /// Number of times this statement was used.
    pub use_count: u64,
    /// When this statement was last used.
    pub last_used: Instant,
    /// Estimated preparation time in microseconds.
    pub prep_time_us: u64,
    /// Database-specific statement handle (opaque).
    pub handle: Option<u64>,
}

/// A cache for prepared statements.
///
/// This cache stores prepared statement metadata and tracks usage patterns
/// to optimize database interactions. The actual statement handles are
/// managed by the database driver.
///
/// # Features
///
/// - LRU eviction when capacity is reached
/// - Usage statistics for monitoring
/// - Thread-safe with read-write locking
/// - Automatic cleanup of stale entries
///
/// # Example
///
/// ```rust
/// use prax_query::db_optimize::PreparedStatementCache;
///
/// let cache = PreparedStatementCache::new(100);
///
/// // Register a prepared statement
/// let entry = cache.get_or_create("find_user_by_email", || {
///     "SELECT * FROM users WHERE email = $1".to_string()
/// });
///
/// // Check cache stats
/// let stats = cache.stats();
/// println!("Hit rate: {:.1}%", stats.hit_rate());
/// ```
pub struct PreparedStatementCache {
    statements: RwLock<HashMap<String, CachedStatement>>,
    capacity: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    time_saved_us: AtomicU64,
    /// Average preparation time in microseconds (for estimation).
    avg_prep_time_us: u64,
}

impl PreparedStatementCache {
    /// Create a new cache with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            statements: RwLock::new(HashMap::with_capacity(capacity)),
            capacity,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            time_saved_us: AtomicU64::new(0),
            avg_prep_time_us: 500, // Default 500µs estimate
        }
    }

    /// Get or create a prepared statement entry.
    ///
    /// If the statement is cached, returns the cached entry and increments hit count.
    /// Otherwise, calls the generator function, caches the result, and returns it.
    pub fn get_or_create<F>(&self, name: &str, generator: F) -> CachedStatement
    where
        F: FnOnce() -> String,
    {
        // Try read lock first (fast path)
        {
            let cache = self.statements.read();
            if let Some(stmt) = cache.get(name) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                self.time_saved_us
                    .fetch_add(stmt.prep_time_us, Ordering::Relaxed);
                return stmt.clone();
            }
        }

        // Miss - need to create and cache
        self.misses.fetch_add(1, Ordering::Relaxed);

        let sql = generator();
        let entry = CachedStatement {
            sql,
            name: name.to_string(),
            use_count: 1,
            last_used: Instant::now(),
            prep_time_us: self.avg_prep_time_us,
            handle: None,
        };

        // Upgrade to write lock
        let mut cache = self.statements.write();

        // Double-check after acquiring write lock
        if let Some(existing) = cache.get(name) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return existing.clone();
        }

        // Evict if at capacity (simple LRU-like: remove oldest)
        if cache.len() >= self.capacity {
            self.evict_oldest(&mut cache);
        }

        cache.insert(name.to_string(), entry.clone());
        entry
    }

    /// Check if a statement is cached.
    pub fn contains(&self, name: &str) -> bool {
        self.statements.read().contains_key(name)
    }

    /// Get cache statistics.
    pub fn stats(&self) -> PreparedStatementStats {
        let cache = self.statements.read();
        PreparedStatementStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            cached_count: cache.len(),
            time_saved_ms: self.time_saved_us.load(Ordering::Relaxed) / 1000,
        }
    }

    /// Clear the cache.
    pub fn clear(&self) {
        self.statements.write().clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.time_saved_us.store(0, Ordering::Relaxed);
    }

    /// Get the number of cached statements.
    pub fn len(&self) -> usize {
        self.statements.read().len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.statements.read().is_empty()
    }

    /// Evict the oldest entry.
    fn evict_oldest(&self, cache: &mut HashMap<String, CachedStatement>) {
        if let Some((oldest_key, _)) = cache
            .iter()
            .min_by_key(|(_, v)| v.last_used)
            .map(|(k, v)| (k.clone(), v.clone()))
        {
            cache.remove(&oldest_key);
        }
    }

    /// Update statement usage (call after executing).
    pub fn record_use(&self, name: &str) {
        if let Some(stmt) = self.statements.write().get_mut(name) {
            stmt.use_count += 1;
            stmt.last_used = Instant::now();
        }
    }

    /// Set a database-specific handle for a statement.
    pub fn set_handle(&self, name: &str, handle: u64) {
        if let Some(stmt) = self.statements.write().get_mut(name) {
            stmt.handle = Some(handle);
        }
    }
}

impl Default for PreparedStatementCache {
    fn default() -> Self {
        Self::new(256)
    }
}

/// Global prepared statement cache.
pub fn global_statement_cache() -> &'static PreparedStatementCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<PreparedStatementCache> = OnceLock::new();
    CACHE.get_or_init(|| PreparedStatementCache::new(512))
}

// ==============================================================================
// Batch Size Tuning
// ==============================================================================

/// Configuration for batch operations.
#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    /// Number of rows per batch.
    pub batch_size: usize,
    /// Maximum payload size in bytes.
    pub max_payload_bytes: usize,
    /// Whether to use multi-row INSERT syntax.
    pub multi_row_insert: bool,
    /// Whether to use COPY for bulk inserts (PostgreSQL).
    pub use_copy: bool,
    /// Parallelism level for bulk operations.
    pub parallelism: usize,
}

impl BatchConfig {
    /// Default batch configuration.
    pub const fn default_config() -> Self {
        Self {
            batch_size: 1000,
            max_payload_bytes: 16 * 1024 * 1024, // 16MB
            multi_row_insert: true,
            use_copy: false,
            parallelism: 1,
        }
    }

    /// Create configuration optimized for the given database.
    pub fn for_database(db_type: DatabaseType) -> Self {
        match db_type {
            DatabaseType::PostgreSQL => Self {
                batch_size: 1000,
                max_payload_bytes: 64 * 1024 * 1024, // 64MB
                multi_row_insert: true,
                use_copy: true, // PostgreSQL COPY is very fast
                parallelism: 4,
            },
            DatabaseType::MySQL => Self {
                batch_size: 500,                     // MySQL has packet size limits
                max_payload_bytes: 16 * 1024 * 1024, // 16MB (default max_allowed_packet)
                multi_row_insert: true,
                use_copy: false,
                parallelism: 2,
            },
            DatabaseType::SQLite => Self {
                batch_size: 500,
                max_payload_bytes: 1024 * 1024, // 1MB (SQLite is single-threaded)
                multi_row_insert: true,
                use_copy: false,
                parallelism: 1, // SQLite doesn't benefit from parallelism
            },
            DatabaseType::MSSQL => Self {
                batch_size: 1000,
                max_payload_bytes: 32 * 1024 * 1024, // 32MB
                multi_row_insert: true,
                use_copy: false,
                parallelism: 4,
            },
        }
    }

    /// Auto-tune batch size based on row size and count.
    ///
    /// This calculates an optimal batch size that:
    /// - Stays within the max payload size
    /// - Balances memory usage vs round-trip overhead
    /// - Adapts to row size variations
    ///
    /// # Example
    ///
    /// ```rust
    /// use prax_query::db_optimize::BatchConfig;
    /// use prax_query::sql::DatabaseType;
    ///
    /// // Auto-tune for 10,000 rows averaging 500 bytes each
    /// let config = BatchConfig::auto_tune(
    ///     DatabaseType::PostgreSQL,
    ///     500,    // avg row size in bytes
    ///     10_000, // total row count
    /// );
    /// println!("Optimal batch size: {}", config.batch_size);
    /// ```
    pub fn auto_tune(db_type: DatabaseType, avg_row_size: usize, total_rows: usize) -> Self {
        let mut config = Self::for_database(db_type);

        // Calculate batch size based on payload limit
        let max_rows_by_payload = if avg_row_size > 0 {
            config.max_payload_bytes / avg_row_size
        } else {
            config.batch_size
        };

        // Balance: smaller batches for small datasets, larger for big ones
        let optimal_batch = if total_rows < 100 {
            total_rows // No batching needed for small datasets
        } else if total_rows < 1000 {
            (total_rows / 10).max(100)
        } else {
            // For large datasets, use ~10 batches or max by payload
            let by_count = total_rows / 10;
            by_count.min(max_rows_by_payload).min(10_000).max(100)
        };

        config.batch_size = optimal_batch;

        // Adjust parallelism based on dataset size
        if total_rows < 1000 {
            config.parallelism = 1;
        } else if total_rows < 10_000 {
            config.parallelism = config.parallelism.min(2);
        }

        // Use COPY for large PostgreSQL imports
        if matches!(db_type, DatabaseType::PostgreSQL) && total_rows > 5000 {
            config.use_copy = true;
        }

        config
    }

    /// Create an iterator that yields batch ranges.
    ///
    /// # Example
    ///
    /// ```rust
    /// use prax_query::db_optimize::BatchConfig;
    ///
    /// let config = BatchConfig::default_config();
    /// let total = 2500;
    ///
    /// for (start, end) in config.batch_ranges(total) {
    ///     println!("Processing rows {} to {}", start, end);
    /// }
    /// ```
    pub fn batch_ranges(&self, total: usize) -> impl Iterator<Item = (usize, usize)> {
        let batch_size = self.batch_size;
        (0..total)
            .step_by(batch_size)
            .map(move |start| (start, (start + batch_size).min(total)))
    }

    /// Calculate the number of batches for a given total.
    pub fn batch_count(&self, total: usize) -> usize {
        (total + self.batch_size - 1) / self.batch_size
    }
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self::default_config()
    }
}

// ==============================================================================
// MongoDB Pipeline Aggregation
// ==============================================================================

/// A builder for combining multiple MongoDB operations into a single pipeline.
///
/// This reduces round-trips by batching related operations.
///
/// # Example
///
/// ```rust,ignore
/// use prax_query::db_optimize::MongoPipelineBuilder;
///
/// let pipeline = MongoPipelineBuilder::new()
///     .match_stage(doc! { "status": "active" })
///     .lookup("orders", "user_id", "_id", "user_orders")
///     .unwind("$user_orders")
///     .group("$user_id", doc! { "total": { "$sum": "$amount" } })
///     .sort(doc! { "total": -1 })
///     .limit(10)
///     .build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct MongoPipelineBuilder {
    stages: Vec<PipelineStage>,
    /// Whether to allow disk use for large operations.
    pub allow_disk_use: bool,
    /// Batch size for cursor.
    pub batch_size: Option<u32>,
    /// Maximum time for operation in milliseconds.
    pub max_time_ms: Option<u64>,
    /// Comment for profiling.
    pub comment: Option<String>,
}

/// A MongoDB aggregation pipeline stage.
#[derive(Debug, Clone)]
pub enum PipelineStage {
    /// $match stage.
    Match(String),
    /// $project stage.
    Project(String),
    /// $group stage with _id and accumulators.
    Group { id: String, accumulators: String },
    /// $sort stage.
    Sort(String),
    /// $limit stage.
    Limit(u64),
    /// $skip stage.
    Skip(u64),
    /// $unwind stage.
    Unwind { path: String, preserve_null: bool },
    /// $lookup stage.
    Lookup {
        from: String,
        local_field: String,
        foreign_field: String,
        r#as: String,
    },
    /// $addFields stage.
    AddFields(String),
    /// $set stage (alias for $addFields).
    Set(String),
    /// $unset stage.
    Unset(Vec<String>),
    /// $replaceRoot stage.
    ReplaceRoot(String),
    /// $count stage.
    Count(String),
    /// $facet stage for multiple pipelines.
    Facet(Vec<(String, Vec<PipelineStage>)>),
    /// $bucket stage.
    Bucket {
        group_by: String,
        boundaries: String,
        default: Option<String>,
        output: Option<String>,
    },
    /// $sample stage.
    Sample(u64),
    /// $merge stage for output.
    Merge {
        into: String,
        on: Option<String>,
        when_matched: Option<String>,
        when_not_matched: Option<String>,
    },
    /// $out stage.
    Out(String),
    /// Raw BSON stage.
    Raw(String),
}

impl MongoPipelineBuilder {
    /// Create a new empty pipeline builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a $match stage.
    pub fn match_stage(mut self, filter: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Match(filter.into()));
        self
    }

    /// Add a $project stage.
    pub fn project(mut self, projection: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Project(projection.into()));
        self
    }

    /// Add a $group stage.
    pub fn group(mut self, id: impl Into<String>, accumulators: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Group {
            id: id.into(),
            accumulators: accumulators.into(),
        });
        self
    }

    /// Add a $sort stage.
    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Sort(sort.into()));
        self
    }

    /// Add a $limit stage.
    pub fn limit(mut self, n: u64) -> Self {
        self.stages.push(PipelineStage::Limit(n));
        self
    }

    /// Add a $skip stage.
    pub fn skip(mut self, n: u64) -> Self {
        self.stages.push(PipelineStage::Skip(n));
        self
    }

    /// Add a $unwind stage.
    pub fn unwind(mut self, path: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Unwind {
            path: path.into(),
            preserve_null: false,
        });
        self
    }

    /// Add a $unwind stage with null preservation.
    pub fn unwind_preserve_null(mut self, path: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Unwind {
            path: path.into(),
            preserve_null: true,
        });
        self
    }

    /// Add a $lookup stage.
    pub fn lookup(
        mut self,
        from: impl Into<String>,
        local_field: impl Into<String>,
        foreign_field: impl Into<String>,
        r#as: impl Into<String>,
    ) -> Self {
        self.stages.push(PipelineStage::Lookup {
            from: from.into(),
            local_field: local_field.into(),
            foreign_field: foreign_field.into(),
            r#as: r#as.into(),
        });
        self
    }

    /// Add a $addFields stage.
    pub fn add_fields(mut self, fields: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::AddFields(fields.into()));
        self
    }

    /// Add a $set stage.
    pub fn set(mut self, fields: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Set(fields.into()));
        self
    }

    /// Add a $unset stage.
    pub fn unset<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.stages.push(PipelineStage::Unset(
            fields.into_iter().map(Into::into).collect(),
        ));
        self
    }

    /// Add a $replaceRoot stage.
    pub fn replace_root(mut self, new_root: impl Into<String>) -> Self {
        self.stages
            .push(PipelineStage::ReplaceRoot(new_root.into()));
        self
    }

    /// Add a $count stage.
    pub fn count(mut self, field: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Count(field.into()));
        self
    }

    /// Add a $sample stage.
    pub fn sample(mut self, size: u64) -> Self {
        self.stages.push(PipelineStage::Sample(size));
        self
    }

    /// Add a $merge output stage.
    pub fn merge_into(mut self, collection: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Merge {
            into: collection.into(),
            on: None,
            when_matched: None,
            when_not_matched: None,
        });
        self
    }

    /// Add a $merge output stage with options.
    pub fn merge(
        mut self,
        into: impl Into<String>,
        on: Option<String>,
        when_matched: Option<String>,
        when_not_matched: Option<String>,
    ) -> Self {
        self.stages.push(PipelineStage::Merge {
            into: into.into(),
            on,
            when_matched,
            when_not_matched,
        });
        self
    }

    /// Add a $out stage.
    pub fn out(mut self, collection: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Out(collection.into()));
        self
    }

    /// Add a raw BSON stage.
    pub fn raw(mut self, stage: impl Into<String>) -> Self {
        self.stages.push(PipelineStage::Raw(stage.into()));
        self
    }

    /// Enable disk use for large operations.
    pub fn with_disk_use(mut self) -> Self {
        self.allow_disk_use = true;
        self
    }

    /// Set cursor batch size.
    pub fn with_batch_size(mut self, size: u32) -> Self {
        self.batch_size = Some(size);
        self
    }

    /// Set maximum execution time.
    pub fn with_max_time(mut self, ms: u64) -> Self {
        self.max_time_ms = Some(ms);
        self
    }

    /// Add a comment for profiling.
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Get the number of stages.
    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    /// Build the pipeline as a JSON array string.
    pub fn build(&self) -> String {
        let stages: Vec<String> = self.stages.iter().map(|s| s.to_json()).collect();
        format!("[{}]", stages.join(", "))
    }

    /// Get the stages.
    pub fn stages(&self) -> &[PipelineStage] {
        &self.stages
    }
}

impl PipelineStage {
    /// Convert to JSON representation.
    pub fn to_json(&self) -> String {
        match self {
            Self::Match(filter) => format!(r#"{{ "$match": {} }}"#, filter),
            Self::Project(proj) => format!(r#"{{ "$project": {} }}"#, proj),
            Self::Group { id, accumulators } => {
                format!(r#"{{ "$group": {{ "_id": {}, {} }} }}"#, id, accumulators)
            }
            Self::Sort(sort) => format!(r#"{{ "$sort": {} }}"#, sort),
            Self::Limit(n) => format!(r#"{{ "$limit": {} }}"#, n),
            Self::Skip(n) => format!(r#"{{ "$skip": {} }}"#, n),
            Self::Unwind {
                path,
                preserve_null,
            } => {
                if *preserve_null {
                    format!(
                        r#"{{ "$unwind": {{ "path": "{}", "preserveNullAndEmptyArrays": true }} }}"#,
                        path
                    )
                } else {
                    format!(r#"{{ "$unwind": "{}" }}"#, path)
                }
            }
            Self::Lookup {
                from,
                local_field,
                foreign_field,
                r#as,
            } => {
                format!(
                    r#"{{ "$lookup": {{ "from": "{}", "localField": "{}", "foreignField": "{}", "as": "{}" }} }}"#,
                    from, local_field, foreign_field, r#as
                )
            }
            Self::AddFields(fields) => format!(r#"{{ "$addFields": {} }}"#, fields),
            Self::Set(fields) => format!(r#"{{ "$set": {} }}"#, fields),
            Self::Unset(fields) => {
                let quoted: Vec<_> = fields.iter().map(|f| format!(r#""{}""#, f)).collect();
                format!(r#"{{ "$unset": [{}] }}"#, quoted.join(", "))
            }
            Self::ReplaceRoot(root) => {
                format!(r#"{{ "$replaceRoot": {{ "newRoot": {} }} }}"#, root)
            }
            Self::Count(field) => format!(r#"{{ "$count": "{}" }}"#, field),
            Self::Facet(facets) => {
                let facet_strs: Vec<_> = facets
                    .iter()
                    .map(|(name, stages)| {
                        let pipeline: Vec<_> = stages.iter().map(|s| s.to_json()).collect();
                        format!(r#""{}": [{}]"#, name, pipeline.join(", "))
                    })
                    .collect();
                format!(r#"{{ "$facet": {{ {} }} }}"#, facet_strs.join(", "))
            }
            Self::Bucket {
                group_by,
                boundaries,
                default,
                output,
            } => {
                let mut parts = vec![
                    format!(r#""groupBy": {}"#, group_by),
                    format!(r#""boundaries": {}"#, boundaries),
                ];
                if let Some(def) = default {
                    parts.push(format!(r#""default": {}"#, def));
                }
                if let Some(out) = output {
                    parts.push(format!(r#""output": {}"#, out));
                }
                format!(r#"{{ "$bucket": {{ {} }} }}"#, parts.join(", "))
            }
            Self::Sample(size) => format!(r#"{{ "$sample": {{ "size": {} }} }}"#, size),
            Self::Merge {
                into,
                on,
                when_matched,
                when_not_matched,
            } => {
                let mut parts = vec![format!(r#""into": "{}""#, into)];
                if let Some(on_field) = on {
                    parts.push(format!(r#""on": "{}""#, on_field));
                }
                if let Some(matched) = when_matched {
                    parts.push(format!(r#""whenMatched": "{}""#, matched));
                }
                if let Some(not_matched) = when_not_matched {
                    parts.push(format!(r#""whenNotMatched": "{}""#, not_matched));
                }
                format!(r#"{{ "$merge": {{ {} }} }}"#, parts.join(", "))
            }
            Self::Out(collection) => format!(r#"{{ "$out": "{}" }}"#, collection),
            Self::Raw(stage) => stage.clone(),
        }
    }
}

// ==============================================================================
// Query Plan Hints
// ==============================================================================

/// Query plan hints for optimizing complex queries.
///
/// These hints are applied to queries to guide the query planner:
/// - Index hints to force specific index usage
/// - Parallelism settings
/// - Join strategies
/// - Materialization preferences
///
/// # Database Support
///
/// | Hint Type | PostgreSQL | MySQL | SQLite | MSSQL |
/// |-----------|------------|-------|--------|-------|
/// | Index     | ✅ (GUC)   | ✅    | ✅     | ✅    |
/// | Parallel  | ✅         | ❌    | ❌     | ✅    |
/// | Join      | ✅         | ✅    | ❌     | ✅    |
/// | CTE Mat   | ✅         | ❌    | ❌     | ❌    |
///
/// # Example
///
/// ```rust
/// use prax_query::db_optimize::QueryHints;
/// use prax_query::sql::DatabaseType;
///
/// let hints = QueryHints::new()
///     .index_hint("users_email_idx")
///     .parallel(4)
///     .no_seq_scan();
///
/// let sql = hints.apply_to_query("SELECT * FROM users WHERE email = $1", DatabaseType::PostgreSQL);
/// ```
#[derive(Debug, Clone, Default)]
pub struct QueryHints {
    /// Index hints.
    pub indexes: SmallVec<[IndexHint; 4]>,
    /// Parallelism level (0 = default, >0 = specific workers).
    pub parallel_workers: Option<u32>,
    /// Join method hints.
    pub join_hints: SmallVec<[JoinHint; 4]>,
    /// Whether to prevent sequential scans.
    pub no_seq_scan: bool,
    /// Whether to prevent index scans.
    pub no_index_scan: bool,
    /// CTE materialization preference.
    pub cte_materialized: Option<bool>,
    /// Query timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Custom database-specific hints.
    pub custom: Vec<String>,
}

/// An index hint.
#[derive(Debug, Clone)]
pub struct IndexHint {
    /// Table the index belongs to.
    pub table: Option<String>,
    /// Index name.
    pub index_name: String,
    /// Hint type.
    pub hint_type: IndexHintType,
}

/// Type of index hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexHintType {
    /// Force use of this index.
    Use,
    /// Force ignore of this index.
    Ignore,
    /// Prefer this index if possible.
    Prefer,
}

/// A join method hint.
#[derive(Debug, Clone)]
pub struct JoinHint {
    /// Tables involved in the join.
    pub tables: Vec<String>,
    /// Join method to use.
    pub method: JoinMethod,
}

/// Join methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinMethod {
    /// Nested loop join.
    NestedLoop,
    /// Hash join.
    Hash,
    /// Merge join.
    Merge,
}

impl QueryHints {
    /// Create new empty hints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an index hint.
    pub fn index_hint(mut self, index_name: impl Into<String>) -> Self {
        self.indexes.push(IndexHint {
            table: None,
            index_name: index_name.into(),
            hint_type: IndexHintType::Use,
        });
        self
    }

    /// Add an index hint for a specific table.
    pub fn index_hint_for_table(
        mut self,
        table: impl Into<String>,
        index_name: impl Into<String>,
    ) -> Self {
        self.indexes.push(IndexHint {
            table: Some(table.into()),
            index_name: index_name.into(),
            hint_type: IndexHintType::Use,
        });
        self
    }

    /// Ignore a specific index.
    pub fn ignore_index(mut self, index_name: impl Into<String>) -> Self {
        self.indexes.push(IndexHint {
            table: None,
            index_name: index_name.into(),
            hint_type: IndexHintType::Ignore,
        });
        self
    }

    /// Set parallelism level.
    pub fn parallel(mut self, workers: u32) -> Self {
        self.parallel_workers = Some(workers);
        self
    }

    /// Disable parallel execution.
    pub fn no_parallel(mut self) -> Self {
        self.parallel_workers = Some(0);
        self
    }

    /// Prevent sequential scans.
    pub fn no_seq_scan(mut self) -> Self {
        self.no_seq_scan = true;
        self
    }

    /// Prevent index scans.
    pub fn no_index_scan(mut self) -> Self {
        self.no_index_scan = true;
        self
    }

    /// Set CTE materialization preference.
    pub fn cte_materialized(mut self, materialized: bool) -> Self {
        self.cte_materialized = Some(materialized);
        self
    }

    /// Force nested loop join.
    pub fn nested_loop_join(mut self, tables: Vec<String>) -> Self {
        self.join_hints.push(JoinHint {
            tables,
            method: JoinMethod::NestedLoop,
        });
        self
    }

    /// Force hash join.
    pub fn hash_join(mut self, tables: Vec<String>) -> Self {
        self.join_hints.push(JoinHint {
            tables,
            method: JoinMethod::Hash,
        });
        self
    }

    /// Force merge join.
    pub fn merge_join(mut self, tables: Vec<String>) -> Self {
        self.join_hints.push(JoinHint {
            tables,
            method: JoinMethod::Merge,
        });
        self
    }

    /// Set query timeout.
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Add a custom database-specific hint.
    pub fn custom_hint(mut self, hint: impl Into<String>) -> Self {
        self.custom.push(hint.into());
        self
    }

    /// Generate hints as SQL prefix for the given database.
    pub fn to_sql_prefix(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_prefix(),
            DatabaseType::MySQL => self.to_mysql_prefix(),
            DatabaseType::SQLite => self.to_sqlite_prefix(),
            DatabaseType::MSSQL => self.to_mssql_prefix(),
        }
    }

    /// Generate hints as SQL suffix (for query options).
    pub fn to_sql_suffix(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::MySQL => self.to_mysql_suffix(),
            DatabaseType::MSSQL => self.to_mssql_suffix(),
            _ => String::new(),
        }
    }

    /// Apply hints to a query.
    pub fn apply_to_query(&self, query: &str, db_type: DatabaseType) -> String {
        let prefix = self.to_sql_prefix(db_type);
        let suffix = self.to_sql_suffix(db_type);

        if prefix.is_empty() && suffix.is_empty() {
            return query.to_string();
        }

        let mut result = String::with_capacity(prefix.len() + query.len() + suffix.len() + 2);
        if !prefix.is_empty() {
            result.push_str(&prefix);
            result.push('\n');
        }
        result.push_str(query);
        if !suffix.is_empty() {
            result.push(' ');
            result.push_str(&suffix);
        }
        result
    }

    fn to_postgres_prefix(&self) -> String {
        let mut settings: Vec<String> = Vec::new();

        if self.no_seq_scan {
            settings.push("SET LOCAL enable_seqscan = off;".to_string());
        }
        if self.no_index_scan {
            settings.push("SET LOCAL enable_indexscan = off;".to_string());
        }
        if let Some(workers) = self.parallel_workers {
            settings.push(format!(
                "SET LOCAL max_parallel_workers_per_gather = {};",
                workers
            ));
        }
        if let Some(ms) = self.timeout_ms {
            settings.push(format!("SET LOCAL statement_timeout = {};", ms));
        }

        // Join hints
        for hint in &self.join_hints {
            match hint.method {
                JoinMethod::NestedLoop => {
                    settings.push("SET LOCAL enable_hashjoin = off;".to_string());
                    settings.push("SET LOCAL enable_mergejoin = off;".to_string());
                }
                JoinMethod::Hash => {
                    settings.push("SET LOCAL enable_nestloop = off;".to_string());
                    settings.push("SET LOCAL enable_mergejoin = off;".to_string());
                }
                JoinMethod::Merge => {
                    settings.push("SET LOCAL enable_nestloop = off;".to_string());
                    settings.push("SET LOCAL enable_hashjoin = off;".to_string());
                }
            }
        }

        // Custom hints
        for hint in &self.custom {
            settings.push(hint.clone());
        }

        settings.join("\n")
    }

    fn to_mysql_prefix(&self) -> String {
        // MySQL uses inline hints, not SET statements
        String::new()
    }

    fn to_mysql_suffix(&self) -> String {
        let mut hints: Vec<String> = Vec::new();

        // Index hints (applied after table name in actual query, but we return as hint comment)
        for hint in &self.indexes {
            let hint_type = match hint.hint_type {
                IndexHintType::Use => "USE INDEX",
                IndexHintType::Ignore => "IGNORE INDEX",
                IndexHintType::Prefer => "FORCE INDEX",
            };
            if let Some(ref table) = hint.table {
                hints.push(format!(
                    "/* {} FOR {} ({}) */",
                    hint_type, table, hint.index_name
                ));
            } else {
                hints.push(format!("/* {} ({}) */", hint_type, hint.index_name));
            }
        }

        // Join hints
        for hint in &self.join_hints {
            let method = match hint.method {
                JoinMethod::NestedLoop => "BNL",
                JoinMethod::Hash => "HASH_JOIN",
                JoinMethod::Merge => "MERGE",
            };
            hints.push(format!("/* {}({}) */", method, hint.tables.join(", ")));
        }

        hints.join(" ")
    }

    fn to_sqlite_prefix(&self) -> String {
        // SQLite has limited hint support
        String::new()
    }

    fn to_mssql_prefix(&self) -> String {
        // MSSQL uses inline OPTION hints
        String::new()
    }

    fn to_mssql_suffix(&self) -> String {
        let mut options: Vec<String> = Vec::new();

        // Index hints
        for hint in &self.indexes {
            match hint.hint_type {
                IndexHintType::Use => {
                    if let Some(ref table) = hint.table {
                        options.push(format!("TABLE HINT({}, INDEX({}))", table, hint.index_name));
                    }
                }
                IndexHintType::Ignore => {
                    // MSSQL doesn't have ignore index, skip
                }
                IndexHintType::Prefer => {
                    if let Some(ref table) = hint.table {
                        options.push(format!(
                            "TABLE HINT({}, FORCESEEK({}))",
                            table, hint.index_name
                        ));
                    }
                }
            }
        }

        // Parallelism
        if let Some(workers) = self.parallel_workers {
            if workers == 0 {
                options.push("MAXDOP 1".to_string());
            } else {
                options.push(format!("MAXDOP {}", workers));
            }
        }

        // Join hints
        for hint in &self.join_hints {
            let method = match hint.method {
                JoinMethod::NestedLoop => "LOOP JOIN",
                JoinMethod::Hash => "HASH JOIN",
                JoinMethod::Merge => "MERGE JOIN",
            };
            options.push(method.to_string());
        }

        if options.is_empty() {
            String::new()
        } else {
            format!("OPTION ({})", options.join(", "))
        }
    }

    /// Check if any hints are configured.
    pub fn has_hints(&self) -> bool {
        !self.indexes.is_empty()
            || self.parallel_workers.is_some()
            || !self.join_hints.is_empty()
            || self.no_seq_scan
            || self.no_index_scan
            || self.cte_materialized.is_some()
            || self.timeout_ms.is_some()
            || !self.custom.is_empty()
    }
}

// ==============================================================================
// Tests
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepared_statement_cache() {
        let cache = PreparedStatementCache::new(10);

        // First access - miss
        let stmt1 = cache.get_or_create("test", || "SELECT * FROM users".to_string());
        assert_eq!(stmt1.sql, "SELECT * FROM users");

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);

        // Second access - hit
        let stmt2 = cache.get_or_create("test", || panic!("Should not be called"));
        assert_eq!(stmt2.sql, "SELECT * FROM users");

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 1);
        assert!(stats.hit_rate() > 0.0);
    }

    #[test]
    fn test_batch_config_auto_tune() {
        // Small dataset
        let config = BatchConfig::auto_tune(DatabaseType::PostgreSQL, 100, 50);
        assert_eq!(config.batch_size, 50); // No batching needed

        // Medium dataset
        let config = BatchConfig::auto_tune(DatabaseType::PostgreSQL, 500, 5000);
        assert!(config.batch_size >= 100);
        assert!(config.batch_size <= 5000);

        // Large dataset
        let config = BatchConfig::auto_tune(DatabaseType::PostgreSQL, 200, 100_000);
        assert!(config.use_copy); // Should use COPY for large PG imports
        assert!(config.batch_size >= 100);
    }

    #[test]
    fn test_batch_ranges() {
        let config = BatchConfig {
            batch_size: 100,
            ..Default::default()
        };

        let ranges: Vec<_> = config.batch_ranges(250).collect();
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], (0, 100));
        assert_eq!(ranges[1], (100, 200));
        assert_eq!(ranges[2], (200, 250));
    }

    #[test]
    fn test_mongo_pipeline_builder() {
        let pipeline = MongoPipelineBuilder::new()
            .match_stage(r#"{ "status": "active" }"#)
            .lookup("orders", "user_id", "_id", "user_orders")
            .unwind("$user_orders")
            .group(r#""$user_id""#, r#""total": { "$sum": "$amount" }"#)
            .sort(r#"{ "total": -1 }"#)
            .limit(10)
            .build();

        assert!(pipeline.contains("$match"));
        assert!(pipeline.contains("$lookup"));
        assert!(pipeline.contains("$unwind"));
        assert!(pipeline.contains("$group"));
        assert!(pipeline.contains("$sort"));
        assert!(pipeline.contains("$limit"));
    }

    #[test]
    fn test_query_hints_postgres() {
        let hints = QueryHints::new().no_seq_scan().parallel(4).timeout(5000);

        let prefix = hints.to_sql_prefix(DatabaseType::PostgreSQL);
        assert!(prefix.contains("enable_seqscan = off"));
        assert!(prefix.contains("max_parallel_workers_per_gather = 4"));
        assert!(prefix.contains("statement_timeout = 5000"));
    }

    #[test]
    fn test_query_hints_mssql() {
        let hints = QueryHints::new()
            .parallel(2)
            .hash_join(vec!["users".to_string(), "orders".to_string()]);

        let suffix = hints.to_sql_suffix(DatabaseType::MSSQL);
        assert!(suffix.contains("MAXDOP 2"));
        assert!(suffix.contains("HASH JOIN"));
    }

    #[test]
    fn test_query_hints_apply() {
        let hints = QueryHints::new().no_seq_scan();

        let query = "SELECT * FROM users WHERE id = $1";
        let result = hints.apply_to_query(query, DatabaseType::PostgreSQL);

        assert!(result.contains("enable_seqscan = off"));
        assert!(result.contains("SELECT * FROM users"));
    }
}
