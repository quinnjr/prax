//! High-performance tenant caching with TTL and background refresh.
//!
//! This module provides an efficient caching layer for tenant lookups with:
//!
//! - **TTL-based expiration** with configurable durations
//! - **LRU eviction** when cache is full
//! - **Background refresh** to avoid cache stampedes
//! - **Negative caching** to prevent repeated lookups of invalid tenants
//! - **Metrics** for monitoring cache performance
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::tenant::cache::{TenantCache, CacheConfig};
//!
//! let cache = TenantCache::new(CacheConfig {
//!     max_entries: 10_000,
//!     ttl: Duration::from_secs(300),
//!     negative_ttl: Duration::from_secs(60),
//!     ..Default::default()
//! });
//!
//! // Get or fetch tenant
//! let ctx = cache.get_or_fetch("tenant-123", || async {
//!     // Fetch from database
//!     db.query("SELECT * FROM tenants WHERE id = $1", &[&"tenant-123"]).await
//! }).await?;
//! ```

use parking_lot::RwLock;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::context::{TenantContext, TenantId};

/// Configuration for the tenant cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of entries in the cache.
    pub max_entries: usize,
    /// Time-to-live for cached entries.
    pub ttl: Duration,
    /// Time-to-live for negative cache entries (tenant not found).
    pub negative_ttl: Duration,
    /// Enable background refresh before TTL expires.
    pub background_refresh: bool,
    /// How long before TTL to start background refresh (e.g., 0.8 = refresh at 80% of TTL).
    pub refresh_threshold: f64,
    /// Enable metrics collection.
    pub enable_metrics: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            ttl: Duration::from_secs(300),         // 5 minutes
            negative_ttl: Duration::from_secs(60), // 1 minute
            background_refresh: true,
            refresh_threshold: 0.8,
            enable_metrics: true,
        }
    }
}

impl CacheConfig {
    /// Create a new config with the given max entries.
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            ..Default::default()
        }
    }

    /// Set the TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Set the negative TTL.
    pub fn with_negative_ttl(mut self, ttl: Duration) -> Self {
        self.negative_ttl = ttl;
        self
    }

    /// Disable background refresh.
    pub fn without_background_refresh(mut self) -> Self {
        self.background_refresh = false;
        self
    }

    /// Set the refresh threshold.
    pub fn with_refresh_threshold(mut self, threshold: f64) -> Self {
        self.refresh_threshold = threshold.clamp(0.1, 0.99);
        self
    }

    /// Disable metrics.
    pub fn without_metrics(mut self) -> Self {
        self.enable_metrics = false;
        self
    }
}

/// A cached tenant entry.
#[derive(Debug, Clone)]
struct CacheEntry {
    /// The cached context (None = negative cache).
    context: Option<TenantContext>,
    /// When this entry was created.
    created_at: Instant,
    /// When this entry expires.
    expires_at: Instant,
    /// Whether a background refresh is in progress.
    refreshing: bool,
    /// Access count for LRU tracking.
    access_count: u64,
}

impl CacheEntry {
    /// Create a positive cache entry.
    fn positive(context: TenantContext, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            context: Some(context),
            created_at: now,
            expires_at: now + ttl,
            refreshing: false,
            access_count: 1,
        }
    }

    /// Create a negative cache entry.
    fn negative(ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            context: None,
            created_at: now,
            expires_at: now + ttl,
            refreshing: false,
            access_count: 1,
        }
    }

    /// Check if the entry is expired.
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    /// Check if the entry should be refreshed.
    fn should_refresh(&self, threshold: f64) -> bool {
        if self.refreshing {
            return false;
        }

        let ttl = self.expires_at.duration_since(self.created_at);
        let elapsed = self.created_at.elapsed();
        let threshold_duration = ttl.mul_f64(threshold);

        elapsed >= threshold_duration
    }

    /// Get remaining TTL.
    fn remaining_ttl(&self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }
}

/// Cache metrics.
#[derive(Debug, Clone, Default)]
pub struct CacheMetrics {
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
    /// Negative cache hits.
    pub negative_hits: u64,
    /// Evictions due to capacity.
    pub evictions: u64,
    /// Evictions due to TTL expiration.
    pub expirations: u64,
    /// Background refreshes triggered.
    pub background_refreshes: u64,
    /// Current cache size.
    pub size: usize,
}

impl CacheMetrics {
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

/// Thread-safe atomic metrics.
pub struct AtomicCacheMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
    negative_hits: AtomicU64,
    evictions: AtomicU64,
    expirations: AtomicU64,
    background_refreshes: AtomicU64,
}

impl Default for AtomicCacheMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicCacheMetrics {
    /// Create new atomic metrics.
    pub fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            negative_hits: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            expirations: AtomicU64::new(0),
            background_refreshes: AtomicU64::new(0),
        }
    }

    /// Record a hit.
    #[inline]
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a miss.
    #[inline]
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a negative hit.
    #[inline]
    pub fn record_negative_hit(&self) {
        self.negative_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an eviction.
    #[inline]
    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an expiration.
    #[inline]
    pub fn record_expiration(&self) {
        self.expirations.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a background refresh.
    #[inline]
    pub fn record_background_refresh(&self) {
        self.background_refreshes.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of the metrics.
    pub fn snapshot(&self, size: usize) -> CacheMetrics {
        CacheMetrics {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            negative_hits: self.negative_hits.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            expirations: self.expirations.load(Ordering::Relaxed),
            background_refreshes: self.background_refreshes.load(Ordering::Relaxed),
            size,
        }
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.negative_hits.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.expirations.store(0, Ordering::Relaxed);
        self.background_refreshes.store(0, Ordering::Relaxed);
    }
}

/// Result of a cache lookup.
#[derive(Debug, Clone)]
pub enum CacheLookup {
    /// Found valid entry.
    Hit(TenantContext),
    /// Found negative entry (tenant doesn't exist).
    NegativeHit,
    /// Entry not found or expired.
    Miss,
    /// Entry found but should be refreshed.
    Stale(TenantContext),
}

/// High-performance tenant cache.
pub struct TenantCache {
    config: CacheConfig,
    entries: RwLock<HashMap<String, CacheEntry>>,
    metrics: AtomicCacheMetrics,
}

impl TenantCache {
    /// Create a new tenant cache with the given config.
    pub fn new(config: CacheConfig) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(config.max_entries)),
            config,
            metrics: AtomicCacheMetrics::new(),
        }
    }

    /// Create with default config.
    pub fn default_config() -> Self {
        Self::new(CacheConfig::default())
    }

    /// Get the cache config.
    pub fn config(&self) -> &CacheConfig {
        &self.config
    }

    /// Look up a tenant in the cache.
    pub fn lookup(&self, tenant_id: &TenantId) -> CacheLookup {
        let key = tenant_id.as_str();

        let entries = self.entries.read();
        match entries.get(key) {
            Some(entry) => {
                if entry.is_expired() {
                    self.metrics.record_expiration();
                    CacheLookup::Miss
                } else if entry.context.is_none() {
                    self.metrics.record_negative_hit();
                    CacheLookup::NegativeHit
                } else if self.config.background_refresh
                    && entry.should_refresh(self.config.refresh_threshold)
                {
                    self.metrics.record_hit();
                    CacheLookup::Stale(entry.context.clone().unwrap())
                } else {
                    self.metrics.record_hit();
                    CacheLookup::Hit(entry.context.clone().unwrap())
                }
            }
            None => {
                self.metrics.record_miss();
                CacheLookup::Miss
            }
        }
    }

    /// Insert a tenant into the cache.
    pub fn insert(&self, tenant_id: TenantId, context: TenantContext) {
        let key = tenant_id.as_str().to_string();
        let entry = CacheEntry::positive(context, self.config.ttl);

        let mut entries = self.entries.write();

        // Check capacity and evict if necessary
        if entries.len() >= self.config.max_entries && !entries.contains_key(&key) {
            self.evict_one(&mut entries);
        }

        entries.insert(key, entry);
    }

    /// Insert a negative entry (tenant not found).
    pub fn insert_negative(&self, tenant_id: TenantId) {
        let key = tenant_id.as_str().to_string();
        let entry = CacheEntry::negative(self.config.negative_ttl);

        let mut entries = self.entries.write();

        if entries.len() >= self.config.max_entries && !entries.contains_key(&key) {
            self.evict_one(&mut entries);
        }

        entries.insert(key, entry);
    }

    /// Invalidate a specific tenant.
    pub fn invalidate(&self, tenant_id: &TenantId) {
        self.entries.write().remove(tenant_id.as_str());
    }

    /// Invalidate all tenants matching a predicate.
    pub fn invalidate_if<F>(&self, predicate: F)
    where
        F: Fn(&str, &TenantContext) -> bool,
    {
        let mut entries = self.entries.write();
        entries.retain(|k, v| {
            if let Some(ref ctx) = v.context {
                !predicate(k, ctx)
            } else {
                true
            }
        });
    }

    /// Clear the entire cache.
    pub fn clear(&self) {
        self.entries.write().clear();
    }

    /// Get the current cache size.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get cache metrics.
    pub fn metrics(&self) -> CacheMetrics {
        self.metrics.snapshot(self.len())
    }

    /// Reset metrics.
    pub fn reset_metrics(&self) {
        self.metrics.reset();
    }

    /// Evict expired entries.
    pub fn evict_expired(&self) -> usize {
        let mut entries = self.entries.write();
        let before = entries.len();

        entries.retain(|_, entry| !entry.is_expired());

        let evicted = before - entries.len();
        for _ in 0..evicted {
            self.metrics.record_expiration();
        }
        evicted
    }

    /// Mark an entry as refreshing (to prevent thundering herd).
    pub fn mark_refreshing(&self, tenant_id: &TenantId) -> bool {
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(tenant_id.as_str())
            && !entry.refreshing
        {
            entry.refreshing = true;
            self.metrics.record_background_refresh();
            return true;
        }
        false
    }

    /// Complete a refresh with a new context.
    pub fn complete_refresh(&self, tenant_id: TenantId, context: TenantContext) {
        let key = tenant_id.as_str().to_string();
        let entry = CacheEntry::positive(context, self.config.ttl);

        self.entries.write().insert(key, entry);
    }

    /// Get or fetch a tenant, using the cache.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        tenant_id: &TenantId,
        fetch: F,
    ) -> Option<TenantContext>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Option<TenantContext>>,
    {
        match self.lookup(tenant_id) {
            CacheLookup::Hit(ctx) => Some(ctx),
            CacheLookup::NegativeHit => None,
            CacheLookup::Stale(ctx) => {
                // Return stale data, background refresh could be triggered separately
                Some(ctx)
            }
            CacheLookup::Miss => {
                // Fetch from source
                match fetch().await {
                    Some(ctx) => {
                        self.insert(tenant_id.clone(), ctx.clone());
                        Some(ctx)
                    }
                    None => {
                        self.insert_negative(tenant_id.clone());
                        None
                    }
                }
            }
        }
    }

    /// Evict one entry (LRU).
    fn evict_one(&self, entries: &mut HashMap<String, CacheEntry>) {
        // First try to evict expired entries
        let expired_key = entries
            .iter()
            .find(|(_, e)| e.is_expired())
            .map(|(k, _)| k.clone());

        if let Some(key) = expired_key {
            entries.remove(&key);
            self.metrics.record_expiration();
            return;
        }

        // Otherwise evict least recently used (lowest access count)
        let lru_key = entries
            .iter()
            .min_by_key(|(_, e)| e.access_count)
            .map(|(k, _)| k.clone());

        if let Some(key) = lru_key {
            entries.remove(&key);
            self.metrics.record_eviction();
        }
    }
}

/// A sharded cache for high-concurrency scenarios.
///
/// Uses multiple shards to reduce lock contention under heavy load.
pub struct ShardedTenantCache {
    shards: Vec<TenantCache>,
    shard_count: usize,
}

impl ShardedTenantCache {
    /// Create a new sharded cache.
    pub fn new(shard_count: usize, config: CacheConfig) -> Self {
        let per_shard_max = config.max_entries / shard_count;
        let shard_config = CacheConfig {
            max_entries: per_shard_max.max(100),
            ..config
        };

        let shards = (0..shard_count)
            .map(|_| TenantCache::new(shard_config.clone()))
            .collect();

        Self {
            shards,
            shard_count,
        }
    }

    /// Create with reasonable defaults for high-concurrency.
    pub fn high_concurrency(max_entries: usize) -> Self {
        // Use number of CPUs for shard count
        let shard_count = num_cpus::get().max(4);
        Self::new(shard_count, CacheConfig::new(max_entries))
    }

    /// Get the shard for a tenant ID.
    fn shard(&self, tenant_id: &TenantId) -> &TenantCache {
        let hash = tenant_id
            .as_str()
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        &self.shards[(hash as usize) % self.shard_count]
    }

    /// Look up a tenant.
    pub fn lookup(&self, tenant_id: &TenantId) -> CacheLookup {
        self.shard(tenant_id).lookup(tenant_id)
    }

    /// Insert a tenant.
    pub fn insert(&self, tenant_id: TenantId, context: TenantContext) {
        self.shard(&tenant_id).insert(tenant_id, context);
    }

    /// Insert a negative entry.
    pub fn insert_negative(&self, tenant_id: TenantId) {
        self.shard(&tenant_id).insert_negative(tenant_id);
    }

    /// Invalidate a tenant.
    pub fn invalidate(&self, tenant_id: &TenantId) {
        self.shard(tenant_id).invalidate(tenant_id);
    }

    /// Clear all shards.
    pub fn clear(&self) {
        for shard in &self.shards {
            shard.clear();
        }
    }

    /// Get total size.
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.len()).sum()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(|s| s.is_empty())
    }

    /// Get aggregated metrics.
    pub fn metrics(&self) -> CacheMetrics {
        let mut total = CacheMetrics::default();
        for shard in &self.shards {
            let m = shard.metrics();
            total.hits += m.hits;
            total.misses += m.misses;
            total.negative_hits += m.negative_hits;
            total.evictions += m.evictions;
            total.expirations += m.expirations;
            total.background_refreshes += m.background_refreshes;
            total.size += m.size;
        }
        total
    }

    /// Evict expired entries from all shards.
    pub fn evict_expired(&self) -> usize {
        self.shards.iter().map(|s| s.evict_expired()).sum()
    }

    /// Get or fetch a tenant.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        tenant_id: &TenantId,
        fetch: F,
    ) -> Option<TenantContext>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Option<TenantContext>>,
    {
        self.shard(tenant_id).get_or_fetch(tenant_id, fetch).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit() {
        let cache = TenantCache::new(CacheConfig::new(100));
        let tenant_id = TenantId::new("test-tenant");
        let context = TenantContext::new(tenant_id.clone());

        cache.insert(tenant_id.clone(), context);

        match cache.lookup(&tenant_id) {
            CacheLookup::Hit(ctx) => assert_eq!(ctx.id.as_str(), "test-tenant"),
            _ => panic!("Expected hit"),
        }
    }

    #[test]
    fn test_cache_miss() {
        let cache = TenantCache::new(CacheConfig::new(100));
        let tenant_id = TenantId::new("unknown");

        match cache.lookup(&tenant_id) {
            CacheLookup::Miss => {}
            _ => panic!("Expected miss"),
        }
    }

    #[test]
    fn test_negative_cache() {
        let cache = TenantCache::new(CacheConfig::new(100));
        let tenant_id = TenantId::new("deleted-tenant");

        cache.insert_negative(tenant_id.clone());

        match cache.lookup(&tenant_id) {
            CacheLookup::NegativeHit => {}
            _ => panic!("Expected negative hit"),
        }
    }

    #[test]
    fn test_cache_eviction() {
        let cache = TenantCache::new(CacheConfig::new(2));

        for i in 0..3 {
            let id = TenantId::new(format!("tenant-{}", i));
            cache.insert(id.clone(), TenantContext::new(id));
        }

        // Should have evicted one
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_cache_metrics() {
        let cache = TenantCache::new(CacheConfig::new(100));
        let id = TenantId::new("test");

        // Miss
        cache.lookup(&id);
        assert_eq!(cache.metrics().misses, 1);

        // Insert and hit
        cache.insert(id.clone(), TenantContext::new(id.clone()));
        cache.lookup(&id);
        assert_eq!(cache.metrics().hits, 1);
    }

    #[test]
    fn test_sharded_cache() {
        let cache = ShardedTenantCache::new(4, CacheConfig::new(100));

        for i in 0..10 {
            let id = TenantId::new(format!("tenant-{}", i));
            cache.insert(id.clone(), TenantContext::new(id));
        }

        assert_eq!(cache.len(), 10);

        for i in 0..10 {
            let id = TenantId::new(format!("tenant-{}", i));
            match cache.lookup(&id) {
                CacheLookup::Hit(_) => {}
                _ => panic!("Expected hit for tenant-{}", i),
            }
        }
    }

    #[tokio::test]
    async fn test_get_or_fetch() {
        let cache = TenantCache::new(CacheConfig::new(100));
        let id = TenantId::new("fetch-tenant");

        // First call should fetch
        let result = cache
            .get_or_fetch(&id, || async { Some(TenantContext::new("fetch-tenant")) })
            .await;

        assert!(result.is_some());

        // Second call should hit cache
        let result2 = cache
            .get_or_fetch(&id, || async {
                panic!("Should not fetch again");
            })
            .await;

        assert!(result2.is_some());
    }
}
