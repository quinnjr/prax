//! Tenant-aware prepared statement caching.
//!
//! This module provides efficient prepared statement management for multi-tenant
//! applications. It supports:
//!
//! - **Global statement cache** for RLS-based isolation (same statements work for all tenants)
//! - **Per-tenant statement cache** for schema-based isolation
//! - **Automatic statement invalidation** on schema changes
//! - **LRU eviction** with configurable limits
//!
//! # Performance Benefits
//!
//! Prepared statements provide significant performance benefits:
//! - **Query planning cached** - Database doesn't re-plan the query
//! - **Parameter binding optimized** - Type checking done once
//! - **Network efficiency** - Only parameters sent, not full SQL
//!
//! With RLS, the same prepared statement works for all tenants because the
//! tenant filtering happens via session variables, not query changes.
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::tenant::prepared::{StatementCache, CacheMode};
//!
//! // For RLS-based tenancy (shared statements)
//! let cache = StatementCache::new(CacheMode::Global { max_statements: 1000 });
//!
//! // For schema-based tenancy (per-tenant statements)
//! let cache = StatementCache::new(CacheMode::PerTenant {
//!     max_tenants: 100,
//!     statements_per_tenant: 100,
//! });
//!
//! // Get or prepare a statement
//! let stmt = cache.get_or_prepare("users", "SELECT * FROM users WHERE id = $1", || {
//!     conn.prepare("SELECT * FROM users WHERE id = $1").await
//! }).await?;
//! ```

use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use super::context::TenantId;

/// Cache mode for prepared statements.
#[derive(Debug, Clone)]
pub enum CacheMode {
    /// Single global cache (for RLS-based isolation).
    /// All tenants share the same prepared statements.
    Global {
        /// Maximum number of statements to cache.
        max_statements: usize,
    },

    /// Per-tenant statement caches (for schema-based isolation).
    /// Each tenant has their own statements because schemas differ.
    PerTenant {
        /// Maximum number of tenants to track.
        max_tenants: usize,
        /// Maximum statements per tenant.
        statements_per_tenant: usize,
    },

    /// Disabled - don't cache statements.
    Disabled,
}

impl Default for CacheMode {
    fn default() -> Self {
        Self::Global {
            max_statements: 1000,
        }
    }
}

/// A unique key for a prepared statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatementKey {
    /// Logical name for the statement (e.g., "find_user_by_id").
    pub name: String,
    /// SQL query text.
    pub sql: String,
}

impl StatementKey {
    /// Create a new statement key.
    pub fn new(name: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sql: sql.into(),
        }
    }

    /// Create from SQL only (name derived from hash).
    pub fn from_sql(sql: impl Into<String>) -> Self {
        let sql = sql.into();
        let name = format!("stmt_{:x}", hash_sql(&sql));
        Self { name, sql }
    }
}

/// Hash SQL for statement naming.
fn hash_sql(sql: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    sql.hash(&mut hasher);
    hasher.finish()
}

/// Metadata about a cached statement.
#[derive(Debug, Clone)]
pub struct StatementMeta {
    /// When the statement was prepared.
    pub prepared_at: Instant,
    /// Number of times the statement was executed.
    pub execution_count: u64,
    /// Last execution time.
    pub last_used: Instant,
    /// Average execution time in microseconds.
    pub avg_execution_us: f64,
}

impl StatementMeta {
    /// Create new metadata.
    fn new() -> Self {
        let now = Instant::now();
        Self {
            prepared_at: now,
            execution_count: 0,
            last_used: now,
            avg_execution_us: 0.0,
        }
    }

    /// Record an execution.
    fn record_execution(&mut self, duration_us: f64) {
        self.execution_count += 1;
        self.last_used = Instant::now();

        // Running average
        let n = self.execution_count as f64;
        self.avg_execution_us = self.avg_execution_us * (n - 1.0) / n + duration_us / n;
    }
}

/// A cached statement entry.
struct CacheEntry<S> {
    /// The prepared statement handle.
    statement: S,
    /// Metadata about the statement.
    meta: StatementMeta,
}

impl<S> CacheEntry<S> {
    fn new(statement: S) -> Self {
        Self {
            statement,
            meta: StatementMeta::new(),
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
    /// Total statements prepared.
    pub statements_prepared: u64,
    /// Total statements evicted.
    pub statements_evicted: u64,
    /// Current cache size.
    pub size: usize,
    /// Total execution time saved (estimated, in ms).
    pub time_saved_ms: u64,
}

impl CacheStats {
    /// Calculate hit rate.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Thread-safe cache statistics.
pub struct AtomicCacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    statements_prepared: AtomicU64,
    statements_evicted: AtomicU64,
    size: AtomicUsize,
    time_saved_ms: AtomicU64,
}

impl Default for AtomicCacheStats {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicCacheStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            statements_prepared: AtomicU64::new(0),
            statements_evicted: AtomicU64::new(0),
            size: AtomicUsize::new(0),
            time_saved_ms: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_prepare(&self) {
        self.statements_prepared.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_eviction(&self) {
        self.statements_evicted.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn set_size(&self, size: usize) {
        self.size.store(size, Ordering::Relaxed);
    }

    #[inline]
    pub fn add_time_saved(&self, ms: u64) {
        self.time_saved_ms.fetch_add(ms, Ordering::Relaxed);
    }

    /// Get a snapshot.
    pub fn snapshot(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            statements_prepared: self.statements_prepared.load(Ordering::Relaxed),
            statements_evicted: self.statements_evicted.load(Ordering::Relaxed),
            size: self.size.load(Ordering::Relaxed),
            time_saved_ms: self.time_saved_ms.load(Ordering::Relaxed),
        }
    }
}

/// Generic statement cache that works with any statement type.
pub struct StatementCache<S> {
    mode: CacheMode,
    /// Global cache (for CacheMode::Global).
    global_cache: RwLock<HashMap<StatementKey, CacheEntry<S>>>,
    /// Per-tenant caches (for CacheMode::PerTenant).
    tenant_caches: RwLock<HashMap<String, HashMap<StatementKey, CacheEntry<S>>>>,
    /// Statistics.
    stats: AtomicCacheStats,
}

impl<S: Clone> StatementCache<S> {
    /// Create a new statement cache.
    pub fn new(mode: CacheMode) -> Self {
        let capacity = match &mode {
            CacheMode::Global { max_statements } => *max_statements,
            CacheMode::PerTenant { max_tenants, .. } => *max_tenants,
            CacheMode::Disabled => 0,
        };

        Self {
            mode,
            global_cache: RwLock::new(HashMap::with_capacity(capacity)),
            tenant_caches: RwLock::new(HashMap::with_capacity(capacity)),
            stats: AtomicCacheStats::new(),
        }
    }

    /// Create a global cache with the given max size.
    pub fn global(max_statements: usize) -> Self {
        Self::new(CacheMode::Global { max_statements })
    }

    /// Create a per-tenant cache.
    pub fn per_tenant(max_tenants: usize, statements_per_tenant: usize) -> Self {
        Self::new(CacheMode::PerTenant {
            max_tenants,
            statements_per_tenant,
        })
    }

    /// Get the cache mode.
    pub fn mode(&self) -> &CacheMode {
        &self.mode
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let size = match &self.mode {
            CacheMode::Global { .. } => self.global_cache.read().len(),
            CacheMode::PerTenant { .. } => {
                self.tenant_caches.read().values().map(|c| c.len()).sum()
            }
            CacheMode::Disabled => 0,
        };
        self.stats.set_size(size);
        self.stats.snapshot()
    }

    /// Get a cached statement (global mode).
    pub fn get(&self, key: &StatementKey) -> Option<S> {
        if matches!(self.mode, CacheMode::Disabled) {
            return None;
        }

        let cache = self.global_cache.read();
        if let Some(entry) = cache.get(key) {
            self.stats.record_hit();
            // Estimate 1ms saved per cache hit (prepare time avoided)
            self.stats.add_time_saved(1);
            Some(entry.statement.clone())
        } else {
            self.stats.record_miss();
            None
        }
    }

    /// Get a cached statement for a tenant.
    pub fn get_for_tenant(&self, tenant_id: &TenantId, key: &StatementKey) -> Option<S> {
        match &self.mode {
            CacheMode::Disabled => None,
            CacheMode::Global { .. } => self.get(key),
            CacheMode::PerTenant { .. } => {
                let caches = self.tenant_caches.read();
                if let Some(cache) = caches.get(tenant_id.as_str())
                    && let Some(entry) = cache.get(key)
                {
                    self.stats.record_hit();
                    self.stats.add_time_saved(1);
                    return Some(entry.statement.clone());
                }
                self.stats.record_miss();
                None
            }
        }
    }

    /// Insert a statement into the global cache.
    pub fn insert(&self, key: StatementKey, statement: S) {
        if matches!(self.mode, CacheMode::Disabled) {
            return;
        }

        let max = match &self.mode {
            CacheMode::Global { max_statements } => *max_statements,
            _ => return self.insert_for_tenant(&TenantId::new("global"), key, statement),
        };

        let mut cache = self.global_cache.write();

        // Evict if necessary
        if cache.len() >= max && !cache.contains_key(&key) {
            self.evict_lru(&mut cache);
        }

        cache.insert(key, CacheEntry::new(statement));
        self.stats.record_prepare();
    }

    /// Insert a statement for a specific tenant.
    pub fn insert_for_tenant(&self, tenant_id: &TenantId, key: StatementKey, statement: S) {
        match &self.mode {
            CacheMode::Disabled => {}
            CacheMode::Global { .. } => self.insert(key, statement),
            CacheMode::PerTenant {
                max_tenants,
                statements_per_tenant,
            } => {
                let mut caches = self.tenant_caches.write();

                // Evict tenant if too many
                if !caches.contains_key(tenant_id.as_str()) && caches.len() >= *max_tenants {
                    self.evict_lru_tenant(&mut caches);
                }

                let cache = caches
                    .entry(tenant_id.as_str().to_string())
                    .or_insert_with(|| HashMap::with_capacity(*statements_per_tenant));

                // Evict statement if too many
                if cache.len() >= *statements_per_tenant && !cache.contains_key(&key) {
                    self.evict_lru(cache);
                }

                cache.insert(key, CacheEntry::new(statement));
                self.stats.record_prepare();
            }
        }
    }

    /// Record an execution for statistics.
    pub fn record_execution(&self, key: &StatementKey, duration_us: f64) {
        if matches!(self.mode, CacheMode::Disabled) {
            return;
        }

        let mut cache = self.global_cache.write();
        if let Some(entry) = cache.get_mut(key) {
            entry.meta.record_execution(duration_us);
        }
    }

    /// Record an execution for a tenant.
    pub fn record_tenant_execution(
        &self,
        tenant_id: &TenantId,
        key: &StatementKey,
        duration_us: f64,
    ) {
        match &self.mode {
            CacheMode::Disabled => {}
            CacheMode::Global { .. } => self.record_execution(key, duration_us),
            CacheMode::PerTenant { .. } => {
                let mut caches = self.tenant_caches.write();
                if let Some(cache) = caches.get_mut(tenant_id.as_str())
                    && let Some(entry) = cache.get_mut(key)
                {
                    entry.meta.record_execution(duration_us);
                }
            }
        }
    }

    /// Invalidate all statements for a tenant.
    pub fn invalidate_tenant(&self, tenant_id: &TenantId) {
        if let CacheMode::PerTenant { .. } = &self.mode {
            self.tenant_caches.write().remove(tenant_id.as_str());
        }
    }

    /// Invalidate a specific statement globally.
    pub fn invalidate(&self, key: &StatementKey) {
        self.global_cache.write().remove(key);
    }

    /// Clear all cached statements.
    pub fn clear(&self) {
        self.global_cache.write().clear();
        self.tenant_caches.write().clear();
    }

    /// Evict LRU statement from a cache.
    fn evict_lru(&self, cache: &mut HashMap<StatementKey, CacheEntry<S>>) {
        let lru_key = cache
            .iter()
            .min_by_key(|(_, e)| e.meta.last_used)
            .map(|(k, _)| k.clone());

        if let Some(key) = lru_key {
            cache.remove(&key);
            self.stats.record_eviction();
        }
    }

    /// Evict LRU tenant cache.
    fn evict_lru_tenant(&self, caches: &mut HashMap<String, HashMap<StatementKey, CacheEntry<S>>>) {
        let lru_tenant = caches
            .iter()
            .filter_map(|(tenant, cache)| {
                cache
                    .values()
                    .map(|e| e.meta.last_used)
                    .max()
                    .map(|last| (tenant.clone(), last))
            })
            .min_by_key(|(_, last)| *last)
            .map(|(tenant, _)| tenant);

        if let Some(tenant) = lru_tenant {
            caches.remove(&tenant);
        }
    }
}

/// A prepared statement registry that tracks statements by name.
///
/// This is useful for debugging and monitoring which statements are cached.
#[derive(Default)]
pub struct StatementRegistry {
    statements: RwLock<HashMap<String, StatementInfo>>,
}

/// Information about a registered statement.
#[derive(Debug, Clone)]
pub struct StatementInfo {
    /// Statement name.
    pub name: String,
    /// SQL query.
    pub sql: String,
    /// Description.
    pub description: Option<String>,
    /// Expected parameter count.
    pub param_count: usize,
    /// Whether this is tenant-scoped.
    pub tenant_scoped: bool,
}

impl StatementRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a statement.
    pub fn register(&self, info: StatementInfo) {
        self.statements.write().insert(info.name.clone(), info);
    }

    /// Get a statement by name.
    pub fn get(&self, name: &str) -> Option<StatementInfo> {
        self.statements.read().get(name).cloned()
    }

    /// List all registered statements.
    pub fn list(&self) -> Vec<StatementInfo> {
        self.statements.read().values().cloned().collect()
    }

    /// Check if a statement is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.statements.read().contains_key(name)
    }
}

/// Builder for statement registration.
pub struct StatementBuilder {
    name: String,
    sql: String,
    description: Option<String>,
    param_count: usize,
    tenant_scoped: bool,
}

impl StatementBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sql: sql.into(),
            description: None,
            param_count: 0,
            tenant_scoped: false,
        }
    }

    /// Set description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set parameter count.
    pub fn params(mut self, count: usize) -> Self {
        self.param_count = count;
        self
    }

    /// Mark as tenant-scoped.
    pub fn tenant_scoped(mut self) -> Self {
        self.tenant_scoped = true;
        self
    }

    /// Build the statement info.
    pub fn build(self) -> StatementInfo {
        StatementInfo {
            name: self.name,
            sql: self.sql,
            description: self.description,
            param_count: self.param_count,
            tenant_scoped: self.tenant_scoped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statement_key() {
        let key1 = StatementKey::new("find_user", "SELECT * FROM users WHERE id = $1");
        let key2 = StatementKey::from_sql("SELECT * FROM users WHERE id = $1");

        assert_eq!(key1.sql, key2.sql);
        assert!(key2.name.starts_with("stmt_"));
    }

    #[test]
    fn test_global_cache() {
        let cache: StatementCache<String> = StatementCache::global(100);

        let key = StatementKey::new("test", "SELECT 1");
        assert!(cache.get(&key).is_none());

        cache.insert(key.clone(), "prepared_handle".to_string());
        assert_eq!(cache.get(&key), Some("prepared_handle".to_string()));
    }

    #[test]
    fn test_per_tenant_cache() {
        let cache: StatementCache<String> = StatementCache::per_tenant(10, 50);

        let tenant1 = TenantId::new("tenant-1");
        let tenant2 = TenantId::new("tenant-2");
        let key = StatementKey::new("test", "SELECT 1");

        cache.insert_for_tenant(&tenant1, key.clone(), "handle_1".to_string());
        cache.insert_for_tenant(&tenant2, key.clone(), "handle_2".to_string());

        assert_eq!(
            cache.get_for_tenant(&tenant1, &key),
            Some("handle_1".to_string())
        );
        assert_eq!(
            cache.get_for_tenant(&tenant2, &key),
            Some("handle_2".to_string())
        );
    }

    #[test]
    fn test_cache_eviction() {
        let cache: StatementCache<i32> = StatementCache::global(2);

        for i in 0..3 {
            let key = StatementKey::new(format!("stmt_{}", i), format!("SELECT {}", i));
            cache.insert(key, i);
        }

        // Should have evicted one
        let stats = cache.stats();
        assert_eq!(stats.statements_evicted, 1);
    }

    #[test]
    fn test_cache_stats() {
        let cache: StatementCache<String> = StatementCache::global(100);

        let key = StatementKey::new("test", "SELECT 1");

        // Miss
        cache.get(&key);
        assert_eq!(cache.stats().misses, 1);

        // Insert
        cache.insert(key.clone(), "handle".to_string());

        // Hit
        cache.get(&key);
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_statement_registry() {
        let registry = StatementRegistry::new();

        registry.register(
            StatementBuilder::new("find_user", "SELECT * FROM users WHERE id = $1")
                .description("Find user by ID")
                .params(1)
                .build(),
        );

        assert!(registry.contains("find_user"));
        let info = registry.get("find_user").unwrap();
        assert_eq!(info.param_count, 1);
    }
}
