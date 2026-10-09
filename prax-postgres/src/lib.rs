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
//! - [`PgListener`]: `LISTEN` / `NOTIFY` on a dedicated, auto-reconnecting
//!   connection, with `PgConnection::notify` / `PgTransaction::notify` for
//!   sending (any role can `NOTIFY` any channel, so treat payloads as
//!   untrusted)
//! - Pool recycling policy ([`PgRecycling`]: `Fast` (default), `Verified`,
//!   `Clean`), bounded by `PoolConfig::recycle_timeout`, plus jittered
//!   `max_lifetime` retirement. `Fast` keeps session state (`SET ROLE`,
//!   `search_path`, tenant GUCs) across checkouts; use `Clean` when that
//!   matters
//! - Row locking (`FOR UPDATE` / `FOR SHARE`, `SKIP LOCKED`, `NOWAIT`,
//!   `OF`) through [`PgEngine`] and `prax_query`'s `.lock(RowLock)`; `OF`
//!   table names must be plain identifiers
//! - Typed array binding for `FilterValue::Array` (use `= ANY($1)`) and raw
//!   `bytea` for `FilterValue::Bytes`
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

pub(crate) mod capabilities;
pub mod config;
pub mod connection;
pub(crate) mod connector;
pub mod deserialize;
pub mod engine;
pub mod error;
pub mod listener;
pub(crate) mod notify;
pub mod pool;
pub mod row;
pub mod row_ref;
pub mod statement;
#[cfg(feature = "tls")]
pub mod tls;
pub mod types;

pub use config::{PgConfig, PgConfigBuilder};
pub use connection::PgConnection;
pub use engine::PgEngine;
pub use error::{PgError, PgResult};
pub use listener::{
    PgListener, PgListenerBuilder, PgListenerEvent, PgListenerOptions, PgNotification,
};
pub use pool::{PgPool, PgPoolBuilder, PgRecycling, PoolConfig, PoolStatus};
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
