//! High-performance tenant-aware connection pool management.
//!
//! This module provides efficient connection pooling strategies for multi-tenant
//! applications with support for:
//!
//! - **Per-tenant connection pools** with lazy initialization
//! - **Shared pools with tenant context** for row-level isolation
//! - **LRU eviction** for tenant pools to bound memory usage
//! - **Pool warmup** for latency-sensitive applications
//! - **Health checking** and automatic pool recovery
//!
//! # Performance Characteristics
//!
//! | Strategy | Memory | Latency | Isolation |
//! |----------|--------|---------|-----------|
//! | Shared Pool | Low | Lowest | Row-level |
//! | Per-tenant Pool | Medium | Low | Schema/DB |
//! | Database-per-tenant | High | Medium | Complete |
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::tenant::pool::{TenantPoolManager, PoolStrategy};
//!
//! // Create a tenant pool manager
//! let manager = TenantPoolManager::builder()
//!     .strategy(PoolStrategy::PerTenant {
//!         max_pools: 100,
//!         pool_size: 5,
//!     })
//!     .warmup_size(2)
//!     .idle_timeout(Duration::from_secs(300))
//!     .build();
//!
//! // Get a connection for a tenant
//! let conn = manager.get("tenant-123").await?;
//! ```

use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::context::TenantId;

/// Strategy for managing tenant connections.
#[derive(Debug, Clone)]
pub enum PoolStrategy {
    /// Single shared pool with tenant context injection (row-level isolation).
    /// Most memory efficient, lowest latency for first request.
    Shared {
        /// Maximum connections in the shared pool.
        max_connections: usize,
    },

    /// Per-tenant connection pools (schema-based isolation).
    /// Medium memory usage, consistent latency per tenant.
    PerTenant {
        /// Maximum number of tenant pools to keep alive.
        max_pools: usize,
        /// Connections per tenant pool.
        pool_size: usize,
    },

    /// Per-tenant databases with dedicated pools (complete isolation).
    /// Highest memory usage, best isolation.
    PerDatabase {
        /// Maximum number of tenant databases.
        max_databases: usize,
        /// Connections per database pool.
        pool_size: usize,
    },
}

impl Default for PoolStrategy {
    fn default() -> Self {
        Self::Shared {
            max_connections: 20,
        }
    }
}

/// Configuration for tenant pool manager.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Pool management strategy.
    pub strategy: PoolStrategy,
    /// Number of connections to pre-warm per tenant.
    pub warmup_size: usize,
    /// Time before idle pools are evicted.
    pub idle_timeout: Duration,
    /// Time before connections are recycled.
    pub max_lifetime: Duration,
    /// Enable connection health checks.
    pub health_check: bool,
    /// Health check interval.
    pub health_check_interval: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            strategy: PoolStrategy::default(),
            warmup_size: 1,
            idle_timeout: Duration::from_secs(300),
            max_lifetime: Duration::from_secs(1800),
            health_check: true,
            health_check_interval: Duration::from_secs(30),
        }
    }
}

impl PoolConfig {
    /// Create a new config builder.
    pub fn builder() -> PoolConfigBuilder {
        PoolConfigBuilder::default()
    }
}

/// Builder for pool configuration.
#[derive(Default)]
pub struct PoolConfigBuilder {
    strategy: Option<PoolStrategy>,
    warmup_size: Option<usize>,
    idle_timeout: Option<Duration>,
    max_lifetime: Option<Duration>,
    health_check: Option<bool>,
    health_check_interval: Option<Duration>,
}

impl PoolConfigBuilder {
    /// Set the pool strategy.
    pub fn strategy(mut self, strategy: PoolStrategy) -> Self {
        self.strategy = Some(strategy);
        self
    }

    /// Use shared pool strategy.
    pub fn shared(mut self, max_connections: usize) -> Self {
        self.strategy = Some(PoolStrategy::Shared { max_connections });
        self
    }

    /// Use per-tenant pool strategy.
    pub fn per_tenant(mut self, max_pools: usize, pool_size: usize) -> Self {
        self.strategy = Some(PoolStrategy::PerTenant {
            max_pools,
            pool_size,
        });
        self
    }

    /// Use per-database pool strategy.
    pub fn per_database(mut self, max_databases: usize, pool_size: usize) -> Self {
        self.strategy = Some(PoolStrategy::PerDatabase {
            max_databases,
            pool_size,
        });
        self
    }

    /// Set warmup size.
    pub fn warmup_size(mut self, size: usize) -> Self {
        self.warmup_size = Some(size);
        self
    }

    /// Set idle timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// Set max lifetime.
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = Some(lifetime);
        self
    }

    /// Enable/disable health checks.
    pub fn health_check(mut self, enabled: bool) -> Self {
        self.health_check = Some(enabled);
        self
    }

    /// Set health check interval.
    pub fn health_check_interval(mut self, interval: Duration) -> Self {
        self.health_check_interval = Some(interval);
        self
    }

    /// Build the config.
    pub fn build(self) -> PoolConfig {
        PoolConfig {
            strategy: self.strategy.unwrap_or_default(),
            warmup_size: self.warmup_size.unwrap_or(1),
            idle_timeout: self.idle_timeout.unwrap_or(Duration::from_secs(300)),
            max_lifetime: self.max_lifetime.unwrap_or(Duration::from_secs(1800)),
            health_check: self.health_check.unwrap_or(true),
            health_check_interval: self
                .health_check_interval
                .unwrap_or(Duration::from_secs(30)),
        }
    }
}

/// Statistics for a tenant pool.
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    /// Total connections acquired.
    pub connections_acquired: u64,
    /// Total connections released.
    pub connections_released: u64,
    /// Currently active connections.
    pub active_connections: usize,
    /// Idle connections available.
    pub idle_connections: usize,
    /// Total wait time for connections (ms).
    pub total_wait_time_ms: u64,
    /// Maximum wait time observed (ms).
    pub max_wait_time_ms: u64,
    /// Connection timeouts.
    pub timeouts: u64,
    /// Failed health checks.
    pub health_check_failures: u64,
    /// Pool creation time.
    pub created_at: Option<Instant>,
    /// Last activity time.
    pub last_activity: Option<Instant>,
}

/// Thread-safe pool statistics.
pub struct AtomicPoolStats {
    connections_acquired: AtomicU64,
    connections_released: AtomicU64,
    active_connections: AtomicUsize,
    idle_connections: AtomicUsize,
    total_wait_time_ms: AtomicU64,
    max_wait_time_ms: AtomicU64,
    timeouts: AtomicU64,
    health_check_failures: AtomicU64,
    created_at: Mutex<Option<Instant>>,
    last_activity: Mutex<Option<Instant>>,
}

impl Default for AtomicPoolStats {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicPoolStats {
    /// Create new atomic stats.
    pub fn new() -> Self {
        Self {
            connections_acquired: AtomicU64::new(0),
            connections_released: AtomicU64::new(0),
            active_connections: AtomicUsize::new(0),
            idle_connections: AtomicUsize::new(0),
            total_wait_time_ms: AtomicU64::new(0),
            max_wait_time_ms: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            health_check_failures: AtomicU64::new(0),
            created_at: Mutex::new(None),
            last_activity: Mutex::new(None),
        }
    }

    /// Record a connection acquisition.
    pub fn record_acquire(&self, wait_time: Duration) {
        self.connections_acquired.fetch_add(1, Ordering::Relaxed);
        self.active_connections.fetch_add(1, Ordering::Relaxed);

        let wait_ms = wait_time.as_millis() as u64;
        self.total_wait_time_ms
            .fetch_add(wait_ms, Ordering::Relaxed);

        // Update max wait time (lock-free)
        let mut current = self.max_wait_time_ms.load(Ordering::Relaxed);
        while wait_ms > current {
            match self.max_wait_time_ms.compare_exchange_weak(
                current,
                wait_ms,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => current = c,
            }
        }

        *self.last_activity.lock() = Some(Instant::now());
    }

    /// Record a connection release.
    pub fn record_release(&self) {
        self.connections_released.fetch_add(1, Ordering::Relaxed);
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
        *self.last_activity.lock() = Some(Instant::now());
    }

    /// Record a timeout.
    pub fn record_timeout(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a health check failure.
    pub fn record_health_failure(&self) {
        self.health_check_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Set idle connection count.
    pub fn set_idle(&self, count: usize) {
        self.idle_connections.store(count, Ordering::Relaxed);
    }

    /// Mark as created.
    pub fn mark_created(&self) {
        *self.created_at.lock() = Some(Instant::now());
    }

    /// Get a snapshot of the stats.
    pub fn snapshot(&self) -> PoolStats {
        PoolStats {
            connections_acquired: self.connections_acquired.load(Ordering::Relaxed),
            connections_released: self.connections_released.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            idle_connections: self.idle_connections.load(Ordering::Relaxed),
            total_wait_time_ms: self.total_wait_time_ms.load(Ordering::Relaxed),
            max_wait_time_ms: self.max_wait_time_ms.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            health_check_failures: self.health_check_failures.load(Ordering::Relaxed),
            created_at: *self.created_at.lock(),
            last_activity: *self.last_activity.lock(),
        }
    }
}

/// LRU entry for tenant pools.
struct LruEntry<T> {
    value: T,
    last_access: Instant,
    access_count: u64,
}

/// LRU cache for tenant pools with capacity limits.
pub struct TenantLruCache<K, V>
where
    K: Eq + Hash + Clone,
{
    entries: RwLock<HashMap<K, LruEntry<V>>>,
    max_size: usize,
    idle_timeout: Duration,
}

impl<K, V> TenantLruCache<K, V>
where
    K: Eq + Hash + Clone,
{
    /// Create a new LRU cache.
    pub fn new(max_size: usize, idle_timeout: Duration) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(max_size)),
            max_size,
            idle_timeout,
        }
    }

    /// Get a value from the cache, updating access time.
    pub fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(key) {
            entry.last_access = Instant::now();
            entry.access_count += 1;
            Some(entry.value.clone())
        } else {
            None
        }
    }

    /// Insert a value into the cache, evicting if necessary.
    pub fn insert(&self, key: K, value: V) {
        let mut entries = self.entries.write();

        // Check if we need to evict
        if entries.len() >= self.max_size && !entries.contains_key(&key) {
            self.evict_one(&mut entries);
        }

        entries.insert(
            key,
            LruEntry {
                value,
                last_access: Instant::now(),
                access_count: 1,
            },
        );
    }

    /// Remove a value from the cache.
    pub fn remove(&self, key: &K) -> Option<V> {
        self.entries.write().remove(key).map(|e| e.value)
    }

    /// Evict expired entries.
    pub fn evict_expired(&self) -> usize {
        let mut entries = self.entries.write();
        let now = Instant::now();
        let before = entries.len();

        entries.retain(|_, entry| now.duration_since(entry.last_access) < self.idle_timeout);

        before - entries.len()
    }

    /// Get cache size.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Evict the least recently used entry.
    fn evict_one(&self, entries: &mut HashMap<K, LruEntry<V>>) {
        let now = Instant::now();

        // First try to evict expired entries
        let expired_key = entries
            .iter()
            .filter(|(_, e)| now.duration_since(e.last_access) >= self.idle_timeout)
            .map(|(k, _)| k.clone())
            .next();

        if let Some(key) = expired_key {
            entries.remove(&key);
            return;
        }

        // Otherwise evict LRU entry
        let lru_key = entries
            .iter()
            .min_by_key(|(_, e)| e.last_access)
            .map(|(k, _)| k.clone());

        if let Some(key) = lru_key {
            entries.remove(&key);
        }
    }
}

/// Tenant-specific pool entry.
pub struct TenantPoolEntry {
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Pool statistics.
    pub stats: Arc<AtomicPoolStats>,
    /// Pool state.
    pub state: PoolState,
    /// Schema name (for schema-based isolation).
    pub schema: Option<String>,
    /// Database name (for database-based isolation).
    pub database: Option<String>,
}

/// State of a tenant pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolState {
    /// Pool is initializing.
    Initializing,
    /// Pool is ready for connections.
    Ready,
    /// Pool is warming up.
    WarmingUp,
    /// Pool is draining connections.
    Draining,
    /// Pool is closed.
    Closed,
}

impl TenantPoolEntry {
    /// Create a new pool entry.
    pub fn new(tenant_id: TenantId) -> Self {
        let stats = Arc::new(AtomicPoolStats::new());
        stats.mark_created();

        Self {
            tenant_id,
            stats,
            state: PoolState::Initializing,
            schema: None,
            database: None,
        }
    }

    /// Set the schema.
    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set the database.
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// Mark as ready.
    pub fn mark_ready(&mut self) {
        self.state = PoolState::Ready;
    }

    /// Check if ready.
    pub fn is_ready(&self) -> bool {
        self.state == PoolState::Ready
    }

    /// Get stats snapshot.
    pub fn stats(&self) -> PoolStats {
        self.stats.snapshot()
    }

    /// Check if pool should be evicted (idle too long).
    pub fn should_evict(&self, idle_timeout: Duration) -> bool {
        if let Some(last) = self.stats.snapshot().last_activity {
            Instant::now().duration_since(last) > idle_timeout
        } else {
            false
        }
    }
}

/// Manager for tenant connection pools.
///
/// This is a placeholder struct that would be implemented with actual database
/// driver integration (tokio-postgres, sqlx, etc.).
pub struct TenantPoolManager {
    config: PoolConfig,
    pools: TenantLruCache<String, Arc<TenantPoolEntry>>,
    global_stats: Arc<AtomicPoolStats>,
}

impl TenantPoolManager {
    /// Create a new pool manager with the given config.
    pub fn new(config: PoolConfig) -> Self {
        let max_pools = match &config.strategy {
            PoolStrategy::Shared { .. } => 1,
            PoolStrategy::PerTenant { max_pools, .. } => *max_pools,
            PoolStrategy::PerDatabase { max_databases, .. } => *max_databases,
        };

        Self {
            pools: TenantLruCache::new(max_pools, config.idle_timeout),
            config,
            global_stats: Arc::new(AtomicPoolStats::new()),
        }
    }

    /// Create a builder.
    pub fn builder() -> TenantPoolManagerBuilder {
        TenantPoolManagerBuilder::default()
    }

    /// Get or create a pool entry for a tenant.
    pub fn get_or_create(&self, tenant_id: &TenantId) -> Arc<TenantPoolEntry> {
        let key = tenant_id.as_str().to_string();

        // Try to get existing
        if let Some(entry) = self.pools.get(&key) {
            return entry;
        }

        // Create new entry
        let entry = Arc::new(TenantPoolEntry::new(tenant_id.clone()));
        self.pools.insert(key, entry.clone());
        entry
    }

    /// Get global statistics.
    pub fn global_stats(&self) -> PoolStats {
        self.global_stats.snapshot()
    }

    /// Get number of active tenant pools.
    pub fn active_pools(&self) -> usize {
        self.pools.len()
    }

    /// Evict expired tenant pools.
    pub fn evict_expired(&self) -> usize {
        self.pools.evict_expired()
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }
}

/// Builder for tenant pool manager.
#[derive(Default)]
pub struct TenantPoolManagerBuilder {
    config: Option<PoolConfig>,
}

impl TenantPoolManagerBuilder {
    /// Set the pool config.
    pub fn config(mut self, config: PoolConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Use shared pool strategy.
    pub fn shared(self, max_connections: usize) -> Self {
        self.config(PoolConfig::builder().shared(max_connections).build())
    }

    /// Use per-tenant strategy.
    pub fn per_tenant(self, max_pools: usize, pool_size: usize) -> Self {
        self.config(
            PoolConfig::builder()
                .per_tenant(max_pools, pool_size)
                .build(),
        )
    }

    /// Build the manager.
    pub fn build(self) -> TenantPoolManager {
        TenantPoolManager::new(self.config.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_builder() {
        let config = PoolConfig::builder()
            .per_tenant(100, 5)
            .warmup_size(2)
            .idle_timeout(Duration::from_secs(600))
            .build();

        assert!(matches!(config.strategy, PoolStrategy::PerTenant { .. }));
        assert_eq!(config.warmup_size, 2);
        assert_eq!(config.idle_timeout, Duration::from_secs(600));
    }

    #[test]
    fn test_atomic_stats() {
        let stats = AtomicPoolStats::new();
        stats.mark_created();

        stats.record_acquire(Duration::from_millis(5));
        stats.record_acquire(Duration::from_millis(10));
        stats.record_release();

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.connections_acquired, 2);
        assert_eq!(snapshot.connections_released, 1);
        assert_eq!(snapshot.active_connections, 1);
        assert_eq!(snapshot.max_wait_time_ms, 10);
    }

    #[test]
    fn test_lru_cache() {
        let cache: TenantLruCache<String, i32> = TenantLruCache::new(3, Duration::from_secs(60));

        cache.insert("a".to_string(), 1);
        cache.insert("b".to_string(), 2);
        cache.insert("c".to_string(), 3);

        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get(&"a".to_string()), Some(1));

        // Inserting d should evict the LRU (b or c)
        cache.insert("d".to_string(), 4);
        assert_eq!(cache.len(), 3);

        // a should still exist (accessed recently)
        assert_eq!(cache.get(&"a".to_string()), Some(1));
    }

    #[test]
    fn test_tenant_pool_entry() {
        let entry = TenantPoolEntry::new(TenantId::new("test")).with_schema("tenant_test");

        assert_eq!(entry.schema, Some("tenant_test".to_string()));
        assert_eq!(entry.state, PoolState::Initializing);
    }

    #[test]
    fn test_pool_manager_creation() {
        let manager = TenantPoolManager::builder().per_tenant(100, 5).build();

        assert_eq!(manager.active_pools(), 0);

        let entry = manager.get_or_create(&TenantId::new("tenant-1"));
        assert_eq!(entry.tenant_id.as_str(), "tenant-1");
        assert_eq!(manager.active_pools(), 1);

        // Getting same tenant should return same entry
        let _entry2 = manager.get_or_create(&TenantId::new("tenant-1"));
        assert_eq!(manager.active_pools(), 1);
    }
}
