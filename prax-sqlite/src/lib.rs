//! SQLite database driver for Prax ORM.
//!
//! This crate provides SQLite support for the Prax ORM, using `tokio-rusqlite`
//! for asynchronous database operations.
//!
//! # Features
//!
//! - Async/await support via `tokio-rusqlite`
//! - Connection pooling
//! - Type-safe query building
//! - Transaction support
//! - In-memory and file-based databases
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_sqlite::{SqlitePool, SqliteConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = SqliteConfig::from_url("sqlite://./mydb.db")?;
//!     let pool = SqlitePool::new(config).await?;
//!
//!     // Use the pool...
//!     Ok(())
//! }
//! ```
//!
//! See [`SqliteEngine`]'s doc block for 0.7 breaking changes.

pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod raw;
pub mod row;
pub mod row_ref;
pub mod types;

pub use config::{DatabasePath, JournalMode, SqliteConfig, SynchronousMode};
pub use connection::SqliteConnection;
pub use engine::SqliteEngine;
pub use error::{SqliteError, SqliteResult};
pub use pool::{PoolConfig, SqlitePool, SqlitePoolBuilder};
pub use raw::{SqliteJsonRow, SqliteRawEngine};
pub use row::FromSqliteRow;
