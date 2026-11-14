//! Builder for `CREATE VIRTUAL TABLE ... USING vector(...)` DDL.
//!
//! sqlite-vector-rs stores **one vector per row** of a virtual table:
//!
//! ```sql
//! CREATE VIRTUAL TABLE "documents_embedding_vectors" USING vector(dim=1536, type=float4, metric=cosine, mode=hnsw);
//! ```
//!
//! The table's `rowid` is the link back to the owning row: insert each
//! vector with the owning row's integer primary key as its `rowid`
//! (`INSERT INTO t(rowid, vector) VALUES (?1, vector_from_json(?2, 'float4'))`).
//! A model with several vector fields uses one virtual table per field
//! (`{table}_{column}_vectors`, see [`vector_table_name`]).

use crate::vector::metric::{DistanceMetric, VectorElementType, VectorIndexKind};

/// Conventional virtual-table name for the vector field `column` of
/// `table`: `{table}_{column}_vectors`. The migration generator uses the
/// same convention.
pub fn vector_table_name(table: &str, column: &str) -> String {
    format!("{table}_{column}_vectors")
}

/// Builder for one sqlite-vector-rs virtual table (one vector per row).
#[derive(Debug, Clone)]
pub struct VectorIndex {
    table_name: String,
    element_type: VectorElementType,
    dimensions: u32,
    metric: DistanceMetric,
    index: Option<VectorIndexKind>,
}

impl VectorIndex {
    /// Start a builder for the virtual table `table_name` holding vectors of
    /// `dimensions` elements of `element_type`, compared with `metric`.
    ///
    /// The search mode defaults to an HNSW index; see [`index`](Self::index).
    pub fn new(
        table_name: impl Into<String>,
        element_type: VectorElementType,
        dimensions: u32,
        metric: DistanceMetric,
    ) -> Self {
        Self {
            table_name: table_name.into(),
            element_type,
            dimensions,
            metric,
            index: Some(VectorIndexKind::Hnsw),
        }
    }

    /// Choose the search mode: `Some(VectorIndexKind::Hnsw)` (the default)
    /// for an approximate HNSW index, `None` for exact (brute-force) search.
    pub fn index(mut self, index: Option<VectorIndexKind>) -> Self {
        self.index = index;
        self
    }

    /// The virtual table name.
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    /// Render the `CREATE VIRTUAL TABLE` statement.
    ///
    /// The table name is quoted as an identifier; every module argument is
    /// a fixed keyword or a number, so no user text reaches the argument
    /// list.
    pub fn to_create_sql(&self) -> String {
        use crate::vector::quote_ident;

        let mode = match self.index {
            Some(kind) => kind.as_sql(),
            None => "exact",
        };
        format!(
            "CREATE VIRTUAL TABLE {} USING vector(dim={}, type={}, metric={}, mode={});",
            quote_ident(&self.table_name),
            self.dimensions,
            self.element_type.as_sql(),
            self.metric.as_sql(),
            mode
        )
    }

    /// Render a DROP statement for this virtual table.
    pub fn to_drop_sql(&self) -> String {
        use crate::vector::quote_ident;
        format!("DROP TABLE IF EXISTS {};", quote_ident(&self.table_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_table() {
        let sql = VectorIndex::new(
            "documents_embedding_vectors",
            VectorElementType::Float4,
            1536,
            DistanceMetric::Cosine,
        )
        .to_create_sql();
        assert_eq!(
            sql,
            "CREATE VIRTUAL TABLE \"documents_embedding_vectors\" USING vector(\
             dim=1536, type=float4, metric=cosine, mode=hnsw);"
        );
    }

    #[test]
    fn test_exact_mode_without_index() {
        let sql = VectorIndex::new(
            "vec_tbl",
            VectorElementType::Float8,
            128,
            DistanceMetric::L2,
        )
        .index(None)
        .to_create_sql();
        assert!(sql.contains("dim=128, type=float8, metric=l2, mode=exact"));
    }

    #[test]
    fn test_inner_product_metric_is_ip() {
        let sql = VectorIndex::new(
            "v",
            VectorElementType::Int1,
            8,
            DistanceMetric::InnerProduct,
        )
        .to_create_sql();
        assert!(sql.contains("type=int1, metric=ip"));
    }

    #[test]
    fn test_table_name_is_quoted() {
        let sql = VectorIndex::new(
            "evil\"name",
            VectorElementType::Float4,
            2,
            DistanceMetric::Cosine,
        )
        .to_create_sql();
        assert!(sql.contains("\"evil\"\"name\""));
    }

    #[test]
    fn test_vector_table_name_convention() {
        assert_eq!(
            vector_table_name("documents", "embedding"),
            "documents_embedding_vectors"
        );
    }

    #[test]
    fn test_drop_sql() {
        let idx = VectorIndex::new(
            "documents_embedding_vectors",
            VectorElementType::Float4,
            4,
            DistanceMetric::Cosine,
        );
        assert_eq!(
            idx.to_drop_sql(),
            "DROP TABLE IF EXISTS \"documents_embedding_vectors\";"
        );
    }
}
