//! Vector support for prax-sqlite (LLM/RAG).
//!
//! Gated behind the `vector` feature. Integrates sqlite-vector-rs for typed
//! vector columns, HNSW indexing, and similarity search.

/// Quote a SQL identifier (table or column name) safely.
///
/// SQLite identifiers are wrapped in double quotes; an embedded double quote
/// is escaped by doubling it. Used by the vector search builders to render
/// user-supplied table/column names without SQL injection.
pub(crate) fn quote_ident(name: &str) -> String {
    let escaped = name.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

/// Escape a single-quoted SQL string literal by doubling embedded quotes.
pub(crate) fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

pub mod error;
pub mod hybrid;
pub mod index;
pub mod metric;
pub mod register;
pub mod search;
pub mod types;

pub use error::{VectorError, VectorResult};
pub use hybrid::HybridSearchBuilder;
pub use index::{VectorColumnDef, VectorIndex};
pub use metric::{DistanceMetric, VectorElementType, VectorIndexKind};
pub use register::register_vector_extension;
pub use search::VectorSearchBuilder;
pub use types::{DoubleEmbedding, Embedding, IntVector, IntVectorElement};

/// Convenient prelude for vector operations.
pub mod prelude {
    pub use super::error::{VectorError, VectorResult};
    pub use super::hybrid::HybridSearchBuilder;
    pub use super::index::{VectorColumnDef, VectorIndex};
    pub use super::metric::{DistanceMetric, VectorElementType, VectorIndexKind};
    pub use super::register::register_vector_extension;
    pub use super::search::VectorSearchBuilder;
    pub use super::types::{DoubleEmbedding, Embedding, IntVector, IntVectorElement};
}
