//! DuckDB database driver for Prax ORM.
//!
//! This crate provides DuckDB support for the Prax ORM, optimized for analytical
//! workloads (OLAP). DuckDB is an in-process analytical database similar to SQLite
//! but designed for fast analytical queries.
//!
//! # Features
//!
//! - **In-process analytics**: No server required, runs embedded
//! - **Columnar storage**: Optimized for analytical queries
//! - **Parquet support**: Native reading/writing of Parquet files
//! - **JSON support**: Query JSON data directly
//! - **SQL compatibility**: Full SQL support with extensions
//! - **Async support**: Async operations via Tokio task spawning
//!
//! # When to Use DuckDB
//!
//! DuckDB excels at:
//! - Analytical queries (aggregations, joins, window functions)
//! - Data transformation and ETL
//! - Querying Parquet, CSV, and JSON files
//! - Embedded analytics in applications
//!
//! For OLTP workloads (many small transactions), consider PostgreSQL or SQLite instead.
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_duckdb::{DuckDbPool, DuckDbConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // In-memory database
//!     let config = DuckDbConfig::in_memory();
//!     let pool = DuckDbPool::new(config).await?;
//!
//!     // Or file-based
//!     let config = DuckDbConfig::from_path("./analytics.duckdb")?;
//!     let pool = DuckDbPool::new(config).await?;
//!
//!     // Query Parquet files directly
//!     let results = pool.get().await?
//!         .query("SELECT * FROM 'data/*.parquet' WHERE year = 2024", &[])
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Analytical Features
//!
//! ```rust,ignore
//! // Window functions
//! let sql = r#"
//!     SELECT
//!         date,
//!         revenue,
//!         SUM(revenue) OVER (
//!             PARTITION BY region
//!             ORDER BY date
//!             ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
//!         ) as cumulative_revenue
//!     FROM sales
//! "#;
//!
//! // COPY to Parquet
//! engine.raw_sql_execute(
//!     "COPY (SELECT * FROM analytics) TO 'output.parquet' (FORMAT PARQUET)",
//!     &[]
//! ).await?;
//! ```

pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod row;
pub mod types;

pub use config::{AccessMode, DuckDbConfig, DuckDbConfigBuilder, ThreadMode};
pub use connection::DuckDbConnection;
pub use engine::{DuckDbEngine, DuckDbQueryResult};
pub use error::{DuckDbError, DuckDbResult};
pub use pool::{DuckDbPool, DuckDbPoolBuilder, PoolConfig};
pub use row::FromDuckDbRow;

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::config::{AccessMode, DuckDbConfig, DuckDbConfigBuilder};
    pub use crate::connection::DuckDbConnection;
    pub use crate::engine::{DuckDbEngine, DuckDbQueryResult};
    pub use crate::error::{DuckDbError, DuckDbResult};
    pub use crate::pool::{DuckDbPool, DuckDbPoolBuilder};
    pub use crate::row::FromDuckDbRow;
}
