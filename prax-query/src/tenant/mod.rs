#![allow(dead_code)]

//! High-performance multi-tenant support for Prax.
//!
//! This module provides comprehensive, performance-optimized multi-tenancy support with
//! multiple isolation strategies:
//!
//! - **Row-Level Security (RLS)**: All tenants share tables, filtered by tenant_id column
//! - **Schema-Based**: Each tenant has their own database schema
//! - **Database-Based**: Each tenant has their own database
//!
//! # Performance Features
//!
//! This implementation includes several performance optimizations:
//!
//! - **Zero-allocation context propagation** via task-local storage
//! - **PostgreSQL RLS integration** for database-level enforcement (no app overhead)
//! - **LRU tenant cache** with TTL and background refresh
//! - **Per-tenant connection pools** with LRU eviction
//! - **Prepared statement caching** that works across tenants (with RLS)
//! - **Sharded caches** for high-concurrency scenarios
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use prax_query::tenant::{TenantContext, TenantConfig, IsolationStrategy};
//! use prax_query::tenant::task_local::with_tenant;
//!
//! // Configure row-level isolation with PostgreSQL RLS
//! let config = TenantConfig::row_level("tenant_id");
//!
//! // Zero-allocation tenant context via task-locals
//! with_tenant("tenant-123", async {
//!     // All queries automatically filtered by tenant
//!     let users = client.user().find_many().exec().await?;
//!     // SQL: SELECT * FROM users WHERE tenant_id = 'tenant-123'
//!     Ok(())
//! }).await?;
//! ```
//!
//! # Isolation Strategies
//!
//! ## Row-Level Security (Most Performant)
//!
//! The simplest and most performant approach where all tenants share tables.
//! When combined with PostgreSQL RLS, filtering happens in the database engine
//! with zero application overhead:
//!
//! ```rust,ignore
//! use prax_query::tenant::rls::{RlsManager, RlsConfig};
//!
//! // Setup PostgreSQL RLS
//! let rls = RlsManager::new(
//!     RlsConfig::new("tenant_id")
//!         .with_session_variable("app.current_tenant")
//!         .add_tables(["users", "orders", "products"])
//! );
//!
//! // Generate and execute setup SQL
//! let setup_sql = rls.setup_sql();
//! conn.execute_batch(&setup_sql).await?;
//!
//! // Now all queries are automatically filtered by the database
//! // Just set the tenant context per-request:
//! conn.execute(&rls.set_tenant_local_sql("tenant-123"), &[]).await?;
//! ```
//!
//! ## Schema-Based
//!
//! Each tenant gets their own schema (PostgreSQL/MySQL):
//!
//! ```rust,ignore
//! let config = TenantConfig::schema_based()
//!     .with_schema_prefix("tenant_")
//!     .with_shared_schema("shared");
//! ```
//!
//! ## Database-Based (Complete Isolation)
//!
//! Each tenant gets their own database:
//!
//! ```rust,ignore
//! use prax_query::tenant::pool::{TenantPoolManager, PoolStrategy};
//!
//! // Use per-tenant connection pools
//! let pool_manager = TenantPoolManager::builder()
//!     .per_tenant(100, 5)  // max 100 tenants, 5 connections each
//!     .build();
//!
//! let config = TenantConfig::database_based()
//!     .with_resolver(|tenant_id| async move {
//!         DatabaseConfig::from_url(&format!("postgres://localhost/{}", tenant_id))
//!     });
//! ```
//!
//! # Caching
//!
//! The module provides high-performance caching for tenant lookups:
//!
//! ```rust,ignore
//! use prax_query::tenant::cache::{TenantCache, ShardedTenantCache, CacheConfig};
//!
//! // Simple cache
//! let cache = TenantCache::new(CacheConfig::new(10_000).with_ttl(Duration::from_secs(300)));
//!
//! // High-concurrency sharded cache
//! let cache = ShardedTenantCache::high_concurrency(10_000);
//!
//! // Get or fetch tenant
//! let ctx = cache.get_or_fetch(&tenant_id, || async {
//!     db.query("SELECT * FROM tenants WHERE id = $1", &[&tenant_id]).await
//! }).await?;
//! ```

mod cache;
mod config;
mod context;
mod middleware;
mod pool;
mod prepared;
mod resolver;
mod rls;
mod strategy;
mod task_local;

// Core types
pub use config::{TenantConfig, TenantConfigBuilder};
pub use context::{TenantContext, TenantId, TenantInfo};
pub use middleware::TenantMiddleware;
pub use resolver::{
    CompositeResolver, DatabaseResolver, DynamicResolver, StaticResolver, TenantResolver,
};
pub use strategy::{
    ColumnType, DatabaseConfig as TenantDatabaseConfig, IsolationStrategy, RowLevelConfig,
    SchemaConfig, SearchPathFormat,
};

// High-performance features
pub use cache::{CacheConfig, CacheLookup, CacheMetrics, ShardedTenantCache, TenantCache};
pub use pool::{
    AtomicPoolStats, PoolConfig, PoolConfigBuilder, PoolState, PoolStats, PoolStrategy,
    TenantLruCache, TenantPoolEntry, TenantPoolManager, TenantPoolManagerBuilder,
};
pub use prepared::{
    CacheMode as StatementCacheMode, CacheStats as StatementCacheStats, StatementCache,
    StatementKey, StatementMeta, StatementRegistry,
};
pub use rls::{PolicyCommand, RlsConfig, RlsManager, RlsManagerBuilder, RlsPolicy, TenantGuard};
pub use task_local::{
    CompositeExtractor, HeaderExtractor, JwtClaimExtractor, SyncTenantGuard, TenantExtractor,
    TenantNotSetError, TenantScope, current_tenant, current_tenant_id, has_tenant, require_tenant,
    set_sync_tenant, sync_tenant_id, with_context, with_current_tenant, with_tenant,
};
