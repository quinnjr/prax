//! MySQL database driver for Prax ORM.
//!
//! This crate provides MySQL support for the Prax ORM, using the `mysql_async` driver
//! for asynchronous database operations.
//!
//! # Features
//!
//! - Async/await support via `mysql_async`
//! - Connection pooling
//! - Type-safe query building
//! - Transaction support
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_mysql::{MysqlPool, MysqlConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = MysqlConfig::from_url("mysql://user:pass@localhost/mydb")?;
//!     let pool = MysqlPool::new(config).await?;
//!
//!     // Use the pool...
//!     Ok(())
//! }
//! ```

pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod row;
pub mod row_ref;
pub mod types;

pub use config::MysqlConfig;
pub use connection::MysqlConnection;
pub use engine::{MysqlEngine, MysqlQueryResult};
pub use error::{MysqlError, MysqlResult};
pub use pool::{MysqlPool, MysqlPoolBuilder, PoolConfig};
pub use row::FromMysqlRow;
