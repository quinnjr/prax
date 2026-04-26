//! # prax-cassandra
//!
//! Apache Cassandra driver for Prax ORM using cdrs-tokio.
//!
//! Provides async CRUD, prepared statements, batches, lightweight transactions,
//! paging, virtual tables (Cassandra 4.0+), and UDF/UDA management.
//!
//! ## Example
//!
//! ```rust,no_run
//! use prax_cassandra::{CassandraConfig, CassandraPool};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = CassandraConfig::builder()
//!         .known_nodes(["127.0.0.1:9042".to_string()])
//!         .default_keyspace("myapp")
//!         .build();
//!
//!     let pool = CassandraPool::connect(config).await?;
//!     // ... use pool ...
//!     Ok(())
//! }
//! ```
//!
//! ## Migration Support
//!
//! Schema migrations use `prax_migrate::CqlDialect`, which works identically
//! for Cassandra and ScyllaDB since both use CQL.

pub mod auth;
pub mod config;
pub mod error;

pub use auth::{PlainSasl, SaslMechanism};
pub use config::{
    CassandraAuth, CassandraConfig, CassandraConfigBuilder, Consistency, RetryPolicyKind,
    TlsConfig,
};
pub use error::{CassandraError, CassandraResult};
