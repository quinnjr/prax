//! Vector index management for pgvector.
//!
//! pgvector supports two approximate nearest-neighbor (ANN) index types:
//!
//! | Index | Algorithm | Best For | Tradeoff |
//! |-------|-----------|----------|----------|
//! | **IVFFlat** | Inverted file with flat quantization | Large datasets, tunable recall | Requires training data |
//! | **HNSW** | Hierarchical navigable small world | Most workloads, no training needed | Higher memory usage |
//!
//! # Choosing an Index
//!
//! - **HNSW** is recommended for most use cases — better recall/speed tradeoff,
//!   no training step, and supports concurrent inserts.
//! - **IVFFlat** is useful when memory is constrained or when you have very
//!   large datasets and can tolerate a training step.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{VectorError, VectorResult};
use crate::ops::{BinaryDistanceMetric, DistanceMetric};

/// The type of ANN index to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum IndexType {
    /// IVFFlat (Inverted File with Flat quantization).
    IvfFlat(IvfFlatConfig),

    /// HNSW (Hierarchical Navigable Small World).
    Hnsw(HnswConfig),
}

impl fmt::Display for IndexType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IvfFlat(_) => write!(f, "ivfflat"),
            Self::Hnsw(_) => write!(f, "hnsw"),
        }
    }
}

/// Configuration for IVFFlat indexes.
///
/// IVFFlat divides vectors into `lists` number of clusters during a training phase.
/// At query time, `probes` clusters are searched.
///
/// # Tuning Guidelines
///
/// - `lists`: Start with `rows / 1000` for up to 1M rows, `sqrt(rows)` for more.
/// - `probes`: Start with `sqrt(lists)` and increase for better recall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IvfFlatConfig {
    /// Number of inverted lists (clusters).
    ///
    /// More lists = faster search but potentially lower recall.
    /// Recommended: `rows / 1000` for up to 1M rows.
    pub lists: usize,
}

impl IvfFlatConfig {
    /// Create a new IVFFlat config with the given number of lists.
    pub fn new(lists: usize) -> Self {
        Self { lists }
    }

    /// Create a config with the recommended number of lists for a given row count.
    pub fn for_row_count(rows: usize) -> Self {
        let lists = if rows <= 1_000_000 {
            (rows / 1000).max(1)
        } else {
            (rows as f64).sqrt() as usize
        };
        Self { lists }
    }
}

impl Default for IvfFlatConfig {
    fn default() -> Self {
        Self { lists: 100 }
    }
}

/// Configuration for HNSW indexes.
///
/// HNSW builds a multi-layered graph that enables efficient approximate nearest-neighbor
/// search without a separate training step.
///
/// # Tuning Guidelines
///
/// - `m`: Number of connections per node. Higher = better recall, more memory.
///   Default: 16. Range: 2-100.
/// - `ef_construction`: Size of the dynamic candidate list during index build.
///   Higher = better recall, slower build. Default: 64. Range: 4-1000.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Maximum number of connections per node per layer.
    ///
    /// Higher values improve recall but increase memory and build time.
    /// Default: 16.
    pub m: Option<usize>,

    /// Size of the dynamic candidate list during construction.
    ///
    /// Higher values improve index quality but slow down build.
    /// Default: 64.
    pub ef_construction: Option<usize>,
}

impl HnswConfig {
    /// Create a new HNSW config with defaults.
    pub fn new() -> Self {
        Self {
            m: None,
            ef_construction: None,
        }
    }

    /// Set the `m` parameter (connections per node).
    pub fn m(mut self, m: usize) -> Self {
        self.m = Some(m);
        self
    }

    /// Set the `ef_construction` parameter.
    pub fn ef_construction(mut self, ef: usize) -> Self {
        self.ef_construction = Some(ef);
        self
    }

    /// High-recall configuration (slower build, better search quality).
    pub fn high_recall() -> Self {
        Self {
            m: Some(32),
            ef_construction: Some(128),
        }
    }

    /// Fast-build configuration (faster build, lower recall).
    pub fn fast_build() -> Self {
        Self {
            m: Some(8),
            ef_construction: Some(32),
        }
    }
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// A vector index definition.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::index::{VectorIndex, HnswConfig};
/// use prax_pgvector::DistanceMetric;
///
/// // Create an HNSW index
/// let index = VectorIndex::hnsw("idx_embedding", "documents", "embedding")
///     .metric(DistanceMetric::Cosine)
///     .config(HnswConfig::high_recall())
///     .build()
///     .unwrap();
///
/// let sql = index.to_create_sql();
/// assert!(sql.contains("USING hnsw"));
/// assert!(sql.contains("vector_cosine_ops"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorIndex {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table: String,
    /// Column name.
    pub column: String,
    /// Distance metric.
    pub metric: DistanceMetric,
    /// Index type and configuration.
    pub index_type: IndexType,
    /// Whether to create concurrently (non-blocking).
    pub concurrent: bool,
    /// Whether to add IF NOT EXISTS clause.
    pub if_not_exists: bool,
}

impl VectorIndex {
    /// Start building an HNSW index.
    pub fn hnsw(
        name: impl Into<String>,
        table: impl Into<String>,
        column: impl Into<String>,
    ) -> VectorIndexBuilder {
        VectorIndexBuilder {
            name: name.into(),
            table: table.into(),
            column: column.into(),
            metric: DistanceMetric::L2,
            index_type: IndexType::Hnsw(HnswConfig::default()),
            concurrent: false,
            if_not_exists: false,
        }
    }

    /// Start building an IVFFlat index.
    pub fn ivfflat(
        name: impl Into<String>,
        table: impl Into<String>,
        column: impl Into<String>,
    ) -> VectorIndexBuilder {
        VectorIndexBuilder {
            name: name.into(),
            table: table.into(),
            column: column.into(),
            metric: DistanceMetric::L2,
            index_type: IndexType::IvfFlat(IvfFlatConfig::default()),
            concurrent: false,
            if_not_exists: false,
        }
    }

    /// Generate the CREATE INDEX SQL statement.
    pub fn to_create_sql(&self) -> String {
        let concurrent = if self.concurrent { " CONCURRENTLY" } else { "" };
        let if_not_exists = if self.if_not_exists {
            " IF NOT EXISTS"
        } else {
            ""
        };

        let (method, with_clause) = match &self.index_type {
            IndexType::IvfFlat(config) => {
                let with = format!(" WITH (lists = {})", config.lists);
                ("ivfflat", with)
            }
            IndexType::Hnsw(config) => {
                let mut with_parts = Vec::new();
                if let Some(m) = config.m {
                    with_parts.push(format!("m = {m}"));
                }
                if let Some(ef) = config.ef_construction {
                    with_parts.push(format!("ef_construction = {ef}"));
                }
                let with = if with_parts.is_empty() {
                    String::new()
                } else {
                    format!(" WITH ({})", with_parts.join(", "))
                };
                ("hnsw", with)
            }
        };

        format!(
            "CREATE INDEX{}{} {} ON {} USING {} ({} {}){}",
            concurrent,
            if_not_exists,
            self.name,
            self.table,
            method,
            self.column,
            self.metric.ops_class(),
            with_clause
        )
    }

    /// Generate the DROP INDEX SQL statement.
    pub fn to_drop_sql(&self) -> String {
        let concurrent = if self.concurrent { " CONCURRENTLY" } else { "" };
        format!("DROP INDEX{} IF EXISTS {}", concurrent, self.name)
    }

    /// Generate SQL to check if this index exists.
    pub fn to_exists_sql(&self) -> String {
        format!(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = '{}')",
            self.name
        )
    }

    /// Generate SQL to get the index size.
    pub fn to_size_sql(&self) -> String {
        format!("SELECT pg_size_pretty(pg_relation_size('{}'))", self.name)
    }
}

/// Builder for [`VectorIndex`].
#[derive(Debug, Clone)]
pub struct VectorIndexBuilder {
    name: String,
    table: String,
    column: String,
    metric: DistanceMetric,
    index_type: IndexType,
    concurrent: bool,
    if_not_exists: bool,
}

impl VectorIndexBuilder {
    /// Set the distance metric.
    pub fn metric(mut self, metric: DistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set the HNSW configuration (only effective for HNSW indexes).
    pub fn config(mut self, config: HnswConfig) -> Self {
        self.index_type = IndexType::Hnsw(config);
        self
    }

    /// Set the IVFFlat configuration (only effective for IVFFlat indexes).
    pub fn ivfflat_config(mut self, config: IvfFlatConfig) -> Self {
        self.index_type = IndexType::IvfFlat(config);
        self
    }

    /// Create the index concurrently (non-blocking).
    pub fn concurrent(mut self) -> Self {
        self.concurrent = true;
        self
    }

    /// Add IF NOT EXISTS clause.
    pub fn if_not_exists(mut self) -> Self {
        self.if_not_exists = true;
        self
    }

    /// Build the index definition.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn build(self) -> VectorResult<VectorIndex> {
        if self.name.is_empty() {
            return Err(VectorError::index("index name cannot be empty"));
        }
        if self.table.is_empty() {
            return Err(VectorError::index("table name cannot be empty"));
        }
        if self.column.is_empty() {
            return Err(VectorError::index("column name cannot be empty"));
        }

        Ok(VectorIndex {
            name: self.name,
            table: self.table,
            column: self.column,
            metric: self.metric,
            index_type: self.index_type,
            concurrent: self.concurrent,
            if_not_exists: self.if_not_exists,
        })
    }
}

/// A binary vector index definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryVectorIndex {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table: String,
    /// Column name.
    pub column: String,
    /// Distance metric.
    pub metric: BinaryDistanceMetric,
    /// HNSW configuration (only HNSW is supported for bit vectors).
    pub hnsw_config: HnswConfig,
    /// Whether to create concurrently.
    pub concurrent: bool,
}

impl BinaryVectorIndex {
    /// Create a new binary vector index builder.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        name: impl Into<String>,
        table: impl Into<String>,
        column: impl Into<String>,
    ) -> BinaryVectorIndexBuilder {
        BinaryVectorIndexBuilder {
            name: name.into(),
            table: table.into(),
            column: column.into(),
            metric: BinaryDistanceMetric::Hamming,
            hnsw_config: HnswConfig::default(),
            concurrent: false,
        }
    }

    /// Generate the CREATE INDEX SQL.
    pub fn to_create_sql(&self) -> String {
        let concurrent = if self.concurrent { " CONCURRENTLY" } else { "" };

        let mut with_parts = Vec::new();
        if let Some(m) = self.hnsw_config.m {
            with_parts.push(format!("m = {m}"));
        }
        if let Some(ef) = self.hnsw_config.ef_construction {
            with_parts.push(format!("ef_construction = {ef}"));
        }
        let with = if with_parts.is_empty() {
            String::new()
        } else {
            format!(" WITH ({})", with_parts.join(", "))
        };

        format!(
            "CREATE INDEX{} {} ON {} USING hnsw ({} {}){}",
            concurrent,
            self.name,
            self.table,
            self.column,
            self.metric.ops_class(),
            with
        )
    }
}

/// Builder for [`BinaryVectorIndex`].
#[derive(Debug, Clone)]
pub struct BinaryVectorIndexBuilder {
    name: String,
    table: String,
    column: String,
    metric: BinaryDistanceMetric,
    hnsw_config: HnswConfig,
    concurrent: bool,
}

impl BinaryVectorIndexBuilder {
    /// Set the distance metric.
    pub fn metric(mut self, metric: BinaryDistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set the HNSW configuration.
    pub fn config(mut self, config: HnswConfig) -> Self {
        self.hnsw_config = config;
        self
    }

    /// Create the index concurrently.
    pub fn concurrent(mut self) -> Self {
        self.concurrent = true;
        self
    }

    /// Build the index definition.
    pub fn build(self) -> VectorResult<BinaryVectorIndex> {
        if self.name.is_empty() {
            return Err(VectorError::index("index name cannot be empty"));
        }
        Ok(BinaryVectorIndex {
            name: self.name,
            table: self.table,
            column: self.column,
            metric: self.metric,
            hnsw_config: self.hnsw_config,
            concurrent: self.concurrent,
        })
    }
}

/// SQL helpers for pgvector extension management.
pub mod extension {
    /// Generate SQL to create the pgvector extension.
    pub fn create_extension_sql() -> &'static str {
        "CREATE EXTENSION IF NOT EXISTS vector"
    }

    /// Generate SQL to create the pgvector extension in a specific schema.
    pub fn create_extension_in_schema_sql(schema: &str) -> String {
        format!("CREATE EXTENSION IF NOT EXISTS vector SCHEMA {schema}")
    }

    /// Generate SQL to drop the pgvector extension.
    pub fn drop_extension_sql() -> &'static str {
        "DROP EXTENSION IF EXISTS vector"
    }

    /// Generate SQL to check if pgvector is installed.
    pub fn check_extension_sql() -> &'static str {
        "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector')"
    }

    /// Generate SQL to get the installed pgvector version.
    pub fn version_sql() -> &'static str {
        "SELECT extversion FROM pg_extension WHERE extname = 'vector'"
    }

    /// Generate SQL to create a vector column.
    pub fn add_vector_column_sql(table: &str, column: &str, dimensions: usize) -> String {
        format!("ALTER TABLE {table} ADD COLUMN {column} vector({dimensions})")
    }

    /// Generate SQL to create a halfvec column.
    pub fn add_halfvec_column_sql(table: &str, column: &str, dimensions: usize) -> String {
        format!("ALTER TABLE {table} ADD COLUMN {column} halfvec({dimensions})")
    }

    /// Generate SQL to create a sparsevec column.
    pub fn add_sparsevec_column_sql(table: &str, column: &str, dimensions: usize) -> String {
        format!("ALTER TABLE {table} ADD COLUMN {column} sparsevec({dimensions})")
    }

    /// Generate SQL to create a bit column.
    pub fn add_bit_column_sql(table: &str, column: &str, dimensions: usize) -> String {
        format!("ALTER TABLE {table} ADD COLUMN {column} bit({dimensions})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_index_create_sql() {
        let index = VectorIndex::hnsw("idx_embedding", "documents", "embedding")
            .metric(DistanceMetric::Cosine)
            .config(HnswConfig::new().m(16).ef_construction(64))
            .build()
            .unwrap();

        let sql = index.to_create_sql();
        assert!(sql.contains("CREATE INDEX"));
        assert!(sql.contains("idx_embedding"));
        assert!(sql.contains("documents"));
        assert!(sql.contains("USING hnsw"));
        assert!(sql.contains("vector_cosine_ops"));
        assert!(sql.contains("m = 16"));
        assert!(sql.contains("ef_construction = 64"));
    }

    #[test]
    fn test_hnsw_index_default_config() {
        let index = VectorIndex::hnsw("idx_emb", "docs", "emb").build().unwrap();

        let sql = index.to_create_sql();
        assert!(sql.contains("USING hnsw"));
        assert!(sql.contains("vector_l2_ops")); // default metric
        assert!(!sql.contains("WITH")); // no config = no WITH clause
    }

    #[test]
    fn test_ivfflat_index_create_sql() {
        let index = VectorIndex::ivfflat("idx_embedding", "documents", "embedding")
            .metric(DistanceMetric::L2)
            .ivfflat_config(IvfFlatConfig::new(200))
            .build()
            .unwrap();

        let sql = index.to_create_sql();
        assert!(sql.contains("USING ivfflat"));
        assert!(sql.contains("vector_l2_ops"));
        assert!(sql.contains("lists = 200"));
    }

    #[test]
    fn test_ivfflat_for_row_count() {
        let config = IvfFlatConfig::for_row_count(500_000);
        assert_eq!(config.lists, 500);

        let config = IvfFlatConfig::for_row_count(5_000_000);
        assert_eq!(config.lists, 2236); // sqrt(5M)
    }

    #[test]
    fn test_concurrent_index() {
        let index = VectorIndex::hnsw("idx_emb", "docs", "emb")
            .concurrent()
            .if_not_exists()
            .build()
            .unwrap();

        let sql = index.to_create_sql();
        assert!(sql.contains("CONCURRENTLY"));
        assert!(sql.contains("IF NOT EXISTS"));
    }

    #[test]
    fn test_drop_index() {
        let index = VectorIndex::hnsw("idx_emb", "docs", "emb").build().unwrap();

        let sql = index.to_drop_sql();
        assert_eq!(sql, "DROP INDEX IF EXISTS idx_emb");
    }

    #[test]
    fn test_concurrent_drop_index() {
        let index = VectorIndex::hnsw("idx_emb", "docs", "emb")
            .concurrent()
            .build()
            .unwrap();

        let sql = index.to_drop_sql();
        assert!(sql.contains("CONCURRENTLY"));
    }

    #[test]
    fn test_index_exists_sql() {
        let index = VectorIndex::hnsw("idx_emb", "docs", "emb").build().unwrap();

        let sql = index.to_exists_sql();
        assert!(sql.contains("pg_indexes"));
        assert!(sql.contains("idx_emb"));
    }

    #[test]
    fn test_index_size_sql() {
        let index = VectorIndex::hnsw("idx_emb", "docs", "emb").build().unwrap();

        let sql = index.to_size_sql();
        assert!(sql.contains("pg_size_pretty"));
        assert!(sql.contains("idx_emb"));
    }

    #[test]
    fn test_empty_name_error() {
        let result = VectorIndex::hnsw("", "docs", "emb").build();
        assert!(result.is_err());
    }

    #[test]
    fn test_hnsw_high_recall() {
        let config = HnswConfig::high_recall();
        assert_eq!(config.m, Some(32));
        assert_eq!(config.ef_construction, Some(128));
    }

    #[test]
    fn test_hnsw_fast_build() {
        let config = HnswConfig::fast_build();
        assert_eq!(config.m, Some(8));
        assert_eq!(config.ef_construction, Some(32));
    }

    #[test]
    fn test_binary_vector_index() {
        let index = BinaryVectorIndex::new("idx_bits", "docs", "binary_emb")
            .metric(BinaryDistanceMetric::Hamming)
            .build()
            .unwrap();

        let sql = index.to_create_sql();
        assert!(sql.contains("USING hnsw"));
        assert!(sql.contains("bit_hamming_ops"));
    }

    #[test]
    fn test_extension_create_sql() {
        assert_eq!(
            extension::create_extension_sql(),
            "CREATE EXTENSION IF NOT EXISTS vector"
        );
    }

    #[test]
    fn test_extension_in_schema() {
        let sql = extension::create_extension_in_schema_sql("public");
        assert!(sql.contains("SCHEMA public"));
    }

    #[test]
    fn test_add_vector_column() {
        let sql = extension::add_vector_column_sql("documents", "embedding", 1536);
        assert_eq!(
            sql,
            "ALTER TABLE documents ADD COLUMN embedding vector(1536)"
        );
    }

    #[test]
    fn test_add_sparsevec_column() {
        let sql = extension::add_sparsevec_column_sql("documents", "sparse_emb", 30000);
        assert!(sql.contains("sparsevec(30000)"));
    }

    #[test]
    fn test_add_bit_column() {
        let sql = extension::add_bit_column_sql("documents", "binary_emb", 1024);
        assert!(sql.contains("bit(1024)"));
    }

    #[test]
    fn test_check_extension_sql() {
        let sql = extension::check_extension_sql();
        assert!(sql.contains("pg_extension"));
    }

    #[test]
    fn test_version_sql() {
        let sql = extension::version_sql();
        assert!(sql.contains("extversion"));
    }

    #[test]
    fn test_index_type_display() {
        let ivf = IndexType::IvfFlat(IvfFlatConfig::default());
        assert_eq!(format!("{ivf}"), "ivfflat");

        let hnsw = IndexType::Hnsw(HnswConfig::default());
        assert_eq!(format!("{hnsw}"), "hnsw");
    }

    #[test]
    fn test_all_metrics_with_ivfflat() {
        for metric in [
            DistanceMetric::L2,
            DistanceMetric::InnerProduct,
            DistanceMetric::Cosine,
            DistanceMetric::L1,
        ] {
            let index = VectorIndex::ivfflat("idx", "t", "c")
                .metric(metric)
                .build()
                .unwrap();
            let sql = index.to_create_sql();
            assert!(sql.contains(metric.ops_class()));
        }
    }

    #[test]
    fn test_all_metrics_with_hnsw() {
        for metric in [
            DistanceMetric::L2,
            DistanceMetric::InnerProduct,
            DistanceMetric::Cosine,
            DistanceMetric::L1,
        ] {
            let index = VectorIndex::hnsw("idx", "t", "c")
                .metric(metric)
                .build()
                .unwrap();
            let sql = index.to_create_sql();
            assert!(sql.contains(metric.ops_class()));
        }
    }
}
