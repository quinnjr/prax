//! Hybrid search builder combining vector similarity with fts5 full-text search
//! via Reciprocal Rank Fusion (RRF).

use crate::vector::metric::{DistanceMetric, VectorElementType};

/// Builder for a hybrid search (vector + fts5) using RRF.
#[derive(Debug, Clone)]
pub struct HybridSearchBuilder {
    main_table: String,
    id_column: String,
    vector_table: Option<String>,
    rowid_column: Option<String>,
    vector_column: Option<String>,
    fts_table: Option<String>,
    element_type: VectorElementType,
    metric: DistanceMetric,
    query_vector_json: Option<String>,
    query_text: Option<String>,
    vector_weight: f64,
    text_weight: f64,
    rrf_k: u32,
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
            rowid_column: None,
            vector_column: None,
            fts_table: None,
            element_type: VectorElementType::Float4,
            metric: DistanceMetric::Cosine,
            query_vector_json: None,
            query_text: None,
            vector_weight: 0.5,
            text_weight: 0.5,
            rrf_k: 60,
            limit: None,
        }
    }

    /// Override the main table id column (default "id").
    pub fn id_column(mut self, name: impl Into<String>) -> Self {
        self.id_column = name.into();
        self
    }

    /// Set the vector virtual table name.
    pub fn vector_table(mut self, name: impl Into<String>) -> Self {
        self.vector_table = Some(name.into());
        self
    }

    /// Set the rowid column name used on the virtual table to join back to
    /// the main table.
    pub fn rowid_column(mut self, name: impl Into<String>) -> Self {
        self.rowid_column = Some(name.into());
        self
    }

    /// Set the vector column name (on the virtual table).
    pub fn vector_column(mut self, name: impl Into<String>) -> Self {
        self.vector_column = Some(name.into());
        self
    }

    /// Set the fts5 virtual table name.
    pub fn fts_table(mut self, name: impl Into<String>) -> Self {
        self.fts_table = Some(name.into());
        self
    }

    /// Set the element type used for vector_from_json calls.
    pub fn element_type(mut self, t: VectorElementType) -> Self {
        self.element_type = t;
        self
    }

    /// Set the vector distance metric.
    pub fn metric(mut self, m: DistanceMetric) -> Self {
        self.metric = m;
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

    /// Set the result limit.
    pub fn limit(mut self, n: u32) -> Self {
        self.limit = Some(n);
        self
    }

    /// Render the SELECT statement.
    ///
    /// Panics if any of vector_table, rowid_column, vector_column,
    /// fts_table, query_vector_json, or query_text is unset.
    pub fn to_sql(&self) -> String {
        let vtable = self
            .vector_table
            .as_deref()
            .expect("vector_table must be set");
        let rowid = self
            .rowid_column
            .as_deref()
            .expect("rowid_column must be set");
        let vcol = self
            .vector_column
            .as_deref()
            .expect("vector_column must be set");
        let ftab = self.fts_table.as_deref().expect("fts_table must be set");
        let qv = self
            .query_vector_json
            .as_deref()
            .expect("query_vector_json must be set");
        let qt = self.query_text.as_deref().expect("query_text must be set");

        let limit_clause = match self.limit {
            Some(n) => format!("\nLIMIT {}", n),
            None => String::new(),
        };

        format!(
            "WITH vec_ranked AS (\n    \
             SELECT \"{rowid}\" AS match_id, \
             ROW_NUMBER() OVER (ORDER BY vector_distance(\"{vcol}\", vector_from_json('{qv}', '{et}'), '{metric}', '{et}')) AS rank\n    \
             FROM \"{vtable}\"\n\
             ),\n\
             fts_ranked AS (\n    \
             SELECT rowid AS match_id, \
             ROW_NUMBER() OVER (ORDER BY bm25(\"{ftab}\")) AS rank\n    \
             FROM \"{ftab}\" WHERE \"{ftab}\" MATCH '{qt}'\n\
             )\n\
             SELECT \"{main}\".*, \
             COALESCE({vw} / ({k} + v.rank), 0) + COALESCE({tw} / ({k} + f.rank), 0) AS score\n\
             FROM \"{main}\"\n\
             LEFT JOIN vec_ranked v ON \"{main}\".\"{id}\" = v.match_id\n\
             LEFT JOIN fts_ranked f ON \"{main}\".\"{id}\" = f.match_id\n\
             WHERE v.match_id IS NOT NULL OR f.match_id IS NOT NULL\n\
             ORDER BY score DESC{limit}",
            rowid = rowid,
            vcol = vcol,
            qv = qv,
            et = self.element_type.as_sql(),
            metric = self.metric.as_sql(),
            vtable = vtable,
            ftab = ftab,
            qt = qt.replace('\'', "''"),
            main = self.main_table,
            id = self.id_column,
            vw = self.vector_weight,
            tw = self.text_weight,
            k = self.rrf_k,
            limit = limit_clause,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> HybridSearchBuilder {
        HybridSearchBuilder::new("documents")
            .vector_table("documents_vectors")
            .rowid_column("document_id")
            .vector_column("embedding")
            .fts_table("documents_fts")
            .query_vector_json("[0.1,0.2]")
            .query_text("machine learning")
    }

    #[test]
    fn test_default_weights_and_rrf_k() {
        let sql = base().to_sql();
        assert!(sql.contains("COALESCE(0.5 / (60 + v.rank), 0)"));
        assert!(sql.contains("COALESCE(0.5 / (60 + f.rank), 0)"));
    }

    #[test]
    fn test_custom_weights_and_rrf_k() {
        let sql = base()
            .vector_weight(0.7)
            .text_weight(0.3)
            .rrf_k(80)
            .to_sql();
        assert!(sql.contains("COALESCE(0.7 / (80 + v.rank), 0)"));
        assert!(sql.contains("COALESCE(0.3 / (80 + f.rank), 0)"));
    }

    #[test]
    fn test_limit() {
        let sql = base().limit(25).to_sql();
        assert!(sql.contains("LIMIT 25"));
    }

    #[test]
    fn test_fts_query_quote_escaping() {
        let sql = base().query_text("it's a test").to_sql();
        assert!(sql.contains("MATCH 'it''s a test'"));
    }

    #[test]
    fn test_vector_table_and_fts_table_appear() {
        let sql = base().to_sql();
        assert!(sql.contains("FROM \"documents_vectors\""));
        assert!(sql.contains("FROM \"documents_fts\""));
        assert!(sql.contains("FROM \"documents\""));
    }

    #[test]
    #[should_panic(expected = "vector_table must be set")]
    fn test_missing_vector_table_panics() {
        HybridSearchBuilder::new("docs")
            .rowid_column("id")
            .vector_column("v")
            .fts_table("docs_fts")
            .query_vector_json("[0.1]")
            .query_text("q")
            .to_sql();
    }
}
