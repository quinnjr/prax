// QueryError is intentionally large; see prax-query/src/lib.rs.
#![allow(clippy::result_large_err)]

//! # prax-postgres
//!
//! PostgreSQL driver for the Prax ORM with connection pooling and prepared statement caching.
//!
//! This crate provides:
//! - Connection pool management using `deadpool-postgres`
//! - Prepared statement caching for improved performance
//! - Type-safe parameter binding
//! - Row deserialization into Prax models
//!
//! ## Example
//!
//! ```rust,ignore
//! use prax_postgres::PgPool;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a connection pool
//!     let pool = PgPool::builder()
//!         .url("postgresql://user:pass@localhost/db")
//!         .max_connections(10)
//!         .build()
//!         .await?;
//!
//!     // Get a connection
//!     let conn = pool.get().await?;
//!
//!     Ok(())
//! }
//! ```

pub mod config;
pub mod connection;
pub mod deserialize;
pub mod engine;
pub mod error;
pub mod pool;
pub mod row;
pub mod row_ref;
pub mod statement;
pub mod types;

pub use config::{PgConfig, PgConfigBuilder};
pub use connection::PgConnection;
pub use engine::PgEngine;
pub use error::{PgError, PgResult};
pub use pool::{PgPool, PgPoolBuilder, PoolConfig, PoolStatus};
pub use row::PgRow;
pub use statement::PreparedStatementCache;

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::config::{PgConfig, PgConfigBuilder};
    pub use crate::connection::PgConnection;
    pub use crate::engine::PgEngine;
    pub use crate::error::{PgError, PgResult};
    pub use crate::pool::{PgPool, PgPoolBuilder};
    pub use crate::row::PgRow;
}
