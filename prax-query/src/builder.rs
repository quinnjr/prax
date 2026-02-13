//! Optimized builder patterns for query construction.
//!
//! This module provides memory-efficient builder types that minimize allocations:
//! - `SmallVec` for small collections (partition columns, order by, etc.)
//! - `Cow<'static, str>` for identifiers that are often static
//! - `SmolStr` for inline small strings (< 24 bytes stored inline)
//! - Reusable builders that can be reset and reused
//!
//! # Performance Characteristics
//!
//! | Type | Stack Size | Inline Capacity | Heap Allocation |
//! |------|------------|-----------------|-----------------|
//! | `SmallVec<[T; 8]>` | 64+ bytes | 8 elements | > 8 elements |
//! | `SmolStr` | 24 bytes | 22 chars | > 22 chars |
//! | `Cow<'static, str>` | 24 bytes | N/A | Only if owned |
//! | `Identifier` | 24 bytes | 22 chars | > 22 chars |
//!
//! # Example
//!
//! ```rust
//! use prax_query::builder::{Identifier, ColumnList, ReusableBuilder};
//!
//! // Identifier that stores small strings inline
//! let col = Identifier::new("user_id"); // No heap allocation
//! let long_col = Identifier::new("very_long_column_name_here"); // May heap allocate
//!
//! // Column list optimized for typical use (1-8 columns)
//! let mut cols = ColumnList::new();
//! cols.push("id");
//! cols.push("name");
//! cols.push("email"); // Still on stack!
//!
//! // Reusable builder pattern
//! let mut builder = ReusableBuilder::new();
//! builder.push("SELECT * FROM users");
//! let sql1 = builder.build();
//! builder.reset(); // Reuse the allocation
//! builder.push("SELECT * FROM posts");
//! let sql2 = builder.build();
//! ```

use smallvec::SmallVec;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::fmt;

// ==============================================================================
// Identifier Type (Inline Small Strings)
// ==============================================================================

/// An identifier (column name, table name, alias) optimized for small strings.
///
/// Uses `SmolStr` internally which stores strings up to 22 bytes inline,
/// avoiding heap allocation for typical identifier names.
///
/// # Examples
///
/// ```rust
/// use prax_query::builder::Identifier;
///
/// let id = Identifier::new("user_id");
/// assert_eq!(id.as_str(), "user_id");
///
/// // From static str (zero-copy)
/// let id: Identifier = "email".into();
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Identifier(SmolStr);

impl Identifier {
    /// Create a new identifier from any string-like type.
    #[inline]
    pub fn new(s: impl AsRef<str>) -> Self {
        Self(SmolStr::new(s.as_ref()))
    }

    /// Create from a static string (zero allocation).
    #[inline]
    pub const fn from_static(s: &'static str) -> Self {
        Self(SmolStr::new_static(s))
    }

    /// Get the identifier as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Check if the string is stored inline (no heap allocation).
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.0.is_heap_allocated() == false
    }

    /// Get the length of the identifier.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the identifier is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Identifier({:?})", self.0.as_str())
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for Identifier {
    #[inline]
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Identifier {
    #[inline]
    fn from(s: String) -> Self {
        Self(SmolStr::new(&s))
    }
}

impl From<&String> for Identifier {
    #[inline]
    fn from(s: &String) -> Self {
        Self(SmolStr::new(s))
    }
}

impl From<Cow<'_, str>> for Identifier {
    #[inline]
    fn from(s: Cow<'_, str>) -> Self {
        Self(SmolStr::new(&s))
    }
}

impl AsRef<str> for Identifier {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Default for Identifier {
    fn default() -> Self {
        Self(SmolStr::default())
    }
}

// ==============================================================================
// Cow Identifier (Copy-on-Write)
// ==============================================================================

/// A copy-on-write identifier that borrows static strings without allocation.
///
/// Use this when identifiers are often string literals but occasionally
/// need to be dynamically generated.
///
/// # Examples
///
/// ```rust
/// use prax_query::builder::CowIdentifier;
///
/// // Static string - zero allocation
/// let id = CowIdentifier::borrowed("user_id");
///
/// // Dynamic string - allocates if not static
/// let dynamic_name = format!("col_{}", 1);
/// let id = CowIdentifier::owned(dynamic_name);
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CowIdentifier<'a>(Cow<'a, str>);

impl<'a> CowIdentifier<'a> {
    /// Create from a borrowed static string (zero allocation).
    #[inline]
    pub const fn borrowed(s: &'a str) -> Self {
        Self(Cow::Borrowed(s))
    }

    /// Create from an owned string.
    #[inline]
    pub fn owned(s: String) -> Self {
        Self(Cow::Owned(s))
    }

    /// Create from any string-like type.
    #[inline]
    pub fn new(s: impl Into<Cow<'a, str>>) -> Self {
        Self(s.into())
    }

    /// Get as string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check if this is a borrowed (non-allocating) reference.
    #[inline]
    pub fn is_borrowed(&self) -> bool {
        matches!(self.0, Cow::Borrowed(_))
    }

    /// Convert to owned, cloning if necessary.
    #[inline]
    pub fn into_owned(self) -> String {
        self.0.into_owned()
    }
}

impl<'a> fmt::Debug for CowIdentifier<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CowIdentifier({:?}, borrowed={})",
            self.0.as_ref(),
            self.is_borrowed()
        )
    }
}

impl<'a> fmt::Display for CowIdentifier<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<'a> From<&'a str> for CowIdentifier<'a> {
    #[inline]
    fn from(s: &'a str) -> Self {
        Self::borrowed(s)
    }
}

impl From<String> for CowIdentifier<'static> {
    #[inline]
    fn from(s: String) -> Self {
        Self::owned(s)
    }
}

impl<'a> AsRef<str> for CowIdentifier<'a> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'a> Default for CowIdentifier<'a> {
    fn default() -> Self {
        Self::borrowed("")
    }
}

// ==============================================================================
// SmallVec-based Collections
// ==============================================================================

/// A list of columns optimized for typical use cases (1-8 columns).
///
/// Uses `SmallVec` to store up to 8 identifiers on the stack,
/// only heap-allocating for larger lists.
pub type ColumnList = SmallVec<[Identifier; 8]>;

/// A list of column names as strings, optimized for 1-8 columns.
pub type ColumnNameList = SmallVec<[String; 8]>;

/// A list of column names as Cow strings for zero-copy static columns.
pub type CowColumnList<'a> = SmallVec<[Cow<'a, str>; 8]>;

/// A list of sort orders, optimized for 1-4 ORDER BY columns.
pub type OrderByList = SmallVec<[(Identifier, crate::types::SortOrder); 4]>;

/// A list of partition columns, optimized for 1-4 PARTITION BY columns.
pub type PartitionByList = SmallVec<[Identifier; 4]>;

/// A list of expressions, optimized for 1-8 items.
pub type ExprList = SmallVec<[String; 8]>;

/// A list of values, optimized for 1-16 items (e.g., IN clauses).
pub type ValueList<T> = SmallVec<[T; 16]>;

// ==============================================================================
// Reusable Builder
// ==============================================================================

/// A reusable string builder that can be reset and reused.
///
/// This is useful for building multiple queries in a loop without
/// reallocating the buffer each time.
///
/// # Example
///
/// ```rust
/// use prax_query::builder::ReusableBuilder;
///
/// let mut builder = ReusableBuilder::with_capacity(256);
///
/// for i in 0..10 {
///     builder.push("SELECT * FROM users WHERE id = ");
///     builder.push(&i.to_string());
///     let sql = builder.take(); // Take ownership without reallocating
///     // Use sql...
///     builder.reset(); // Clear for next iteration
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ReusableBuilder {
    buffer: String,
    /// Track the initial capacity for efficient reset
    initial_capacity: usize,
}

impl ReusableBuilder {
    /// Create a new builder with default capacity.
    #[inline]
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            initial_capacity: 0,
        }
    }

    /// Create with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: String::with_capacity(capacity),
            initial_capacity: capacity,
        }
    }

    /// Push a string slice.
    #[inline]
    pub fn push(&mut self, s: &str) -> &mut Self {
        self.buffer.push_str(s);
        self
    }

    /// Push a single character.
    #[inline]
    pub fn push_char(&mut self, c: char) -> &mut Self {
        self.buffer.push(c);
        self
    }

    /// Push formatted content.
    #[inline]
    pub fn push_fmt(&mut self, args: fmt::Arguments<'_>) -> &mut Self {
        use std::fmt::Write;
        let _ = self.buffer.write_fmt(args);
        self
    }

    /// Push a space character.
    #[inline]
    pub fn space(&mut self) -> &mut Self {
        self.buffer.push(' ');
        self
    }

    /// Push a comma and space.
    #[inline]
    pub fn comma(&mut self) -> &mut Self {
        self.buffer.push_str(", ");
        self
    }

    /// Get the current content as a slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.buffer
    }

    /// Get the current length.
    #[inline]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Build and return a clone of the content.
    #[inline]
    pub fn build(&self) -> String {
        self.buffer.clone()
    }

    /// Take ownership of the buffer, leaving an empty string.
    #[inline]
    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.buffer)
    }

    /// Reset the builder for reuse, keeping capacity.
    #[inline]
    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    /// Reset and shrink to initial capacity if grown significantly.
    #[inline]
    pub fn reset_shrink(&mut self) {
        self.buffer.clear();
        if self.buffer.capacity() > self.initial_capacity * 2 {
            self.buffer.shrink_to(self.initial_capacity);
        }
    }

    /// Reserve additional capacity.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.buffer.reserve(additional);
    }

    /// Get the current capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }
}

impl Default for ReusableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ReusableBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.buffer)
    }
}

impl From<ReusableBuilder> for String {
    fn from(builder: ReusableBuilder) -> String {
        builder.buffer
    }
}

// ==============================================================================
// Builder Pool (for high-throughput scenarios)
// ==============================================================================

/// A pool of reusable builders for high-throughput scenarios.
///
/// This is useful when building many queries concurrently, as it
/// allows reusing allocated buffers across requests.
///
/// # Example
///
/// ```rust
/// use prax_query::builder::BuilderPool;
///
/// let pool = BuilderPool::new(16, 256); // 16 builders, 256 byte capacity each
///
/// // Get a builder from the pool
/// let mut builder = pool.get();
/// builder.push("SELECT * FROM users");
/// let sql = builder.take();
/// pool.put(builder); // Return to pool for reuse
/// ```
pub struct BuilderPool {
    builders: parking_lot::Mutex<Vec<ReusableBuilder>>,
    capacity: usize,
}

impl BuilderPool {
    /// Create a new pool with the specified size and builder capacity.
    pub fn new(pool_size: usize, builder_capacity: usize) -> Self {
        let builders: Vec<_> = (0..pool_size)
            .map(|_| ReusableBuilder::with_capacity(builder_capacity))
            .collect();
        Self {
            builders: parking_lot::Mutex::new(builders),
            capacity: builder_capacity,
        }
    }

    /// Get a builder from the pool, or create a new one if empty.
    #[inline]
    pub fn get(&self) -> ReusableBuilder {
        self.builders
            .lock()
            .pop()
            .unwrap_or_else(|| ReusableBuilder::with_capacity(self.capacity))
    }

    /// Return a builder to the pool for reuse.
    #[inline]
    pub fn put(&self, mut builder: ReusableBuilder) {
        builder.reset_shrink();
        self.builders.lock().push(builder);
    }

    /// Get the current pool size.
    pub fn len(&self) -> usize {
        self.builders.lock().len()
    }

    /// Check if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.builders.lock().is_empty()
    }
}

// ==============================================================================
// Optimized Window Spec Builder
// ==============================================================================

/// An optimized window specification using SmallVec for partition/order columns.
///
/// This is a more memory-efficient version of `WindowSpec` that uses
/// stack-allocated small vectors for typical use cases.
#[derive(Debug, Clone, Default)]
pub struct OptimizedWindowSpec {
    /// Partition columns (typically 1-4).
    pub partition_by: PartitionByList,
    /// Order by columns with direction (typically 1-4).
    pub order_by: SmallVec<[(Identifier, crate::types::SortOrder); 4]>,
    /// Frame type.
    pub frame: Option<WindowFrame>,
    /// Reference to a named window.
    pub window_ref: Option<Identifier>,
}

/// Window frame specification.
#[derive(Debug, Clone)]
pub struct WindowFrame {
    /// Frame type (ROWS, RANGE, GROUPS).
    pub frame_type: FrameType,
    /// Start bound.
    pub start: FrameBound,
    /// End bound (None = CURRENT ROW).
    pub end: Option<FrameBound>,
}

/// Frame type for window functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Rows,
    Range,
    Groups,
}

/// Frame bound specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameBound {
    UnboundedPreceding,
    Preceding(u32),
    CurrentRow,
    Following(u32),
    UnboundedFollowing,
}

impl OptimizedWindowSpec {
    /// Create a new empty window spec.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add partition by columns.
    #[inline]
    pub fn partition_by<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Identifier>,
    {
        self.partition_by
            .extend(columns.into_iter().map(Into::into));
        self
    }

    /// Add a single partition column.
    #[inline]
    pub fn partition_by_col(mut self, column: impl Into<Identifier>) -> Self {
        self.partition_by.push(column.into());
        self
    }

    /// Add order by column with sort direction.
    #[inline]
    pub fn order_by(
        mut self,
        column: impl Into<Identifier>,
        order: crate::types::SortOrder,
    ) -> Self {
        self.order_by.push((column.into(), order));
        self
    }

    /// Set frame to ROWS BETWEEN ... AND ...
    #[inline]
    pub fn rows(mut self, start: FrameBound, end: Option<FrameBound>) -> Self {
        self.frame = Some(WindowFrame {
            frame_type: FrameType::Rows,
            start,
            end,
        });
        self
    }

    /// Set frame to ROWS UNBOUNDED PRECEDING.
    #[inline]
    pub fn rows_unbounded_preceding(self) -> Self {
        self.rows(FrameBound::UnboundedPreceding, Some(FrameBound::CurrentRow))
    }

    /// Set a reference to a named window.
    #[inline]
    pub fn window_ref(mut self, name: impl Into<Identifier>) -> Self {
        self.window_ref = Some(name.into());
        self
    }

    /// Generate SQL for the OVER clause.
    pub fn to_sql(&self, _db_type: crate::sql::DatabaseType) -> String {
        let mut parts: SmallVec<[String; 4]> = SmallVec::new();

        // Window reference
        if let Some(ref name) = self.window_ref {
            return format!("OVER {}", name);
        }

        // PARTITION BY
        if !self.partition_by.is_empty() {
            let cols: Vec<_> = self.partition_by.iter().map(|c| c.as_str()).collect();
            parts.push(format!("PARTITION BY {}", cols.join(", ")));
        }

        // ORDER BY
        if !self.order_by.is_empty() {
            let cols: Vec<_> = self
                .order_by
                .iter()
                .map(|(col, order)| {
                    format!(
                        "{} {}",
                        col.as_str(),
                        match order {
                            crate::types::SortOrder::Asc => "ASC",
                            crate::types::SortOrder::Desc => "DESC",
                        }
                    )
                })
                .collect();
            parts.push(format!("ORDER BY {}", cols.join(", ")));
        }

        // Frame clause
        if let Some(ref frame) = self.frame {
            let frame_type = match frame.frame_type {
                FrameType::Rows => "ROWS",
                FrameType::Range => "RANGE",
                FrameType::Groups => "GROUPS",
            };

            let start = frame_bound_to_sql(&frame.start);

            if let Some(ref end) = frame.end {
                let end_sql = frame_bound_to_sql(end);
                parts.push(format!("{} BETWEEN {} AND {}", frame_type, start, end_sql));
            } else {
                parts.push(format!("{} {}", frame_type, start));
            }
        }

        if parts.is_empty() {
            "OVER ()".to_string()
        } else {
            format!("OVER ({})", parts.join(" "))
        }
    }
}

fn frame_bound_to_sql(bound: &FrameBound) -> &'static str {
    match bound {
        FrameBound::UnboundedPreceding => "UNBOUNDED PRECEDING",
        FrameBound::Preceding(_) => "PRECEDING", // Would need dynamic
        FrameBound::CurrentRow => "CURRENT ROW",
        FrameBound::Following(_) => "FOLLOWING", // Would need dynamic
        FrameBound::UnboundedFollowing => "UNBOUNDED FOLLOWING",
    }
}

// ==============================================================================
// Tests
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identifier_inline() {
        let id = Identifier::new("user_id");
        assert_eq!(id.as_str(), "user_id");
        assert!(id.is_inline()); // Should be stored inline (< 22 chars)
    }

    #[test]
    fn test_identifier_from_static() {
        let id = Identifier::from_static("email");
        assert_eq!(id.as_str(), "email");
    }

    #[test]
    fn test_cow_identifier_borrowed() {
        let id = CowIdentifier::borrowed("user_id");
        assert!(id.is_borrowed());
        assert_eq!(id.as_str(), "user_id");
    }

    #[test]
    fn test_cow_identifier_owned() {
        let id = CowIdentifier::owned("dynamic".to_string());
        assert!(!id.is_borrowed());
        assert_eq!(id.as_str(), "dynamic");
    }

    #[test]
    fn test_column_list_stack_allocation() {
        let mut cols: ColumnList = SmallVec::new();
        cols.push(Identifier::new("id"));
        cols.push(Identifier::new("name"));
        cols.push(Identifier::new("email"));
        cols.push(Identifier::new("created_at"));

        // Should not have spilled to heap (< 8 items)
        assert!(!cols.spilled());
        assert_eq!(cols.len(), 4);
    }

    #[test]
    fn test_column_list_heap_spillover() {
        let mut cols: ColumnList = SmallVec::new();
        for i in 0..10 {
            cols.push(Identifier::new(format!("col_{}", i)));
        }

        // Should have spilled to heap (> 8 items)
        assert!(cols.spilled());
        assert_eq!(cols.len(), 10);
    }

    #[test]
    fn test_reusable_builder() {
        let mut builder = ReusableBuilder::with_capacity(64);

        builder.push("SELECT * FROM users");
        assert_eq!(builder.as_str(), "SELECT * FROM users");

        builder.reset();
        assert!(builder.is_empty());
        assert!(builder.capacity() >= 64); // Capacity preserved

        builder.push("SELECT * FROM posts");
        assert_eq!(builder.as_str(), "SELECT * FROM posts");
    }

    #[test]
    fn test_reusable_builder_take() {
        let mut builder = ReusableBuilder::new();
        builder.push("test");

        let taken = builder.take();
        assert_eq!(taken, "test");
        assert!(builder.is_empty());
    }

    #[test]
    fn test_builder_pool() {
        let pool = BuilderPool::new(4, 128);

        // Get all builders
        let b1 = pool.get();
        let b2 = pool.get();
        let _b3 = pool.get();
        let _b4 = pool.get();

        // Pool should be empty
        assert!(pool.is_empty());

        // Return some
        pool.put(b1);
        pool.put(b2);

        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_optimized_window_spec() {
        use crate::types::SortOrder;

        let spec = OptimizedWindowSpec::new()
            .partition_by(["dept", "team"])
            .order_by("salary", SortOrder::Desc)
            .rows_unbounded_preceding();

        let sql = spec.to_sql(crate::sql::DatabaseType::PostgreSQL);
        assert!(sql.contains("PARTITION BY"));
        assert!(sql.contains("ORDER BY"));
        assert!(sql.contains("ROWS"));
    }
}
