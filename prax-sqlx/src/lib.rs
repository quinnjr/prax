//! # Prax SQLx Backend
//!
//! This crate provides a SQLx-based query engine for Prax ORM, offering
//! compile-time checked queries as an alternative to the default async drivers.
//!
//! ## Features
//!
//! - **Compile-time query checking** - Validate SQL queries at compile time
//! - **Multi-database support** - PostgreSQL, MySQL, and SQLite through a unified API
//! - **Type-safe queries** - Strong typing for query parameters and results
//! - **Connection pooling** - Built-in connection pool management via SQLx
//! - **Async/await** - Full async support with tokio runtime
//!
//! ## Usage
//!
//! ```rust,ignore
//! use prax_sqlx::{SqlxEngine, SqlxConfig};
//!
//! // Create configuration
//! let config = SqlxConfig::from_url("postgres://user:pass@localhost/db")?;
//!
//! // Create engine
//! let engine = SqlxEngine::new(config).await?;
//!
//! // Execute queries
//! let users: Vec<User> = engine
//!     .query_many("SELECT * FROM users WHERE active = $1", &[&true])
//!     .await?;
//! ```
//!
//! ## Compile-Time Checking
//!
//! Use the `sqlx::query!` macro for compile-time SQL verification:
//!
//! ```rust,ignore
//! use prax_sqlx::checked;
//!
//! // This query is checked at compile time
//! let users = checked::query_as!(
//!     User,
//!     "SELECT id, name, email FROM users WHERE id = $1",
//!     user_id
//! )
//! .fetch_all(&pool)
//! .await?;
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod row;
pub mod row_ref;
pub mod types;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "mysql")]
pub mod mysql;

#[cfg(feature = "sqlite")]
pub mod sqlite;

/// Re-export commonly used types
pub use config::SqlxConfig;
pub use engine::SqlxEngine;
pub use error::{SqlxError, SqlxResult};
pub use pool::{SqlxPool, SqlxPoolBuilder};

/// Re-export SQLx types for convenience
pub use sqlx::{self, FromRow, Row};

/// Checked query macros for compile-time SQL verification
pub mod checked {
    pub use sqlx::{query, query_as, query_scalar};
}
