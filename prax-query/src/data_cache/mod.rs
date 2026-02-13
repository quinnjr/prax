//! High-performance data caching layer for Prax ORM.
//!
//! This module provides a flexible, multi-tier caching system for query results
//! with support for:
//!
//! - **In-memory caching** using [moka](https://github.com/moka-rs/moka) for
//!   high-performance concurrent access
//! - **Redis caching** for distributed cache across multiple instances
//! - **Tiered caching** combining L1 (memory) and L2 (Redis) for optimal performance
//! - **Automatic invalidation** based on TTL, entity changes, or custom patterns
//! - **Cache-aside pattern** with transparent integration into queries
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        Application                               │
//! └─────────────────────────────────────────────────────────────────┘
//!                                │
//!                                ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     Prax Query Builder                          │
//! │                  .cache(CacheOptions::new())                    │
//! └─────────────────────────────────────────────────────────────────┘
//!                                │
//!                                ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Cache Manager                               │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐ │
//! │  │ L1: Memory  │ -> │ L2: Redis   │ -> │   Database          │ │
//! │  │ (< 1ms)     │    │ (1-5ms)     │    │   (10-100ms)        │ │
//! │  └─────────────┘    └─────────────┘    └─────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use prax_query::data_cache::{CacheManager, MemoryCache, RedisCache, TieredCache};
//! use std::time::Duration;
//!
//! // In-memory only (single instance)
//! let cache = MemoryCache::builder()
//!     .max_capacity(10_000)
//!     .time_to_live(Duration::from_secs(300))
//!     .build();
//!
//! // Redis only (distributed)
//! let redis = RedisCache::new("redis://localhost:6379").await?;
//!
//! // Tiered: Memory (L1) + Redis (L2)
//! let tiered = TieredCache::new(cache, redis);
//!
//! // Use with queries
//! let users = client
//!     .user()
//!     .find_many()
//!     .cache(CacheOptions::ttl(Duration::from_secs(60)))
//!     .exec()
//!     .await?;
//! ```
//!
//! # Cache Invalidation
//!
//! ```rust,ignore
//! use prax_query::data_cache::{InvalidationStrategy, EntityTag};
//!
//! // Invalidate by entity type
//! cache.invalidate_entity("User").await?;
//!
//! // Invalidate by specific record
//! cache.invalidate_record("User", &user_id).await?;
//!
//! // Invalidate by pattern
//! cache.invalidate_pattern("user:*:profile").await?;
//!
//! // Tag-based invalidation
//! cache.invalidate_tags(&[EntityTag::new("User"), EntityTag::new("tenant:123")]).await?;
//! ```
//!
//! # Performance Characteristics
//!
//! | Backend | Latency | Capacity | Distribution | Best For |
//! |---------|---------|----------|--------------|----------|
//! | Memory | < 1ms | Limited by RAM | Single instance | Hot data, sessions |
//! | Redis | 1-5ms | Large | Multi-instance | Shared state, large datasets |
//! | Tiered | < 1ms (L1 hit) | Both | Multi-instance | Production systems |

mod backend;
mod invalidation;
mod key;
mod memory;
mod options;
mod redis;
mod stats;
mod tiered;

pub use backend::{CacheBackend, CacheEntry, CacheError, CacheResult};
pub use invalidation::{EntityTag, InvalidationEvent, InvalidationStrategy};
pub use key::{CacheKey, CacheKeyBuilder, KeyPattern};
pub use memory::{MemoryCache, MemoryCacheBuilder, MemoryCacheConfig};
pub use options::{CacheOptions, CachePolicy, WritePolicy};
pub use redis::{RedisCache, RedisCacheConfig, RedisConnection};
pub use stats::{CacheMetrics, CacheStats};
pub use tiered::{TieredCache, TieredCacheConfig};

use std::sync::Arc;

/// The main cache manager that coordinates caching operations.
///
/// This is the primary entry point for the caching system. It wraps any
/// `CacheBackend` implementation and provides a unified API.
#[derive(Clone)]
pub struct CacheManager<B: CacheBackend> {
    backend: Arc<B>,
    default_options: CacheOptions,
    metrics: Arc<CacheMetrics>,
}

impl<B: CacheBackend> CacheManager<B> {
    /// Create a new cache manager with the given backend.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            default_options: CacheOptions::default(),
            metrics: Arc::new(CacheMetrics::new()),
        }
    }

    /// Create with custom default options.
    pub fn with_options(backend: B, options: CacheOptions) -> Self {
        Self {
            backend: Arc::new(backend),
            default_options: options,
            metrics: Arc::new(CacheMetrics::new()),
        }
    }

    /// Get the cache backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Get the metrics collector.
    pub fn metrics(&self) -> &CacheMetrics {
        &self.metrics
    }

    /// Get a value from the cache.
    pub async fn get<T>(&self, key: &CacheKey) -> CacheResult<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let start = std::time::Instant::now();
        let result = self.backend.get(key).await;
        let duration = start.elapsed();

        match &result {
            Ok(Some(_)) => self.metrics.record_hit(duration),
            Ok(None) => self.metrics.record_miss(duration),
            Err(_) => self.metrics.record_error(),
        }

        result
    }

    /// Set a value in the cache.
    pub async fn set<T>(
        &self,
        key: &CacheKey,
        value: &T,
        options: Option<&CacheOptions>,
    ) -> CacheResult<()>
    where
        T: serde::Serialize + Sync,
    {
        let opts = options.unwrap_or(&self.default_options);
        let start = std::time::Instant::now();
        let result = self.backend.set(key, value, opts.ttl).await;
        let duration = start.elapsed();

        if result.is_ok() {
            self.metrics.record_write(duration);
        } else {
            self.metrics.record_error();
        }

        result
    }

    /// Get or compute a value.
    ///
    /// If the value exists in cache, returns it. Otherwise, calls the
    /// provided function to compute the value, caches it, and returns it.
    pub async fn get_or_set<T, F, Fut>(
        &self,
        key: &CacheKey,
        f: F,
        options: Option<&CacheOptions>,
    ) -> CacheResult<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Sync,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = CacheResult<T>>,
    {
        // Try to get from cache first
        if let Some(value) = self.get::<T>(key).await? {
            return Ok(value);
        }

        // Compute the value
        let value = f().await?;

        // Store in cache (ignore errors - cache is best-effort)
        let _ = self.set(key, &value, options).await;

        Ok(value)
    }

    /// Delete a value from the cache.
    pub async fn delete(&self, key: &CacheKey) -> CacheResult<bool> {
        self.backend.delete(key).await
    }

    /// Check if a key exists in the cache.
    pub async fn exists(&self, key: &CacheKey) -> CacheResult<bool> {
        self.backend.exists(key).await
    }

    /// Invalidate cache entries by pattern.
    pub async fn invalidate_pattern(&self, pattern: &KeyPattern) -> CacheResult<u64> {
        self.backend.invalidate_pattern(pattern).await
    }

    /// Invalidate all entries for an entity type.
    pub async fn invalidate_entity(&self, entity: &str) -> CacheResult<u64> {
        let pattern = KeyPattern::entity(entity);
        self.invalidate_pattern(&pattern).await
    }

    /// Invalidate a specific record.
    pub async fn invalidate_record<I: std::fmt::Display>(
        &self,
        entity: &str,
        id: I,
    ) -> CacheResult<u64> {
        let pattern = KeyPattern::record(entity, id);
        self.invalidate_pattern(&pattern).await
    }

    /// Invalidate entries by tags.
    pub async fn invalidate_tags(&self, tags: &[EntityTag]) -> CacheResult<u64> {
        self.backend.invalidate_tags(tags).await
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) -> CacheResult<()> {
        self.backend.clear().await
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        self.metrics.snapshot()
    }
}

/// Builder for creating cache managers with different configurations.
pub struct CacheManagerBuilder {
    default_options: CacheOptions,
}

impl Default for CacheManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheManagerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            default_options: CacheOptions::default(),
        }
    }

    /// Set default cache options.
    pub fn default_options(mut self, options: CacheOptions) -> Self {
        self.default_options = options;
        self
    }

    /// Build a cache manager with an in-memory backend.
    pub fn memory(self, config: MemoryCacheConfig) -> CacheManager<MemoryCache> {
        let backend = MemoryCache::new(config);
        CacheManager::with_options(backend, self.default_options)
    }

    /// Build a cache manager with a Redis backend.
    pub async fn redis(self, config: RedisCacheConfig) -> CacheResult<CacheManager<RedisCache>> {
        let backend = RedisCache::new(config).await?;
        Ok(CacheManager::with_options(backend, self.default_options))
    }

    /// Build a cache manager with a tiered backend.
    pub async fn tiered(
        self,
        memory_config: MemoryCacheConfig,
        redis_config: RedisCacheConfig,
    ) -> CacheResult<CacheManager<TieredCache<MemoryCache, RedisCache>>> {
        let memory = MemoryCache::new(memory_config);
        let redis = RedisCache::new(redis_config).await?;
        let backend = TieredCache::new(memory, redis);
        Ok(CacheManager::with_options(backend, self.default_options))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_memory_cache_basic() {
        let cache = CacheManager::new(MemoryCache::new(MemoryCacheConfig::default()));

        let key = CacheKey::new("test", "key1");

        // Set a value
        cache.set(&key, &"hello world", None).await.unwrap();

        // Get it back
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert_eq!(value, Some("hello world".to_string()));

        // Delete it
        cache.delete(&key).await.unwrap();

        // Should be gone
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn test_get_or_set() {
        let cache = CacheManager::new(MemoryCache::new(MemoryCacheConfig::default()));

        let key = CacheKey::new("test", "computed");
        let mut call_count = 0;

        // First call should compute
        let value: String = cache
            .get_or_set(
                &key,
                || {
                    call_count += 1;
                    async { Ok("computed value".to_string()) }
                },
                None,
            )
            .await
            .unwrap();

        assert_eq!(value, "computed value");
        assert_eq!(call_count, 1);

        // Second call should use cache
        let value: String = cache
            .get_or_set(
                &key,
                || {
                    call_count += 1;
                    async { Ok("should not be called".to_string()) }
                },
                None,
            )
            .await
            .unwrap();

        assert_eq!(value, "computed value");
        assert_eq!(call_count, 1); // Not incremented
    }
}
