//! Fluent builder for top-k vector similarity search queries.

use crate::vector::metric::{DistanceMetric, VectorElementType};

/// Builder for a vector similarity search.
#[derive(Debug, Clone)]
pub struct VectorSearchBuilder {
    main_table: String,
    main_id_column: String,
    vector_table: String,
    rowid_column: String,
    vector_column: String,
    element_type: VectorElementType,
    metric: DistanceMetric,
    query_json: Option<String>,
    limit: Option<u32>,
}

impl VectorSearchBuilder {
    /// Create a new builder. Defaults:
    /// - `vector_table` = `{main_table}_vectors`
    /// - `rowid_column` = `{singular(main_table)}_id`
    /// - `main_id_column` = `id` (the primary key on `main_table`)
    ///
    /// Override any of these via the corresponding setter when the schema
    /// uses different conventions.
    pub fn new(main_table: impl Into<String>, vector_column: impl Into<String>) -> Self {
        let main = main_table.into();
        let vector_table = format!("{}_vectors", main);
        let rowid = format!("{}_id", singularize(&main));
        Self {
            main_table: main,
            main_id_column: "id".to_string(),
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

    /// Override the rowid column on the virtual table
    /// (default `{singular(main)}_id`).
    pub fn rowid_column(mut self, name: impl Into<String>) -> Self {
        self.rowid_column = name.into();
        self
    }

    /// Override the primary-key column name on the main table (default
    /// `id`). Use this when your main table's PK is not called `id`.
    pub fn main_id_column(mut self, name: impl Into<String>) -> Self {
        self.main_id_column = name.into();
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
    ///
    /// Returns [`VectorError::BuilderIncomplete`] if `query_json` /
    /// `query_embedding` was not set. All identifiers are safely quoted.
    pub fn to_sql(&self) -> crate::vector::error::VectorResult<String> {
        use crate::vector::error::VectorError;
        use crate::vector::{escape_sql_literal, quote_ident};

        let q = self
            .query_json
            .as_deref()
            .ok_or(VectorError::BuilderIncomplete {
                field: "query_json",
            })?;

        let limit_clause = match self.limit {
            Some(n) => format!("\nLIMIT {}", n),
            None => String::new(),
        };

        Ok(format!(
            "SELECT {main}.*, \
             vector_distance(v.{vector_column}, vector_from_json('{q}', '{et}'), '{metric}', '{et}') AS distance\n\
             FROM {vtable} v\n\
             JOIN {main} ON {main}.{main_id} = v.{rowid}\n\
             ORDER BY distance ASC{limit}",
            main = quote_ident(&self.main_table),
            main_id = quote_ident(&self.main_id_column),
            vector_column = quote_ident(&self.vector_column),
            q = escape_sql_literal(q),
            et = self.element_type.as_sql(),
            metric = self.metric.as_sql(),
            vtable = quote_ident(&self.vector_table),
            rowid = quote_ident(&self.rowid_column),
            limit = limit_clause,
        ))
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
            .to_sql()
            .unwrap();

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
            .to_sql()
            .unwrap();
        assert!(sql.contains("FROM \"docs_vec_tbl\" v"));
        assert!(sql.contains("v.\"doc_ref\""));
    }

    #[test]
    fn test_metric_and_element_type_in_sql() {
        let sql = VectorSearchBuilder::new("docs", "e")
            .metric(DistanceMetric::L2)
            .element_type(VectorElementType::Float8)
            .query_json("[1.0]")
            .to_sql()
            .unwrap();
        assert!(sql.contains("'l2'"));
        assert!(sql.contains("'float8'"));
    }

    #[test]
    fn test_query_embedding_uses_float4() {
        let emb = Embedding::new(vec![0.5, 1.5]).unwrap();
        let sql = VectorSearchBuilder::new("items", "embedding")
            .query_embedding(&emb)
            .to_sql()
            .unwrap();
        assert!(sql.contains("'float4'"));
        assert!(sql.contains("[0.5,1.5]"));
    }

    #[test]
    fn test_missing_query_returns_error() {
        let result = VectorSearchBuilder::new("documents", "embedding").to_sql();
        match result {
            Err(crate::vector::error::VectorError::BuilderIncomplete { field }) => {
                assert_eq!(field, "query_json");
            }
            other => panic!("expected BuilderIncomplete error, got {:?}", other),
        }
    }

    #[test]
    fn test_identifier_with_embedded_double_quote_is_escaped() {
        let sql = VectorSearchBuilder::new("tbl\"evil", "emb")
            .query_json("[0.1]")
            .to_sql()
            .unwrap();
        // Embedded " in the table name should be doubled inside the SQL ident.
        assert!(sql.contains("\"tbl\"\"evil\""));
    }

    #[test]
    fn test_main_id_column_override() {
        // Schemas whose primary key isn't called "id" must be able to
        // override the JOIN column, otherwise the generated query would
        // fail with "no such column: table.id" at runtime.
        let sql = VectorSearchBuilder::new("documents", "embedding")
            .main_id_column("doc_uid")
            .query_json("[0.1]")
            .to_sql()
            .unwrap();
        assert!(sql.contains("ON \"documents\".\"doc_uid\" = v.\"document_id\""));
        assert!(!sql.contains("\"documents\".\"id\""));
    }
}
