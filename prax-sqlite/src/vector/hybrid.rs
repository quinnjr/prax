//! Hybrid search builder combining vector similarity with fts5 full-text search
//! via Reciprocal Rank Fusion (RRF).

use crate::vector::metric::VectorElementType;

/// Builder for a hybrid search (vector + fts5) using RRF.
///
/// The vector side is a sqlite-vector-rs KNN query
/// (`knn_match(distance, ...)`) returning the `candidates` nearest rows,
/// ranked by distance; the text side is an fts5 `MATCH` ranked by `bm25`.
/// Both are joined back to the main table on its primary key (the vector
/// table's `rowid` and the fts5 table's `rowid`).
#[derive(Debug, Clone)]
pub struct HybridSearchBuilder {
    main_table: String,
    id_column: String,
    vector_table: Option<String>,
    fts_table: Option<String>,
    element_type: VectorElementType,
    query_vector_json: Option<String>,
    query_text: Option<String>,
    vector_weight: f64,
    text_weight: f64,
    rrf_k: u32,
    candidates: u32,
    limit: Option<u32>,
}

impl HybridSearchBuilder {
    /// Start a hybrid-search builder.
    pub fn new(main_table: impl Into<String>) -> Self {
        let main = main_table.into();
        Self {
            main_table: main,
            id_column: "id".to_string(),
            vector_table: None,
            fts_table: None,
            element_type: VectorElementType::Float4,
            query_vector_json: None,
            query_text: None,
            vector_weight: 0.5,
            text_weight: 0.5,
            rrf_k: 60,
            candidates: 100,
            limit: None,
        }
    }

    /// Override the main table id column (default "id").
    pub fn id_column(mut self, name: impl Into<String>) -> Self {
        self.id_column = name.into();
        self
    }

    /// Set the vector virtual table name (see
    /// [`vector_table_name`](crate::vector::vector_table_name)).
    pub fn vector_table(mut self, name: impl Into<String>) -> Self {
        self.vector_table = Some(name.into());
        self
    }

    /// Set the fts5 virtual table name.
    pub fn fts_table(mut self, name: impl Into<String>) -> Self {
        self.fts_table = Some(name.into());
        self
    }

    /// Element type of the query vector; must match the vector table's
    /// `type` (default Float4).
    pub fn element_type(mut self, t: VectorElementType) -> Self {
        self.element_type = t;
        self
    }

    /// Set the query vector as a JSON array literal.
    pub fn query_vector_json(mut self, json: impl Into<String>) -> Self {
        self.query_vector_json = Some(json.into());
        self
    }

    /// Supply the query embedding via an Embedding value.
    pub fn query_embedding(mut self, embedding: &crate::vector::types::Embedding) -> Self {
        self.element_type = VectorElementType::Float4;
        self.query_vector_json = Some(embedding.to_json());
        self
    }

    /// Set the FTS query string.
    pub fn query_text(mut self, text: impl Into<String>) -> Self {
        self.query_text = Some(text.into());
        self
    }

    /// Set the RRF weight applied to the vector rank.
    pub fn vector_weight(mut self, w: f64) -> Self {
        self.vector_weight = w;
        self
    }

    /// Set the RRF weight applied to the text rank.
    pub fn text_weight(mut self, w: f64) -> Self {
        self.text_weight = w;
        self
    }

    /// Set the RRF k-constant (default 60).
    pub fn rrf_k(mut self, k: u32) -> Self {
        self.rrf_k = k;
        self
    }

    /// Number of nearest vector matches fed into the fusion (default 100).
    pub fn candidates(mut self, n: u32) -> Self {
        self.candidates = n;
        self
    }

    /// Set the result limit.
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n);
        self
    }

    /// Render the SELECT statement.
    ///
    /// Returns [`VectorError::BuilderIncomplete`](crate::vector::VectorError)
    /// if any of `vector_table`, `fts_table`, `query_vector_json` or
    /// `query_text` is unset. Identifiers are quoted with doubled-double-quote
    /// escaping; the query vector and FTS text are escaped as string
    /// literals.
    pub fn to_sql(&self) -> crate::vector::error::VectorResult<String> {
        use crate::vector::error::VectorError;
        use crate::vector::{escape_sql_literal, quote_ident};

        let vtable = self
            .vector_table
            .as_deref()
            .ok_or(VectorError::BuilderIncomplete {
                field: "vector_table",
            })?;
        let ftab = self
            .fts_table
            .as_deref()
            .ok_or(VectorError::BuilderIncomplete { field: "fts_table" })?;
        let qv = self
            .query_vector_json
            .as_deref()
            .ok_or(VectorError::BuilderIncomplete {
                field: "query_vector_json",
            })?;
        let qt = self
            .query_text
            .as_deref()
            .ok_or(VectorError::BuilderIncomplete {
                field: "query_text",
            })?;

        let limit_clause = match self.limit {
            Some(n) => format!("\nLIMIT {}", n),
            None => String::new(),
        };

        Ok(format!(
            "WITH vec_ranked AS (\n    \
             SELECT match_id, ROW_NUMBER() OVER (ORDER BY distance) AS rank\n    \
             FROM (SELECT rowid AS match_id, distance FROM {vtable} \
             WHERE knn_match(distance, vector_from_json('{qv}', '{et}')) LIMIT {cand})\n\
             ),\n\
             fts_ranked AS (\n    \
             SELECT rowid AS match_id, \
             ROW_NUMBER() OVER (ORDER BY bm25({ftab})) AS rank\n    \
             FROM {ftab} WHERE {ftab} MATCH '{qt}'\n\
             )\n\
             SELECT {main}.*, \
             COALESCE({vw} / ({k} + v.rank), 0) + COALESCE({tw} / ({k} + f.rank), 0) AS score\n\
             FROM {main}\n\
             LEFT JOIN vec_ranked v ON {main}.{id} = v.match_id\n\
             LEFT JOIN fts_ranked f ON {main}.{id} = f.match_id\n\
             WHERE v.match_id IS NOT NULL OR f.match_id IS NOT NULL\n\
             ORDER BY score DESC{limit}",
            qv = escape_sql_literal(qv),
            et = self.element_type.as_sql(),
            vtable = quote_ident(vtable),
            cand = self.candidates,
            ftab = quote_ident(ftab),
            qt = escape_sql_literal(qt),
            main = quote_ident(&self.main_table),
            id = quote_ident(&self.id_column),
            vw = self.vector_weight,
            tw = self.text_weight,
            k = self.rrf_k,
            limit = limit_clause,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> HybridSearchBuilder {
        HybridSearchBuilder::new("documents")
            .vector_table("documents_embedding_vectors")
            .fts_table("documents_fts")
            .query_vector_json("[0.1,0.2]")
            .query_text("machine learning")
    }

    #[test]
    fn test_default_weights_and_rrf_k() {
        let sql = base().to_sql().unwrap();
        assert!(sql.contains("COALESCE(0.5 / (60 + v.rank), 0)"));
        assert!(sql.contains("COALESCE(0.5 / (60 + f.rank), 0)"));
    }

    #[test]
    fn test_custom_weights_and_rrf_k() {
        let sql = base()
            .vector_weight(0.7)
            .text_weight(0.3)
            .rrf_k(80)
            .to_sql()
            .unwrap();
        assert!(sql.contains("COALESCE(0.7 / (80 + v.rank), 0)"));
        assert!(sql.contains("COALESCE(0.3 / (80 + f.rank), 0)"));
    }

    #[test]
    fn test_limit() {
        let sql = base().limit(25).to_sql().unwrap();
        assert!(sql.contains("LIMIT 25"));
    }

    #[test]
    fn test_fts_query_quote_escaping() {
        let sql = base().query_text("it's a test").to_sql().unwrap();
        assert!(sql.contains("MATCH 'it''s a test'"));
    }

    #[test]
    fn test_vector_table_and_fts_table_appear() {
        let sql = base().to_sql().unwrap();
        assert!(sql.contains("FROM \"documents_embedding_vectors\" WHERE knn_match(distance, vector_from_json('[0.1,0.2]', 'float4')) LIMIT 100"));
        assert!(sql.contains("FROM \"documents_fts\""));
        assert!(sql.contains("FROM \"documents\""));
    }

    #[test]
    fn test_missing_vector_table_returns_error() {
        let result = HybridSearchBuilder::new("docs")
            .fts_table("docs_fts")
            .query_vector_json("[0.1]")
            .query_text("q")
            .to_sql();
        match result {
            Err(crate::vector::error::VectorError::BuilderIncomplete { field }) => {
                assert_eq!(field, "vector_table");
            }
            other => panic!("expected BuilderIncomplete error, got {:?}", other),
        }
    }

    #[test]
    fn test_identifier_with_embedded_double_quote_is_escaped() {
        let sql = base().vector_table("tbl\"evil").to_sql().unwrap();
        // The double quote should be doubled ("") inside the SQL identifier.
        assert!(sql.contains("\"tbl\"\"evil\""));
    }

    #[test]
    fn test_candidates_override() {
        let sql = base().candidates(25).to_sql().unwrap();
        assert!(sql.contains("'float4')) LIMIT 25)"));
    }
}
