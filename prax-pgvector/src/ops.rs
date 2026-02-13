//! Distance operators and similarity metrics for pgvector.
//!
//! pgvector supports multiple distance functions, each with a corresponding
//! PostgreSQL operator. This module provides type-safe abstractions for these.
//!
//! # Operators
//!
//! | Metric | Operator | Index Ops Class |
//! |--------|----------|-----------------|
//! | L2 (Euclidean) | `<->` | `vector_l2_ops` |
//! | Inner Product | `<#>` | `vector_ip_ops` |
//! | Cosine | `<=>` | `vector_cosine_ops` |
//! | L1 (Manhattan) | `<+>` | `vector_l1_ops` |
//! | Hamming | `<~>` | `bit_hamming_ops` |
//! | Jaccard | `<%>` | `bit_jaccard_ops` |

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::Embedding;

/// Vector distance metric supported by pgvector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DistanceMetric {
    /// Euclidean distance (L2 norm).
    ///
    /// Operator: `<->`
    /// Range: [0, ∞)
    /// Use when: comparing absolute distances between vectors.
    L2,

    /// Negative inner product.
    ///
    /// Operator: `<#>`
    /// Range: (-∞, ∞)
    /// Use when: vectors are normalized and you want maximum inner product.
    ///
    /// Note: pgvector returns *negative* inner product so that smaller = more similar,
    /// consistent with the ORDER BY ASC convention.
    InnerProduct,

    /// Cosine distance (1 - cosine similarity).
    ///
    /// Operator: `<=>`
    /// Range: [0, 2]
    /// Use when: comparing direction regardless of magnitude.
    Cosine,

    /// Manhattan distance (L1 norm).
    ///
    /// Operator: `<+>`
    /// Range: [0, ∞)
    /// Use when: you need L1 distance, often in recommendation systems.
    L1,
}

impl DistanceMetric {
    /// Get the PostgreSQL operator for this metric.
    pub fn operator(&self) -> &'static str {
        match self {
            Self::L2 => "<->",
            Self::InnerProduct => "<#>",
            Self::Cosine => "<=>",
            Self::L1 => "<+>",
        }
    }

    /// Get the operator class name for index creation.
    pub fn ops_class(&self) -> &'static str {
        match self {
            Self::L2 => "vector_l2_ops",
            Self::InnerProduct => "vector_ip_ops",
            Self::Cosine => "vector_cosine_ops",
            Self::L1 => "vector_l1_ops",
        }
    }

    /// Get a human-readable name for this metric.
    pub fn name(&self) -> &'static str {
        match self {
            Self::L2 => "euclidean",
            Self::InnerProduct => "inner_product",
            Self::Cosine => "cosine",
            Self::L1 => "manhattan",
        }
    }

    /// Whether this metric benefits from normalized vectors.
    pub fn prefers_normalized(&self) -> bool {
        matches!(self, Self::InnerProduct | Self::Cosine)
    }
}

impl fmt::Display for DistanceMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Binary vector distance metric supported by pgvector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BinaryDistanceMetric {
    /// Hamming distance (number of differing bits).
    ///
    /// Operator: `<~>`
    Hamming,

    /// Jaccard distance (1 - Jaccard index).
    ///
    /// Operator: `<%>`
    Jaccard,
}

impl BinaryDistanceMetric {
    /// Get the PostgreSQL operator for this metric.
    pub fn operator(&self) -> &'static str {
        match self {
            Self::Hamming => "<~>",
            Self::Jaccard => "<%>",
        }
    }

    /// Get the operator class name for index creation.
    pub fn ops_class(&self) -> &'static str {
        match self {
            Self::Hamming => "bit_hamming_ops",
            Self::Jaccard => "bit_jaccard_ops",
        }
    }

    /// Get a human-readable name for this metric.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hamming => "hamming",
            Self::Jaccard => "jaccard",
        }
    }
}

impl fmt::Display for BinaryDistanceMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Generate SQL for computing the distance between a column and a query vector.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::{Embedding, DistanceMetric, ops::distance_sql};
///
/// let query = Embedding::new(vec![0.1, 0.2, 0.3]);
/// let sql = distance_sql("embedding", &query, DistanceMetric::Cosine);
/// assert!(sql.contains("<=>"));
/// ```
pub fn distance_sql(column: &str, query_vector: &Embedding, metric: DistanceMetric) -> String {
    format!(
        "{} {} {}",
        column,
        metric.operator(),
        query_vector.to_sql_literal()
    )
}

/// Generate SQL for computing distance with a parameter placeholder.
///
/// This is preferred over [`distance_sql`] when using parameterized queries
/// to prevent SQL injection.
///
/// # Examples
///
/// ```rust
/// use prax_pgvector::{DistanceMetric, ops::distance_param_sql};
///
/// let sql = distance_param_sql("embedding", "$1", DistanceMetric::L2);
/// assert_eq!(sql, "embedding <-> $1");
/// ```
pub fn distance_param_sql(column: &str, param: &str, metric: DistanceMetric) -> String {
    format!("{} {} {}", column, metric.operator(), param)
}

/// Generate an ORDER BY clause for nearest-neighbor search.
///
/// Returns a SQL fragment like: `embedding <-> '[0.1,0.2,0.3]'::vector`
/// suitable for use in ORDER BY.
pub fn order_by_distance(column: &str, query_vector: &Embedding, metric: DistanceMetric) -> String {
    distance_sql(column, query_vector, metric)
}

/// Generate a complete nearest-neighbor search query.
///
/// This generates SQL like:
/// ```sql
/// SELECT *, embedding <-> $1 AS distance
/// FROM documents
/// ORDER BY distance
/// LIMIT 10
/// ```
pub fn nearest_neighbor_sql(
    table: &str,
    column: &str,
    metric: DistanceMetric,
    param_index: usize,
    limit: usize,
    extra_columns: &[&str],
) -> String {
    let distance_expr = distance_param_sql(column, &format!("${param_index}"), metric);

    let select_cols = if extra_columns.is_empty() {
        "*".to_string()
    } else {
        let mut cols = vec!["*".to_string()];
        cols.extend(extra_columns.iter().map(|c| (*c).to_string()));
        cols.join(", ")
    };

    format!(
        "SELECT {}, {} AS distance FROM {} ORDER BY distance LIMIT {}",
        select_cols, distance_expr, table, limit
    )
}

/// Generate SQL for a distance-filtered search (within a radius).
///
/// Returns SQL like:
/// ```sql
/// SELECT *, embedding <-> $1 AS distance
/// FROM documents
/// WHERE embedding <-> $1 < 0.5
/// ORDER BY distance
/// LIMIT 100
/// ```
pub fn radius_search_sql(
    table: &str,
    column: &str,
    metric: DistanceMetric,
    param_index: usize,
    max_distance: f64,
    limit: Option<usize>,
) -> String {
    let param = format!("${param_index}");
    let distance_expr = distance_param_sql(column, &param, metric);

    let limit_clause = limit.map(|l| format!(" LIMIT {l}")).unwrap_or_default();

    format!(
        "SELECT *, {} AS distance FROM {} WHERE {} < {} ORDER BY distance{}",
        distance_expr, table, distance_expr, max_distance, limit_clause
    )
}

/// Configuration for setting pgvector search parameters.
///
/// These SET commands tune the behavior of approximate index scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    /// Number of IVFFlat lists to probe (default: 1).
    ///
    /// Higher values improve recall at the cost of speed.
    pub ivfflat_probes: Option<usize>,

    /// HNSW search ef parameter (default: 40).
    ///
    /// Higher values improve recall at the cost of speed.
    pub hnsw_ef_search: Option<usize>,
}

impl SearchParams {
    /// Create new search parameters.
    pub fn new() -> Self {
        Self {
            ivfflat_probes: None,
            hnsw_ef_search: None,
        }
    }

    /// Set the number of IVFFlat probes.
    pub fn probes(mut self, probes: usize) -> Self {
        self.ivfflat_probes = Some(probes);
        self
    }

    /// Set the HNSW ef_search parameter.
    pub fn ef_search(mut self, ef: usize) -> Self {
        self.hnsw_ef_search = Some(ef);
        self
    }

    /// Generate the SET commands for these parameters.
    pub fn to_set_sql(&self) -> Vec<String> {
        let mut statements = Vec::new();

        if let Some(probes) = self.ivfflat_probes {
            statements.push(format!("SET ivfflat.probes = {probes}"));
        }
        if let Some(ef) = self.hnsw_ef_search {
            statements.push(format!("SET hnsw.ef_search = {ef}"));
        }

        statements
    }

    /// Check if any parameters are set.
    pub fn has_params(&self) -> bool {
        self.ivfflat_probes.is_some() || self.hnsw_ef_search.is_some()
    }
}

impl Default for SearchParams {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distance_metric_operator() {
        assert_eq!(DistanceMetric::L2.operator(), "<->");
        assert_eq!(DistanceMetric::InnerProduct.operator(), "<#>");
        assert_eq!(DistanceMetric::Cosine.operator(), "<=>");
        assert_eq!(DistanceMetric::L1.operator(), "<+>");
    }

    #[test]
    fn test_distance_metric_ops_class() {
        assert_eq!(DistanceMetric::L2.ops_class(), "vector_l2_ops");
        assert_eq!(DistanceMetric::InnerProduct.ops_class(), "vector_ip_ops");
        assert_eq!(DistanceMetric::Cosine.ops_class(), "vector_cosine_ops");
        assert_eq!(DistanceMetric::L1.ops_class(), "vector_l1_ops");
    }

    #[test]
    fn test_distance_metric_prefers_normalized() {
        assert!(!DistanceMetric::L2.prefers_normalized());
        assert!(DistanceMetric::InnerProduct.prefers_normalized());
        assert!(DistanceMetric::Cosine.prefers_normalized());
        assert!(!DistanceMetric::L1.prefers_normalized());
    }

    #[test]
    fn test_binary_distance_metric_operator() {
        assert_eq!(BinaryDistanceMetric::Hamming.operator(), "<~>");
        assert_eq!(BinaryDistanceMetric::Jaccard.operator(), "<%>");
    }

    #[test]
    fn test_binary_distance_metric_ops_class() {
        assert_eq!(BinaryDistanceMetric::Hamming.ops_class(), "bit_hamming_ops");
        assert_eq!(BinaryDistanceMetric::Jaccard.ops_class(), "bit_jaccard_ops");
    }

    #[test]
    fn test_distance_sql() {
        let query = Embedding::new(vec![0.1, 0.2, 0.3]);
        let sql = distance_sql("embedding", &query, DistanceMetric::Cosine);
        assert!(sql.contains("<=>"));
        assert!(sql.contains("::vector"));
    }

    #[test]
    fn test_distance_param_sql() {
        let sql = distance_param_sql("embedding", "$1", DistanceMetric::L2);
        assert_eq!(sql, "embedding <-> $1");
    }

    #[test]
    fn test_nearest_neighbor_sql() {
        let sql =
            nearest_neighbor_sql("documents", "embedding", DistanceMetric::Cosine, 1, 10, &[]);
        assert!(sql.contains("SELECT *"));
        assert!(sql.contains("<=>"));
        assert!(sql.contains("$1"));
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("AS distance"));
        assert!(sql.contains("ORDER BY distance"));
    }

    #[test]
    fn test_radius_search_sql() {
        let sql = radius_search_sql(
            "documents",
            "embedding",
            DistanceMetric::L2,
            1,
            0.5,
            Some(100),
        );
        assert!(sql.contains("<->"));
        assert!(sql.contains("< 0.5"));
        assert!(sql.contains("LIMIT 100"));
    }

    #[test]
    fn test_radius_search_sql_no_limit() {
        let sql = radius_search_sql("documents", "embedding", DistanceMetric::L2, 1, 1.0, None);
        assert!(!sql.contains("LIMIT"));
    }

    #[test]
    fn test_search_params_probes() {
        let params = SearchParams::new().probes(10);
        let sql = params.to_set_sql();
        assert_eq!(sql.len(), 1);
        assert_eq!(sql[0], "SET ivfflat.probes = 10");
    }

    #[test]
    fn test_search_params_ef_search() {
        let params = SearchParams::new().ef_search(200);
        let sql = params.to_set_sql();
        assert_eq!(sql.len(), 1);
        assert_eq!(sql[0], "SET hnsw.ef_search = 200");
    }

    #[test]
    fn test_search_params_both() {
        let params = SearchParams::new().probes(10).ef_search(200);
        let sql = params.to_set_sql();
        assert_eq!(sql.len(), 2);
        assert!(params.has_params());
    }

    #[test]
    fn test_search_params_empty() {
        let params = SearchParams::new();
        assert!(!params.has_params());
        assert!(params.to_set_sql().is_empty());
    }

    #[test]
    fn test_distance_metric_display() {
        assert_eq!(format!("{}", DistanceMetric::L2), "euclidean");
        assert_eq!(format!("{}", DistanceMetric::Cosine), "cosine");
    }
}
