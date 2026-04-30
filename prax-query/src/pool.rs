//! Arena-based filter pool for efficient nested filter construction.
//!
//! This module provides a `FilterPool` that uses bump allocation to efficiently
//! construct complex nested filter trees with minimal allocations.
//!
//! # When to Use
//!
//! Use `FilterPool` when:
//! - Building deeply nested filter trees (depth > 3)
//! - Constructing many filters in a tight loop
//! - Performance profiling shows filter allocation as a bottleneck
//!
//! For simple filters, use the regular `Filter` constructors directly.
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```rust
//! use prax_query::pool::FilterPool;
//! use prax_query::{Filter, FilterValue};
//!
//! let mut pool = FilterPool::new();
//!
//! // Build a complex nested filter efficiently
//! let filter = pool.build(|b| {
//!     b.and(vec![
//!         b.eq("status", "active"),
//!         b.or(vec![
//!             b.gt("age", 18),
//!             b.eq("verified", true),
//!         ]),
//!         b.not(b.eq("deleted", true)),
//!     ])
//! });
//!
//! // The filter is now a regular owned Filter
//! assert!(!filter.is_none());
//! ```
//!
//! ## Reusing the Pool
//!
//! ```rust
//! use prax_query::pool::FilterPool;
//!
//! let mut pool = FilterPool::new();
//!
//! // Build first filter
//! let filter1 = pool.build(|b| b.eq("id", 1));
//!
//! // Reset and reuse the pool
//! pool.reset();
//!
//! // Build second filter (reuses the same memory)
//! let filter2 = pool.build(|b| b.eq("id", 2));
//! ```

use bumpalo::Bump;
use std::borrow::Cow;

use crate::filter::{Filter, FilterValue};

/// A memory pool for efficient filter construction.
///
/// Uses bump allocation to minimize allocations when building complex filter trees.
/// The pool can be reused by calling `reset()` after each filter is built.
///
/// # Performance
///
/// - Filter construction in the pool: O(1) allocation per filter tree
/// - Materialization to owned filter: O(n) where n is the number of nodes
/// - Pool reset: O(1) (just resets the bump pointer)
///
/// # Thread Safety
///
/// `FilterPool` is not thread-safe. Each thread should have its own pool.
pub struct FilterPool {
    arena: Bump,
}

impl FilterPool {
    /// Create a new filter pool with default capacity.
    ///
    /// The pool starts with a small initial allocation and grows as needed.
    pub fn new() -> Self {
        Self { arena: Bump::new() }
    }

    /// Create a new filter pool with the specified initial capacity in bytes.
    ///
    /// Use this when you know approximately how much memory your filters will need.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            arena: Bump::with_capacity(capacity),
        }
    }

    /// Reset the pool, freeing all allocated memory for reuse.
    ///
    /// This is very fast (O(1)) as it just resets the bump pointer.
    /// Call this between filter constructions to reuse memory.
    pub fn reset(&mut self) {
        self.arena.reset();
    }

    /// Get the amount of memory currently allocated in the pool.
    pub fn allocated_bytes(&self) -> usize {
        self.arena.allocated_bytes()
    }

    /// Build a filter using the pool's arena for temporary allocations.
    ///
    /// The closure receives a `FilterBuilder` that provides efficient methods
    /// for constructing nested filters. The resulting filter is materialized
    /// into an owned `Filter` that can be used after the pool is reset.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::pool::FilterPool;
    /// use prax_query::Filter;
    ///
    /// let mut pool = FilterPool::new();
    /// let filter = pool.build(|b| {
    ///     b.and(vec![
    ///         b.eq("active", true),
    ///         b.gt("score", 100),
    ///     ])
    /// });
    /// ```
    pub fn build<F>(&self, f: F) -> Filter
    where
        F: for<'a> FnOnce(&'a FilterBuilder<'a>) -> PooledFilter<'a>,
    {
        let builder = FilterBuilder::new(&self.arena);
        let pooled = f(&builder);
        pooled.materialize()
    }
}

impl Default for FilterPool {
    fn default() -> Self {
        Self::new()
    }
}

/// A filter that lives in the pool's arena.
///
/// This is a temporary representation used during filter construction.
/// Call `materialize()` to convert it to an owned `Filter`.
#[derive(Debug, Clone, Copy)]
pub enum PooledFilter<'a> {
    /// No filter (always true).
    None,
    /// Equals comparison.
    Equals(&'a str, PooledValue<'a>),
    /// Not equals comparison.
    NotEquals(&'a str, PooledValue<'a>),
    /// Less than comparison.
    Lt(&'a str, PooledValue<'a>),
    /// Less than or equal comparison.
    Lte(&'a str, PooledValue<'a>),
    /// Greater than comparison.
    Gt(&'a str, PooledValue<'a>),
    /// Greater than or equal comparison.
    Gte(&'a str, PooledValue<'a>),
    /// In a list of values.
    In(&'a str, &'a [PooledValue<'a>]),
    /// Not in a list of values.
    NotIn(&'a str, &'a [PooledValue<'a>]),
    /// Contains (LIKE %value%).
    Contains(&'a str, PooledValue<'a>),
    /// Starts with (LIKE value%).
    StartsWith(&'a str, PooledValue<'a>),
    /// Ends with (LIKE %value).
    EndsWith(&'a str, PooledValue<'a>),
    /// Is null check.
    IsNull(&'a str),
    /// Is not null check.
    IsNotNull(&'a str),
    /// Logical AND of multiple filters.
    And(&'a [PooledFilter<'a>]),
    /// Logical OR of multiple filters.
    Or(&'a [PooledFilter<'a>]),
    /// Logical NOT of a filter.
    Not(&'a PooledFilter<'a>),
}

impl<'a> PooledFilter<'a> {
    /// Materialize the pooled filter into an owned Filter.
    ///
    /// This copies all data from the arena into owned allocations.
    pub fn materialize(&self) -> Filter {
        match self {
            PooledFilter::None => Filter::None,
            PooledFilter::Equals(field, value) => {
                Filter::Equals(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::NotEquals(field, value) => {
                Filter::NotEquals(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::Lt(field, value) => {
                Filter::Lt(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::Lte(field, value) => {
                Filter::Lte(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::Gt(field, value) => {
                Filter::Gt(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::Gte(field, value) => {
                Filter::Gte(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::In(field, values) => Filter::In(
                Cow::Owned((*field).to_string()),
                values.iter().map(|v| v.materialize()).collect(),
            ),
            PooledFilter::NotIn(field, values) => Filter::NotIn(
                Cow::Owned((*field).to_string()),
                values.iter().map(|v| v.materialize()).collect(),
            ),
            PooledFilter::Contains(field, value) => {
                Filter::Contains(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::StartsWith(field, value) => {
                Filter::StartsWith(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::EndsWith(field, value) => {
                Filter::EndsWith(Cow::Owned((*field).to_string()), value.materialize())
            }
            PooledFilter::IsNull(field) => Filter::IsNull(Cow::Owned((*field).to_string())),
            PooledFilter::IsNotNull(field) => Filter::IsNotNull(Cow::Owned((*field).to_string())),
            PooledFilter::And(filters) => Filter::And(
                filters
                    .iter()
                    .map(|f| f.materialize())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            PooledFilter::Or(filters) => Filter::Or(
                filters
                    .iter()
                    .map(|f| f.materialize())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            PooledFilter::Not(filter) => Filter::Not(Box::new(filter.materialize())),
        }
    }
}

/// A filter value that lives in the pool's arena.
#[derive(Debug, Clone, Copy)]
pub enum PooledValue<'a> {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// String value (borrowed from arena).
    String(&'a str),
    /// JSON value (borrowed from arena).
    Json(&'a str),
}

impl<'a> PooledValue<'a> {
    /// Materialize the pooled value into an owned FilterValue.
    pub fn materialize(&self) -> FilterValue {
        match self {
            PooledValue::Null => FilterValue::Null,
            PooledValue::Bool(b) => FilterValue::Bool(*b),
            PooledValue::Int(i) => FilterValue::Int(*i),
            PooledValue::Float(f) => FilterValue::Float(*f),
            PooledValue::String(s) => FilterValue::String((*s).to_string()),
            PooledValue::Json(s) => FilterValue::Json(serde_json::from_str(s).unwrap_or_default()),
        }
    }
}

/// A builder for constructing filters within a pool.
///
/// Provides ergonomic methods for building filter trees with minimal allocations.
pub struct FilterBuilder<'a> {
    arena: &'a Bump,
}

impl<'a> FilterBuilder<'a> {
    fn new(arena: &'a Bump) -> Self {
        Self { arena }
    }

    /// Create a pooled string from a string slice.
    fn alloc_str(&self, s: &str) -> &'a str {
        self.arena.alloc_str(s)
    }

    /// Create a pooled slice from a vector of pooled filters.
    fn alloc_filters(&self, filters: Vec<PooledFilter<'a>>) -> &'a [PooledFilter<'a>] {
        self.arena.alloc_slice_fill_iter(filters)
    }

    /// Create a pooled slice from a vector of pooled values.
    fn alloc_values(&self, values: Vec<PooledValue<'a>>) -> &'a [PooledValue<'a>] {
        self.arena.alloc_slice_fill_iter(values)
    }

    /// Convert a value into a pooled value.
    pub fn value<V: IntoPooledValue<'a>>(&self, v: V) -> PooledValue<'a> {
        v.into_pooled(self)
    }

    /// Create an empty filter (matches everything).
    pub fn none(&self) -> PooledFilter<'a> {
        PooledFilter::None
    }

    /// Create an equals filter.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::pool::FilterPool;
    ///
    /// let pool = FilterPool::new();
    /// let filter = pool.build(|b| b.eq("status", "active"));
    /// ```
    pub fn eq<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::Equals(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create a not equals filter.
    pub fn ne<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::NotEquals(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create a less than filter.
    pub fn lt<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::Lt(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create a less than or equal filter.
    pub fn lte<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::Lte(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create a greater than filter.
    pub fn gt<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::Gt(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create a greater than or equal filter.
    pub fn gte<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::Gte(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create an IN filter.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::pool::FilterPool;
    ///
    /// let pool = FilterPool::new();
    /// let filter = pool.build(|b| {
    ///     b.is_in("status", vec![b.value("pending"), b.value("processing")])
    /// });
    /// ```
    pub fn is_in(&self, field: &str, values: Vec<PooledValue<'a>>) -> PooledFilter<'a> {
        PooledFilter::In(self.alloc_str(field), self.alloc_values(values))
    }

    /// Create a NOT IN filter.
    pub fn not_in(&self, field: &str, values: Vec<PooledValue<'a>>) -> PooledFilter<'a> {
        PooledFilter::NotIn(self.alloc_str(field), self.alloc_values(values))
    }

    /// Create a contains filter (LIKE %value%).
    pub fn contains<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::Contains(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create a starts with filter (LIKE value%).
    pub fn starts_with<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::StartsWith(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create an ends with filter (LIKE %value).
    pub fn ends_with<V: IntoPooledValue<'a>>(&self, field: &str, value: V) -> PooledFilter<'a> {
        PooledFilter::EndsWith(self.alloc_str(field), value.into_pooled(self))
    }

    /// Create an IS NULL filter.
    pub fn is_null(&self, field: &str) -> PooledFilter<'a> {
        PooledFilter::IsNull(self.alloc_str(field))
    }

    /// Create an IS NOT NULL filter.
    pub fn is_not_null(&self, field: &str) -> PooledFilter<'a> {
        PooledFilter::IsNotNull(self.alloc_str(field))
    }

    /// Create an AND filter combining multiple filters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::pool::FilterPool;
    ///
    /// let pool = FilterPool::new();
    /// let filter = pool.build(|b| {
    ///     b.and(vec![
    ///         b.eq("active", true),
    ///         b.gt("score", 100),
    ///         b.is_not_null("email"),
    ///     ])
    /// });
    /// ```
    pub fn and(&self, filters: Vec<PooledFilter<'a>>) -> PooledFilter<'a> {
        // Filter out None filters
        let filters: Vec<_> = filters
            .into_iter()
            .filter(|f| !matches!(f, PooledFilter::None))
            .collect();

        match filters.len() {
            0 => PooledFilter::None,
            1 => filters.into_iter().next().unwrap(),
            _ => PooledFilter::And(self.alloc_filters(filters)),
        }
    }

    /// Create an OR filter combining multiple filters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::pool::FilterPool;
    ///
    /// let pool = FilterPool::new();
    /// let filter = pool.build(|b| {
    ///     b.or(vec![
    ///         b.eq("role", "admin"),
    ///         b.eq("role", "moderator"),
    ///     ])
    /// });
    /// ```
    pub fn or(&self, filters: Vec<PooledFilter<'a>>) -> PooledFilter<'a> {
        // Filter out None filters
        let filters: Vec<_> = filters
            .into_iter()
            .filter(|f| !matches!(f, PooledFilter::None))
            .collect();

        match filters.len() {
            0 => PooledFilter::None,
            1 => filters.into_iter().next().unwrap(),
            _ => PooledFilter::Or(self.alloc_filters(filters)),
        }
    }

    /// Create a NOT filter.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::pool::FilterPool;
    ///
    /// let pool = FilterPool::new();
    /// let filter = pool.build(|b| b.not(b.eq("deleted", true)));
    /// ```
    pub fn not(&self, filter: PooledFilter<'a>) -> PooledFilter<'a> {
        if matches!(filter, PooledFilter::None) {
            return PooledFilter::None;
        }
        PooledFilter::Not(self.arena.alloc(filter))
    }
}

/// Trait for types that can be converted to a pooled value.
pub trait IntoPooledValue<'a> {
    fn into_pooled(self, builder: &FilterBuilder<'a>) -> PooledValue<'a>;
}

impl<'a> IntoPooledValue<'a> for bool {
    fn into_pooled(self, _builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        PooledValue::Bool(self)
    }
}

impl<'a> IntoPooledValue<'a> for i32 {
    fn into_pooled(self, _builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        PooledValue::Int(self as i64)
    }
}

impl<'a> IntoPooledValue<'a> for i64 {
    fn into_pooled(self, _builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        PooledValue::Int(self)
    }
}

impl<'a> IntoPooledValue<'a> for f64 {
    fn into_pooled(self, _builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        PooledValue::Float(self)
    }
}

impl<'a> IntoPooledValue<'a> for &str {
    fn into_pooled(self, builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        PooledValue::String(builder.alloc_str(self))
    }
}

impl<'a> IntoPooledValue<'a> for String {
    fn into_pooled(self, builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        PooledValue::String(builder.alloc_str(&self))
    }
}

impl<'a> IntoPooledValue<'a> for PooledValue<'a> {
    fn into_pooled(self, _builder: &FilterBuilder<'a>) -> PooledValue<'a> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_basic_filter() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| b.eq("id", 42));

        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_pool_and_filter() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| b.and(vec![b.eq("active", true), b.gt("score", 100)]));

        assert!(matches!(filter, Filter::And(_)));
    }

    #[test]
    fn test_pool_or_filter() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| {
            b.or(vec![
                b.eq("status", "pending"),
                b.eq("status", "processing"),
            ])
        });

        assert!(matches!(filter, Filter::Or(_)));
    }

    #[test]
    fn test_pool_nested_filter() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| {
            b.and(vec![
                b.eq("active", true),
                b.or(vec![b.gt("age", 18), b.eq("verified", true)]),
                b.not(b.eq("deleted", true)),
            ])
        });

        assert!(matches!(filter, Filter::And(_)));
    }

    #[test]
    fn test_pool_in_filter() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| {
            b.is_in(
                "status",
                vec![
                    b.value("pending"),
                    b.value("processing"),
                    b.value("completed"),
                ],
            )
        });

        assert!(matches!(filter, Filter::In(_, _)));
    }

    #[test]
    fn test_pool_reset() {
        let mut pool = FilterPool::new();

        // Build first filter
        let _ = pool.build(|b| b.eq("id", 1));
        let bytes1 = pool.allocated_bytes();

        // Reset pool
        pool.reset();

        // Build second filter (should reuse memory)
        let _ = pool.build(|b| b.eq("id", 2));
        let bytes2 = pool.allocated_bytes();

        // After reset, memory usage should be similar
        assert!(bytes2 <= bytes1 * 2); // Allow some growth
    }

    #[test]
    fn test_pool_empty_and() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| b.and(vec![]));

        assert!(matches!(filter, Filter::None));
    }

    #[test]
    fn test_pool_single_and() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| b.and(vec![b.eq("id", 1)]));

        // Single element AND should be simplified
        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_pool_null_filters() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| b.is_null("deleted_at"));

        assert!(matches!(filter, Filter::IsNull(_)));
    }

    #[test]
    fn test_pool_deeply_nested() {
        let pool = FilterPool::new();

        // Build a deeply nested filter tree
        let filter = pool.build(|b| {
            b.and(vec![
                b.or(vec![
                    b.and(vec![b.eq("a", 1), b.eq("b", 2)]),
                    b.and(vec![b.eq("c", 3), b.eq("d", 4)]),
                ]),
                b.not(b.or(vec![b.eq("e", 5), b.eq("f", 6)])),
            ])
        });

        // Verify structure
        assert!(matches!(filter, Filter::And(_)));

        // Generate SQL to verify correctness
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert!(sql.contains("OR"));
        assert!(sql.contains("NOT"));
        assert_eq!(params.len(), 6);
    }

    #[test]
    fn test_pool_string_values() {
        let pool = FilterPool::new();
        let filter = pool.build(|b| {
            b.and(vec![
                b.eq("name", "Alice"),
                b.contains("email", "@example.com"),
                b.starts_with("phone", "+1"),
            ])
        });

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 3);
    }
}
