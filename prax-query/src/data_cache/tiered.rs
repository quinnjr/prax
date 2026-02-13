//! Tiered cache combining multiple cache backends.
//!
//! This module implements a multi-level cache where:
//! - L1 (local): Fast in-memory cache for hot data
//! - L2 (distributed): Redis cache for shared state
//!
//! # Cache Flow
//!
//! ```text
//! GET request:
//! 1. Check L1 (memory) -> Hit? Return
//! 2. Check L2 (Redis) -> Hit? Populate L1, Return
//! 3. Miss -> Fetch from source, populate L1 & L2
//!
//! SET request:
//! 1. Write to L2 (Redis) first
//! 2. Write to L1 (memory)
//!
//! INVALIDATE:
//! 1. Invalidate L2 (Redis)
//! 2. Invalidate L1 (memory)
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::data_cache::{TieredCache, MemoryCache, RedisCache};
//!
//! let memory = MemoryCache::builder()
//!     .max_capacity(1000)
//!     .time_to_live(Duration::from_secs(60))
//!     .build();
//!
//! let redis = RedisCache::new(RedisCacheConfig::default()).await?;
//!
//! let cache = TieredCache::new(memory, redis);
//! ```

use std::time::Duration;

use super::backend::{BackendStats, CacheBackend, CacheResult};
use super::invalidation::EntityTag;
use super::key::{CacheKey, KeyPattern};

/// Configuration for tiered cache.
#[derive(Debug, Clone)]
pub struct TieredCacheConfig {
    /// Whether to write to L1 on L2 hit (write-through to L1).
    pub write_through_l1: bool,
    /// Whether to write to L2 on L1 write (write-through to L2).
    pub write_through_l2: bool,
    /// L1 TTL (usually shorter than L2).
    pub l1_ttl: Option<Duration>,
    /// L2 TTL.
    pub l2_ttl: Option<Duration>,
    /// Whether L1 failures should fail the operation.
    pub l1_required: bool,
    /// Whether L2 failures should fail the operation.
    pub l2_required: bool,
}

impl Default for TieredCacheConfig {
    fn default() -> Self {
        Self {
            write_through_l1: true,
            write_through_l2: true,
            l1_ttl: Some(Duration::from_secs(60)), // 1 minute L1
            l2_ttl: Some(Duration::from_secs(300)), // 5 minutes L2
            l1_required: false,
            l2_required: false,
        }
    }
}

impl TieredCacheConfig {
    /// Set L1 TTL.
    pub fn with_l1_ttl(mut self, ttl: Duration) -> Self {
        self.l1_ttl = Some(ttl);
        self
    }

    /// Set L2 TTL.
    pub fn with_l2_ttl(mut self, ttl: Duration) -> Self {
        self.l2_ttl = Some(ttl);
        self
    }

    /// Make L1 required.
    pub fn require_l1(mut self) -> Self {
        self.l1_required = true;
        self
    }

    /// Make L2 required.
    pub fn require_l2(mut self) -> Self {
        self.l2_required = true;
        self
    }

    /// Disable write-through to L1.
    pub fn no_write_l1(mut self) -> Self {
        self.write_through_l1 = false;
        self
    }

    /// Disable write-through to L2.
    pub fn no_write_l2(mut self) -> Self {
        self.write_through_l2 = false;
        self
    }
}

/// A tiered cache with L1 (local) and L2 (distributed) layers.
pub struct TieredCache<L1, L2>
where
    L1: CacheBackend,
    L2: CacheBackend,
{
    l1: L1,
    l2: L2,
    config: TieredCacheConfig,
}

impl<L1, L2> TieredCache<L1, L2>
where
    L1: CacheBackend,
    L2: CacheBackend,
{
    /// Create a new tiered cache with default config.
    pub fn new(l1: L1, l2: L2) -> Self {
        Self {
            l1,
            l2,
            config: TieredCacheConfig::default(),
        }
    }

    /// Create with custom config.
    pub fn with_config(l1: L1, l2: L2, config: TieredCacheConfig) -> Self {
        Self { l1, l2, config }
    }

    /// Get the L1 cache.
    pub fn l1(&self) -> &L1 {
        &self.l1
    }

    /// Get the L2 cache.
    pub fn l2(&self) -> &L2 {
        &self.l2
    }

    /// Get the config.
    pub fn config(&self) -> &TieredCacheConfig {
        &self.config
    }
}

impl<L1, L2> CacheBackend for TieredCache<L1, L2>
where
    L1: CacheBackend,
    L2: CacheBackend,
{
    async fn get<T>(&self, key: &CacheKey) -> CacheResult<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        // Try L1 first
        match self.l1.get::<T>(key).await {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {} // Continue to L2
            Err(e) if self.config.l1_required => return Err(e),
            Err(_) => {} // L1 error but not required, continue
        }

        // Try L2
        match self.l2.get::<T>(key).await {
            Ok(Some(value)) => {
                // Note: We can't populate L1 here because T isn't guaranteed to be Serialize
                // The caller should use get_and_populate if they want L1 population
                Ok(Some(value))
            }
            Ok(None) => Ok(None),
            Err(e) if self.config.l2_required => Err(e),
            Err(_) => Ok(None),
        }
    }

    async fn set<T>(&self, key: &CacheKey, value: &T, ttl: Option<Duration>) -> CacheResult<()>
    where
        T: serde::Serialize + Sync,
    {
        // Write to L2 first (source of truth for distributed)
        if self.config.write_through_l2 {
            let l2_ttl = ttl.or(self.config.l2_ttl);
            match self.l2.set(key, value, l2_ttl).await {
                Ok(()) => {}
                Err(e) if self.config.l2_required => return Err(e),
                Err(_) => {} // Log but continue
            }
        }

        // Write to L1
        if self.config.write_through_l1 {
            let l1_ttl = ttl
                .map(|t| t.min(self.config.l1_ttl.unwrap_or(t)))
                .or(self.config.l1_ttl);

            match self.l1.set(key, value, l1_ttl).await {
                Ok(()) => {}
                Err(e) if self.config.l1_required => return Err(e),
                Err(_) => {} // Log but continue
            }
        }

        Ok(())
    }

    async fn delete(&self, key: &CacheKey) -> CacheResult<bool> {
        // Delete from both layers
        let l2_deleted = match self.l2.delete(key).await {
            Ok(deleted) => deleted,
            Err(e) if self.config.l2_required => return Err(e),
            Err(_) => false,
        };

        let l1_deleted = match self.l1.delete(key).await {
            Ok(deleted) => deleted,
            Err(e) if self.config.l1_required => return Err(e),
            Err(_) => false,
        };

        Ok(l1_deleted || l2_deleted)
    }

    async fn exists(&self, key: &CacheKey) -> CacheResult<bool> {
        // Check L1 first
        if let Ok(true) = self.l1.exists(key).await {
            return Ok(true);
        }

        // Check L2
        self.l2.exists(key).await
    }

    // Note: get_many uses the default sequential implementation
    // A more optimized version would batch L1/L2 lookups but requires complex trait bounds

    async fn invalidate_pattern(&self, pattern: &KeyPattern) -> CacheResult<u64> {
        // Invalidate both layers
        let l2_count = self.l2.invalidate_pattern(pattern).await.unwrap_or(0);
        let l1_count = self.l1.invalidate_pattern(pattern).await.unwrap_or(0);

        Ok(l1_count.max(l2_count))
    }

    async fn invalidate_tags(&self, tags: &[EntityTag]) -> CacheResult<u64> {
        let l2_count = self.l2.invalidate_tags(tags).await.unwrap_or(0);
        let l1_count = self.l1.invalidate_tags(tags).await.unwrap_or(0);

        Ok(l1_count.max(l2_count))
    }

    async fn clear(&self) -> CacheResult<()> {
        // Clear both layers
        let l2_result = self.l2.clear().await;
        let l1_result = self.l1.clear().await;

        // Return first error if any layer is required
        if self.config.l2_required {
            l2_result?;
        }
        if self.config.l1_required {
            l1_result?;
        }

        Ok(())
    }

    async fn len(&self) -> CacheResult<usize> {
        // Return L2 size as it's the source of truth
        self.l2.len().await
    }

    async fn stats(&self) -> CacheResult<BackendStats> {
        let l1_stats = self.l1.stats().await.unwrap_or_default();
        let l2_stats = self.l2.stats().await.unwrap_or_default();

        Ok(BackendStats {
            entries: l2_stats.entries,           // L2 is source of truth
            memory_bytes: l1_stats.memory_bytes, // L1 memory usage
            connections: l2_stats.connections,   // L2 connections
            info: Some(format!(
                "Tiered: L1={} entries, L2={} entries",
                l1_stats.entries, l2_stats.entries
            )),
        })
    }
}

/// Builder for tiered cache.
pub struct TieredCacheBuilder<L1, L2>
where
    L1: CacheBackend,
    L2: CacheBackend,
{
    l1: Option<L1>,
    l2: Option<L2>,
    config: TieredCacheConfig,
}

impl<L1, L2> Default for TieredCacheBuilder<L1, L2>
where
    L1: CacheBackend,
    L2: CacheBackend,
{
    fn default() -> Self {
        Self {
            l1: None,
            l2: None,
            config: TieredCacheConfig::default(),
        }
    }
}

impl<L1, L2> TieredCacheBuilder<L1, L2>
where
    L1: CacheBackend,
    L2: CacheBackend,
{
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the L1 cache.
    pub fn l1(mut self, cache: L1) -> Self {
        self.l1 = Some(cache);
        self
    }

    /// Set the L2 cache.
    pub fn l2(mut self, cache: L2) -> Self {
        self.l2 = Some(cache);
        self
    }

    /// Set the config.
    pub fn config(mut self, config: TieredCacheConfig) -> Self {
        self.config = config;
        self
    }

    /// Set L1 TTL.
    pub fn l1_ttl(mut self, ttl: Duration) -> Self {
        self.config.l1_ttl = Some(ttl);
        self
    }

    /// Set L2 TTL.
    pub fn l2_ttl(mut self, ttl: Duration) -> Self {
        self.config.l2_ttl = Some(ttl);
        self
    }

    /// Build the tiered cache.
    ///
    /// # Panics
    /// Panics if L1 or L2 is not set.
    pub fn build(self) -> TieredCache<L1, L2> {
        TieredCache {
            l1: self.l1.expect("L1 cache must be set"),
            l2: self.l2.expect("L2 cache must be set"),
            config: self.config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::backend::NoopCache;
    use super::super::memory::{MemoryCache, MemoryCacheConfig};
    use super::*;

    #[tokio::test]
    async fn test_tiered_cache_l1_hit() {
        let l1 = MemoryCache::new(MemoryCacheConfig::new(100));
        let l2 = MemoryCache::new(MemoryCacheConfig::new(100));

        let cache = TieredCache::new(l1, l2);
        let key = CacheKey::new("test", "key1");

        // Set value
        cache.set(&key, &"hello", None).await.unwrap();

        // Should hit L1
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert_eq!(value, Some("hello".to_string()));
    }

    #[tokio::test]
    async fn test_tiered_cache_l2_fallback() {
        let l1 = MemoryCache::new(MemoryCacheConfig::new(100));
        let l2 = MemoryCache::new(MemoryCacheConfig::new(100));

        // Only set in L2
        let key = CacheKey::new("test", "key1");
        l2.set(&key, &"from l2", None).await.unwrap();

        let cache = TieredCache::with_config(
            l1,
            l2,
            TieredCacheConfig {
                write_through_l1: true,
                ..Default::default()
            },
        );

        // Should get from L2
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert_eq!(value, Some("from l2".to_string()));

        // Note: L1 population on L2 hit would require T: Serialize
        // Use the set method to populate both caches explicitly
    }

    #[tokio::test]
    async fn test_tiered_cache_invalidation() {
        let l1 = MemoryCache::new(MemoryCacheConfig::new(100));
        let l2 = MemoryCache::new(MemoryCacheConfig::new(100));

        let cache = TieredCache::new(l1, l2);
        let key = CacheKey::new("User", "id:1");

        // Set value
        cache.set(&key, &"user data", None).await.unwrap();

        // Invalidate by pattern
        let count = cache
            .invalidate_pattern(&KeyPattern::entity("User"))
            .await
            .unwrap();

        assert!(count >= 1);

        // Should be gone
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn test_tiered_cache_with_noop_l2() {
        let l1 = MemoryCache::new(MemoryCacheConfig::new(100));
        let l2 = NoopCache;

        let cache = TieredCache::new(l1, l2);
        let key = CacheKey::new("test", "key1");

        // Set should still work (L1 only)
        cache.set(&key, &"hello", None).await.unwrap();

        // Should get from L1
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert_eq!(value, Some("hello".to_string()));
    }
}
