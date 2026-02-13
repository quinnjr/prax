//! Cache key generation and patterns.

use std::fmt::{self, Display, Write};
use std::hash::{Hash, Hasher};

/// A cache key that uniquely identifies a cached value.
///
/// Keys are structured as `prefix:namespace:identifier` to enable
/// pattern-based invalidation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    /// The key prefix (usually the app name or "prax").
    prefix: String,
    /// The namespace (usually entity name like "User", "Post").
    namespace: String,
    /// The unique identifier within the namespace.
    identifier: String,
    /// Optional tenant ID for multi-tenant apps.
    tenant: Option<String>,
}

impl CacheKey {
    /// Create a new cache key.
    pub fn new(namespace: impl Into<String>, identifier: impl Into<String>) -> Self {
        Self {
            prefix: "prax".to_string(),
            namespace: namespace.into(),
            identifier: identifier.into(),
            tenant: None,
        }
    }

    /// Create a cache key with a custom prefix.
    pub fn with_prefix(
        prefix: impl Into<String>,
        namespace: impl Into<String>,
        identifier: impl Into<String>,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            namespace: namespace.into(),
            identifier: identifier.into(),
            tenant: None,
        }
    }

    /// Create a key for a specific entity record.
    pub fn entity_record<I: Display>(entity: &str, id: I) -> Self {
        Self::new(entity, format!("id:{}", id))
    }

    /// Create a key for a query result.
    pub fn query(entity: &str, query_hash: u64) -> Self {
        Self::new(entity, format!("query:{:x}", query_hash))
    }

    /// Create a key for a find-unique query.
    pub fn find_unique<I: Display>(entity: &str, field: &str, value: I) -> Self {
        Self::new(entity, format!("unique:{}:{}", field, value))
    }

    /// Create a key for a find-many query with filters.
    pub fn find_many(entity: &str, filter_hash: u64) -> Self {
        Self::new(entity, format!("many:{:x}", filter_hash))
    }

    /// Create a key for an aggregation.
    pub fn aggregate(entity: &str, agg_hash: u64) -> Self {
        Self::new(entity, format!("agg:{:x}", agg_hash))
    }

    /// Create a key for a relation.
    pub fn relation<I: Display>(from_entity: &str, from_id: I, relation: &str) -> Self {
        Self::new(from_entity, format!("rel:{}:{}:{}", from_id, relation, ""))
    }

    /// Set the tenant for multi-tenant apps.
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Get the full key string.
    pub fn as_str(&self) -> String {
        let mut key = String::with_capacity(64);
        key.push_str(&self.prefix);
        key.push(':');

        if let Some(ref tenant) = self.tenant {
            key.push_str(tenant);
            key.push(':');
        }

        key.push_str(&self.namespace);
        key.push(':');
        key.push_str(&self.identifier);
        key
    }

    /// Get the namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Get the identifier.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// Get the prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Get the tenant if set.
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
}

impl Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.prefix.hash(state);
        self.namespace.hash(state);
        self.identifier.hash(state);
        self.tenant.hash(state);
    }
}

impl From<&str> for CacheKey {
    fn from(s: &str) -> Self {
        // Parse "prefix:namespace:identifier" or "namespace:identifier"
        let parts: Vec<&str> = s.split(':').collect();
        match parts.len() {
            2 => Self::new(parts[0], parts[1]),
            3 => Self::with_prefix(parts[0], parts[1], parts[2]),
            _ => Self::new("default", s),
        }
    }
}

impl From<String> for CacheKey {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

/// A builder for constructing complex cache keys.
#[derive(Debug, Default)]
pub struct CacheKeyBuilder {
    prefix: Option<String>,
    namespace: Option<String>,
    tenant: Option<String>,
    parts: Vec<String>,
}

impl CacheKeyBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the prefix.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Set the namespace (entity).
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// Set the tenant.
    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Add a key part.
    pub fn part(mut self, part: impl Into<String>) -> Self {
        self.parts.push(part.into());
        self
    }

    /// Add a field-value pair.
    pub fn field<V: Display>(mut self, name: &str, value: V) -> Self {
        self.parts.push(format!("{}:{}", name, value));
        self
    }

    /// Add an ID.
    pub fn id<I: Display>(mut self, id: I) -> Self {
        self.parts.push(format!("id:{}", id));
        self
    }

    /// Add a hash.
    pub fn hash(mut self, hash: u64) -> Self {
        self.parts.push(format!("{:x}", hash));
        self
    }

    /// Build the cache key.
    pub fn build(self) -> CacheKey {
        let namespace = self.namespace.unwrap_or_else(|| "default".to_string());
        let identifier = if self.parts.is_empty() {
            "default".to_string()
        } else {
            self.parts.join(":")
        };

        let mut key = if let Some(prefix) = self.prefix {
            CacheKey::with_prefix(prefix, namespace, identifier)
        } else {
            CacheKey::new(namespace, identifier)
        };

        if let Some(tenant) = self.tenant {
            key = key.with_tenant(tenant);
        }

        key
    }
}

/// A pattern for matching cache keys.
///
/// Supports glob-style patterns with `*` wildcards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPattern {
    pattern: String,
}

impl KeyPattern {
    /// Create a new pattern.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
        }
    }

    /// Create a pattern matching all keys for an entity.
    pub fn entity(entity: &str) -> Self {
        Self::new(format!("prax:{}:*", entity))
    }

    /// Create a pattern matching a specific record (with relations).
    pub fn record<I: Display>(entity: &str, id: I) -> Self {
        Self::new(format!("prax:{}:*{}*", entity, id))
    }

    /// Create a pattern for a tenant's data.
    pub fn tenant(tenant: &str) -> Self {
        Self::new(format!("prax:{}:*", tenant))
    }

    /// Create a pattern matching all keys.
    pub fn all() -> Self {
        Self::new("prax:*")
    }

    /// Create a pattern with a custom prefix.
    pub fn with_prefix(prefix: &str, pattern: &str) -> Self {
        Self::new(format!("{}:{}", prefix, pattern))
    }

    /// Get the pattern string.
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Check if a key matches this pattern.
    pub fn matches(&self, key: &CacheKey) -> bool {
        self.matches_str(&key.as_str())
    }

    /// Check if a string matches this pattern.
    pub fn matches_str(&self, key: &str) -> bool {
        glob_match(&self.pattern, key)
    }

    /// Convert to a Redis-compatible pattern.
    pub fn to_redis_pattern(&self) -> String {
        self.pattern.clone()
    }
}

impl Display for KeyPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

/// Simple glob matching with `*` wildcards.
fn glob_match(pattern: &str, text: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    let mut text_chars = text.chars().peekable();

    while let Some(p) = pattern_chars.next() {
        match p {
            '*' => {
                // Match any number of characters
                if pattern_chars.peek().is_none() {
                    return true; // Trailing * matches everything
                }

                // Try matching from current position
                let remaining_pattern: String = pattern_chars.collect();
                let remaining_text: String = text_chars.collect();

                for i in 0..=remaining_text.len() {
                    if glob_match(&remaining_pattern, &remaining_text[i..]) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                // Match exactly one character
                if text_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                // Match literal character
                match text_chars.next() {
                    Some(t) if t == c => {}
                    _ => return false,
                }
            }
        }
    }

    text_chars.next().is_none()
}

/// Helper to compute a hash for cache keys.
pub fn compute_hash<T: Hash>(value: &T) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Helper to compute a hash from multiple values.
pub fn compute_hash_many<T: Hash>(values: &[T]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    for value in values {
        value.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_creation() {
        let key = CacheKey::new("User", "id:123");
        assert_eq!(key.as_str(), "prax:User:id:123");
    }

    #[test]
    fn test_cache_key_with_tenant() {
        let key = CacheKey::new("User", "id:123").with_tenant("tenant-1");
        assert_eq!(key.as_str(), "prax:tenant-1:User:id:123");
    }

    #[test]
    fn test_entity_record_key() {
        let key = CacheKey::entity_record("User", 42);
        assert_eq!(key.as_str(), "prax:User:id:42");
    }

    #[test]
    fn test_find_unique_key() {
        let key = CacheKey::find_unique("User", "email", "test@example.com");
        assert_eq!(key.as_str(), "prax:User:unique:email:test@example.com");
    }

    #[test]
    fn test_key_builder() {
        let key = CacheKeyBuilder::new()
            .namespace("User")
            .field("status", "active")
            .id(123)
            .build();

        assert!(key.as_str().contains("User"));
        assert!(key.as_str().contains("status:active"));
    }

    #[test]
    fn test_key_pattern_entity() {
        let pattern = KeyPattern::entity("User");
        assert_eq!(pattern.as_str(), "prax:User:*");

        let key1 = CacheKey::entity_record("User", 1);
        let key2 = CacheKey::entity_record("Post", 1);

        assert!(pattern.matches(&key1));
        assert!(!pattern.matches(&key2));
    }

    #[test]
    fn test_glob_matching() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("prax:*", "prax:User:123"));
        assert!(glob_match("prax:User:*", "prax:User:id:123"));
        assert!(!glob_match("prax:Post:*", "prax:User:id:123"));
        assert!(glob_match("*:User:*", "prax:User:id:123"));
    }

    #[test]
    fn test_compute_hash() {
        let hash1 = compute_hash(&"test");
        let hash2 = compute_hash(&"test");
        let hash3 = compute_hash(&"other");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
