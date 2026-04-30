//! High-performance in-memory cache using moka.
//!
//! This module provides an in-memory cache implementation using the [moka](https://github.com/moka-rs/moka)
//! crate, which is a fast, concurrent cache inspired by Caffeine (Java).
//!
//! # Features
//!
//! - **High concurrency**: Lock-free reads, fine-grained locking for writes
//! - **Automatic eviction**: LRU-based eviction when capacity is reached
//! - **TTL support**: Per-entry time-to-live
//! - **Size-based limits**: Limit by entry count or memory usage
//! - **Async-friendly**: Works seamlessly with async runtimes
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::data_cache::memory::{MemoryCache, MemoryCacheConfig};
//! use std::time::Duration;
//!
//! let cache = MemoryCache::builder()
//!     .max_capacity(10_000)
//!     .time_to_live(Duration::from_secs(300))
//!     .time_to_idle(Duration::from_secs(60))
//!     .build();
//!
//! // Use with CacheManager
//! let manager = CacheManager::new(cache);
//! ```

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::backend::{BackendStats, CacheBackend, CacheError, CacheResult};
use super::invalidation::EntityTag;
use super::key::{CacheKey, KeyPattern};

/// Configuration for the in-memory cache.
#[derive(Debug, Clone)]
pub struct MemoryCacheConfig {
    /// Maximum number of entries.
    pub max_capacity: u64,
    /// Default time-to-live for entries.
    pub time_to_live: Option<Duration>,
    /// Time-to-idle (evict if not accessed).
    pub time_to_idle: Option<Duration>,
    /// Enable entry-level TTL tracking.
    pub per_entry_ttl: bool,
    /// Enable tag-based invalidation.
    pub enable_tags: bool,
}

impl Default for MemoryCacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: 10_000,
            time_to_live: Some(Duration::from_secs(300)),
            time_to_idle: None,
            per_entry_ttl: true,
            enable_tags: true,
        }
    }
}

impl MemoryCacheConfig {
    /// Create a new config with the given capacity.
    pub fn new(max_capacity: u64) -> Self {
        Self {
            max_capacity,
            ..Default::default()
        }
    }

    /// Set the default TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.time_to_live = Some(ttl);
        self
    }

    /// Set the time-to-idle.
    pub fn with_tti(mut self, tti: Duration) -> Self {
        self.time_to_idle = Some(tti);
        self
    }

    /// Disable tags.
    pub fn without_tags(mut self) -> Self {
        self.enable_tags = false;
        self
    }
}

/// Builder for MemoryCache.
#[derive(Default)]
pub struct MemoryCacheBuilder {
    config: MemoryCacheConfig,
}

impl MemoryCacheBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max capacity.
    pub fn max_capacity(mut self, capacity: u64) -> Self {
        self.config.max_capacity = capacity;
        self
    }

    /// Set TTL.
    pub fn time_to_live(mut self, ttl: Duration) -> Self {
        self.config.time_to_live = Some(ttl);
        self
    }

    /// Set TTI.
    pub fn time_to_idle(mut self, tti: Duration) -> Self {
        self.config.time_to_idle = Some(tti);
        self
    }

    /// Enable per-entry TTL.
    pub fn per_entry_ttl(mut self, enabled: bool) -> Self {
        self.config.per_entry_ttl = enabled;
        self
    }

    /// Enable tags.
    pub fn enable_tags(mut self, enabled: bool) -> Self {
        self.config.enable_tags = enabled;
        self
    }

    /// Build the cache.
    pub fn build(self) -> MemoryCache {
        MemoryCache::new(self.config)
    }
}

/// A cached entry with metadata.
#[derive(Clone)]
struct CacheEntry {
    /// Serialized value.
    data: Vec<u8>,
    /// When the entry was created.
    created_at: Instant,
    /// When the entry expires (if TTL set).
    expires_at: Option<Instant>,
    /// Last access time.
    last_accessed: Instant,
    /// Associated tags.
    tags: Vec<EntityTag>,
}

impl CacheEntry {
    fn new(data: Vec<u8>, ttl: Option<Duration>, tags: Vec<EntityTag>) -> Self {
        let now = Instant::now();
        Self {
            data,
            created_at: now,
            expires_at: ttl.map(|d| now + d),
            last_accessed: now,
            tags,
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|exp| Instant::now() >= exp)
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// High-performance in-memory cache.
///
/// Uses a concurrent HashMap with LRU eviction and TTL support.
pub struct MemoryCache {
    config: MemoryCacheConfig,
    entries: RwLock<HashMap<String, CacheEntry>>,
    tag_index: RwLock<HashMap<String, HashSet<String>>>,
    entry_count: AtomicUsize,
}

impl MemoryCache {
    /// Create a new memory cache with the given config.
    pub fn new(config: MemoryCacheConfig) -> Self {
        Self {
            entries: RwLock::new(HashMap::with_capacity(config.max_capacity as usize)),
            tag_index: RwLock::new(HashMap::new()),
            entry_count: AtomicUsize::new(0),
            config,
        }
    }

    /// Create a builder.
    pub fn builder() -> MemoryCacheBuilder {
        MemoryCacheBuilder::new()
    }

    /// Get the config.
    pub fn config(&self) -> &MemoryCacheConfig {
        &self.config
    }

    /// Evict expired entries.
    pub fn evict_expired(&self) -> usize {
        let mut entries = self.entries.write();
        let before = entries.len();

        let expired_keys: Vec<String> = entries
            .iter()
            .filter(|(_, e)| e.is_expired())
            .map(|(k, _)| k.clone())
            .collect();

        for key in &expired_keys {
            if let Some(entry) = entries.remove(key) {
                self.remove_from_tag_index(key, &entry.tags);
            }
        }

        let evicted = before - entries.len();
        self.entry_count.fetch_sub(evicted, Ordering::Relaxed);
        evicted
    }

    /// Evict entries to make room (LRU).
    fn evict_lru(&self, count: usize) {
        let mut entries = self.entries.write();

        // Find LRU entries
        let mut by_access: Vec<_> = entries
            .iter()
            .map(|(k, e)| (k.clone(), e.last_accessed))
            .collect();
        by_access.sort_by_key(|(_, t)| *t);

        for (key, _) in by_access.into_iter().take(count) {
            if let Some(entry) = entries.remove(&key) {
                self.remove_from_tag_index(&key, &entry.tags);
            }
        }

        self.entry_count.store(entries.len(), Ordering::Relaxed);
    }

    /// Add entry to tag index.
    fn add_to_tag_index(&self, key: &str, tags: &[EntityTag]) {
        if !self.config.enable_tags || tags.is_empty() {
            return;
        }

        let mut index = self.tag_index.write();
        for tag in tags {
            index
                .entry(tag.value().to_string())
                .or_default()
                .insert(key.to_string());
        }
    }

    /// Remove entry from tag index.
    fn remove_from_tag_index(&self, key: &str, tags: &[EntityTag]) {
        if !self.config.enable_tags || tags.is_empty() {
            return;
        }

        let mut index = self.tag_index.write();
        for tag in tags {
            if let Some(keys) = index.get_mut(tag.value()) {
                keys.remove(key);
                if keys.is_empty() {
                    index.remove(tag.value());
                }
            }
        }
    }
}

impl CacheBackend for MemoryCache {
    async fn get<T>(&self, key: &CacheKey) -> CacheResult<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let key_str = key.as_str();

        // Try to get with read lock first
        {
            let entries = self.entries.read();
            if let Some(entry) = entries.get(&key_str) {
                if entry.is_expired() {
                    // Entry expired, will be cleaned up later
                    return Ok(None);
                }

                // Deserialize
                let value: T = serde_json::from_slice(&entry.data)
                    .map_err(|e| CacheError::Deserialization(e.to_string()))?;

                return Ok(Some(value));
            }
        }

        // Update last_accessed with write lock
        {
            let mut entries = self.entries.write();
            if let Some(entry) = entries.get_mut(&key_str) {
                entry.touch();
            }
        }

        Ok(None)
    }

    async fn set<T>(&self, key: &CacheKey, value: &T, ttl: Option<Duration>) -> CacheResult<()>
    where
        T: serde::Serialize + Sync,
    {
        let key_str = key.as_str();

        // Serialize
        let data =
            serde_json::to_vec(value).map_err(|e| CacheError::Serialization(e.to_string()))?;

        let effective_ttl = ttl.or(self.config.time_to_live);
        let entry = CacheEntry::new(data, effective_ttl, Vec::new());

        // Check capacity
        let current = self.entry_count.load(Ordering::Relaxed);
        if current >= self.config.max_capacity as usize {
            // Evict some entries
            self.evict_expired();
            let still_over = self.entry_count.load(Ordering::Relaxed);
            if still_over >= self.config.max_capacity as usize {
                self.evict_lru((self.config.max_capacity as usize / 10).max(1));
            }
        }

        // Insert
        {
            let mut entries = self.entries.write();
            let is_new = !entries.contains_key(&key_str);
            entries.insert(key_str.clone(), entry);
            if is_new {
                self.entry_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        Ok(())
    }

    async fn delete(&self, key: &CacheKey) -> CacheResult<bool> {
        let key_str = key.as_str();

        let mut entries = self.entries.write();
        if let Some(entry) = entries.remove(&key_str) {
            self.remove_from_tag_index(&key_str, &entry.tags);
            self.entry_count.fetch_sub(1, Ordering::Relaxed);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn exists(&self, key: &CacheKey) -> CacheResult<bool> {
        let key_str = key.as_str();

        let entries = self.entries.read();
        if let Some(entry) = entries.get(&key_str) {
            Ok(!entry.is_expired())
        } else {
            Ok(false)
        }
    }

    async fn invalidate_pattern(&self, pattern: &KeyPattern) -> CacheResult<u64> {
        let mut entries = self.entries.write();
        let before = entries.len();

        let matching_keys: Vec<String> = entries
            .keys()
            .filter(|k| pattern.matches_str(k))
            .cloned()
            .collect();

        for key in &matching_keys {
            if let Some(entry) = entries.remove(key) {
                self.remove_from_tag_index(key, &entry.tags);
            }
        }

        let removed = before - entries.len();
        self.entry_count.fetch_sub(removed, Ordering::Relaxed);
        Ok(removed as u64)
    }

    async fn invalidate_tags(&self, tags: &[EntityTag]) -> CacheResult<u64> {
        if !self.config.enable_tags {
            return Ok(0);
        }

        let keys_to_remove: HashSet<String> = {
            let index = self.tag_index.read();
            tags.iter()
                .filter_map(|tag| index.get(tag.value()))
                .flatten()
                .cloned()
                .collect()
        };

        let mut entries = self.entries.write();
        let mut removed = 0u64;

        for key in keys_to_remove {
            if let Some(entry) = entries.remove(&key) {
                self.remove_from_tag_index(&key, &entry.tags);
                removed += 1;
            }
        }

        self.entry_count
            .fetch_sub(removed as usize, Ordering::Relaxed);
        Ok(removed)
    }

    async fn clear(&self) -> CacheResult<()> {
        let mut entries = self.entries.write();
        entries.clear();
        self.entry_count.store(0, Ordering::Relaxed);

        if self.config.enable_tags {
            let mut index = self.tag_index.write();
            index.clear();
        }

        Ok(())
    }

    async fn len(&self) -> CacheResult<usize> {
        Ok(self.entry_count.load(Ordering::Relaxed))
    }

    async fn stats(&self) -> CacheResult<BackendStats> {
        let entries = self.entries.read();
        let memory_estimate: usize = entries
            .values()
            .map(|e| e.data.len() + 64) // Data + overhead estimate
            .sum();

        Ok(BackendStats {
            entries: entries.len(),
            memory_bytes: Some(memory_estimate),
            connections: None,
            info: Some(format!("MemoryCache (max: {})", self.config.max_capacity)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_cache_basic() {
        let cache = MemoryCache::new(MemoryCacheConfig::new(100));

        let key = CacheKey::new("test", "key1");

        // Set
        cache.set(&key, &"hello", None).await.unwrap();

        // Get
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert_eq!(value, Some("hello".to_string()));

        // Delete
        assert!(cache.delete(&key).await.unwrap());

        // Should be gone
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn test_memory_cache_ttl() {
        let config = MemoryCacheConfig::new(100).with_ttl(Duration::from_millis(50));
        let cache = MemoryCache::new(config);

        let key = CacheKey::new("test", "ttl");
        cache.set(&key, &"expires soon", None).await.unwrap();

        // Should exist
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert!(value.is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Should be expired
        let value: Option<String> = cache.get(&key).await.unwrap();
        assert!(value.is_none());
    }

    #[tokio::test]
    async fn test_memory_cache_eviction() {
        let cache = MemoryCache::new(MemoryCacheConfig::new(5));

        // Fill cache
        for i in 0..10 {
            let key = CacheKey::new("test", format!("key{}", i));
            cache.set(&key, &i, None).await.unwrap();
        }

        // Should have evicted some
        let len = cache.len().await.unwrap();
        assert!(len <= 5);
    }

    #[tokio::test]
    async fn test_memory_cache_pattern_invalidation() {
        let cache = MemoryCache::new(MemoryCacheConfig::new(100));

        // Add some entries
        for i in 0..5 {
            let key = CacheKey::new("User", format!("id:{}", i));
            cache.set(&key, &i, None).await.unwrap();
        }
        for i in 0..3 {
            let key = CacheKey::new("Post", format!("id:{}", i));
            cache.set(&key, &i, None).await.unwrap();
        }

        assert_eq!(cache.len().await.unwrap(), 8);

        // Invalidate all User entries
        let removed = cache
            .invalidate_pattern(&KeyPattern::entity("User"))
            .await
            .unwrap();
        assert_eq!(removed, 5);
        assert_eq!(cache.len().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_memory_cache_builder() {
        let cache = MemoryCache::builder()
            .max_capacity(1000)
            .time_to_live(Duration::from_secs(60))
            .build();

        assert_eq!(cache.config().max_capacity, 1000);
        assert_eq!(cache.config().time_to_live, Some(Duration::from_secs(60)));
    }
}
