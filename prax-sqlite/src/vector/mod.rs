//! Vector support for prax-sqlite (LLM/RAG).
//!
//! Gated behind the `vector` feature. Integrates sqlite-vector-rs for typed
//! vector columns, HNSW indexing, and similarity search.

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
