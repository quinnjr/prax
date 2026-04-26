//! Vector support for prax-sqlite (LLM/RAG).
//!
//! Gated behind the `vector` feature. Integrates sqlite-vector-rs for typed
//! vector columns, HNSW indexing, and similarity search.

pub mod error;
pub mod metric;

pub use error::{VectorError, VectorResult};
pub use metric::{DistanceMetric, VectorElementType, VectorIndexKind};
