//! Typed arena allocation for query builder chains.
//!
//! This module provides arena-based allocation for efficient query construction
//! with minimal heap allocations.
//!
//! # Benefits
//!
//! - **Batch deallocation**: All allocations freed at once when scope ends
//! - **Cache-friendly**: Contiguous memory allocation
//! - **Fast allocation**: O(1) bump pointer allocation
//! - **No fragmentation**: No individual deallocation overhead
//!
//! # Example
//!
//! ```rust
//! use prax_query::mem_optimize::arena::QueryArena;
//!
//! let arena = QueryArena::new();
//!
//! // Build query within arena scope
//! let sql = arena.scope(|scope| {
//!     let filter = scope.eq("status", "active");
//!     let filter = scope.and(vec![
//!         filter,
//!         scope.gt("age", 18),
//!     ]);
//!     scope.build_select("users", filter)
//! });
//!
//! // Arena memory freed, but sql String is owned
//! ```

use bumpalo::Bump;
use std::cell::Cell;
use std::fmt::Write;

use super::interning::InternedStr;

// ============================================================================
// Query Arena
// ============================================================================

/// Arena allocator for query building.
///
/// Provides fast allocation with batch deallocation when the scope ends.
pub struct QueryArena {
    bump: Bump,
    stats: Cell<ArenaStats>,
}

impl QueryArena {
    /// Create a new query arena with default capacity.
    pub fn new() -> Self {
        Self {
            bump: Bump::new(),
            stats: Cell::new(ArenaStats::default()),
        }
    }

    /// Create an arena with specified initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bump: Bump::with_capacity(capacity),
            stats: Cell::new(ArenaStats::default()),
        }
    }

    /// Execute a closure with an arena scope.
    ///
    /// The scope provides allocation methods. All allocations are valid
    /// within the closure and freed when it returns.
    pub fn scope<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ArenaScope<'_>) -> R,
    {
        let scope = ArenaScope::new(&self.bump, &self.stats);
        f(&scope)
    }

    /// Reset the arena for reuse.
    ///
    /// This is O(1) - just resets the bump pointer.
    pub fn reset(&mut self) {
        self.bump.reset();
        self.stats.set(ArenaStats::default());
    }

    /// Get the number of bytes currently allocated.
    pub fn allocated_bytes(&self) -> usize {
        self.bump.allocated_bytes()
    }

    /// Get arena statistics.
    pub fn stats(&self) -> ArenaStats {
        self.stats.get()
    }
}

impl Default for QueryArena {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Arena Scope
// ============================================================================

/// A scope for allocating within an arena.
///
/// All allocations made through this scope are freed when the scope ends.
pub struct ArenaScope<'a> {
    bump: &'a Bump,
    stats: &'a Cell<ArenaStats>,
}

impl<'a> ArenaScope<'a> {
    fn new(bump: &'a Bump, stats: &'a Cell<ArenaStats>) -> Self {
        Self { bump, stats }
    }

    fn record_alloc(&self, bytes: usize) {
        let mut s = self.stats.get();
        s.allocations += 1;
        s.total_bytes += bytes;
        self.stats.set(s);
    }

    /// Allocate a string in the arena.
    #[inline]
    pub fn alloc_str(&self, s: &str) -> &'a str {
        self.record_alloc(s.len());
        self.bump.alloc_str(s)
    }

    /// Allocate a slice in the arena.
    #[inline]
    pub fn alloc_slice<T: Copy>(&self, slice: &[T]) -> &'a [T] {
        self.record_alloc(std::mem::size_of_val(slice));
        self.bump.alloc_slice_copy(slice)
    }

    /// Allocate a slice from an iterator.
    #[inline]
    pub fn alloc_slice_iter<T, I>(&self, iter: I) -> &'a [T]
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = iter.into_iter();
        self.record_alloc(iter.len() * std::mem::size_of::<T>());
        self.bump.alloc_slice_fill_iter(iter)
    }

    /// Allocate a single value in the arena.
    #[inline]
    pub fn alloc<T>(&self, value: T) -> &'a T {
        self.record_alloc(std::mem::size_of::<T>());
        self.bump.alloc(value)
    }

    /// Allocate a mutable value in the arena.
    #[inline]
    pub fn alloc_mut<T>(&self, value: T) -> &'a mut T {
        self.record_alloc(std::mem::size_of::<T>());
        self.bump.alloc(value)
    }

    // ========================================================================
    // Filter Construction
    // ========================================================================

    /// Create an equality filter.
    #[inline]
    pub fn eq<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::Equals(self.alloc_str(field), value.into())
    }

    /// Create a not-equals filter.
    #[inline]
    pub fn ne<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::NotEquals(self.alloc_str(field), value.into())
    }

    /// Create a less-than filter.
    #[inline]
    pub fn lt<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::Lt(self.alloc_str(field), value.into())
    }

    /// Create a less-than-or-equal filter.
    #[inline]
    pub fn lte<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::Lte(self.alloc_str(field), value.into())
    }

    /// Create a greater-than filter.
    #[inline]
    pub fn gt<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::Gt(self.alloc_str(field), value.into())
    }

    /// Create a greater-than-or-equal filter.
    #[inline]
    pub fn gte<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::Gte(self.alloc_str(field), value.into())
    }

    /// Create an IN filter.
    #[inline]
    pub fn is_in(&self, field: &str, values: Vec<ScopedValue<'a>>) -> ScopedFilter<'a> {
        ScopedFilter::In(self.alloc_str(field), self.alloc_slice_iter(values))
    }

    /// Create a NOT IN filter.
    #[inline]
    pub fn not_in(&self, field: &str, values: Vec<ScopedValue<'a>>) -> ScopedFilter<'a> {
        ScopedFilter::NotIn(self.alloc_str(field), self.alloc_slice_iter(values))
    }

    /// Create a CONTAINS filter.
    #[inline]
    pub fn contains<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::Contains(self.alloc_str(field), value.into())
    }

    /// Create a STARTS WITH filter.
    #[inline]
    pub fn starts_with<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::StartsWith(self.alloc_str(field), value.into())
    }

    /// Create an ENDS WITH filter.
    #[inline]
    pub fn ends_with<V: Into<ScopedValue<'a>>>(&self, field: &str, value: V) -> ScopedFilter<'a> {
        ScopedFilter::EndsWith(self.alloc_str(field), value.into())
    }

    /// Create an IS NULL filter.
    #[inline]
    pub fn is_null(&self, field: &str) -> ScopedFilter<'a> {
        ScopedFilter::IsNull(self.alloc_str(field))
    }

    /// Create an IS NOT NULL filter.
    #[inline]
    pub fn is_not_null(&self, field: &str) -> ScopedFilter<'a> {
        ScopedFilter::IsNotNull(self.alloc_str(field))
    }

    /// Combine filters with AND.
    #[inline]
    pub fn and(&self, filters: Vec<ScopedFilter<'a>>) -> ScopedFilter<'a> {
        // Filter out None filters
        let filters: Vec<_> = filters
            .into_iter()
            .filter(|f| !matches!(f, ScopedFilter::None))
            .collect();

        match filters.len() {
            0 => ScopedFilter::None,
            1 => filters.into_iter().next().unwrap(),
            _ => ScopedFilter::And(self.alloc_slice_iter(filters)),
        }
    }

    /// Combine filters with OR.
    #[inline]
    pub fn or(&self, filters: Vec<ScopedFilter<'a>>) -> ScopedFilter<'a> {
        let filters: Vec<_> = filters
            .into_iter()
            .filter(|f| !matches!(f, ScopedFilter::None))
            .collect();

        match filters.len() {
            0 => ScopedFilter::None,
            1 => filters.into_iter().next().unwrap(),
            _ => ScopedFilter::Or(self.alloc_slice_iter(filters)),
        }
    }

    /// Negate a filter.
    #[inline]
    pub fn not(&self, filter: ScopedFilter<'a>) -> ScopedFilter<'a> {
        if matches!(filter, ScopedFilter::None) {
            return ScopedFilter::None;
        }
        ScopedFilter::Not(self.alloc(filter))
    }

    // ========================================================================
    // Query Building
    // ========================================================================

    /// Build a SELECT query string.
    pub fn build_select(&self, table: &str, filter: ScopedFilter<'a>) -> String {
        let mut sql = String::with_capacity(128);
        sql.push_str("SELECT * FROM ");
        sql.push_str(table);

        if !matches!(filter, ScopedFilter::None) {
            sql.push_str(" WHERE ");
            filter.write_sql(&mut sql, &mut 1);
        }

        sql
    }

    /// Build a SELECT query with specific columns.
    pub fn build_select_columns(
        &self,
        table: &str,
        columns: &[&str],
        filter: ScopedFilter<'a>,
    ) -> String {
        let mut sql = String::with_capacity(128);
        sql.push_str("SELECT ");

        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(col);
        }

        sql.push_str(" FROM ");
        sql.push_str(table);

        if !matches!(filter, ScopedFilter::None) {
            sql.push_str(" WHERE ");
            filter.write_sql(&mut sql, &mut 1);
        }

        sql
    }

    /// Build a complete query with all parts.
    pub fn build_query(&self, query: &ScopedQuery<'a>) -> String {
        let mut sql = String::with_capacity(256);

        // SELECT
        sql.push_str("SELECT ");
        if query.columns.is_empty() {
            sql.push('*');
        } else {
            for (i, col) in query.columns.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(col);
            }
        }

        // FROM
        sql.push_str(" FROM ");
        sql.push_str(query.table);

        // WHERE
        if !matches!(query.filter, ScopedFilter::None) {
            sql.push_str(" WHERE ");
            query.filter.write_sql(&mut sql, &mut 1);
        }

        // ORDER BY
        if !query.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            for (i, (col, dir)) in query.order_by.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(col);
                sql.push(' ');
                sql.push_str(dir);
            }
        }

        // LIMIT
        if let Some(limit) = query.limit {
            write!(sql, " LIMIT {}", limit).unwrap();
        }

        // OFFSET
        if let Some(offset) = query.offset {
            write!(sql, " OFFSET {}", offset).unwrap();
        }

        sql
    }

    /// Create a new query builder.
    pub fn query(&self, table: &str) -> ScopedQuery<'a> {
        ScopedQuery {
            table: self.alloc_str(table),
            columns: &[],
            filter: ScopedFilter::None,
            order_by: &[],
            limit: None,
            offset: None,
        }
    }
}

// ============================================================================
// Scoped Filter
// ============================================================================

/// A filter allocated within an arena scope.
#[derive(Debug, Clone)]
pub enum ScopedFilter<'a> {
    /// No filter.
    None,
    /// Equality.
    Equals(&'a str, ScopedValue<'a>),
    /// Not equals.
    NotEquals(&'a str, ScopedValue<'a>),
    /// Less than.
    Lt(&'a str, ScopedValue<'a>),
    /// Less than or equal.
    Lte(&'a str, ScopedValue<'a>),
    /// Greater than.
    Gt(&'a str, ScopedValue<'a>),
    /// Greater than or equal.
    Gte(&'a str, ScopedValue<'a>),
    /// In list.
    In(&'a str, &'a [ScopedValue<'a>]),
    /// Not in list.
    NotIn(&'a str, &'a [ScopedValue<'a>]),
    /// Contains.
    Contains(&'a str, ScopedValue<'a>),
    /// Starts with.
    StartsWith(&'a str, ScopedValue<'a>),
    /// Ends with.
    EndsWith(&'a str, ScopedValue<'a>),
    /// Is null.
    IsNull(&'a str),
    /// Is not null.
    IsNotNull(&'a str),
    /// And.
    And(&'a [ScopedFilter<'a>]),
    /// Or.
    Or(&'a [ScopedFilter<'a>]),
    /// Not.
    Not(&'a ScopedFilter<'a>),
}

impl<'a> ScopedFilter<'a> {
    /// Write SQL to a string buffer.
    pub fn write_sql(&self, buf: &mut String, param_idx: &mut usize) {
        match self {
            ScopedFilter::None => {}
            ScopedFilter::Equals(field, _) => {
                write!(buf, "{} = ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::NotEquals(field, _) => {
                write!(buf, "{} != ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::Lt(field, _) => {
                write!(buf, "{} < ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::Lte(field, _) => {
                write!(buf, "{} <= ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::Gt(field, _) => {
                write!(buf, "{} > ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::Gte(field, _) => {
                write!(buf, "{} >= ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::In(field, values) => {
                write!(buf, "{} IN (", field).unwrap();
                for (i, _) in values.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    write!(buf, "${}", param_idx).unwrap();
                    *param_idx += 1;
                }
                buf.push(')');
            }
            ScopedFilter::NotIn(field, values) => {
                write!(buf, "{} NOT IN (", field).unwrap();
                for (i, _) in values.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    write!(buf, "${}", param_idx).unwrap();
                    *param_idx += 1;
                }
                buf.push(')');
            }
            ScopedFilter::Contains(field, _) => {
                write!(buf, "{} LIKE ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::StartsWith(field, _) => {
                write!(buf, "{} LIKE ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::EndsWith(field, _) => {
                write!(buf, "{} LIKE ${}", field, param_idx).unwrap();
                *param_idx += 1;
            }
            ScopedFilter::IsNull(field) => {
                write!(buf, "{} IS NULL", field).unwrap();
            }
            ScopedFilter::IsNotNull(field) => {
                write!(buf, "{} IS NOT NULL", field).unwrap();
            }
            ScopedFilter::And(filters) => {
                buf.push('(');
                for (i, filter) in filters.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(" AND ");
                    }
                    filter.write_sql(buf, param_idx);
                }
                buf.push(')');
            }
            ScopedFilter::Or(filters) => {
                buf.push('(');
                for (i, filter) in filters.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(" OR ");
                    }
                    filter.write_sql(buf, param_idx);
                }
                buf.push(')');
            }
            ScopedFilter::Not(filter) => {
                buf.push_str("NOT (");
                filter.write_sql(buf, param_idx);
                buf.push(')');
            }
        }
    }
}

// ============================================================================
// Scoped Value
// ============================================================================

/// A value allocated within an arena scope.
#[derive(Debug, Clone)]
pub enum ScopedValue<'a> {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float.
    Float(f64),
    /// String (borrowed from arena).
    String(&'a str),
    /// Interned string (shared reference).
    Interned(InternedStr),
}

impl<'a> From<bool> for ScopedValue<'a> {
    fn from(v: bool) -> Self {
        ScopedValue::Bool(v)
    }
}

impl<'a> From<i32> for ScopedValue<'a> {
    fn from(v: i32) -> Self {
        ScopedValue::Int(v as i64)
    }
}

impl<'a> From<i64> for ScopedValue<'a> {
    fn from(v: i64) -> Self {
        ScopedValue::Int(v)
    }
}

impl<'a> From<f64> for ScopedValue<'a> {
    fn from(v: f64) -> Self {
        ScopedValue::Float(v)
    }
}

impl<'a> From<&'a str> for ScopedValue<'a> {
    fn from(v: &'a str) -> Self {
        ScopedValue::String(v)
    }
}

impl<'a> From<InternedStr> for ScopedValue<'a> {
    fn from(v: InternedStr) -> Self {
        ScopedValue::Interned(v)
    }
}

// ============================================================================
// Scoped Query
// ============================================================================

/// A query being built within an arena scope.
#[derive(Debug, Clone)]
pub struct ScopedQuery<'a> {
    /// Table name.
    pub table: &'a str,
    /// Columns to select.
    pub columns: &'a [&'a str],
    /// Filter.
    pub filter: ScopedFilter<'a>,
    /// Order by clauses.
    pub order_by: &'a [(&'a str, &'a str)],
    /// Limit.
    pub limit: Option<usize>,
    /// Offset.
    pub offset: Option<usize>,
}

impl<'a> ScopedQuery<'a> {
    /// Set columns to select.
    pub fn select(mut self, columns: &'a [&'a str]) -> Self {
        self.columns = columns;
        self
    }

    /// Set filter.
    pub fn filter(mut self, filter: ScopedFilter<'a>) -> Self {
        self.filter = filter;
        self
    }

    /// Set order by.
    pub fn order_by(mut self, order_by: &'a [(&'a str, &'a str)]) -> Self {
        self.order_by = order_by;
        self
    }

    /// Set limit.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set offset.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Statistics for arena usage.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArenaStats {
    /// Number of allocations.
    pub allocations: usize,
    /// Total bytes allocated.
    pub total_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_basic_filter() {
        let arena = QueryArena::new();

        let sql = arena.scope(|scope| scope.build_select("users", scope.eq("id", 42)));

        assert!(sql.contains("SELECT * FROM users"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("id = $1"));
    }

    #[test]
    fn test_arena_complex_filter() {
        let arena = QueryArena::new();

        let sql = arena.scope(|scope| {
            let filter = scope.and(vec![
                scope.eq("active", true),
                scope.or(vec![scope.gt("age", 18), scope.is_not_null("verified_at")]),
            ]);
            scope.build_select("users", filter)
        });

        assert!(sql.contains("AND"));
        assert!(sql.contains("OR"));
    }

    #[test]
    fn test_arena_reset() {
        let mut arena = QueryArena::with_capacity(1024);

        // Use arena
        let _ = arena.scope(|scope| scope.build_select("users", scope.eq("id", 1)));
        let bytes1 = arena.allocated_bytes();

        // Reset
        arena.reset();

        // Use again
        let _ = arena.scope(|scope| scope.build_select("posts", scope.eq("id", 2)));
        let bytes2 = arena.allocated_bytes();

        // Should be similar (reusing memory)
        assert!(bytes2 <= bytes1 * 2);
    }

    #[test]
    fn test_arena_query_builder() {
        let arena = QueryArena::new();

        let sql = arena.scope(|scope| {
            let query = scope
                .query("users")
                .filter(scope.eq("active", true))
                .limit(10)
                .offset(20);
            scope.build_query(&query)
        });

        assert!(sql.contains("SELECT * FROM users"));
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("OFFSET 20"));
    }

    #[test]
    fn test_arena_in_filter() {
        let arena = QueryArena::new();

        let sql = arena.scope(|scope| {
            let filter = scope.is_in(
                "status",
                vec!["pending".into(), "processing".into(), "completed".into()],
            );
            scope.build_select("orders", filter)
        });

        assert!(sql.contains("IN"));
        assert!(sql.contains("$1"));
        assert!(sql.contains("$2"));
        assert!(sql.contains("$3"));
    }

    #[test]
    fn test_arena_stats() {
        let arena = QueryArena::new();

        arena.scope(|scope| {
            let _ = scope.alloc_str("test string");
            let _ = scope.alloc_str("another string");
        });

        let stats = arena.stats();
        assert_eq!(stats.allocations, 2);
    }
}
