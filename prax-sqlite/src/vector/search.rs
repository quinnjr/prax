//! Fluent builder for top-k vector similarity search queries.
//!
//! Renders a sqlite-vector-rs KNN query against the vector virtual table
//! (`WHERE knn_match(distance, <query>) LIMIT k`) and joins the matches back
//! to the main table on its primary key (the virtual table's `rowid`):
//!
//! ```sql
//! SELECT "documents".*, v.distance AS distance
//! FROM (SELECT rowid AS match_id, distance FROM "documents_embedding_vectors"
//!       WHERE knn_match(distance, vector_from_json('[...]', 'float4')) LIMIT 10) v
//! JOIN "documents" ON "documents"."id" = v.match_id
//! ORDER BY v.distance ASC
//! ```
//!
//! The distance metric is fixed per virtual table (see
//! [`VectorIndex`](crate::vector::VectorIndex)), so it isn't set here.

use crate::vector::metric::VectorElementType;

/// Builder for a vector similarity search.
#[derive(Debug, Clone)]
pub struct VectorSearchBuilder {
    main_table: String,
    main_id_column: String,
    vector_table: String,
    element_type: VectorElementType,
    query_json: Option<String>,
    limit: Option<u32>,
}

impl VectorSearchBuilder {
    /// Create a new builder for the vector field `vector_column` of
    /// `main_table`. Defaults:
    /// - `vector_table` = `{main_table}_{vector_column}_vectors`
    ///   ([`vector_table_name`](crate::vector::vector_table_name));
    /// - `main_id_column` = `id` (the integer primary key the vectors'
    ///   `rowid`s refer to).
    pub fn new(main_table: impl Into<String>, vector_column: impl Into<String>) -> Self {
        let main = main_table.into();
        let vector_table = crate::vector::index::vector_table_name(&main, &vector_column.into());
        Self {
            main_table: main,
            main_id_column: "id".to_string(),
            vector_table,
            element_type: VectorElementType::Float4,
            query_json: None,
            limit: None,
        }
    }

    /// Override the virtual table name.
    pub fn vector_table(mut self, name: impl Into<String>) -> Self {
        self.vector_table = name.into();
        self
    }

    /// Override the primary-key column on the main table (default `id`).
    pub fn main_id_column(mut self, name: impl Into<String>) -> Self {
        self.main_id_column = name.into();
        self
    }

    /// Element type of the query vector; must match the virtual table's
    /// `type` (default Float4).
    pub fn element_type(mut self, t: VectorElementType) -> Self {
        self.element_type = t;
        self
    }

    /// Supply the query vector as a pre-serialized JSON array string.
    pub fn query_json(mut self, json: impl Into<String>) -> Self {
        self.query_json = Some(json.into());
        self
    }

    /// Supply a query embedding (Float4).
    pub fn query_embedding(mut self, embedding: &crate::vector::types::Embedding) -> Self {
        self.element_type = VectorElementType::Float4;
        self.query_json = Some(embedding.to_json());
        self
    }

    /// Number of nearest neighbours to return (top-k). Without it,
    /// sqlite-vector-rs returns up to 100 matches.
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n);
        self
    }

    /// Render the full SELECT statement.
    ///
    /// Returns [`VectorError::BuilderIncomplete`](crate::vector::VectorError)
    /// if `query_json` / `query_embedding` was not set. Identifiers are
    /// quoted and the query vector is escaped as a string literal.
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
            Some(n) => format!(" LIMIT {n}"),
            None => String::new(),
        };

        Ok(format!(
            "SELECT {main}.*, v.distance AS distance\n\
             FROM (SELECT rowid AS match_id, distance FROM {vtable} \
             WHERE knn_match(distance, vector_from_json('{q}', '{et}')){limit}) v\n\
             JOIN {main} ON {main}.{main_id} = v.match_id\n\
             ORDER BY v.distance ASC",
            main = quote_ident(&self.main_table),
            main_id = quote_ident(&self.main_id_column),
            q = escape_sql_literal(q),
            et = self.element_type.as_sql(),
            vtable = quote_ident(&self.vector_table),
            limit = limit_clause,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::types::Embedding;

    #[test]
    fn test_default_table_naming_and_knn_shape() {
        let sql = VectorSearchBuilder::new("documents", "embedding")
            .query_json("[0.1,0.2,0.3]")
            .limit(10)
            .to_sql()
            .unwrap();

        assert!(sql.contains("FROM \"documents_embedding_vectors\" WHERE knn_match(distance, vector_from_json('[0.1,0.2,0.3]', 'float4')) LIMIT 10) v"));
        assert!(sql.contains("JOIN \"documents\" ON \"documents\".\"id\" = v.match_id"));
        assert!(sql.ends_with("ORDER BY v.distance ASC"));
        assert!(!sql.contains("vector_distance"));
    }

    #[test]
    fn test_no_limit_leaves_extension_default() {
        let sql = VectorSearchBuilder::new("d", "e")
            .query_json("[1]")
            .to_sql()
            .unwrap();
        assert!(sql.contains("'float4'))) v"));
    }

    #[test]
    fn test_custom_vector_table() {
        let sql = VectorSearchBuilder::new("docs", "emb")
            .vector_table("docs_vec_tbl")
            .query_json("[0.1]")
            .to_sql()
            .unwrap();
        assert!(sql.contains("FROM \"docs_vec_tbl\" WHERE"));
    }

    #[test]
    fn test_element_type_in_sql() {
        let sql = VectorSearchBuilder::new("docs", "e")
            .element_type(VectorElementType::Float8)
            .query_json("[1.0]")
            .to_sql()
            .unwrap();
        assert!(sql.contains("'float8'"));
    }

    #[test]
    fn test_query_embedding_uses_float4() {
        let emb = Embedding::new(vec![0.5, 1.5]).unwrap();
        let sql = VectorSearchBuilder::new("items", "embedding")
            .query_embedding(&emb)
            .to_sql()
            .unwrap();
        assert!(sql.contains("'[0.5,1.5]', 'float4'"));
    }

    #[test]
    fn test_query_json_quote_is_escaped() {
        let sql = VectorSearchBuilder::new("d", "e")
            .query_json("[1]') --")
            .to_sql()
            .unwrap();
        assert!(sql.contains("'[1]'') --'"));
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
        assert!(sql.contains("\"tbl\"\"evil\""));
    }

    #[test]
    fn test_main_id_column_override() {
        let sql = VectorSearchBuilder::new("documents", "embedding")
            .main_id_column("doc_uid")
            .query_json("[0.1]")
            .to_sql()
            .unwrap();
        assert!(sql.contains("ON \"documents\".\"doc_uid\" = v.match_id"));
        assert!(!sql.contains("\"documents\".\"id\""));
    }
}
