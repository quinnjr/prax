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
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod row;
pub mod types;
pub mod udf;
pub mod virtual_tables;

pub use auth::{PlainSasl, SaslMechanism};
pub use config::{
    CassandraAuth, CassandraConfig, CassandraConfigBuilder, Consistency, RetryPolicyKind, TlsConfig,
};
pub use connection::CassandraConnection;
pub use engine::{BatchBuilder, QueryResult};
pub use error::{CassandraError, CassandraResult};
pub use pool::CassandraPool;
pub use row::{FromRow, Row};
pub use udf::{UdaDefinition, UdfDefinition, UdfLanguage};
pub use virtual_tables::{ClusterInfo, PeerInfo, VirtualTables};
