//! # prax-mssql
//!
//! Microsoft SQL Server driver for the Prax ORM with connection pooling
//! and Row-Level Security (RLS) support.
//!
//! This crate provides:
//! - Connection pool management using `bb8` and `tiberius`
//! - Prepared statement support
//! - Type-safe parameter binding
//! - Row deserialization into Prax models
//! - Row-Level Security (RLS) policy generation
//!
//! ## Example
//!
//! ```rust,ignore
//! use prax_mssql::MssqlPool;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a connection pool
//!     let pool = MssqlPool::builder()
//!         .host("localhost")
//!         .database("mydb")
//!         .username("sa")
//!         .password("YourPassword123!")
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
//!
//! ## Row-Level Security
//!
//! Generate SQL Server security policies from Prax schema policies:
//!
//! ```rust,ignore
//! use prax_mssql::rls::SecurityPolicyGenerator;
//! use prax_schema::Policy;
//!
//! let generator = SecurityPolicyGenerator::new("Security");
//! let statements = generator.generate(&policy, "dbo.Users", "UserId");
//!
//! // Execute the generated SQL
//! conn.batch_execute(&statements.to_sql()).await?;
//! ```

pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod rls;
pub mod row;
pub mod types;

pub use config::{MssqlConfig, MssqlConfigBuilder};
pub use connection::MssqlConnection;
pub use engine::MssqlEngine;
pub use error::{MssqlError, MssqlResult};
pub use pool::{MssqlPool, MssqlPoolBuilder, PoolConfig, PoolStatus};
pub use rls::{BlockOperation, SecurityPolicy, SecurityPolicyGenerator};
pub use row::MssqlRow;

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::config::{MssqlConfig, MssqlConfigBuilder};
    pub use crate::connection::MssqlConnection;
    pub use crate::engine::MssqlEngine;
    pub use crate::error::{MssqlError, MssqlResult};
    pub use crate::pool::{MssqlPool, MssqlPoolBuilder};
    pub use crate::rls::{BlockOperation, SecurityPolicy, SecurityPolicyGenerator};
    pub use crate::row::MssqlRow;
}
