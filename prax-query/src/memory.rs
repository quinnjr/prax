//! Memory optimization utilities for prax-query.
//!
//! This module provides memory-efficient alternatives and pooling mechanisms
//! to reduce allocation counts and peak memory usage.
//!
//! # Optimization Strategies
//!
//! 1. **Object Pooling**: Reuse allocated objects instead of allocating new ones
//! 2. **Compact Types**: Smaller type representations for common cases
//! 3. **Inline Storage**: Store small data inline to avoid heap allocation
//! 4. **String Deduplication**: Share identical strings via interning
//!
//! # Example
//!
//! ```rust
//! use prax_query::memory::{StringPool, CompactFilter};
//! use std::sync::Arc;
//!
//! // Reuse strings from a pool
//! let pool = StringPool::new();
//! let s1 = pool.intern("id");
//! let s2 = pool.intern("id");
//! // s1 and s2 point to the same allocation
//!
//! // Use compact filters for simple cases
//! let filter = CompactFilter::eq_int(Arc::from("id"), 42);
//! ```

use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::info;

// ============================================================================
// String Pool
// ============================================================================

/// A thread-safe pool for interning strings.
///
/// Reduces memory usage by ensuring only one copy of each unique string exists.
#[derive(Debug, Default)]
pub struct StringPool {
    strings: Mutex<HashSet<Arc<str>>>,
}

impl StringPool {
    /// Create a new empty string pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a pool with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        info!(capacity, "StringPool initialized");
        Self {
            strings: Mutex::new(HashSet::with_capacity(capacity)),
        }
    }

    /// Intern a string, returning a shared reference.
    pub fn intern(&self, s: &str) -> Arc<str> {
        let mut strings = self.strings.lock();

        // Check if already interned
        if let Some(existing) = strings.get(s) {
            return Arc::clone(existing);
        }

        // Intern new string
        let arc: Arc<str> = Arc::from(s);
        strings.insert(Arc::clone(&arc));
        arc
    }

    /// Get the number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.lock().len()
    }

    /// Check if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.lock().is_empty()
    }

    /// Clear all interned strings.
    pub fn clear(&self) {
        self.strings.lock().clear();
    }

    /// Get memory statistics.
    pub fn stats(&self) -> PoolStats {
        let strings = self.strings.lock();
        let count = strings.len();
        let total_bytes: usize = strings.iter().map(|s| s.len()).sum();
        PoolStats {
            count,
            total_bytes,
            avg_bytes: total_bytes.checked_div(count).unwrap_or(0),
        }
    }
}

/// Statistics for a pool.
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    /// Number of items in the pool.
    pub count: usize,
    /// Total bytes used.
    pub total_bytes: usize,
    /// Average bytes per item.
    pub avg_bytes: usize,
}

// ============================================================================
// Compact Filter Types
// ============================================================================

/// A compact filter representation for simple equality filters.
///
/// Uses less memory than the full `Filter` enum for common cases:
/// - Equality on integer fields: 16 bytes vs ~40 bytes
/// - Equality on string fields with short values: ~32 bytes vs ~56 bytes
#[derive(Debug, Clone, PartialEq)]
pub enum CompactFilter {
    /// Equality on integer field.
    EqInt {
        /// Field name (interned).
        field: Arc<str>,
        /// Integer value.
        value: i64,
    },
    /// Equality on boolean field.
    EqBool {
        /// Field name (interned).
        field: Arc<str>,
        /// Boolean value.
        value: bool,
    },
    /// Equality on string field.
    EqStr {
        /// Field name (interned).
        field: Arc<str>,
        /// String value.
        value: Arc<str>,
    },
    /// IS NULL check.
    IsNull {
        /// Field name (interned).
        field: Arc<str>,
    },
    /// IS NOT NULL check.
    IsNotNull {
        /// Field name (interned).
        field: Arc<str>,
    },
    /// Greater than on integer field.
    GtInt {
        /// Field name (interned).
        field: Arc<str>,
        /// Integer value.
        value: i64,
    },
    /// Less than on integer field.
    LtInt {
        /// Field name (interned).
        field: Arc<str>,
        /// Integer value.
        value: i64,
    },
    /// AND of two compact filters.
    And(Box<CompactFilter>, Box<CompactFilter>),
    /// OR of two compact filters.
    Or(Box<CompactFilter>, Box<CompactFilter>),
}

impl CompactFilter {
    /// Create an equality filter on an integer field.
    #[inline]
    pub fn eq_int(field: impl Into<Arc<str>>, value: i64) -> Self {
        Self::EqInt {
            field: field.into(),
            value,
        }
    }

    /// Create an equality filter on a boolean field.
    #[inline]
    pub fn eq_bool(field: impl Into<Arc<str>>, value: bool) -> Self {
        Self::EqBool {
            field: field.into(),
            value,
        }
    }

    /// Create an equality filter on a string field.
    #[inline]
    pub fn eq_str(field: impl Into<Arc<str>>, value: impl Into<Arc<str>>) -> Self {
        Self::EqStr {
            field: field.into(),
            value: value.into(),
        }
    }

    /// Create an IS NULL filter.
    #[inline]
    pub fn is_null(field: impl Into<Arc<str>>) -> Self {
        Self::IsNull {
            field: field.into(),
        }
    }

    /// Create an IS NOT NULL filter.
    #[inline]
    pub fn is_not_null(field: impl Into<Arc<str>>) -> Self {
        Self::IsNotNull {
            field: field.into(),
        }
    }

    /// Create a greater-than filter on an integer field.
    #[inline]
    pub fn gt_int(field: impl Into<Arc<str>>, value: i64) -> Self {
        Self::GtInt {
            field: field.into(),
            value,
        }
    }

    /// Create a less-than filter on an integer field.
    #[inline]
    pub fn lt_int(field: impl Into<Arc<str>>, value: i64) -> Self {
        Self::LtInt {
            field: field.into(),
            value,
        }
    }

    /// Combine with another filter using AND.
    #[inline]
    pub fn and(self, other: Self) -> Self {
        Self::And(Box::new(self), Box::new(other))
    }

    /// Combine with another filter using OR.
    #[inline]
    pub fn or(self, other: Self) -> Self {
        Self::Or(Box::new(self), Box::new(other))
    }

    /// Convert to SQL condition string for PostgreSQL.
    pub fn to_sql_postgres(&self, param_offset: &mut usize) -> String {
        match self {
            Self::EqInt { field, .. } => {
                *param_offset += 1;
                format!("{} = ${}", field, *param_offset)
            }
            Self::EqBool { field, .. } => {
                *param_offset += 1;
                format!("{} = ${}", field, *param_offset)
            }
            Self::EqStr { field, .. } => {
                *param_offset += 1;
                format!("{} = ${}", field, *param_offset)
            }
            Self::IsNull { field } => format!("{} IS NULL", field),
            Self::IsNotNull { field } => format!("{} IS NOT NULL", field),
            Self::GtInt { field, .. } => {
                *param_offset += 1;
                format!("{} > ${}", field, *param_offset)
            }
            Self::LtInt { field, .. } => {
                *param_offset += 1;
                format!("{} < ${}", field, *param_offset)
            }
            Self::And(left, right) => {
                let left_sql = left.to_sql_postgres(param_offset);
                let right_sql = right.to_sql_postgres(param_offset);
                format!("({} AND {})", left_sql, right_sql)
            }
            Self::Or(left, right) => {
                let left_sql = left.to_sql_postgres(param_offset);
                let right_sql = right.to_sql_postgres(param_offset);
                format!("({} OR {})", left_sql, right_sql)
            }
        }
    }

    /// Get the approximate size in bytes.
    pub fn size_bytes(&self) -> usize {
        match self {
            Self::EqInt { .. } | Self::GtInt { .. } | Self::LtInt { .. } => 24, // Arc<str> + i64
            Self::EqBool { .. } => 17,                                          // Arc<str> + bool
            Self::EqStr { field, value } => 16 + field.len() + value.len(),
            Self::IsNull { .. } | Self::IsNotNull { .. } => 16, // Arc<str>
            Self::And(l, r) | Self::Or(l, r) => 16 + l.size_bytes() + r.size_bytes(),
        }
    }
}

// ============================================================================
// Reusable Buffer Pool
// ============================================================================

/// A pool of reusable String buffers.
///
/// Reduces allocation by reusing String buffers for SQL generation.
#[derive(Debug, Default)]
pub struct BufferPool {
    buffers: Mutex<Vec<String>>,
    default_capacity: usize,
}

impl BufferPool {
    /// Create a new buffer pool.
    pub fn new() -> Self {
        Self {
            buffers: Mutex::new(Vec::new()),
            default_capacity: 256,
        }
    }

    /// Create a pool with custom default buffer capacity.
    pub fn with_capacity(default_capacity: usize) -> Self {
        info!(default_capacity, "BufferPool initialized");
        Self {
            buffers: Mutex::new(Vec::new()),
            default_capacity,
        }
    }

    /// Get a buffer from the pool or create a new one.
    pub fn get(&self) -> PooledBuffer<'_> {
        let buffer = self
            .buffers
            .lock()
            .pop()
            .unwrap_or_else(|| String::with_capacity(self.default_capacity));
        PooledBuffer { buffer, pool: self }
    }

    /// Return a buffer to the pool.
    fn return_buffer(&self, mut buffer: String) {
        buffer.clear();
        // Only keep reasonably sized buffers to avoid memory bloat
        if buffer.capacity() <= 4096 {
            self.buffers.lock().push(buffer);
        }
    }

    /// Get the number of available buffers.
    pub fn available(&self) -> usize {
        self.buffers.lock().len()
    }

    /// Clear all pooled buffers.
    pub fn clear(&self) {
        self.buffers.lock().clear();
    }
}

/// A buffer borrowed from a pool.
///
/// Automatically returns to the pool when dropped.
pub struct PooledBuffer<'a> {
    buffer: String,
    pool: &'a BufferPool,
}

impl<'a> PooledBuffer<'a> {
    /// Get mutable access to the buffer.
    pub fn as_mut_str(&mut self) -> &mut String {
        &mut self.buffer
    }

    /// Take ownership of the buffer (does not return to pool).
    pub fn take(mut self) -> String {
        std::mem::take(&mut self.buffer)
    }
}

impl<'a> std::ops::Deref for PooledBuffer<'a> {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl<'a> std::ops::DerefMut for PooledBuffer<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}

impl<'a> Drop for PooledBuffer<'a> {
    fn drop(&mut self) {
        if !self.buffer.is_empty() || self.buffer.capacity() > 0 {
            let buffer = std::mem::take(&mut self.buffer);
            self.pool.return_buffer(buffer);
        }
    }
}

// ============================================================================
// Memory Usage Tracking
// ============================================================================

/// Track memory usage for debugging and optimization.
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// Number of allocations.
    pub allocations: u64,
    /// Number of deallocations.
    pub deallocations: u64,
    /// Current bytes allocated.
    pub current_bytes: usize,
    /// Peak bytes allocated.
    pub peak_bytes: usize,
    /// Total bytes allocated (lifetime).
    pub total_bytes: usize,
}

impl MemoryStats {
    /// Create new empty stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an allocation.
    pub fn record_alloc(&mut self, bytes: usize) {
        self.allocations += 1;
        self.current_bytes += bytes;
        self.total_bytes += bytes;
        if self.current_bytes > self.peak_bytes {
            self.peak_bytes = self.current_bytes;
        }
    }

    /// Record a deallocation.
    pub fn record_dealloc(&mut self, bytes: usize) {
        self.deallocations += 1;
        self.current_bytes = self.current_bytes.saturating_sub(bytes);
    }

    /// Get the net allocation count.
    pub fn net_allocations(&self) -> i64 {
        self.allocations as i64 - self.deallocations as i64
    }
}

// ============================================================================
// Global Pools (Lazy Initialized)
// ============================================================================

/// Global string pool for common field names.
pub static GLOBAL_STRING_POOL: std::sync::LazyLock<StringPool> = std::sync::LazyLock::new(|| {
    let pool = StringPool::with_capacity(128);
    // Pre-populate with common field names
    for name in COMMON_FIELD_NAMES {
        pool.intern(name);
    }
    pool
});

/// Global buffer pool for SQL generation.
pub static GLOBAL_BUFFER_POOL: std::sync::LazyLock<BufferPool> =
    std::sync::LazyLock::new(|| BufferPool::with_capacity(256));

/// Common field names to pre-populate the string pool.
const COMMON_FIELD_NAMES: &[&str] = &[
    "id",
    "uuid",
    "name",
    "email",
    "username",
    "password",
    "title",
    "description",
    "content",
    "body",
    "status",
    "type",
    "role",
    "active",
    "enabled",
    "deleted",
    "verified",
    "published",
    "count",
    "score",
    "priority",
    "order",
    "position",
    "age",
    "amount",
    "price",
    "quantity",
    "user_id",
    "post_id",
    "comment_id",
    "category_id",
    "parent_id",
    "author_id",
    "owner_id",
    "created_at",
    "updated_at",
    "deleted_at",
    "published_at",
    "expires_at",
    "starts_at",
    "ends_at",
    "last_login_at",
    "verified_at",
    "slug",
    "url",
    "path",
    "key",
    "value",
    "token",
    "code",
    "version",
];

/// Intern a string using the global pool.
#[inline]
pub fn intern(s: &str) -> Arc<str> {
    GLOBAL_STRING_POOL.intern(s)
}

/// Get a buffer from the global pool.
#[inline]
pub fn get_buffer() -> PooledBuffer<'static> {
    GLOBAL_BUFFER_POOL.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_pool_interning() {
        let pool = StringPool::new();

        let s1 = pool.intern("hello");
        let s2 = pool.intern("hello");

        // Should be the same Arc
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_string_pool_different_strings() {
        let pool = StringPool::new();

        let s1 = pool.intern("hello");
        let s2 = pool.intern("world");

        assert!(!Arc::ptr_eq(&s1, &s2));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_compact_filter_eq_int() {
        let filter = CompactFilter::eq_int(Arc::from("id"), 42);
        let mut offset = 0;
        let sql = filter.to_sql_postgres(&mut offset);
        assert_eq!(sql, "id = $1");
        assert_eq!(offset, 1);
    }

    #[test]
    fn test_compact_filter_and() {
        let filter = CompactFilter::eq_int(Arc::from("id"), 42)
            .and(CompactFilter::eq_bool(Arc::from("active"), true));
        let mut offset = 0;
        let sql = filter.to_sql_postgres(&mut offset);
        assert_eq!(sql, "(id = $1 AND active = $2)");
        assert_eq!(offset, 2);
    }

    #[test]
    fn test_compact_filter_is_null() {
        let filter = CompactFilter::is_null(Arc::from("deleted_at"));
        let mut offset = 0;
        let sql = filter.to_sql_postgres(&mut offset);
        assert_eq!(sql, "deleted_at IS NULL");
        assert_eq!(offset, 0); // No params for IS NULL
    }

    #[test]
    fn test_buffer_pool() {
        let pool = BufferPool::new();

        {
            let mut buffer = pool.get();
            buffer.push_str("hello");
            assert_eq!(&*buffer, "hello");
        } // Buffer returned to pool here

        assert_eq!(pool.available(), 1);

        {
            let buffer = pool.get();
            assert!(buffer.is_empty()); // Buffer was cleared
        }
    }

    #[test]
    fn test_global_intern() {
        let s1 = intern("id");
        let s2 = intern("id");
        assert!(Arc::ptr_eq(&s1, &s2));
    }

    #[test]
    fn test_memory_stats() {
        let mut stats = MemoryStats::new();

        stats.record_alloc(100);
        stats.record_alloc(200);
        assert_eq!(stats.current_bytes, 300);
        assert_eq!(stats.peak_bytes, 300);

        stats.record_dealloc(100);
        assert_eq!(stats.current_bytes, 200);
        assert_eq!(stats.peak_bytes, 300); // Peak unchanged
    }
}
