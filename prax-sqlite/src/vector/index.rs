//! Builder for CREATE VIRTUAL TABLE ... USING vector(...) DDL.

use crate::vector::metric::{DistanceMetric, VectorElementType, VectorIndexKind};

/// A single column in a vector virtual table.
#[derive(Debug, Clone)]
pub struct VectorColumnDef {
    name: String,
    element_type: VectorElementType,
    dimensions: u32,
    metric: DistanceMetric,
    index: Option<VectorIndexKind>,
}

/// Builder for CREATE VIRTUAL TABLE DDL for sqlite-vector-rs.
#[derive(Debug, Clone)]
pub struct VectorIndex {
    table_name: String,
    rowid_column: Option<String>,
    columns: Vec<VectorColumnDef>,
}

impl VectorIndex {
    /// Create a new builder for a virtual table.
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            table_name: table_name.into(),
            rowid_column: None,
            columns: Vec::new(),
        }
    }

    /// Set the rowid column name (for joins back to the main table).
    pub fn rowid_column(mut self, name: impl Into<String>) -> Self {
        self.rowid_column = Some(name.into());
        self
    }

    /// Add a column with a specific element type, dimensions, metric, and
    /// optional index kind.
    pub fn column(
        mut self,
        name: impl Into<String>,
        element_type: VectorElementType,
        dimensions: u32,
        metric: DistanceMetric,
        index: Option<VectorIndexKind>,
    ) -> Self {
        self.columns.push(VectorColumnDef {
            name: name.into(),
            element_type,
            dimensions,
            metric,
            index,
        });
        self
    }

    /// Render the CREATE VIRTUAL TABLE statement.
    pub fn to_create_sql(&self) -> String {
        let mut clauses: Vec<String> = Vec::new();

        if let Some(rowid) = &self.rowid_column {
            clauses.push(format!("rowid_column='{}'", rowid));
        }

        for col in &self.columns {
            let index_part = match col.index {
                Some(k) => format!(" {}", k.as_sql()),
                None => String::new(),
            };
            clauses.push(format!(
                "{}='{}[{}] {}{}'",
                col.name,
                col.element_type.as_sql(),
                col.dimensions,
                col.metric.as_sql(),
                index_part
            ));
        }

        format!(
            "CREATE VIRTUAL TABLE \"{}\" USING vector(\n    {}\n);",
            self.table_name,
            clauses.join(",\n    ")
        )
    }

    /// Render a DROP statement for this virtual table.
    pub fn to_drop_sql(&self) -> String {
        format!("DROP TABLE IF EXISTS \"{}\";", self.table_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_column_with_rowid_and_hnsw() {
        let sql = VectorIndex::new("documents_vectors")
            .rowid_column("document_id")
            .column(
                "embedding",
                VectorElementType::Float4,
                1536,
                DistanceMetric::Cosine,
                Some(VectorIndexKind::Hnsw),
            )
            .to_create_sql();

        assert!(sql.contains("CREATE VIRTUAL TABLE \"documents_vectors\" USING vector("));
        assert!(sql.contains("rowid_column='document_id'"));
        assert!(sql.contains("embedding='float4[1536] cosine hnsw'"));
    }

    #[test]
    fn test_column_without_index() {
        let sql = VectorIndex::new("vec_tbl")
            .column(
                "v",
                VectorElementType::Float8,
                128,
                DistanceMetric::L2,
                None,
            )
            .to_create_sql();
        assert!(sql.contains("v='float8[128] l2'"));
        assert!(!sql.contains(" hnsw"));
    }

    #[test]
    fn test_multiple_columns() {
        let sql = VectorIndex::new("multi")
            .rowid_column("id")
            .column(
                "a",
                VectorElementType::Float4,
                4,
                DistanceMetric::Cosine,
                Some(VectorIndexKind::Hnsw),
            )
            .column(
                "b",
                VectorElementType::Int1,
                8,
                DistanceMetric::InnerProduct,
                None,
            )
            .to_create_sql();
        assert!(sql.contains("a='float4[4] cosine hnsw'"));
        assert!(sql.contains("b='int1[8] inner'"));
    }

    #[test]
    fn test_drop_sql() {
        let idx = VectorIndex::new("documents_vectors");
        assert_eq!(
            idx.to_drop_sql(),
            "DROP TABLE IF EXISTS \"documents_vectors\";"
        );
    }
}
