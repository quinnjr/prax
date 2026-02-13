//! Vector filter operations for integration with the prax query builder.
//!
//! This module provides filter types that can be used with prax-query's
//! filter system to perform vector similarity searches as part of WHERE clauses.
//!
//! # Examples
//!
//! ```rust
//! use prax_pgvector::filter::{VectorFilter, VectorOrderBy};
//! use prax_pgvector::{Embedding, DistanceMetric};
//!
//! // Create a nearest-neighbor filter
//! let query_vec = Embedding::new(vec![0.1, 0.2, 0.3]);
//! let filter = VectorFilter::nearest("embedding", query_vec, DistanceMetric::Cosine, 10);
//!
//! // Create a distance-filtered search
//! let query_vec = Embedding::new(vec![0.1, 0.2, 0.3]);
//! let filter = VectorFilter::within_distance("embedding", query_vec, DistanceMetric::L2, 0.5);
//! ```

use serde::{Deserialize, Serialize};

use crate::ops::DistanceMetric;
use crate::types::Embedding;

/// A vector filter operation for use in WHERE and ORDER BY clauses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorFilter {
    /// Column containing the vector.
    pub column: String,
    /// Query vector to compare against.
    pub query_vector: Embedding,
    /// Distance metric to use.
    pub metric: DistanceMetric,
    /// Type of vector filter.
    pub filter_type: VectorFilterType,
}

/// The type of vector filter operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum VectorFilterType {
    /// K-nearest neighbor search (ORDER BY distance LIMIT k).
    Nearest {
        /// Maximum number of results to return.
        limit: usize,
    },

    /// Distance-based filter (WHERE distance < threshold).
    WithinDistance {
        /// Maximum distance threshold.
        max_distance: f64,
        /// Optional result limit.
        limit: Option<usize>,
    },

    /// Distance range filter (WHERE distance BETWEEN min AND max).
    DistanceRange {
        /// Minimum distance.
        min_distance: f64,
        /// Maximum distance.
        max_distance: f64,
        /// Optional result limit.
        limit: Option<usize>,
    },
}

impl VectorFilter {
    /// Create a k-nearest neighbor filter.
    ///
    /// This generates an ORDER BY with the vector distance operator and LIMIT.
    pub fn nearest(
        column: impl Into<String>,
        query_vector: Embedding,
        metric: DistanceMetric,
        limit: usize,
    ) -> Self {
        Self {
            column: column.into(),
            query_vector,
            metric,
            filter_type: VectorFilterType::Nearest { limit },
        }
    }

    /// Create a distance-based filter.
    ///
    /// This generates a WHERE clause filtering by maximum distance.
    pub fn within_distance(
        column: impl Into<String>,
        query_vector: Embedding,
        metric: DistanceMetric,
        max_distance: f64,
    ) -> Self {
        Self {
            column: column.into(),
            query_vector,
            metric,
            filter_type: VectorFilterType::WithinDistance {
                max_distance,
                limit: None,
            },
        }
    }

    /// Create a distance range filter.
    pub fn distance_range(
        column: impl Into<String>,
        query_vector: Embedding,
        metric: DistanceMetric,
        min_distance: f64,
        max_distance: f64,
    ) -> Self {
        Self {
            column: column.into(),
            query_vector,
            metric,
            filter_type: VectorFilterType::DistanceRange {
                min_distance,
                max_distance,
                limit: None,
            },
        }
    }

    /// Add a limit to this filter.
    pub fn with_limit(mut self, limit: usize) -> Self {
        match &mut self.filter_type {
            VectorFilterType::Nearest { limit: l } => *l = limit,
            VectorFilterType::WithinDistance { limit: l, .. } => *l = Some(limit),
            VectorFilterType::DistanceRange { limit: l, .. } => *l = Some(limit),
        }
        self
    }

    /// Generate the distance expression SQL fragment.
    ///
    /// Returns something like: `embedding <=> $1`
    pub fn distance_expr_sql(&self, param_index: usize) -> String {
        format!(
            "{} {} ${}",
            self.column,
            self.metric.operator(),
            param_index
        )
    }

    /// Generate the WHERE clause SQL fragment.
    ///
    /// Returns `None` for nearest-neighbor searches (which only use ORDER BY).
    pub fn where_sql(&self, param_index: usize) -> Option<String> {
        let distance_expr = self.distance_expr_sql(param_index);

        match &self.filter_type {
            VectorFilterType::Nearest { .. } => None,
            VectorFilterType::WithinDistance { max_distance, .. } => {
                Some(format!("{distance_expr} < {max_distance}"))
            }
            VectorFilterType::DistanceRange {
                min_distance,
                max_distance,
                ..
            } => Some(format!(
                "{distance_expr} BETWEEN {min_distance} AND {max_distance}"
            )),
        }
    }

    /// Generate the ORDER BY clause SQL fragment.
    pub fn order_by_sql(&self, param_index: usize) -> String {
        self.distance_expr_sql(param_index)
    }

    /// Generate the LIMIT clause.
    pub fn limit_sql(&self) -> Option<String> {
        let limit = match &self.filter_type {
            VectorFilterType::Nearest { limit } => Some(*limit),
            VectorFilterType::WithinDistance { limit, .. } => *limit,
            VectorFilterType::DistanceRange { limit, .. } => *limit,
        };

        limit.map(|l| format!("LIMIT {l}"))
    }

    /// Generate the complete SELECT query incorporating this vector filter.
    ///
    /// This produces a query like:
    /// ```sql
    /// SELECT *, embedding <=> $1 AS distance
    /// FROM documents
    /// WHERE embedding <=> $1 < 0.5
    /// ORDER BY distance
    /// LIMIT 10
    /// ```
    pub fn to_select_sql(
        &self,
        table: &str,
        param_index: usize,
        extra_where: Option<&str>,
        select_columns: &str,
    ) -> String {
        let distance_expr = self.distance_expr_sql(param_index);

        let mut sql = format!(
            "SELECT {}, {} AS distance FROM {}",
            select_columns, distance_expr, table
        );

        // WHERE clause
        let mut where_parts = Vec::new();
        if let Some(vec_where) = self.where_sql(param_index) {
            where_parts.push(vec_where);
        }
        if let Some(extra) = extra_where {
            where_parts.push(extra.to_string());
        }
        if !where_parts.is_empty() {
            sql.push_str(&format!(" WHERE {}", where_parts.join(" AND ")));
        }

        // ORDER BY
        sql.push_str(&format!(" ORDER BY {}", self.order_by_sql(param_index)));

        // LIMIT
        if let Some(limit) = self.limit_sql() {
            sql.push_str(&format!(" {limit}"));
        }

        sql
    }
}

/// Vector ordering specification for use with query builders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorOrderBy {
    /// Column containing the vector.
    pub column: String,
    /// Query vector to compare against.
    pub query_vector: Embedding,
    /// Distance metric.
    pub metric: DistanceMetric,
    /// Whether to include the distance as a result column.
    pub include_distance: bool,
    /// Alias for the distance column.
    pub distance_alias: String,
}

impl VectorOrderBy {
    /// Create a new vector ordering.
    pub fn new(column: impl Into<String>, query_vector: Embedding, metric: DistanceMetric) -> Self {
        Self {
            column: column.into(),
            query_vector,
            metric,
            include_distance: true,
            distance_alias: "distance".to_string(),
        }
    }

    /// Set the distance column alias.
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.distance_alias = alias.into();
        self
    }

    /// Don't include the distance as a result column.
    pub fn without_distance(mut self) -> Self {
        self.include_distance = false;
        self
    }

    /// Generate the SELECT addition for the distance column.
    pub fn select_distance_sql(&self, param_index: usize) -> Option<String> {
        if self.include_distance {
            Some(format!(
                "{} {} ${} AS {}",
                self.column,
                self.metric.operator(),
                param_index,
                self.distance_alias
            ))
        } else {
            None
        }
    }

    /// Generate the ORDER BY clause.
    pub fn order_by_sql(&self, param_index: usize) -> String {
        if self.include_distance {
            self.distance_alias.clone()
        } else {
            format!(
                "{} {} ${}",
                self.column,
                self.metric.operator(),
                param_index
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_embedding() -> Embedding {
        Embedding::new(vec![0.1, 0.2, 0.3])
    }

    #[test]
    fn test_nearest_filter() {
        let filter =
            VectorFilter::nearest("embedding", test_embedding(), DistanceMetric::Cosine, 10);
        assert!(filter.where_sql(1).is_none());
        assert_eq!(filter.order_by_sql(1), "embedding <=> $1");
        assert_eq!(filter.limit_sql(), Some("LIMIT 10".to_string()));
    }

    #[test]
    fn test_within_distance_filter() {
        let filter =
            VectorFilter::within_distance("embedding", test_embedding(), DistanceMetric::L2, 0.5);
        let where_sql = filter.where_sql(1).unwrap();
        assert!(where_sql.contains("<->"));
        assert!(where_sql.contains("< 0.5"));
    }

    #[test]
    fn test_distance_range_filter() {
        let filter = VectorFilter::distance_range(
            "embedding",
            test_embedding(),
            DistanceMetric::L2,
            0.1,
            0.5,
        );
        let where_sql = filter.where_sql(1).unwrap();
        assert!(where_sql.contains("BETWEEN"));
        assert!(where_sql.contains("0.1"));
        assert!(where_sql.contains("0.5"));
    }

    #[test]
    fn test_filter_with_limit() {
        let filter =
            VectorFilter::within_distance("embedding", test_embedding(), DistanceMetric::L2, 0.5)
                .with_limit(50);

        assert_eq!(filter.limit_sql(), Some("LIMIT 50".to_string()));
    }

    #[test]
    fn test_to_select_sql_nearest() {
        let filter =
            VectorFilter::nearest("embedding", test_embedding(), DistanceMetric::Cosine, 5);
        let sql = filter.to_select_sql("documents", 1, None, "*");

        assert!(sql.contains("SELECT *, embedding <=> $1 AS distance"));
        assert!(sql.contains("FROM documents"));
        assert!(sql.contains("ORDER BY"));
        assert!(sql.contains("LIMIT 5"));
        assert!(!sql.contains("WHERE")); // No WHERE for nearest
    }

    #[test]
    fn test_to_select_sql_with_extra_where() {
        let filter =
            VectorFilter::within_distance("embedding", test_embedding(), DistanceMetric::L2, 0.5)
                .with_limit(20);

        let sql = filter.to_select_sql("documents", 1, Some("category = 'tech'"), "*");
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("< 0.5"));
        assert!(sql.contains("category = 'tech'"));
        assert!(sql.contains("AND"));
    }

    #[test]
    fn test_vector_order_by() {
        let order = VectorOrderBy::new("embedding", test_embedding(), DistanceMetric::Cosine);
        assert!(order.include_distance);

        let select = order.select_distance_sql(1).unwrap();
        assert!(select.contains("<=>"));
        assert!(select.contains("AS distance"));

        let order_by = order.order_by_sql(1);
        assert_eq!(order_by, "distance");
    }

    #[test]
    fn test_vector_order_by_without_distance() {
        let order = VectorOrderBy::new("embedding", test_embedding(), DistanceMetric::L2)
            .without_distance();

        assert!(order.select_distance_sql(1).is_none());
        let order_by = order.order_by_sql(1);
        assert!(order_by.contains("<->"));
    }

    #[test]
    fn test_vector_order_by_custom_alias() {
        let order = VectorOrderBy::new("embedding", test_embedding(), DistanceMetric::Cosine)
            .alias("similarity");

        let select = order.select_distance_sql(1).unwrap();
        assert!(select.contains("AS similarity"));
    }

    #[test]
    fn test_distance_expr_sql() {
        let filter =
            VectorFilter::nearest("emb", test_embedding(), DistanceMetric::InnerProduct, 5);
        let expr = filter.distance_expr_sql(2);
        assert_eq!(expr, "emb <#> $2");
    }
}
