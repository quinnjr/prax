//! Fluent builder for top-k vector similarity search queries.

use crate::vector::metric::{DistanceMetric, VectorElementType};

/// Builder for a vector similarity search.
#[derive(Debug, Clone)]
pub struct VectorSearchBuilder {
    main_table: String,
    vector_table: String,
    rowid_column: String,
    vector_column: String,
    element_type: VectorElementType,
    metric: DistanceMetric,
    query_json: Option<String>,
    limit: Option<u32>,
}

impl VectorSearchBuilder {
    /// Create a new builder. The vector_table defaults to `{main_table}_vectors`
    /// and the rowid column defaults to `{main_table}_id` in singular form.
    pub fn new(main_table: impl Into<String>, vector_column: impl Into<String>) -> Self {
        let main = main_table.into();
        let vector_table = format!("{}_vectors", main);
        let rowid = format!("{}_id", singularize(&main));
        Self {
            main_table: main,
            vector_table,
            rowid_column: rowid,
            vector_column: vector_column.into(),
            element_type: VectorElementType::Float4,
            metric: DistanceMetric::Cosine,
            query_json: None,
            limit: None,
        }
    }

    /// Override the virtual table name (default `{main_table}_vectors`).
    pub fn vector_table(mut self, name: impl Into<String>) -> Self {
        self.vector_table = name.into();
        self
    }

    /// Override the rowid column (default `{singular(main)}_id`).
    pub fn rowid_column(mut self, name: impl Into<String>) -> Self {
        self.rowid_column = name.into();
        self
    }

    /// Set the element type (default Float4).
    pub fn element_type(mut self, t: VectorElementType) -> Self {
        self.element_type = t;
        self
    }

    /// Set the distance metric (default Cosine).
    pub fn metric(mut self, m: DistanceMetric) -> Self {
        self.metric = m;
        self
    }

    /// Supply the query vector as a pre-serialized JSON array string.
    pub fn query_json(mut self, json: impl Into<String>) -> Self {
        self.query_json = Some(json.into());
        self
    }

    /// Supply a query embedding (uses its element type + to_json).
    pub fn query_embedding(mut self, embedding: &crate::vector::types::Embedding) -> Self {
        self.element_type = VectorElementType::Float4;
        self.query_json = Some(embedding.to_json());
        self
    }

    /// Set the result limit (top-k).
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n);
        self
    }

    /// Render the full SELECT statement.
    pub fn to_sql(&self) -> String {
        let query = self.query_json.clone().unwrap_or_else(|| "?".to_string());
        let limit_clause = match self.limit {
            Some(n) => format!("\nLIMIT {}", n),
            None => String::new(),
        };

        format!(
            "SELECT \"{main}\".*, \
             vector_distance(v.\"{vector_column}\", vector_from_json('{q}', '{et}'), '{metric}', '{et}') AS distance\n\
             FROM \"{vtable}\" v\n\
             JOIN \"{main}\" ON \"{main}\".\"id\" = v.\"{rowid}\"\n\
             ORDER BY distance{limit}",
            main = self.main_table,
            vector_column = self.vector_column,
            q = query,
            et = self.element_type.as_sql(),
            metric = self.metric.as_sql(),
            vtable = self.vector_table,
            rowid = self.rowid_column,
            limit = limit_clause,
        )
    }
}

// NOTE: Keep this in sync with the singularize helper in
// prax-migrate/src/sql.rs (SqliteGenerator::singularize). The search-time
// default rowid column name must match the migration-time name, otherwise
// VectorSearchBuilder's default JOIN will point at a column that does not
// exist. Users with irregular plurals should call .rowid_column() manually
// on both the migration-side schema and this builder.
fn singularize(name: &str) -> String {
    if name.ends_with('s') && !name.ends_with("ss") {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::types::Embedding;

    #[test]
    fn test_default_table_and_rowid_naming() {
        let sql = VectorSearchBuilder::new("documents", "embedding")
            .query_json("[0.1,0.2,0.3]")
            .limit(10)
            .to_sql();

        assert!(sql.contains("FROM \"documents_vectors\" v"));
        assert!(sql.contains("JOIN \"documents\" ON \"documents\".\"id\" = v.\"document_id\""));
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("ORDER BY distance"));
    }

    #[test]
    fn test_custom_vector_table_and_rowid() {
        let sql = VectorSearchBuilder::new("docs", "emb")
            .vector_table("docs_vec_tbl")
            .rowid_column("doc_ref")
            .query_json("[0.1]")
            .to_sql();
        assert!(sql.contains("FROM \"docs_vec_tbl\" v"));
        assert!(sql.contains("v.\"doc_ref\""));
    }

    #[test]
    fn test_metric_and_element_type_in_sql() {
        let sql = VectorSearchBuilder::new("docs", "e")
            .metric(DistanceMetric::L2)
            .element_type(VectorElementType::Float8)
            .query_json("[1.0]")
            .to_sql();
        assert!(sql.contains("'l2'"));
        assert!(sql.contains("'float8'"));
    }

    #[test]
    fn test_query_embedding_uses_float4() {
        let emb = Embedding::new(vec![0.5, 1.5]).unwrap();
        let sql = VectorSearchBuilder::new("items", "embedding")
            .query_embedding(&emb)
            .to_sql();
        assert!(sql.contains("'float4'"));
        assert!(sql.contains("[0.5,1.5]"));
    }
}
