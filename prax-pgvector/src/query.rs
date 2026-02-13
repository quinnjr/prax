//! High-level query builder for vector similarity search.
//!
//! This module provides a fluent builder API for constructing vector search queries
//! that integrate with the prax-postgres engine.
//!
//! # Examples
//!
//! ```rust
//! use prax_pgvector::query::VectorSearchBuilder;
//! use prax_pgvector::{Embedding, DistanceMetric};
//!
//! let query = VectorSearchBuilder::new("documents", "embedding")
//!     .query(Embedding::new(vec![0.1, 0.2, 0.3]))
//!     .metric(DistanceMetric::Cosine)
//!     .limit(10)
//!     .select(&["id", "title", "content"])
//!     .where_clause("category = 'tech'")
//!     .build();
//!
//! let sql = query.to_sql();
//! assert!(sql.contains("<=>")); // cosine distance operator
//! ```

use serde::{Deserialize, Serialize};

use crate::ops::{DistanceMetric, SearchParams};
use crate::types::Embedding;

/// A fully constructed vector search query ready for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchQuery {
    /// The table to search.
    pub table: String,
    /// The vector column.
    pub column: String,
    /// The query vector.
    pub query_vector: Embedding,
    /// Distance metric.
    pub metric: DistanceMetric,
    /// Maximum number of results.
    pub limit: usize,
    /// Columns to select (empty = all).
    pub select_columns: Vec<String>,
    /// Additional WHERE conditions.
    pub where_clauses: Vec<String>,
    /// Whether to include the distance in results.
    pub include_distance: bool,
    /// Alias for the distance column.
    pub distance_alias: String,
    /// Maximum distance threshold (radius search).
    pub max_distance: Option<f64>,
    /// Minimum distance threshold.
    pub min_distance: Option<f64>,
    /// Additional ORDER BY clauses (after distance).
    pub extra_order_by: Vec<String>,
    /// Offset for pagination.
    pub offset: Option<usize>,
    /// Search parameters (probes, ef_search).
    pub search_params: SearchParams,
}

impl VectorSearchQuery {
    /// Generate the complete SQL query.
    ///
    /// The query vector should be passed as parameter `$1`.
    pub fn to_sql(&self) -> String {
        self.to_sql_with_param(1)
    }

    /// Generate the complete SQL query with a custom parameter index.
    pub fn to_sql_with_param(&self, param_index: usize) -> String {
        let param = format!("${param_index}");
        let distance_expr = format!("{} {} {}", self.column, self.metric.operator(), param);

        // SELECT clause
        let select = if self.select_columns.is_empty() {
            "*".to_string()
        } else {
            self.select_columns.join(", ")
        };

        let distance_select = if self.include_distance {
            format!(", {} AS {}", distance_expr, self.distance_alias)
        } else {
            String::new()
        };

        // WHERE clause
        let mut where_parts = Vec::new();

        if let Some(max) = self.max_distance {
            where_parts.push(format!("{distance_expr} < {max}"));
        }
        if let Some(min) = self.min_distance {
            where_parts.push(format!("{distance_expr} >= {min}"));
        }
        where_parts.extend(self.where_clauses.clone());

        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_parts.join(" AND "))
        };

        // ORDER BY clause
        let order_by_main = if self.include_distance {
            self.distance_alias.clone()
        } else {
            distance_expr
        };

        let order_by = if self.extra_order_by.is_empty() {
            order_by_main
        } else {
            let mut parts = vec![order_by_main];
            parts.extend(self.extra_order_by.clone());
            parts.join(", ")
        };

        // LIMIT and OFFSET
        let limit = format!(" LIMIT {}", self.limit);
        let offset = self
            .offset
            .map(|o| format!(" OFFSET {o}"))
            .unwrap_or_default();

        format!(
            "SELECT {}{} FROM {}{}  ORDER BY {}{}{}",
            select, distance_select, self.table, where_clause, order_by, limit, offset
        )
    }

    /// Generate SET commands for search parameters.
    ///
    /// These should be executed before the search query to tune index scan behavior.
    pub fn param_set_sql(&self) -> Vec<String> {
        self.search_params.to_set_sql()
    }
}

/// Fluent builder for vector search queries.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::query::VectorSearchBuilder;
/// use prax_pgvector::{Embedding, DistanceMetric};
///
/// let query = VectorSearchBuilder::new("documents", "embedding")
///     .query(Embedding::new(vec![0.1, 0.2, 0.3]))
///     .metric(DistanceMetric::Cosine)
///     .limit(10)
///     .ef_search(200)
///     .build();
/// ```
pub struct VectorSearchBuilder {
    table: String,
    column: String,
    query_vector: Option<Embedding>,
    metric: DistanceMetric,
    limit: usize,
    select_columns: Vec<String>,
    where_clauses: Vec<String>,
    include_distance: bool,
    distance_alias: String,
    max_distance: Option<f64>,
    min_distance: Option<f64>,
    extra_order_by: Vec<String>,
    offset: Option<usize>,
    search_params: SearchParams,
}

impl VectorSearchBuilder {
    /// Create a new search builder for a table and vector column.
    pub fn new(table: impl Into<String>, column: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            column: column.into(),
            query_vector: None,
            metric: DistanceMetric::L2,
            limit: 10,
            select_columns: Vec::new(),
            where_clauses: Vec::new(),
            include_distance: true,
            distance_alias: "distance".to_string(),
            max_distance: None,
            min_distance: None,
            extra_order_by: Vec::new(),
            offset: None,
            search_params: SearchParams::new(),
        }
    }

    /// Set the query vector.
    pub fn query(mut self, embedding: Embedding) -> Self {
        self.query_vector = Some(embedding);
        self
    }

    /// Set the distance metric.
    pub fn metric(mut self, metric: DistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set the result limit.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set specific columns to select.
    pub fn select(mut self, columns: &[&str]) -> Self {
        self.select_columns = columns.iter().map(|c| (*c).to_string()).collect();
        self
    }

    /// Add a WHERE condition.
    pub fn where_clause(mut self, condition: impl Into<String>) -> Self {
        self.where_clauses.push(condition.into());
        self
    }

    /// Set the maximum distance (radius search).
    pub fn max_distance(mut self, distance: f64) -> Self {
        self.max_distance = Some(distance);
        self
    }

    /// Set the minimum distance.
    pub fn min_distance(mut self, distance: f64) -> Self {
        self.min_distance = Some(distance);
        self
    }

    /// Don't include the distance in the results.
    pub fn without_distance(mut self) -> Self {
        self.include_distance = false;
        self
    }

    /// Set a custom distance column alias.
    pub fn distance_alias(mut self, alias: impl Into<String>) -> Self {
        self.distance_alias = alias.into();
        self
    }

    /// Add an additional ORDER BY clause (after distance).
    pub fn then_order_by(mut self, clause: impl Into<String>) -> Self {
        self.extra_order_by.push(clause.into());
        self
    }

    /// Set the offset for pagination.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Set the IVFFlat probes parameter.
    pub fn probes(mut self, probes: usize) -> Self {
        self.search_params = self.search_params.probes(probes);
        self
    }

    /// Set the HNSW ef_search parameter.
    pub fn ef_search(mut self, ef: usize) -> Self {
        self.search_params = self.search_params.ef_search(ef);
        self
    }

    /// Build the vector search query.
    ///
    /// # Panics
    ///
    /// Panics if no query vector has been set. Use [`Self::try_build`] for
    /// a non-panicking alternative.
    pub fn build(self) -> VectorSearchQuery {
        self.try_build()
            .expect("query vector must be set before building")
    }

    /// Try to build the vector search query.
    ///
    /// Returns `None` if no query vector has been set.
    pub fn try_build(self) -> Option<VectorSearchQuery> {
        let query_vector = self.query_vector?;

        Some(VectorSearchQuery {
            table: self.table,
            column: self.column,
            query_vector,
            metric: self.metric,
            limit: self.limit,
            select_columns: self.select_columns,
            where_clauses: self.where_clauses,
            include_distance: self.include_distance,
            distance_alias: self.distance_alias,
            max_distance: self.max_distance,
            min_distance: self.min_distance,
            extra_order_by: self.extra_order_by,
            offset: self.offset,
            search_params: self.search_params,
        })
    }
}

/// Builder for hybrid search queries that combine vector similarity with full-text search.
///
/// This generates queries that use both pgvector distance operators and
/// PostgreSQL tsvector/tsquery for combined similarity scoring.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::query::HybridSearchBuilder;
/// use prax_pgvector::{Embedding, DistanceMetric};
///
/// let query = HybridSearchBuilder::new("documents")
///     .vector_column("embedding")
///     .text_column("content")
///     .query_vector(Embedding::new(vec![0.1, 0.2, 0.3]))
///     .query_text("machine learning")
///     .metric(DistanceMetric::Cosine)
///     .vector_weight(0.7)
///     .text_weight(0.3)
///     .limit(10)
///     .build();
///
/// let sql = query.to_sql();
/// ```
pub struct HybridSearchBuilder {
    table: String,
    vector_column: Option<String>,
    text_column: Option<String>,
    query_vector: Option<Embedding>,
    query_text: Option<String>,
    metric: DistanceMetric,
    vector_weight: f64,
    text_weight: f64,
    limit: usize,
    language: String,
    where_clauses: Vec<String>,
}

impl HybridSearchBuilder {
    /// Create a new hybrid search builder.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            vector_column: None,
            text_column: None,
            query_vector: None,
            query_text: None,
            metric: DistanceMetric::Cosine,
            vector_weight: 0.5,
            text_weight: 0.5,
            limit: 10,
            language: "english".to_string(),
            where_clauses: Vec::new(),
        }
    }

    /// Set the vector column name.
    pub fn vector_column(mut self, column: impl Into<String>) -> Self {
        self.vector_column = Some(column.into());
        self
    }

    /// Set the text column name.
    pub fn text_column(mut self, column: impl Into<String>) -> Self {
        self.text_column = Some(column.into());
        self
    }

    /// Set the query vector.
    pub fn query_vector(mut self, embedding: Embedding) -> Self {
        self.query_vector = Some(embedding);
        self
    }

    /// Set the text query.
    pub fn query_text(mut self, text: impl Into<String>) -> Self {
        self.query_text = Some(text.into());
        self
    }

    /// Set the vector distance metric.
    pub fn metric(mut self, metric: DistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set the weight for the vector similarity component (0.0 to 1.0).
    pub fn vector_weight(mut self, weight: f64) -> Self {
        self.vector_weight = weight;
        self
    }

    /// Set the weight for the text relevance component (0.0 to 1.0).
    pub fn text_weight(mut self, weight: f64) -> Self {
        self.text_weight = weight;
        self
    }

    /// Set the result limit.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set the text search language.
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    /// Add a WHERE condition.
    pub fn where_clause(mut self, condition: impl Into<String>) -> Self {
        self.where_clauses.push(condition.into());
        self
    }

    /// Build the hybrid search query.
    pub fn build(self) -> HybridSearchQuery {
        HybridSearchQuery {
            table: self.table,
            vector_column: self
                .vector_column
                .unwrap_or_else(|| "embedding".to_string()),
            text_column: self.text_column.unwrap_or_else(|| "content".to_string()),
            query_vector: self.query_vector,
            query_text: self.query_text,
            metric: self.metric,
            vector_weight: self.vector_weight,
            text_weight: self.text_weight,
            limit: self.limit,
            language: self.language,
            where_clauses: self.where_clauses,
        }
    }
}

/// A hybrid search query combining vector similarity and full-text search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchQuery {
    /// Table name.
    pub table: String,
    /// Vector column.
    pub vector_column: String,
    /// Text column.
    pub text_column: String,
    /// Query vector.
    pub query_vector: Option<Embedding>,
    /// Text query.
    pub query_text: Option<String>,
    /// Distance metric.
    pub metric: DistanceMetric,
    /// Weight for vector similarity (0.0-1.0).
    pub vector_weight: f64,
    /// Weight for text relevance (0.0-1.0).
    pub text_weight: f64,
    /// Result limit.
    pub limit: usize,
    /// Text search language.
    pub language: String,
    /// Additional WHERE conditions.
    pub where_clauses: Vec<String>,
}

impl HybridSearchQuery {
    /// Generate the SQL query using Reciprocal Rank Fusion (RRF).
    ///
    /// RRF combines rankings from multiple retrieval methods:
    /// `score = sum(1 / (k + rank_i))` where k is a constant (typically 60).
    ///
    /// The query vector should be `$1` and the text query should be `$2`.
    pub fn to_sql(&self) -> String {
        let vec_distance = format!("{} {} $1", self.vector_column, self.metric.operator());
        let text_rank = format!(
            "ts_rank(to_tsvector('{}', {}), plainto_tsquery('{}', $2))",
            self.language, self.text_column, self.language
        );

        let where_clause = if self.where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.where_clauses.join(" AND "))
        };

        // Use RRF scoring: combine vector and text rankings
        format!(
            "WITH vector_results AS (\
                SELECT *, ROW_NUMBER() OVER (ORDER BY {vec_distance}) AS vec_rank \
                FROM {table}{where_clause} \
                ORDER BY {vec_distance} \
                LIMIT {fetch_limit}\
            ), \
            text_results AS (\
                SELECT *, ROW_NUMBER() OVER (ORDER BY {text_rank} DESC) AS text_rank \
                FROM {table}{where_clause} \
                WHERE to_tsvector('{lang}', {text_col}) @@ plainto_tsquery('{lang}', $2) \
                ORDER BY {text_rank} DESC \
                LIMIT {fetch_limit}\
            ) \
            SELECT COALESCE(v.*, t.*), \
                ({vec_weight} / (60.0 + COALESCE(v.vec_rank, 1000))) + \
                ({text_weight} / (60.0 + COALESCE(t.text_rank, 1000))) AS rrf_score \
            FROM vector_results v \
            FULL OUTER JOIN text_results t ON v.id = t.id \
            ORDER BY rrf_score DESC \
            LIMIT {limit}",
            table = self.table,
            where_clause = where_clause,
            fetch_limit = self.limit * 3, // Fetch more for fusion
            vec_weight = self.vector_weight,
            text_weight = self.text_weight,
            lang = self.language,
            text_col = self.text_column,
            limit = self.limit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_embedding() -> Embedding {
        Embedding::new(vec![0.1, 0.2, 0.3])
    }

    #[test]
    fn test_basic_search_query() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .metric(DistanceMetric::Cosine)
            .limit(10)
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("SELECT *"));
        assert!(sql.contains("AS distance"));
        assert!(sql.contains("<=>"));
        assert!(sql.contains("$1"));
        assert!(sql.contains("FROM documents"));
        assert!(sql.contains("LIMIT 10"));
    }

    #[test]
    fn test_search_with_select() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .select(&["id", "title"])
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("SELECT id, title"));
    }

    #[test]
    fn test_search_with_where() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .where_clause("category = 'tech'")
            .where_clause("published = true")
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("category = 'tech'"));
        assert!(sql.contains("published = true"));
        assert!(sql.contains("AND"));
    }

    #[test]
    fn test_search_with_max_distance() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .metric(DistanceMetric::L2)
            .max_distance(0.5)
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("< 0.5"));
    }

    #[test]
    fn test_search_with_distance_range() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .min_distance(0.1)
            .max_distance(0.5)
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("< 0.5"));
        assert!(sql.contains(">= 0.1"));
    }

    #[test]
    fn test_search_without_distance() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .without_distance()
            .build();

        let sql = query.to_sql();
        assert!(!sql.contains("AS distance"));
    }

    #[test]
    fn test_search_custom_alias() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .distance_alias("similarity")
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("AS similarity"));
    }

    #[test]
    fn test_search_with_pagination() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .limit(10)
            .offset(20)
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("OFFSET 20"));
    }

    #[test]
    fn test_search_with_extra_order_by() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .then_order_by("created_at DESC")
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("ORDER BY distance, created_at DESC"));
    }

    #[test]
    fn test_search_params() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .probes(10)
            .ef_search(200)
            .build();

        let set_sql = query.param_set_sql();
        assert_eq!(set_sql.len(), 2);
        assert!(set_sql[0].contains("ivfflat.probes = 10"));
        assert!(set_sql[1].contains("hnsw.ef_search = 200"));
    }

    #[test]
    fn test_try_build_without_vector() {
        let result = VectorSearchBuilder::new("documents", "embedding").try_build();
        assert!(result.is_none());
    }

    #[test]
    fn test_custom_param_index() {
        let query = VectorSearchBuilder::new("documents", "embedding")
            .query(test_embedding())
            .build();

        let sql = query.to_sql_with_param(3);
        assert!(sql.contains("$3"));
    }

    #[test]
    fn test_hybrid_search() {
        let query = HybridSearchBuilder::new("documents")
            .vector_column("embedding")
            .text_column("content")
            .query_vector(test_embedding())
            .query_text("machine learning")
            .metric(DistanceMetric::Cosine)
            .vector_weight(0.7)
            .text_weight(0.3)
            .limit(10)
            .build();

        let sql = query.to_sql();
        assert!(sql.contains("vector_results"));
        assert!(sql.contains("text_results"));
        assert!(sql.contains("rrf_score"));
        assert!(sql.contains("<=>"));
        assert!(sql.contains("ts_rank"));
        assert!(sql.contains("FULL OUTER JOIN"));
    }

    #[test]
    fn test_all_metrics_produce_valid_sql() {
        for metric in [
            DistanceMetric::L2,
            DistanceMetric::InnerProduct,
            DistanceMetric::Cosine,
            DistanceMetric::L1,
        ] {
            let query = VectorSearchBuilder::new("t", "c")
                .query(test_embedding())
                .metric(metric)
                .build();
            let sql = query.to_sql();
            assert!(sql.contains(metric.operator()), "failed for {metric}");
        }
    }
}
