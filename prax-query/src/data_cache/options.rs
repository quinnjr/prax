//! Cache options and configuration.

use std::time::Duration;

use super::invalidation::EntityTag;

/// Options for caching a query result.
#[derive(Debug, Clone)]
pub struct CacheOptions {
    /// Time-to-live for the cached entry.
    pub ttl: Option<Duration>,
    /// Cache policy to use.
    pub policy: CachePolicy,
    /// Write policy (when to write to cache).
    pub write_policy: WritePolicy,
    /// Tags for invalidation.
    pub tags: Vec<EntityTag>,
    /// Skip caching if result is larger than this size (bytes).
    pub max_size: Option<usize>,
    /// Whether to cache empty results.
    pub cache_empty: bool,
    /// Whether to bypass cache for this query.
    pub bypass: bool,
    /// Stale-while-revalidate duration.
    pub stale_while_revalidate: Option<Duration>,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            ttl: Some(Duration::from_secs(300)), // 5 minutes
            policy: CachePolicy::CacheFirst,
            write_policy: WritePolicy::WriteThrough,
            tags: Vec::new(),
            max_size: Some(1024 * 1024), // 1MB
            cache_empty: true,
            bypass: false,
            stale_while_revalidate: None,
        }
    }
}

impl CacheOptions {
    /// Create options with a specific TTL.
    pub fn ttl(duration: Duration) -> Self {
        Self {
            ttl: Some(duration),
            ..Default::default()
        }
    }

    /// Create options that don't expire.
    pub fn no_expire() -> Self {
        Self {
            ttl: None,
            ..Default::default()
        }
    }

    /// Create options that bypass the cache.
    pub fn bypass() -> Self {
        Self {
            bypass: true,
            ..Default::default()
        }
    }

    /// Set the TTL.
    pub fn with_ttl(mut self, duration: Duration) -> Self {
        self.ttl = Some(duration);
        self
    }

    /// Set the cache policy.
    pub fn with_policy(mut self, policy: CachePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Set the write policy.
    pub fn with_write_policy(mut self, policy: WritePolicy) -> Self {
        self.write_policy = policy;
        self
    }

    /// Add a tag.
    pub fn with_tag(mut self, tag: impl Into<EntityTag>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Add multiple tags.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = EntityTag>) -> Self {
        self.tags.extend(tags);
        self
    }

    /// Set max size.
    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_size = Some(size);
        self
    }

    /// Don't limit size.
    pub fn no_size_limit(mut self) -> Self {
        self.max_size = None;
        self
    }

    /// Don't cache empty results.
    pub fn no_cache_empty(mut self) -> Self {
        self.cache_empty = false;
        self
    }

    /// Enable stale-while-revalidate.
    pub fn stale_while_revalidate(mut self, duration: Duration) -> Self {
        self.stale_while_revalidate = Some(duration);
        self
    }

    /// Short TTL preset (1 minute).
    pub fn short() -> Self {
        Self::ttl(Duration::from_secs(60))
    }

    /// Medium TTL preset (5 minutes).
    pub fn medium() -> Self {
        Self::ttl(Duration::from_secs(300))
    }

    /// Long TTL preset (1 hour).
    pub fn long() -> Self {
        Self::ttl(Duration::from_secs(3600))
    }

    /// Very long TTL preset (1 day).
    pub fn daily() -> Self {
        Self::ttl(Duration::from_secs(86400))
    }
}

/// Cache lookup/write policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CachePolicy {
    /// Check cache first, fetch on miss.
    #[default]
    CacheFirst,

    /// Always fetch from source, update cache.
    NetworkFirst,

    /// Return cached value only, never fetch.
    CacheOnly,

    /// Fetch from source only, don't use cache.
    NetworkOnly,

    /// Return stale data while revalidating.
    StaleWhileRevalidate,
}

impl CachePolicy {
    /// Check if this policy should check cache.
    pub fn should_check_cache(&self) -> bool {
        matches!(
            self,
            Self::CacheFirst | Self::CacheOnly | Self::StaleWhileRevalidate
        )
    }

    /// Check if this policy should fetch from source.
    pub fn should_fetch(&self) -> bool {
        matches!(
            self,
            Self::CacheFirst | Self::NetworkFirst | Self::NetworkOnly | Self::StaleWhileRevalidate
        )
    }

    /// Check if this policy should update cache.
    pub fn should_update_cache(&self) -> bool {
        matches!(
            self,
            Self::CacheFirst | Self::NetworkFirst | Self::StaleWhileRevalidate
        )
    }
}

/// When to write to the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritePolicy {
    /// Write to cache immediately.
    #[default]
    WriteThrough,

    /// Write to cache in background.
    WriteBack,

    /// Write to cache after a delay.
    WriteDelayed,
}

/// Preset configurations for common use cases.
pub mod presets {
    use super::*;

    /// For frequently changing data (news feeds, notifications).
    pub fn volatile() -> CacheOptions {
        CacheOptions::ttl(Duration::from_secs(30))
            .with_policy(CachePolicy::StaleWhileRevalidate)
            .stale_while_revalidate(Duration::from_secs(60))
    }

    /// For user profiles and settings.
    pub fn user_data() -> CacheOptions {
        CacheOptions::ttl(Duration::from_secs(300)).with_policy(CachePolicy::CacheFirst)
    }

    /// For reference/lookup data that rarely changes.
    pub fn reference_data() -> CacheOptions {
        CacheOptions::ttl(Duration::from_secs(3600)).with_policy(CachePolicy::CacheFirst)
    }

    /// For static data that almost never changes.
    pub fn static_data() -> CacheOptions {
        CacheOptions::ttl(Duration::from_secs(86400)).with_policy(CachePolicy::CacheFirst)
    }

    /// For session data.
    pub fn session() -> CacheOptions {
        CacheOptions::ttl(Duration::from_secs(1800)) // 30 minutes
            .with_policy(CachePolicy::CacheFirst)
    }

    /// For real-time data that shouldn't be cached.
    pub fn realtime() -> CacheOptions {
        CacheOptions::bypass()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = CacheOptions::default();
        assert_eq!(opts.ttl, Some(Duration::from_secs(300)));
        assert_eq!(opts.policy, CachePolicy::CacheFirst);
        assert!(opts.cache_empty);
        assert!(!opts.bypass);
    }

    #[test]
    fn test_options_builder() {
        let opts = CacheOptions::ttl(Duration::from_secs(60))
            .with_policy(CachePolicy::NetworkFirst)
            .with_tag(EntityTag::new("User"))
            .no_cache_empty();

        assert_eq!(opts.ttl, Some(Duration::from_secs(60)));
        assert_eq!(opts.policy, CachePolicy::NetworkFirst);
        assert_eq!(opts.tags.len(), 1);
        assert!(!opts.cache_empty);
    }

    #[test]
    fn test_cache_policy() {
        assert!(CachePolicy::CacheFirst.should_check_cache());
        assert!(CachePolicy::CacheFirst.should_fetch());
        assert!(CachePolicy::CacheFirst.should_update_cache());

        assert!(!CachePolicy::NetworkOnly.should_check_cache());
        assert!(CachePolicy::NetworkOnly.should_fetch());
        assert!(!CachePolicy::NetworkOnly.should_update_cache());

        assert!(CachePolicy::CacheOnly.should_check_cache());
        assert!(!CachePolicy::CacheOnly.should_fetch());
        assert!(!CachePolicy::CacheOnly.should_update_cache());
    }

    #[test]
    fn test_presets() {
        let volatile = presets::volatile();
        assert_eq!(volatile.ttl, Some(Duration::from_secs(30)));

        let reference = presets::reference_data();
        assert_eq!(reference.ttl, Some(Duration::from_secs(3600)));

        let realtime = presets::realtime();
        assert!(realtime.bypass);
    }
}
