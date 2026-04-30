//! Cache backend trait and core types.

use super::invalidation::EntityTag;
use super::key::{CacheKey, KeyPattern};
use std::future::Future;
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during cache operations.
#[derive(Error, Debug)]
pub enum CacheError {
    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Deserialization error.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Connection error.
    #[error("connection error: {0}")]
    Connection(String),

    /// Operation timeout.
    #[error("operation timed out")]
    Timeout,

    /// Key not found.
    #[error("key not found: {0}")]
    NotFound(String),

    /// Backend-specific error.
    #[error("backend error: {0}")]
    Backend(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

/// Result type for cache operations.
pub type CacheResult<T> = Result<T, CacheError>;

/// A cached entry with metadata.
#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    /// The cached value.
    pub value: T,
    /// When the entry was created.
    pub created_at: std::time::Instant,
    /// Time-to-live for this entry.
    pub ttl: Option<Duration>,
    /// Tags associated with this entry.
    pub tags: Vec<EntityTag>,
    /// Size in bytes (if known).
    pub size_bytes: Option<usize>,
}

impl<T> CacheEntry<T> {
    /// Create a new cache entry.
    pub fn new(value: T) -> Self {
        Self {
            value,
            created_at: std::time::Instant::now(),
            ttl: None,
            tags: Vec::new(),
            size_bytes: None,
        }
    }

    /// Set the TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Set tags.
    pub fn with_tags(mut self, tags: Vec<EntityTag>) -> Self {
        self.tags = tags;
        self
    }

    /// Set size.
    pub fn with_size(mut self, size: usize) -> Self {
        self.size_bytes = Some(size);
        self
    }

    /// Check if the entry is expired.
    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.ttl {
            self.created_at.elapsed() >= ttl
        } else {
            false
        }
    }

    /// Get remaining TTL.
    pub fn remaining_ttl(&self) -> Option<Duration> {
        self.ttl
            .map(|ttl| ttl.saturating_sub(self.created_at.elapsed()))
    }
}

/// The core trait for cache backends.
///
/// This trait defines the interface that all cache backends must implement.
/// It supports both synchronous and asynchronous operations.
pub trait CacheBackend: Send + Sync + 'static {
    /// Get a value from the cache.
    fn get<T>(&self, key: &CacheKey) -> impl Future<Output = CacheResult<Option<T>>> + Send
    where
        T: serde::de::DeserializeOwned;

    /// Set a value in the cache.
    fn set<T>(
        &self,
        key: &CacheKey,
        value: &T,
        ttl: Option<Duration>,
    ) -> impl Future<Output = CacheResult<()>> + Send
    where
        T: serde::Serialize + Sync;

    /// Delete a value from the cache.
    fn delete(&self, key: &CacheKey) -> impl Future<Output = CacheResult<bool>> + Send;

    /// Check if a key exists.
    fn exists(&self, key: &CacheKey) -> impl Future<Output = CacheResult<bool>> + Send;

    /// Get multiple values at once.
    ///
    /// Default implementation fetches sequentially. Override for batch optimization.
    fn get_many<T>(
        &self,
        keys: &[CacheKey],
    ) -> impl Future<Output = CacheResult<Vec<Option<T>>>> + Send
    where
        T: serde::de::DeserializeOwned + Send,
    {
        async move {
            let mut results = Vec::with_capacity(keys.len());
            for key in keys {
                results.push(self.get::<T>(key).await?);
            }
            Ok(results)
        }
    }

    /// Set multiple values at once.
    ///
    /// Default implementation sets sequentially. Override for batch optimization.
    fn set_many<T>(
        &self,
        entries: &[(&CacheKey, &T)],
        ttl: Option<Duration>,
    ) -> impl Future<Output = CacheResult<()>> + Send
    where
        T: serde::Serialize + Sync + Send,
    {
        async move {
            for (key, value) in entries {
                self.set(key, *value, ttl).await?;
            }
            Ok(())
        }
    }

    /// Delete multiple keys at once.
    ///
    /// Default implementation deletes sequentially. Override for batch optimization.
    fn delete_many(&self, keys: &[CacheKey]) -> impl Future<Output = CacheResult<u64>> + Send {
        async move {
            let mut count = 0u64;
            for key in keys {
                if self.delete(key).await? {
                    count += 1;
                }
            }
            Ok(count)
        }
    }

    /// Invalidate entries matching a pattern.
    fn invalidate_pattern(
        &self,
        pattern: &KeyPattern,
    ) -> impl Future<Output = CacheResult<u64>> + Send;

    /// Invalidate entries by tags.
    fn invalidate_tags(&self, tags: &[EntityTag]) -> impl Future<Output = CacheResult<u64>> + Send;

    /// Clear all entries.
    fn clear(&self) -> impl Future<Output = CacheResult<()>> + Send;

    /// Get the approximate number of entries.
    fn len(&self) -> impl Future<Output = CacheResult<usize>> + Send;

    /// Check if the cache is empty.
    fn is_empty(&self) -> impl Future<Output = CacheResult<bool>> + Send {
        async move { Ok(self.len().await? == 0) }
    }

    /// Get cache statistics if available.
    fn stats(&self) -> impl Future<Output = CacheResult<BackendStats>> + Send {
        async move {
            Ok(BackendStats {
                entries: self.len().await?,
                ..Default::default()
            })
        }
    }
}

/// Statistics from a cache backend.
#[derive(Debug, Clone, Default)]
pub struct BackendStats {
    /// Number of entries.
    pub entries: usize,
    /// Memory usage in bytes.
    pub memory_bytes: Option<usize>,
    /// Number of connections (for Redis).
    pub connections: Option<usize>,
    /// Backend-specific info.
    pub info: Option<String>,
}

/// A no-op cache backend that doesn't cache anything.
///
/// Useful for testing or when caching should be disabled.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopCache;

impl CacheBackend for NoopCache {
    async fn get<T>(&self, _key: &CacheKey) -> CacheResult<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        Ok(None)
    }

    async fn set<T>(&self, _key: &CacheKey, _value: &T, _ttl: Option<Duration>) -> CacheResult<()>
    where
        T: serde::Serialize + Sync,
    {
        Ok(())
    }

    async fn delete(&self, _key: &CacheKey) -> CacheResult<bool> {
        Ok(false)
    }

    async fn exists(&self, _key: &CacheKey) -> CacheResult<bool> {
        Ok(false)
    }

    async fn invalidate_pattern(&self, _pattern: &KeyPattern) -> CacheResult<u64> {
        Ok(0)
    }

    async fn invalidate_tags(&self, _tags: &[EntityTag]) -> CacheResult<u64> {
        Ok(0)
    }

    async fn clear(&self) -> CacheResult<()> {
        Ok(())
    }

    async fn len(&self) -> CacheResult<usize> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry() {
        let entry = CacheEntry::new("test value")
            .with_ttl(Duration::from_secs(60))
            .with_tags(vec![EntityTag::new("User")]);

        assert!(!entry.is_expired());
        assert!(entry.remaining_ttl().unwrap() > Duration::from_secs(59));
    }

    #[tokio::test]
    async fn test_noop_cache() {
        let cache = NoopCache;

        // Set should succeed but not store
        cache
            .set(&CacheKey::new("test", "key"), &"value", None)
            .await
            .unwrap();

        // Get should return None
        let result: Option<String> = cache.get(&CacheKey::new("test", "key")).await.unwrap();
        assert!(result.is_none());
    }
}
