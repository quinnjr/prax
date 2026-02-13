//! # prax-pgvector
//!
//! pgvector integration for the Prax ORM — vector similarity search, embeddings,
//! and index management for PostgreSQL.
//!
//! This crate provides type-safe wrappers around [pgvector](https://github.com/pgvector/pgvector)
//! functionality, integrating with the `prax-postgres` driver for seamless vector operations.
//!
//! ## Features
//!
//! - **Vector types**: [`Embedding`], [`SparseEmbedding`], [`BinaryVector`] (and [`HalfEmbedding`]
//!   with the `halfvec` feature)
//! - **Distance metrics**: L2, inner product, cosine, L1, Hamming, Jaccard
//! - **Index management**: IVFFlat and HNSW index creation with tuning parameters
//! - **Query builder**: Fluent API for vector similarity search
//! - **Hybrid search**: Combined vector + full-text search using Reciprocal Rank Fusion
//! - **Filter integration**: Vector filters for prax-query WHERE/ORDER BY clauses
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use prax_pgvector::prelude::*;
//!
//! // Create an embedding
//! let query = Embedding::new(vec![0.1, 0.2, 0.3, /* ... */]);
//!
//! // Build a similarity search
//! let search = VectorSearchBuilder::new("documents", "embedding")
//!     .query(query)
//!     .metric(DistanceMetric::Cosine)
//!     .limit(10)
//!     .ef_search(200) // Tune HNSW recall
//!     .build();
//!
//! let sql = search.to_sql();
//! // SELECT *, embedding <=> $1 AS distance FROM documents ORDER BY distance LIMIT 10
//! ```
//!
//! ## Index Management
//!
//! ```rust,ignore
//! use prax_pgvector::index::{VectorIndex, HnswConfig};
//! use prax_pgvector::DistanceMetric;
//!
//! // Create an HNSW index
//! let index = VectorIndex::hnsw("idx_doc_embedding", "documents", "embedding")
//!     .metric(DistanceMetric::Cosine)
//!     .config(HnswConfig::high_recall())
//!     .concurrent()
//!     .build()
//!     .unwrap();
//!
//! println!("{}", index.to_create_sql());
//! // CREATE INDEX CONCURRENTLY idx_doc_embedding ON documents
//! //   USING hnsw (embedding vector_cosine_ops)
//! //   WITH (m = 32, ef_construction = 128)
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `halfvec` | Enable half-precision (float16) vector support |

pub mod error;
pub mod filter;
pub mod index;
pub mod ops;
pub mod query;
pub mod types;

// Re-export primary types at crate root for convenience.
pub use error::{VectorError, VectorResult};
pub use ops::{BinaryDistanceMetric, DistanceMetric};
pub use types::{BinaryVector, Embedding, SparseEmbedding};

#[cfg(feature = "halfvec")]
pub use types::HalfEmbedding;

/// Prelude for convenient imports.
///
/// ```rust,ignore
/// use prax_pgvector::prelude::*;
/// ```
pub mod prelude {
    pub use crate::error::{VectorError, VectorResult};
    pub use crate::filter::{VectorFilter, VectorOrderBy};
    pub use crate::index::{HnswConfig, IvfFlatConfig, VectorIndex};
    pub use crate::ops::{BinaryDistanceMetric, DistanceMetric, SearchParams};
    pub use crate::query::{HybridSearchBuilder, VectorSearchBuilder, VectorSearchQuery};
    pub use crate::types::{BinaryVector, Embedding, SparseEmbedding};

    #[cfg(feature = "halfvec")]
    pub use crate::types::HalfEmbedding;
}
