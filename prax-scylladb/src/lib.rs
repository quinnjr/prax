//! # prax-scylladb
//!
//! ScyllaDB database driver for Prax ORM - high-performance Cassandra-compatible database.
//!
//! ScyllaDB is a drop-in replacement for Apache Cassandra that offers significantly
//! better performance. This driver provides async support for ScyllaDB operations
//! within the Prax ORM ecosystem.
//!
//! ## Features
//!
//! - **High Performance**: Built on the official `scylla` async driver
//! - **Connection Pooling**: Automatic connection management with configurable pool sizes
//! - **Prepared Statements**: Efficient query execution with automatic caching
//! - **Async/Await**: Full async support with Tokio runtime
//! - **Type Safety**: Strong typing with automatic CQL type conversions
//! - **Lightweight Transactions**: Support for conditional updates (LWT)
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use prax_scylladb::{ScyllaConfig, ScyllaPool};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure the connection
//!     let config = ScyllaConfig::builder()
//!         .known_nodes(["127.0.0.1:9042"])
//!         .default_keyspace("my_keyspace")
//!         .build();
//!
//!     // Create connection pool
//!     let pool = ScyllaPool::connect(config).await?;
//!
//!     // Execute queries
//!     let result = pool
//!         .query("SELECT * FROM users WHERE id = ?", (user_id,))
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Configuration
//!
//! ```rust
//! use prax_scylladb::ScyllaConfig;
//!
//! let config = ScyllaConfig::builder()
//!     .known_nodes(["node1:9042", "node2:9042", "node3:9042"])
//!     .default_keyspace("production")
//!     .username("admin")
//!     .password("secret")
//!     .connection_timeout_secs(10)
//!     .request_timeout_secs(30)
//!     .build();
//! ```
//!
//! ## Prepared Statements
//!
//! For frequently executed queries, use prepared statements:
//!
//! ```rust,no_run
//! use prax_scylladb::ScyllaEngine;
//!
//! async fn get_user(engine: &ScyllaEngine, id: uuid::Uuid) -> Result<Option<User>, Error> {
//!     engine
//!         .query_one("SELECT * FROM users WHERE id = ?", (id,))
//!         .await
//! }
//! ```
//!
//! ## Batch Operations
//!
//! Execute multiple statements atomically:
//!
//! ```rust,no_run
//! use prax_scylladb::ScyllaEngine;
//!
//! async fn transfer_funds(
//!     engine: &ScyllaEngine,
//!     from: uuid::Uuid,
//!     to: uuid::Uuid,
//!     amount: i64,
//! ) -> Result<(), Error> {
//!     engine.batch()
//!         .add("UPDATE accounts SET balance = balance - ? WHERE id = ?", (amount, from))
//!         .add("UPDATE accounts SET balance = balance + ? WHERE id = ?", (amount, to))
//!         .execute()
//!         .await
//! }
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod config;
mod connection;
mod engine;
mod error;
mod pool;
mod row;
pub mod row_ref;
mod types;

pub use config::{ScyllaConfig, ScyllaConfigBuilder};
pub use connection::ScyllaConnection;
pub use engine::{ScyllaBatch, ScyllaEngine};
pub use error::{ScyllaError, ScyllaResult};
pub use pool::ScyllaPool;
pub use row::FromScyllaRow;
pub use types::{ScyllaValue, ToCqlValue};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use crate::config::{ScyllaConfig, ScyllaConfigBuilder};
    pub use crate::connection::ScyllaConnection;
    pub use crate::engine::{ScyllaBatch, ScyllaEngine};
    pub use crate::error::{ScyllaError, ScyllaResult};
    pub use crate::pool::ScyllaPool;
    pub use crate::row::FromScyllaRow;
    pub use crate::types::{ScyllaValue, ToCqlValue};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = ScyllaConfig::builder()
            .known_nodes(["127.0.0.1:9042"])
            .default_keyspace("test")
            .build();

        assert_eq!(config.default_keyspace(), Some("test"));
        assert_eq!(config.known_nodes().len(), 1);
    }

    #[test]
    fn test_config_from_url() {
        let config = ScyllaConfig::from_url("scylla://localhost:9042/my_keyspace").unwrap();
        assert_eq!(config.default_keyspace(), Some("my_keyspace"));
    }
}
