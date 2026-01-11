//! Enhanced string interning for efficient identifier storage.
//!
//! This module provides both global and scoped string interning to minimize
//! memory allocations for repeated field names, table names, and other identifiers.
//!
//! # Interning Strategies
//!
//! 1. **Static interning**: Compile-time constants for common fields (zero allocation)
//! 2. **Global interning**: Thread-safe, lifetime of program, for repeated identifiers
//! 3. **Scoped interning**: Per-request/query, automatically freed when scope ends
//!
//! # Performance
//!
//! - First intern: O(n) allocation + hash lookup
//! - Subsequent lookups: O(n) hash lookup, no allocation
//! - Cloning interned string: O(1)
//!
//! # Example
//!
//! ```rust
//! use prax_query::mem_optimize::interning::{GlobalInterner, InternedStr};
//!
//! // Get the global interner
//! let interner = GlobalInterner::get();
//!
//! // Intern a string
//! let s1 = interner.intern("user_id");
//! let s2 = interner.intern("user_id");
//!
//! // Same memory location
//! assert!(InternedStr::ptr_eq(&s1, &s2));
//! ```

use parking_lot::{Mutex, RwLock};
use smol_str::SmolStr;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

// ============================================================================
// Interned String Types
// ============================================================================

/// An interned string that shares memory with other identical strings.
///
/// This is a thin wrapper around `Arc<str>` that provides cheap cloning
/// and comparison operations.
#[derive(Clone, Debug)]
pub struct InternedStr(Arc<str>);

impl InternedStr {
    /// Create a new interned string from a raw Arc.
    #[inline]
    pub fn new(s: Arc<str>) -> Self {
        Self(s)
    }

    /// Get the string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check if two interned strings point to the same memory.
    #[inline]
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    /// Get the inner Arc.
    #[inline]
    pub fn into_arc(self) -> Arc<str> {
        self.0
    }

    /// Convert to a SmolStr (small string optimization).
    #[inline]
    pub fn to_smol(&self) -> SmolStr {
        SmolStr::new(&*self.0)
    }

    /// Convert to Cow<'static, str>.
    ///
    /// Returns Cow::Owned because Arc<str> cannot be borrowed with 'static lifetime.
    #[inline]
    pub fn to_cow(&self) -> Cow<'static, str> {
        Cow::Owned(self.0.to_string())
    }
}

impl AsRef<str> for InternedStr {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for InternedStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq for InternedStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Fast path: pointer equality
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        // Slow path: string comparison
        *self.0 == *other.0
    }
}

impl Eq for InternedStr {}

impl Hash for InternedStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

impl std::fmt::Display for InternedStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<&str> for InternedStr {
    fn from(s: &str) -> Self {
        GlobalInterner::get().intern(s)
    }
}

impl From<String> for InternedStr {
    fn from(s: String) -> Self {
        GlobalInterner::get().intern(&s)
    }
}

// ============================================================================
// Global Interner
// ============================================================================

/// Thread-safe global string interner.
///
/// Strings interned here live for the lifetime of the program. Use this for
/// identifiers that appear frequently across many queries.
pub struct GlobalInterner {
    strings: RwLock<HashSet<Arc<str>>>,
    stats: Mutex<InternerStats>,
}

impl GlobalInterner {
    /// Get the global interner instance.
    pub fn get() -> &'static Self {
        static INSTANCE: std::sync::OnceLock<GlobalInterner> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(|| {
            let interner = GlobalInterner {
                strings: RwLock::new(HashSet::with_capacity(256)),
                stats: Mutex::new(InternerStats::default()),
            };
            // Pre-populate with common identifiers
            interner.prepopulate();
            interner
        })
    }

    /// Pre-populate with common SQL identifiers.
    fn prepopulate(&self) {
        for name in COMMON_IDENTIFIERS {
            self.intern(name);
        }
    }

    /// Intern a string, returning an interned reference.
    ///
    /// If the string has been interned before, returns the existing reference.
    /// Otherwise, creates a new interned entry.
    #[inline]
    pub fn intern(&self, s: &str) -> InternedStr {
        // Fast path: check if already interned (read lock)
        {
            let strings = self.strings.read();
            if let Some(existing) = strings.get(s) {
                let mut stats = self.stats.lock();
                stats.hits += 1;
                return InternedStr(Arc::clone(existing));
            }
        }

        // Slow path: need to insert (write lock)
        let mut strings = self.strings.write();

        // Double-check after acquiring write lock
        if let Some(existing) = strings.get(s) {
            let mut stats = self.stats.lock();
            stats.hits += 1;
            return InternedStr(Arc::clone(existing));
        }

        // Insert new string
        let arc: Arc<str> = Arc::from(s);
        strings.insert(Arc::clone(&arc));

        let mut stats = self.stats.lock();
        stats.misses += 1;
        stats.total_bytes += s.len();

        InternedStr(arc)
    }

    /// Try to get an already-interned string without creating a new entry.
    #[inline]
    pub fn lookup(&self, s: &str) -> Option<InternedStr> {
        let strings = self.strings.read();
        strings.get(s).map(|arc| InternedStr(Arc::clone(arc)))
    }

    /// Get the number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.read().len()
    }

    /// Check if the interner is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.read().is_empty()
    }

    /// Get interning statistics.
    pub fn stats(&self) -> InternerStats {
        self.stats.lock().clone()
    }

    /// Clear all interned strings (use with caution!).
    ///
    /// This invalidates all existing `InternedStr` references from this interner.
    /// Only use during testing or shutdown.
    pub fn clear(&self) {
        self.strings.write().clear();
        *self.stats.lock() = InternerStats::default();
    }
}

// ============================================================================
// Scoped Interner
// ============================================================================

/// A scoped string interner for temporary use.
///
/// Strings interned here are freed when the interner is dropped.
/// Use this for request-scoped interning to avoid memory growth.
#[derive(Default)]
pub struct ScopedInterner {
    strings: HashSet<Arc<str>>,
    stats: InternerStats,
}

impl ScopedInterner {
    /// Create a new scoped interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a scoped interner with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            strings: HashSet::with_capacity(capacity),
            stats: InternerStats::default(),
        }
    }

    /// Intern a string within this scope.
    #[inline]
    pub fn intern(&mut self, s: &str) -> InternedStr {
        if let Some(existing) = self.strings.get(s) {
            self.stats.hits += 1;
            return InternedStr(Arc::clone(existing));
        }

        let arc: Arc<str> = Arc::from(s);
        self.strings.insert(Arc::clone(&arc));
        self.stats.misses += 1;
        self.stats.total_bytes += s.len();

        InternedStr(arc)
    }

    /// Try to get an already-interned string.
    #[inline]
    pub fn get(&self, s: &str) -> Option<InternedStr> {
        self.strings.get(s).map(|arc| InternedStr(Arc::clone(arc)))
    }

    /// Get the number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Get statistics.
    pub fn stats(&self) -> &InternerStats {
        &self.stats
    }

    /// Clear all interned strings.
    pub fn clear(&mut self) {
        self.strings.clear();
        self.stats = InternerStats::default();
    }
}

// ============================================================================
// Identifier Cache
// ============================================================================

/// Cache for auto-interning common identifier patterns.
///
/// This cache recognizes common patterns like `table.column` and automatically
/// interns both the full identifier and its components.
pub struct IdentifierCache {
    /// Full identifiers (e.g., "users.email")
    full: RwLock<HashMap<String, InternedStr>>,
    /// Components (e.g., "users", "email")
    components: RwLock<HashSet<Arc<str>>>,
}

impl IdentifierCache {
    /// Create a new identifier cache.
    pub fn new() -> Self {
        Self {
            full: RwLock::new(HashMap::with_capacity(128)),
            components: RwLock::new(HashSet::with_capacity(256)),
        }
    }

    /// Get the global identifier cache.
    pub fn global() -> &'static Self {
        static INSTANCE: std::sync::OnceLock<IdentifierCache> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(Self::new)
    }

    /// Intern a table.column identifier.
    ///
    /// Also interns the individual components.
    pub fn intern_qualified(&self, table: &str, column: &str) -> InternedStr {
        let key = format!("{}.{}", table, column);

        // Check cache
        if let Some(cached) = self.full.read().get(&key) {
            return cached.clone();
        }

        // Intern components
        self.intern_component(table);
        self.intern_component(column);

        // Intern full identifier
        let interned = GlobalInterner::get().intern(&key);

        // Cache it
        self.full.write().insert(key, interned.clone());

        interned
    }

    /// Intern just a component (table name or column name).
    pub fn intern_component(&self, name: &str) -> InternedStr {
        // Check if already in components
        {
            let components = self.components.read();
            if let Some(existing) = components.get(name) {
                return InternedStr(Arc::clone(existing));
            }
        }

        // Intern via global interner
        let interned = GlobalInterner::get().intern(name);

        // Add to components
        self.components.write().insert(interned.0.clone());

        interned
    }

    /// Get a cached qualified identifier.
    pub fn get_qualified(&self, table: &str, column: &str) -> Option<InternedStr> {
        let key = format!("{}.{}", table, column);
        self.full.read().get(&key).cloned()
    }

    /// Get cached component count.
    pub fn component_count(&self) -> usize {
        self.components.read().len()
    }

    /// Get cached full identifier count.
    pub fn qualified_count(&self) -> usize {
        self.full.read().len()
    }
}

impl Default for IdentifierCache {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Statistics for an interner.
#[derive(Debug, Clone, Default)]
pub struct InternerStats {
    /// Cache hits.
    pub hits: u64,
    /// Cache misses (new strings interned).
    pub misses: u64,
    /// Total bytes interned.
    pub total_bytes: usize,
}

impl InternerStats {
    /// Get the hit ratio.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

// ============================================================================
// Common Identifiers
// ============================================================================

/// Common SQL identifiers to pre-populate.
const COMMON_IDENTIFIERS: &[&str] = &[
    // Common column names
    "id",
    "uuid",
    "name",
    "email",
    "username",
    "password",
    "password_hash",
    "title",
    "description",
    "content",
    "body",
    "text",
    "status",
    "state",
    "type",
    "kind",
    "role",
    "active",
    "enabled",
    "deleted",
    "archived",
    "verified",
    "confirmed",
    "published",
    "visible",
    "public",
    "private",
    // Numeric fields
    "count",
    "total",
    "score",
    "rating",
    "priority",
    "order",
    "position",
    "rank",
    "level",
    "index",
    "sequence",
    "age",
    "amount",
    "price",
    "cost",
    "quantity",
    "weight",
    "height",
    "width",
    "length",
    "size",
    // Foreign keys
    "user_id",
    "account_id",
    "organization_id",
    "tenant_id",
    "post_id",
    "comment_id",
    "article_id",
    "product_id",
    "order_id",
    "item_id",
    "category_id",
    "tag_id",
    "parent_id",
    "author_id",
    "owner_id",
    "creator_id",
    "assignee_id",
    "reviewer_id",
    // Timestamps
    "created_at",
    "updated_at",
    "deleted_at",
    "published_at",
    "expires_at",
    "starts_at",
    "ends_at",
    "last_login_at",
    "last_seen_at",
    "verified_at",
    "confirmed_at",
    // URL/path fields
    "slug",
    "url",
    "uri",
    "path",
    "permalink",
    "link",
    "href",
    "src",
    "source",
    "destination",
    // Auth fields
    "key",
    "value",
    "token",
    "secret",
    "code",
    "pin",
    "otp",
    "api_key",
    "access_token",
    "refresh_token",
    // Metadata
    "version",
    "revision",
    "checksum",
    "hash",
    "signature",
    "fingerprint",
    "metadata",
    "data",
    "payload",
    "config",
    "settings",
    "options",
    "preferences",
    // Common table names
    "users",
    "accounts",
    "organizations",
    "tenants",
    "posts",
    "comments",
    "articles",
    "products",
    "orders",
    "items",
    "categories",
    "tags",
    "files",
    "images",
    "documents",
    "messages",
    "notifications",
    "events",
    "logs",
    "sessions",
    "tokens",
    // SQL keywords used as identifiers
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS",
    "NULL",
    "TRUE",
    "FALSE",
    "ASC",
    "DESC",
    "LIMIT",
    "OFFSET",
    "ORDER",
    "BY",
    "GROUP",
    "HAVING",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "ON",
    "AS",
];

// ============================================================================
// Convenience Functions
// ============================================================================

/// Intern a string using the global interner.
#[inline]
pub fn intern(s: &str) -> InternedStr {
    GlobalInterner::get().intern(s)
}

/// Try to get an already-interned string from the global interner.
#[inline]
pub fn get_interned(s: &str) -> Option<InternedStr> {
    GlobalInterner::get().lookup(s)
}

/// Intern a qualified identifier (table.column).
#[inline]
pub fn intern_qualified(table: &str, column: &str) -> InternedStr {
    IdentifierCache::global().intern_qualified(table, column)
}

/// Intern just a component (table or column name).
#[inline]
pub fn intern_component(name: &str) -> InternedStr {
    IdentifierCache::global().intern_component(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_interner_dedup() {
        let interner = GlobalInterner::get();

        let s1 = interner.intern("test_field");
        let s2 = interner.intern("test_field");

        // Should be the same pointer
        assert!(InternedStr::ptr_eq(&s1, &s2));
    }

    #[test]
    fn test_scoped_interner() {
        let mut interner = ScopedInterner::new();

        let s1 = interner.intern("scoped_field");
        let s2 = interner.intern("scoped_field");

        assert!(InternedStr::ptr_eq(&s1, &s2));
        assert_eq!(interner.len(), 1);
    }

    #[test]
    fn test_identifier_cache_qualified() {
        let cache = IdentifierCache::new();

        let id1 = cache.intern_qualified("users", "email");
        let id2 = cache.intern_qualified("users", "email");

        assert!(InternedStr::ptr_eq(&id1, &id2));
        assert_eq!(id1.as_str(), "users.email");
    }

    #[test]
    fn test_interned_str_equality() {
        let interner = GlobalInterner::get();

        let s1 = interner.intern("equal_test");
        let s2 = interner.intern("equal_test");
        let s3 = interner.intern("different");

        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_interned_str_hash() {
        use std::collections::HashSet;

        let interner = GlobalInterner::get();

        let s1 = interner.intern("hash_test");
        let s2 = interner.intern("hash_test");

        let mut set = HashSet::new();
        set.insert(s1.clone());

        assert!(set.contains(&s2));
    }

    #[test]
    fn test_interner_stats() {
        let mut interner = ScopedInterner::new();

        // First intern - miss
        let _ = interner.intern("stats_test");
        assert_eq!(interner.stats().misses, 1);
        assert_eq!(interner.stats().hits, 0);

        // Second intern - hit
        let _ = interner.intern("stats_test");
        assert_eq!(interner.stats().misses, 1);
        assert_eq!(interner.stats().hits, 1);

        assert!(interner.stats().hit_ratio() > 0.4);
    }

    #[test]
    fn test_common_identifiers_prepopulated() {
        let interner = GlobalInterner::get();

        // These should be hits (pre-populated)
        let _ = interner.intern("id");
        let _ = interner.intern("created_at");
        let _ = interner.intern("user_id");

        // Verify they're in the interner
        assert!(interner.lookup("id").is_some());
        assert!(interner.lookup("email").is_some());
    }

    #[test]
    fn test_interned_str_from() {
        let s1: InternedStr = "from_str".into();
        let s2: InternedStr = String::from("from_string").into();

        assert_eq!(s1.as_str(), "from_str");
        assert_eq!(s2.as_str(), "from_string");
    }
}
