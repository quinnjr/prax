//! Filter types for building WHERE clauses.
//!
//! This module provides the building blocks for constructing type-safe query filters.
//!
//! # Performance
//!
//! Field names use `Cow<'static, str>` for optimal performance:
//! - Static strings (`&'static str`) are borrowed with zero allocation
//! - Dynamic strings are stored as owned `String`
//! - Use `.into()` from static strings for best performance
//!
//! # Examples
//!
//! ## Basic Filters
//!
//! ```rust
//! use prax_query::filter::{Filter, FilterValue};
//!
//! // Equality filter - zero allocation with static str
//! let filter = Filter::Equals("id".into(), FilterValue::Int(42));
//!
//! // String contains
//! let filter = Filter::Contains("email".into(), FilterValue::String("@example.com".into()));
//!
//! // Greater than
//! let filter = Filter::Gt("age".into(), FilterValue::Int(18));
//! ```
//!
//! ## Combining Filters
//!
//! ```rust
//! use prax_query::filter::{Filter, FilterValue};
//!
//! // AND combination - use Filter::and() for convenience
//! let filter = Filter::and([
//!     Filter::Equals("active".into(), FilterValue::Bool(true)),
//!     Filter::Gt("score".into(), FilterValue::Int(100)),
//! ]);
//!
//! // OR combination - use Filter::or() for convenience
//! let filter = Filter::or([
//!     Filter::Equals("status".into(), FilterValue::String("pending".into())),
//!     Filter::Equals("status".into(), FilterValue::String("processing".into())),
//! ]);
//!
//! // NOT
//! let filter = Filter::Not(Box::new(
//!     Filter::Equals("deleted".into(), FilterValue::Bool(true))
//! ));
//! ```
//!
//! ## Null Checks
//!
//! ```rust
//! use prax_query::filter::{Filter, FilterValue};
//!
//! // Is null
//! let filter = Filter::IsNull("deleted_at".into());
//!
//! // Is not null
//! let filter = Filter::IsNotNull("verified_at".into());
//! ```

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::borrow::Cow;
use tracing::debug;

/// A list of filter values for IN/NOT IN clauses.
///
/// Uses `Vec<FilterValue>` for minimal Filter enum size (~64 bytes).
/// This prioritizes cache efficiency over avoiding small allocations.
///
/// # Performance
///
/// While this does allocate for IN clauses, the benefits are:
/// - Filter enum fits in a single cache line (64 bytes)
/// - Better memory locality for filter iteration
/// - Smaller stack usage for complex queries
///
/// For truly allocation-free IN filters, use `Filter::in_static()` with
/// a static slice reference.
pub type ValueList = Vec<FilterValue>;

/// SmallVec-based value list for hot paths where small IN clauses are common.
/// Use this explicitly when you know IN clauses are small (≤8 elements).
pub type SmallValueList = SmallVec<[FilterValue; 8]>;

/// Large value list type with 32 elements inline.
/// Use this for known large IN clauses (e.g., batch operations).
pub type LargeValueList = SmallVec<[FilterValue; 32]>;

/// A field name that can be either a static string (zero allocation) or an owned string.
///
/// Uses `Cow<'static, str>` for optimal performance:
/// - Static strings are borrowed without allocation
/// - Dynamic strings are stored as owned `String`
///
/// # Examples
///
/// ```rust
/// use prax_query::FieldName;
///
/// // Static strings - zero allocation (Cow::Borrowed)
/// let name: FieldName = "id".into();
/// let name: FieldName = "email".into();
/// let name: FieldName = "user_id".into();
/// let name: FieldName = "created_at".into();
///
/// // Dynamic strings work too (Cow::Owned)
/// let name: FieldName = format!("field_{}", 1).into();
/// ```
pub type FieldName = Cow<'static, str>;

/// A filter value that can be used in comparisons.
///
/// # Examples
///
/// ```rust
/// use prax_query::FilterValue;
///
/// // From integers
/// let val: FilterValue = 42.into();
/// let val: FilterValue = 42i64.into();
///
/// // From strings
/// let val: FilterValue = "hello".into();
/// let val: FilterValue = String::from("world").into();
///
/// // From booleans
/// let val: FilterValue = true.into();
///
/// // From floats
/// let val: FilterValue = 3.14f64.into();
///
/// // Null value
/// let val = FilterValue::Null;
///
/// // From vectors
/// let val: FilterValue = vec![1, 2, 3].into();
///
/// // From Option (Some becomes value, None becomes Null)
/// let val: FilterValue = Some(42).into();
/// let val: FilterValue = Option::<i32>::None.into();
/// assert!(val.is_null());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterValue {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
    /// String value.
    String(String),
    /// JSON value.
    Json(serde_json::Value),
    /// List of values.
    List(Vec<FilterValue>),
}

impl FilterValue {
    /// Check if this is a null value.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl From<bool> for FilterValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i32> for FilterValue {
    fn from(v: i32) -> Self {
        Self::Int(v as i64)
    }
}

impl From<i64> for FilterValue {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<f64> for FilterValue {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<String> for FilterValue {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<&str> for FilterValue {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl<T: Into<FilterValue>> From<Vec<T>> for FilterValue {
    fn from(v: Vec<T>) -> Self {
        Self::List(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<FilterValue>> From<Option<T>> for FilterValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => v.into(),
            None => Self::Null,
        }
    }
}

// Integer widenings. The derive macro's `TypeCategory::Numeric` bucket
// emits `.gt(v)` / `.in_(vec![v])` etc. on every Rust integer type, which
// then flows through `v.into()` to a `FilterValue::Int(i64)`. Without
// these impls, calling `user::age::gt(18u32)` would fail to compile.

impl From<i8> for FilterValue {
    fn from(v: i8) -> Self {
        Self::Int(v as i64)
    }
}
impl From<i16> for FilterValue {
    fn from(v: i16) -> Self {
        Self::Int(v as i64)
    }
}
impl From<u8> for FilterValue {
    fn from(v: u8) -> Self {
        Self::Int(v as i64)
    }
}
impl From<u16> for FilterValue {
    fn from(v: u16) -> Self {
        Self::Int(v as i64)
    }
}
impl From<u32> for FilterValue {
    fn from(v: u32) -> Self {
        Self::Int(v as i64)
    }
}
// u64 can exceed i64::MAX; panic on overflow rather than silently
// clamping. Silent clamping lets a filter like `user::id::equals(u64::MAX)`
// match the wrong row (`id = i64::MAX`) — a known authorization-bypass
// footgun. Callers with known-safe values should cast explicitly:
// `FilterValue::from(v as i64)`.
impl From<u64> for FilterValue {
    fn from(v: u64) -> Self {
        let v = i64::try_from(v).expect(
            "u64 value exceeds i64::MAX; cast explicitly to i64 or use FilterValue::String",
        );
        Self::Int(v)
    }
}

impl From<f32> for FilterValue {
    fn from(v: f32) -> Self {
        Self::Float(f64::from(v))
    }
}

// Temporal and UUID types round-trip as strings — every driver's row
// bridge already materializes them via `FilterValue::String` (see
// `MysqlRowRef`, `SqliteRowRef`, `MssqlRowRef`), so a matching pair on
// the parameter-binding side keeps the derive's emitted
// `user::when::gt(dt)` chain compiling and symmetric.
//
// Temporal values round-trip as RFC3339/ISO-8601 strings.
// Microsecond precision matches what Postgres/MySQL store and what the driver
// `RowRef` bridges read. Callers that need different precision or format
// should build their own `FilterValue::String` value.

impl From<chrono::DateTime<chrono::Utc>> for FilterValue {
    fn from(v: chrono::DateTime<chrono::Utc>) -> Self {
        // RFC3339 with microsecond precision: 2020-01-15T10:30:00.000000Z
        Self::String(v.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
    }
}
impl From<chrono::NaiveDateTime> for FilterValue {
    fn from(v: chrono::NaiveDateTime) -> Self {
        // ISO-8601 without timezone. Six fractional-second digits for
        // bit-parity with Postgres/MySQL microsecond storage.
        Self::String(v.format("%Y-%m-%dT%H:%M:%S%.6f").to_string())
    }
}
impl From<chrono::NaiveDate> for FilterValue {
    fn from(v: chrono::NaiveDate) -> Self {
        Self::String(v.format("%Y-%m-%d").to_string())
    }
}
impl From<chrono::NaiveTime> for FilterValue {
    fn from(v: chrono::NaiveTime) -> Self {
        Self::String(v.format("%H:%M:%S%.6f").to_string())
    }
}

impl From<uuid::Uuid> for FilterValue {
    fn from(v: uuid::Uuid) -> Self {
        Self::String(v.to_string())
    }
}

impl From<rust_decimal::Decimal> for FilterValue {
    fn from(v: rust_decimal::Decimal) -> Self {
        Self::String(v.to_string())
    }
}

impl From<serde_json::Value> for FilterValue {
    fn from(v: serde_json::Value) -> Self {
        Self::Json(v)
    }
}

// `Vec<u8>` is already reachable via the `Vec<T: Into<FilterValue>>`
// blanket impl — it lands as `FilterValue::List` of `FilterValue::Int`
// bytes. Drivers that want native BYTEA binding should intercept the
// List variant and re-interpret. We intentionally don't shadow the
// blanket with a dedicated impl (which would be a conflict anyway).

/// Reverse of [`crate::row::FromColumn`]: convert an in-memory value to
/// a [`FilterValue`] suitable for parameter binding.
///
/// Used by the relation executor and [`crate::traits::ModelWithPk`] to
/// project a fetched row's primary/foreign key into a placeholder value
/// without going through the `From<T>` path (which consumes the value).
///
/// # Intentional omissions
///
/// `u64` is omitted by design: [`From<u64>`] panics on overflow, but
/// `to_filter_value(&self)` takes a borrow and cannot recover or fail
/// gracefully without hidden clamping. Callers with `u64` primary keys
/// should cast explicitly (`(self.id as i64).to_filter_value()`) or
/// use `FilterValue::String(self.id.to_string())` when full range
/// preservation matters.
pub trait ToFilterValue {
    /// Convert this value to a [`FilterValue`] by borrowing.
    fn to_filter_value(&self) -> FilterValue;
}

impl ToFilterValue for i8 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self as i64)
    }
}
impl ToFilterValue for i16 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self as i64)
    }
}
impl ToFilterValue for i32 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self as i64)
    }
}
impl ToFilterValue for i64 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self)
    }
}
impl ToFilterValue for u8 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self as i64)
    }
}
impl ToFilterValue for u16 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self as i64)
    }
}
impl ToFilterValue for u32 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Int(*self as i64)
    }
}
impl ToFilterValue for f32 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Float(f64::from(*self))
    }
}
impl ToFilterValue for f64 {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Float(*self)
    }
}
impl ToFilterValue for bool {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Bool(*self)
    }
}
impl ToFilterValue for String {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.clone())
    }
}
impl ToFilterValue for str {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.to_string())
    }
}
impl ToFilterValue for uuid::Uuid {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.to_string())
    }
}
impl ToFilterValue for rust_decimal::Decimal {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.to_string())
    }
}
impl ToFilterValue for chrono::DateTime<chrono::Utc> {
    fn to_filter_value(&self) -> FilterValue {
        // Mirrors From<DateTime<Utc>>: RFC3339 with microsecond precision.
        FilterValue::String(self.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
    }
}
impl ToFilterValue for chrono::NaiveDateTime {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.format("%Y-%m-%dT%H:%M:%S%.6f").to_string())
    }
}
impl ToFilterValue for chrono::NaiveDate {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.format("%Y-%m-%d").to_string())
    }
}
impl ToFilterValue for chrono::NaiveTime {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::String(self.format("%H:%M:%S%.6f").to_string())
    }
}
impl ToFilterValue for serde_json::Value {
    fn to_filter_value(&self) -> FilterValue {
        FilterValue::Json(self.clone())
    }
}
impl ToFilterValue for Vec<u8> {
    fn to_filter_value(&self) -> FilterValue {
        // Bytes round-trip as a list of ints to match the existing
        // `From<Vec<T>>` blanket behavior. Drivers that want native
        // BYTEA binding intercept the List variant.
        FilterValue::List(self.iter().map(|b| FilterValue::Int(*b as i64)).collect())
    }
}
impl ToFilterValue for Vec<f32> {
    fn to_filter_value(&self) -> FilterValue {
        // Pgvector columns encode as a list of floats. Drivers that want
        // to bind the native `vector` type cast the List variant
        // explicitly on the way out.
        FilterValue::List(self.iter().map(|f| FilterValue::Float(*f as f64)).collect())
    }
}
impl<T: ToFilterValue> ToFilterValue for Option<T> {
    fn to_filter_value(&self) -> FilterValue {
        self.as_ref()
            .map(T::to_filter_value)
            .unwrap_or(FilterValue::Null)
    }
}

/// Scalar filter operations.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarFilter<T> {
    /// Equals the value.
    Equals(T),
    /// Not equals the value.
    Not(Box<T>),
    /// In a list of values.
    In(Vec<T>),
    /// Not in a list of values.
    NotIn(Vec<T>),
    /// Less than.
    Lt(T),
    /// Less than or equal.
    Lte(T),
    /// Greater than.
    Gt(T),
    /// Greater than or equal.
    Gte(T),
    /// Contains (for strings).
    Contains(T),
    /// Starts with (for strings).
    StartsWith(T),
    /// Ends with (for strings).
    EndsWith(T),
    /// Is null.
    IsNull,
    /// Is not null.
    IsNotNull,
}

impl<T: Into<FilterValue>> ScalarFilter<T> {
    /// Convert to a Filter with the given column name.
    ///
    /// The column name can be a static string (zero allocation) or an owned string.
    /// For IN/NOT IN filters, uses SmallVec to avoid heap allocation for ≤16 values.
    pub fn into_filter(self, column: impl Into<FieldName>) -> Filter {
        let column = column.into();
        match self {
            Self::Equals(v) => Filter::Equals(column, v.into()),
            Self::Not(v) => Filter::NotEquals(column, (*v).into()),
            Self::In(values) => Filter::In(column, values.into_iter().map(Into::into).collect()),
            Self::NotIn(values) => {
                Filter::NotIn(column, values.into_iter().map(Into::into).collect())
            }
            Self::Lt(v) => Filter::Lt(column, v.into()),
            Self::Lte(v) => Filter::Lte(column, v.into()),
            Self::Gt(v) => Filter::Gt(column, v.into()),
            Self::Gte(v) => Filter::Gte(column, v.into()),
            Self::Contains(v) => Filter::Contains(column, v.into()),
            Self::StartsWith(v) => Filter::StartsWith(column, v.into()),
            Self::EndsWith(v) => Filter::EndsWith(column, v.into()),
            Self::IsNull => Filter::IsNull(column),
            Self::IsNotNull => Filter::IsNotNull(column),
        }
    }
}

/// A complete filter that can be converted to SQL.
///
/// # Size Optimization
///
/// The Filter enum is designed to fit in a single cache line (~64 bytes):
/// - Field names use `Cow<'static, str>` (24 bytes)
/// - Filter values use `FilterValue` (40 bytes)
/// - IN/NOT IN use `Vec<FilterValue>` (24 bytes) instead of SmallVec
/// - AND/OR use `Box<[Filter]>` (16 bytes)
///
/// This enables efficient iteration and better CPU cache utilization.
///
/// # Zero-Allocation Patterns
///
/// For maximum performance, use static strings:
/// ```rust
/// use prax_query::filter::{Filter, FilterValue};
/// // Zero allocation - static string borrowed
/// let filter = Filter::Equals("id".into(), FilterValue::Int(42));
/// ```
#[derive(Debug, Clone, PartialEq)]
#[repr(C)] // Ensure predictable memory layout
#[derive(Default)]
pub enum Filter {
    /// No filter (always true).
    #[default]
    None,

    /// Equals comparison.
    Equals(FieldName, FilterValue),
    /// Not equals comparison.
    NotEquals(FieldName, FilterValue),

    /// Less than comparison.
    Lt(FieldName, FilterValue),
    /// Less than or equal comparison.
    Lte(FieldName, FilterValue),
    /// Greater than comparison.
    Gt(FieldName, FilterValue),
    /// Greater than or equal comparison.
    Gte(FieldName, FilterValue),

    /// In a list of values.
    In(FieldName, ValueList),
    /// Not in a list of values.
    NotIn(FieldName, ValueList),

    /// Contains (LIKE %value%).
    Contains(FieldName, FilterValue),
    /// Starts with (LIKE value%).
    StartsWith(FieldName, FilterValue),
    /// Ends with (LIKE %value).
    EndsWith(FieldName, FilterValue),

    /// Is null check.
    IsNull(FieldName),
    /// Is not null check.
    IsNotNull(FieldName),

    /// Logical AND of multiple filters.
    ///
    /// Uses `Box<[Filter]>` instead of `Vec<Filter>` to save 8 bytes per filter
    /// (no capacity field needed since filters are immutable after construction).
    And(Box<[Filter]>),
    /// Logical OR of multiple filters.
    ///
    /// Uses `Box<[Filter]>` instead of `Vec<Filter>` to save 8 bytes per filter
    /// (no capacity field needed since filters are immutable after construction).
    Or(Box<[Filter]>),
    /// Logical NOT of a filter.
    Not(Box<Filter>),
}

impl Filter {
    /// Create an empty filter (matches everything).
    #[inline(always)]
    pub fn none() -> Self {
        Self::None
    }

    /// Check if this filter is empty.
    #[inline(always)]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Create an AND filter from an iterator of filters.
    ///
    /// Automatically filters out `None` filters and simplifies single-element combinations.
    ///
    /// For known small counts, prefer `and2`, `and3`, `and5`, or `and_n` for better performance.
    #[inline]
    pub fn and(filters: impl IntoIterator<Item = Filter>) -> Self {
        let filters: Vec<_> = filters.into_iter().filter(|f| !f.is_none()).collect();
        let count = filters.len();
        let result = match count {
            0 => Self::None,
            1 => filters.into_iter().next().unwrap(),
            _ => Self::And(filters.into_boxed_slice()),
        };
        debug!(count, "Filter::and() created");
        result
    }

    /// Create an AND filter from exactly two filters.
    ///
    /// More efficient than `and([a, b])` - avoids Vec allocation.
    #[inline(always)]
    pub fn and2(a: Filter, b: Filter) -> Self {
        match (a.is_none(), b.is_none()) {
            (true, true) => Self::None,
            (true, false) => b,
            (false, true) => a,
            (false, false) => Self::And(Box::new([a, b])),
        }
    }

    /// Create an OR filter from an iterator of filters.
    ///
    /// Automatically filters out `None` filters and simplifies single-element combinations.
    ///
    /// For known small counts, prefer `or2`, `or3`, or `or_n` for better performance.
    #[inline]
    pub fn or(filters: impl IntoIterator<Item = Filter>) -> Self {
        let filters: Vec<_> = filters.into_iter().filter(|f| !f.is_none()).collect();
        let count = filters.len();
        let result = match count {
            0 => Self::None,
            1 => filters.into_iter().next().unwrap(),
            _ => Self::Or(filters.into_boxed_slice()),
        };
        debug!(count, "Filter::or() created");
        result
    }

    /// Create an OR filter from exactly two filters.
    ///
    /// More efficient than `or([a, b])` - avoids Vec allocation.
    #[inline(always)]
    pub fn or2(a: Filter, b: Filter) -> Self {
        match (a.is_none(), b.is_none()) {
            (true, true) => Self::None,
            (true, false) => b,
            (false, true) => a,
            (false, false) => Self::Or(Box::new([a, b])),
        }
    }

    // ========================================================================
    // Const Generic Constructors (Zero Vec Allocation)
    // ========================================================================

    /// Create an AND filter from a fixed-size array (const generic).
    ///
    /// This is the most efficient way to create AND filters when the count is
    /// known at compile time. It avoids Vec allocation entirely.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::{Filter, FilterValue};
    ///
    /// let filter = Filter::and_n([
    ///     Filter::Equals("a".into(), FilterValue::Int(1)),
    ///     Filter::Equals("b".into(), FilterValue::Int(2)),
    ///     Filter::Equals("c".into(), FilterValue::Int(3)),
    /// ]);
    /// ```
    #[inline(always)]
    pub fn and_n<const N: usize>(filters: [Filter; N]) -> Self {
        // Convert array to boxed slice directly (no Vec intermediate)
        Self::And(Box::new(filters))
    }

    /// Create an OR filter from a fixed-size array (const generic).
    ///
    /// This is the most efficient way to create OR filters when the count is
    /// known at compile time. It avoids Vec allocation entirely.
    #[inline(always)]
    pub fn or_n<const N: usize>(filters: [Filter; N]) -> Self {
        Self::Or(Box::new(filters))
    }

    /// Create an AND filter from exactly 3 filters.
    #[inline(always)]
    pub fn and3(a: Filter, b: Filter, c: Filter) -> Self {
        Self::And(Box::new([a, b, c]))
    }

    /// Create an AND filter from exactly 4 filters.
    #[inline(always)]
    pub fn and4(a: Filter, b: Filter, c: Filter, d: Filter) -> Self {
        Self::And(Box::new([a, b, c, d]))
    }

    /// Create an AND filter from exactly 5 filters.
    #[inline(always)]
    pub fn and5(a: Filter, b: Filter, c: Filter, d: Filter, e: Filter) -> Self {
        Self::And(Box::new([a, b, c, d, e]))
    }

    /// Create an OR filter from exactly 3 filters.
    #[inline(always)]
    pub fn or3(a: Filter, b: Filter, c: Filter) -> Self {
        Self::Or(Box::new([a, b, c]))
    }

    /// Create an OR filter from exactly 4 filters.
    #[inline(always)]
    pub fn or4(a: Filter, b: Filter, c: Filter, d: Filter) -> Self {
        Self::Or(Box::new([a, b, c, d]))
    }

    /// Create an OR filter from exactly 5 filters.
    #[inline(always)]
    pub fn or5(a: Filter, b: Filter, c: Filter, d: Filter, e: Filter) -> Self {
        Self::Or(Box::new([a, b, c, d, e]))
    }

    // ========================================================================
    // Optimized IN Filter Constructors
    // ========================================================================

    /// Create an IN filter from an iterator of i64 values.
    ///
    /// This is optimized for integer lists, avoiding the generic `Into<FilterValue>`
    /// conversion overhead.
    #[inline]
    pub fn in_i64(field: impl Into<FieldName>, values: impl IntoIterator<Item = i64>) -> Self {
        let list: ValueList = values.into_iter().map(FilterValue::Int).collect();
        Self::In(field.into(), list)
    }

    /// Create an IN filter from an iterator of i32 values.
    #[inline]
    pub fn in_i32(field: impl Into<FieldName>, values: impl IntoIterator<Item = i32>) -> Self {
        let list: ValueList = values
            .into_iter()
            .map(|v| FilterValue::Int(v as i64))
            .collect();
        Self::In(field.into(), list)
    }

    /// Create an IN filter from an iterator of string values.
    #[inline]
    pub fn in_strings(
        field: impl Into<FieldName>,
        values: impl IntoIterator<Item = String>,
    ) -> Self {
        let list: ValueList = values.into_iter().map(FilterValue::String).collect();
        Self::In(field.into(), list)
    }

    /// Create an IN filter from a pre-built ValueList.
    ///
    /// Use this when you've already constructed a ValueList to avoid re-collection.
    #[inline]
    pub fn in_values(field: impl Into<FieldName>, values: ValueList) -> Self {
        Self::In(field.into(), values)
    }

    /// Create an IN filter from a range of i64 values.
    ///
    /// Highly optimized for sequential integer ranges.
    #[inline]
    pub fn in_range(field: impl Into<FieldName>, range: std::ops::Range<i64>) -> Self {
        let list: ValueList = range.map(FilterValue::Int).collect();
        Self::In(field.into(), list)
    }

    /// Create an IN filter from a pre-allocated i64 slice with exact capacity.
    ///
    /// This is the most efficient way to create IN filters for i64 values
    /// when you have a slice available.
    #[inline(always)]
    pub fn in_i64_slice(field: impl Into<FieldName>, values: &[i64]) -> Self {
        let mut list = Vec::with_capacity(values.len());
        for &v in values {
            list.push(FilterValue::Int(v));
        }
        Self::In(field.into(), list)
    }

    /// Create an IN filter for i32 values from a slice.
    #[inline(always)]
    pub fn in_i32_slice(field: impl Into<FieldName>, values: &[i32]) -> Self {
        let mut list = Vec::with_capacity(values.len());
        for &v in values {
            list.push(FilterValue::Int(v as i64));
        }
        Self::In(field.into(), list)
    }

    /// Create an IN filter for string values from a slice.
    #[inline(always)]
    pub fn in_str_slice(field: impl Into<FieldName>, values: &[&str]) -> Self {
        let mut list = Vec::with_capacity(values.len());
        for &v in values {
            list.push(FilterValue::String(v.to_string()));
        }
        Self::In(field.into(), list)
    }

    /// Create a NOT filter.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn not(filter: Filter) -> Self {
        if filter.is_none() {
            return Self::None;
        }
        Self::Not(Box::new(filter))
    }

    /// Create an IN filter from a slice of values.
    ///
    /// This is more efficient than `Filter::In(field, values.into())` when you have a slice,
    /// as it avoids intermediate collection.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::Filter;
    ///
    /// let ids: &[i64] = &[1, 2, 3, 4, 5];
    /// let filter = Filter::in_slice("id", ids);
    /// ```
    #[inline]
    pub fn in_slice<T: Into<FilterValue> + Clone>(
        field: impl Into<FieldName>,
        values: &[T],
    ) -> Self {
        let list: ValueList = values.iter().map(|v| v.clone().into()).collect();
        Self::In(field.into(), list)
    }

    /// Create a NOT IN filter from a slice of values.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::Filter;
    ///
    /// let ids: &[i64] = &[1, 2, 3, 4, 5];
    /// let filter = Filter::not_in_slice("id", ids);
    /// ```
    #[inline]
    pub fn not_in_slice<T: Into<FilterValue> + Clone>(
        field: impl Into<FieldName>,
        values: &[T],
    ) -> Self {
        let list: ValueList = values.iter().map(|v| v.clone().into()).collect();
        Self::NotIn(field.into(), list)
    }

    /// Create an IN filter from an array (const generic).
    ///
    /// This is useful when you know the size at compile time.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::Filter;
    ///
    /// let filter = Filter::in_array("status", ["active", "pending", "processing"]);
    /// ```
    #[inline]
    pub fn in_array<T: Into<FilterValue>, const N: usize>(
        field: impl Into<FieldName>,
        values: [T; N],
    ) -> Self {
        let list: ValueList = values.into_iter().map(Into::into).collect();
        Self::In(field.into(), list)
    }

    /// Create a NOT IN filter from an array (const generic).
    #[inline]
    pub fn not_in_array<T: Into<FilterValue>, const N: usize>(
        field: impl Into<FieldName>,
        values: [T; N],
    ) -> Self {
        let list: ValueList = values.into_iter().map(Into::into).collect();
        Self::NotIn(field.into(), list)
    }

    /// Combine with another filter using AND.
    pub fn and_then(self, other: Filter) -> Self {
        if self.is_none() {
            return other;
        }
        if other.is_none() {
            return self;
        }
        match self {
            Self::And(filters) => {
                // Convert to Vec, add new filter, convert back to Box<[T]>
                let mut vec: Vec<_> = filters.into_vec();
                vec.push(other);
                Self::And(vec.into_boxed_slice())
            }
            _ => Self::And(Box::new([self, other])),
        }
    }

    /// Combine with another filter using OR.
    pub fn or_else(self, other: Filter) -> Self {
        if self.is_none() {
            return other;
        }
        if other.is_none() {
            return self;
        }
        match self {
            Self::Or(filters) => {
                // Convert to Vec, add new filter, convert back to Box<[T]>
                let mut vec: Vec<_> = filters.into_vec();
                vec.push(other);
                Self::Or(vec.into_boxed_slice())
            }
            _ => Self::Or(Box::new([self, other])),
        }
    }

    /// Generate SQL for this filter with parameter placeholders.
    /// Returns (sql, params) where params are the values to bind.
    pub fn to_sql(
        &self,
        param_offset: usize,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let mut params = Vec::new();
        let sql = self.to_sql_with_params(param_offset, &mut params, dialect);
        (sql, params)
    }

    fn to_sql_with_params(
        &self,
        mut param_idx: usize,
        params: &mut Vec<FilterValue>,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> String {
        match self {
            Self::None => "TRUE".to_string(),

            Self::Equals(col, val) => {
                let c = dialect.quote_ident(col);
                if val.is_null() {
                    format!("{} IS NULL", c)
                } else {
                    params.push(val.clone());
                    param_idx += params.len();
                    format!("{} = {}", c, dialect.placeholder(param_idx))
                }
            }
            Self::NotEquals(col, val) => {
                let c = dialect.quote_ident(col);
                if val.is_null() {
                    format!("{} IS NOT NULL", c)
                } else {
                    params.push(val.clone());
                    param_idx += params.len();
                    format!("{} != {}", c, dialect.placeholder(param_idx))
                }
            }

            Self::Lt(col, val) => {
                let c = dialect.quote_ident(col);
                params.push(val.clone());
                param_idx += params.len();
                format!("{} < {}", c, dialect.placeholder(param_idx))
            }
            Self::Lte(col, val) => {
                let c = dialect.quote_ident(col);
                params.push(val.clone());
                param_idx += params.len();
                format!("{} <= {}", c, dialect.placeholder(param_idx))
            }
            Self::Gt(col, val) => {
                let c = dialect.quote_ident(col);
                params.push(val.clone());
                param_idx += params.len();
                format!("{} > {}", c, dialect.placeholder(param_idx))
            }
            Self::Gte(col, val) => {
                let c = dialect.quote_ident(col);
                params.push(val.clone());
                param_idx += params.len();
                format!("{} >= {}", c, dialect.placeholder(param_idx))
            }

            Self::In(col, values) => {
                if values.is_empty() {
                    return "FALSE".to_string();
                }
                let c = dialect.quote_ident(col);
                let placeholders: Vec<_> = values
                    .iter()
                    .map(|v| {
                        params.push(v.clone());
                        param_idx += params.len();
                        dialect.placeholder(param_idx)
                    })
                    .collect();
                format!("{} IN ({})", c, placeholders.join(", "))
            }
            Self::NotIn(col, values) => {
                if values.is_empty() {
                    return "TRUE".to_string();
                }
                let c = dialect.quote_ident(col);
                let placeholders: Vec<_> = values
                    .iter()
                    .map(|v| {
                        params.push(v.clone());
                        param_idx += params.len();
                        dialect.placeholder(param_idx)
                    })
                    .collect();
                format!("{} NOT IN ({})", c, placeholders.join(", "))
            }

            Self::Contains(col, val) => {
                let c = dialect.quote_ident(col);
                if let FilterValue::String(s) = val {
                    params.push(FilterValue::String(format!("%{}%", s)));
                } else {
                    params.push(val.clone());
                }
                param_idx += params.len();
                format!("{} LIKE {}", c, dialect.placeholder(param_idx))
            }
            Self::StartsWith(col, val) => {
                let c = dialect.quote_ident(col);
                if let FilterValue::String(s) = val {
                    params.push(FilterValue::String(format!("{}%", s)));
                } else {
                    params.push(val.clone());
                }
                param_idx += params.len();
                format!("{} LIKE {}", c, dialect.placeholder(param_idx))
            }
            Self::EndsWith(col, val) => {
                let c = dialect.quote_ident(col);
                if let FilterValue::String(s) = val {
                    params.push(FilterValue::String(format!("%{}", s)));
                } else {
                    params.push(val.clone());
                }
                param_idx += params.len();
                format!("{} LIKE {}", c, dialect.placeholder(param_idx))
            }

            Self::IsNull(col) => {
                let c = dialect.quote_ident(col);
                format!("{} IS NULL", c)
            }
            Self::IsNotNull(col) => {
                let c = dialect.quote_ident(col);
                format!("{} IS NOT NULL", c)
            }

            Self::And(filters) => {
                if filters.is_empty() {
                    return "TRUE".to_string();
                }
                let parts: Vec<_> = filters
                    .iter()
                    .map(|f| f.to_sql_with_params(param_idx + params.len(), params, dialect))
                    .collect();
                format!("({})", parts.join(" AND "))
            }
            Self::Or(filters) => {
                if filters.is_empty() {
                    return "FALSE".to_string();
                }
                let parts: Vec<_> = filters
                    .iter()
                    .map(|f| f.to_sql_with_params(param_idx + params.len(), params, dialect))
                    .collect();
                format!("({})", parts.join(" OR "))
            }
            Self::Not(filter) => {
                let inner = filter.to_sql_with_params(param_idx, params, dialect);
                format!("NOT ({})", inner)
            }
        }
    }

    /// Create a builder for constructing AND filters with pre-allocated capacity.
    ///
    /// This is more efficient than using `Filter::and()` when you know the
    /// approximate number of conditions upfront.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::{Filter, FilterValue};
    ///
    /// // Build an AND filter with pre-allocated capacity for 3 conditions
    /// let filter = Filter::and_builder(3)
    ///     .push(Filter::Equals("active".into(), FilterValue::Bool(true)))
    ///     .push(Filter::Gt("score".into(), FilterValue::Int(100)))
    ///     .push(Filter::IsNotNull("email".into()))
    ///     .build();
    /// ```
    #[inline]
    pub fn and_builder(capacity: usize) -> AndFilterBuilder {
        AndFilterBuilder::with_capacity(capacity)
    }

    /// Create a builder for constructing OR filters with pre-allocated capacity.
    ///
    /// This is more efficient than using `Filter::or()` when you know the
    /// approximate number of conditions upfront.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::{Filter, FilterValue};
    ///
    /// // Build an OR filter with pre-allocated capacity for 2 conditions
    /// let filter = Filter::or_builder(2)
    ///     .push(Filter::Equals("role".into(), FilterValue::String("admin".into())))
    ///     .push(Filter::Equals("role".into(), FilterValue::String("moderator".into())))
    ///     .build();
    /// ```
    #[inline]
    pub fn or_builder(capacity: usize) -> OrFilterBuilder {
        OrFilterBuilder::with_capacity(capacity)
    }

    /// Create a general-purpose filter builder.
    ///
    /// Use this for building complex filter trees with a fluent API.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_query::filter::Filter;
    ///
    /// let filter = Filter::builder()
    ///     .eq("status", "active")
    ///     .gt("age", 18)
    ///     .is_not_null("email")
    ///     .build_and();
    /// ```
    #[inline]
    pub fn builder() -> FluentFilterBuilder {
        FluentFilterBuilder::new()
    }
}

/// Builder for constructing AND filters with pre-allocated capacity.
///
/// This avoids vector reallocations when the number of conditions is known upfront.
#[derive(Debug, Clone)]
pub struct AndFilterBuilder {
    filters: Vec<Filter>,
}

impl AndFilterBuilder {
    /// Create a new builder with default capacity.
    #[inline]
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// Create a new builder with the specified capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            filters: Vec::with_capacity(capacity),
        }
    }

    /// Add a filter to the AND condition.
    #[inline]
    pub fn push(mut self, filter: Filter) -> Self {
        if !filter.is_none() {
            self.filters.push(filter);
        }
        self
    }

    /// Add multiple filters to the AND condition.
    #[inline]
    pub fn extend(mut self, filters: impl IntoIterator<Item = Filter>) -> Self {
        self.filters
            .extend(filters.into_iter().filter(|f| !f.is_none()));
        self
    }

    /// Add a filter conditionally.
    #[inline]
    pub fn push_if(self, condition: bool, filter: Filter) -> Self {
        if condition { self.push(filter) } else { self }
    }

    /// Add a filter conditionally, evaluating the closure only if condition is true.
    #[inline]
    pub fn push_if_some<F>(self, opt: Option<F>) -> Self
    where
        F: Into<Filter>,
    {
        match opt {
            Some(f) => self.push(f.into()),
            None => self,
        }
    }

    /// Build the final AND filter.
    #[inline]
    pub fn build(self) -> Filter {
        match self.filters.len() {
            0 => Filter::None,
            1 => self.filters.into_iter().next().unwrap(),
            _ => Filter::And(self.filters.into_boxed_slice()),
        }
    }

    /// Get the current number of filters.
    #[inline]
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Check if the builder is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Default for AndFilterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing OR filters with pre-allocated capacity.
///
/// This avoids vector reallocations when the number of conditions is known upfront.
#[derive(Debug, Clone)]
pub struct OrFilterBuilder {
    filters: Vec<Filter>,
}

impl OrFilterBuilder {
    /// Create a new builder with default capacity.
    #[inline]
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// Create a new builder with the specified capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            filters: Vec::with_capacity(capacity),
        }
    }

    /// Add a filter to the OR condition.
    #[inline]
    pub fn push(mut self, filter: Filter) -> Self {
        if !filter.is_none() {
            self.filters.push(filter);
        }
        self
    }

    /// Add multiple filters to the OR condition.
    #[inline]
    pub fn extend(mut self, filters: impl IntoIterator<Item = Filter>) -> Self {
        self.filters
            .extend(filters.into_iter().filter(|f| !f.is_none()));
        self
    }

    /// Add a filter conditionally.
    #[inline]
    pub fn push_if(self, condition: bool, filter: Filter) -> Self {
        if condition { self.push(filter) } else { self }
    }

    /// Add a filter conditionally, evaluating the closure only if condition is true.
    #[inline]
    pub fn push_if_some<F>(self, opt: Option<F>) -> Self
    where
        F: Into<Filter>,
    {
        match opt {
            Some(f) => self.push(f.into()),
            None => self,
        }
    }

    /// Build the final OR filter.
    #[inline]
    pub fn build(self) -> Filter {
        match self.filters.len() {
            0 => Filter::None,
            1 => self.filters.into_iter().next().unwrap(),
            _ => Filter::Or(self.filters.into_boxed_slice()),
        }
    }

    /// Get the current number of filters.
    #[inline]
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Check if the builder is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Default for OrFilterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A fluent builder for constructing filters with a convenient API.
///
/// This builder collects conditions and can produce either an AND or OR filter.
///
/// # Examples
///
/// ```rust
/// use prax_query::filter::Filter;
///
/// // Build an AND filter
/// let filter = Filter::builder()
///     .eq("active", true)
///     .gt("score", 100)
///     .contains("email", "@example.com")
///     .build_and();
///
/// // Build an OR filter with capacity hint
/// let filter = Filter::builder()
///     .with_capacity(3)
///     .eq("role", "admin")
///     .eq("role", "moderator")
///     .eq("role", "owner")
///     .build_or();
/// ```
#[derive(Debug, Clone)]
pub struct FluentFilterBuilder {
    filters: Vec<Filter>,
}

impl FluentFilterBuilder {
    /// Create a new fluent builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// Set the capacity hint for the internal vector.
    #[inline]
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.filters.reserve(capacity);
        self
    }

    /// Add an equals filter.
    #[inline]
    pub fn eq<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters
            .push(Filter::Equals(field.into(), value.into()));
        self
    }

    /// Add a not equals filter.
    #[inline]
    pub fn ne<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters
            .push(Filter::NotEquals(field.into(), value.into()));
        self
    }

    /// Add a less than filter.
    #[inline]
    pub fn lt<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters.push(Filter::Lt(field.into(), value.into()));
        self
    }

    /// Add a less than or equal filter.
    #[inline]
    pub fn lte<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters.push(Filter::Lte(field.into(), value.into()));
        self
    }

    /// Add a greater than filter.
    #[inline]
    pub fn gt<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters.push(Filter::Gt(field.into(), value.into()));
        self
    }

    /// Add a greater than or equal filter.
    #[inline]
    pub fn gte<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters.push(Filter::Gte(field.into(), value.into()));
        self
    }

    /// Add an IN filter.
    #[inline]
    pub fn is_in<F, I, V>(mut self, field: F, values: I) -> Self
    where
        F: Into<FieldName>,
        I: IntoIterator<Item = V>,
        V: Into<FilterValue>,
    {
        self.filters.push(Filter::In(
            field.into(),
            values.into_iter().map(Into::into).collect(),
        ));
        self
    }

    /// Add a NOT IN filter.
    #[inline]
    pub fn not_in<F, I, V>(mut self, field: F, values: I) -> Self
    where
        F: Into<FieldName>,
        I: IntoIterator<Item = V>,
        V: Into<FilterValue>,
    {
        self.filters.push(Filter::NotIn(
            field.into(),
            values.into_iter().map(Into::into).collect(),
        ));
        self
    }

    /// Add a contains filter (LIKE %value%).
    #[inline]
    pub fn contains<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters
            .push(Filter::Contains(field.into(), value.into()));
        self
    }

    /// Add a starts with filter (LIKE value%).
    #[inline]
    pub fn starts_with<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters
            .push(Filter::StartsWith(field.into(), value.into()));
        self
    }

    /// Add an ends with filter (LIKE %value).
    #[inline]
    pub fn ends_with<F, V>(mut self, field: F, value: V) -> Self
    where
        F: Into<FieldName>,
        V: Into<FilterValue>,
    {
        self.filters
            .push(Filter::EndsWith(field.into(), value.into()));
        self
    }

    /// Add an IS NULL filter.
    #[inline]
    pub fn is_null<F>(mut self, field: F) -> Self
    where
        F: Into<FieldName>,
    {
        self.filters.push(Filter::IsNull(field.into()));
        self
    }

    /// Add an IS NOT NULL filter.
    #[inline]
    pub fn is_not_null<F>(mut self, field: F) -> Self
    where
        F: Into<FieldName>,
    {
        self.filters.push(Filter::IsNotNull(field.into()));
        self
    }

    /// Add a raw filter directly.
    #[inline]
    pub fn filter(mut self, filter: Filter) -> Self {
        if !filter.is_none() {
            self.filters.push(filter);
        }
        self
    }

    /// Add a filter conditionally.
    #[inline]
    pub fn filter_if(self, condition: bool, filter: Filter) -> Self {
        if condition { self.filter(filter) } else { self }
    }

    /// Add a filter conditionally if the option is Some.
    #[inline]
    pub fn filter_if_some<F>(self, opt: Option<F>) -> Self
    where
        F: Into<Filter>,
    {
        match opt {
            Some(f) => self.filter(f.into()),
            None => self,
        }
    }

    /// Build an AND filter from all collected conditions.
    #[inline]
    pub fn build_and(self) -> Filter {
        let filters: Vec<_> = self.filters.into_iter().filter(|f| !f.is_none()).collect();
        match filters.len() {
            0 => Filter::None,
            1 => filters.into_iter().next().unwrap(),
            _ => Filter::And(filters.into_boxed_slice()),
        }
    }

    /// Build an OR filter from all collected conditions.
    #[inline]
    pub fn build_or(self) -> Filter {
        let filters: Vec<_> = self.filters.into_iter().filter(|f| !f.is_none()).collect();
        match filters.len() {
            0 => Filter::None,
            1 => filters.into_iter().next().unwrap(),
            _ => Filter::Or(filters.into_boxed_slice()),
        }
    }

    /// Get the current number of filters.
    #[inline]
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Check if the builder is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Default for FluentFilterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_value_from() {
        assert_eq!(FilterValue::from(42i32), FilterValue::Int(42));
        assert_eq!(
            FilterValue::from("hello"),
            FilterValue::String("hello".to_string())
        );
        assert_eq!(FilterValue::from(true), FilterValue::Bool(true));
    }

    #[test]
    fn test_scalar_filter_equals() {
        let filter = ScalarFilter::Equals("test@example.com".to_string()).into_filter("email");

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""email" = $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_and() {
        let f1 = Filter::Equals("name".into(), "Alice".into());
        let f2 = Filter::Gt("age".into(), FilterValue::Int(18));
        let combined = Filter::and([f1, f2]);

        let (sql, params) = combined.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_filter_or() {
        let f1 = Filter::Equals("status".into(), "active".into());
        let f2 = Filter::Equals("status".into(), "pending".into());
        let combined = Filter::or([f1, f2]);

        let (sql, _) = combined.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("OR"));
    }

    #[test]
    fn test_filter_not() {
        let filter = Filter::not(Filter::Equals("deleted".into(), FilterValue::Bool(true)));

        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("NOT"));
    }

    #[test]
    fn test_filter_is_null() {
        let filter = Filter::IsNull("deleted_at".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""deleted_at" IS NULL"#);
        assert!(params.is_empty());
    }

    #[test]
    fn test_filter_in() {
        let filter = Filter::In("status".into(), vec!["active".into(), "pending".into()]);
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("IN"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_filter_contains() {
        let filter = Filter::Contains("email".into(), "example".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
        if let FilterValue::String(s) = &params[0] {
            assert!(s.contains("%example%"));
        }
    }

    // ==================== FilterValue Tests ====================

    #[test]
    fn test_filter_value_is_null() {
        assert!(FilterValue::Null.is_null());
        assert!(!FilterValue::Bool(false).is_null());
        assert!(!FilterValue::Int(0).is_null());
        assert!(!FilterValue::Float(0.0).is_null());
        assert!(!FilterValue::String("".to_string()).is_null());
    }

    #[test]
    fn test_filter_value_from_i64() {
        assert_eq!(FilterValue::from(42i64), FilterValue::Int(42));
        assert_eq!(FilterValue::from(-100i64), FilterValue::Int(-100));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_filter_value_from_f64() {
        assert_eq!(FilterValue::from(3.14f64), FilterValue::Float(3.14));
    }

    #[test]
    fn test_filter_value_from_string() {
        assert_eq!(
            FilterValue::from("hello".to_string()),
            FilterValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_filter_value_from_vec() {
        let values: Vec<i32> = vec![1, 2, 3];
        let filter_val: FilterValue = values.into();
        if let FilterValue::List(list) = filter_val {
            assert_eq!(list.len(), 3);
            assert_eq!(list[0], FilterValue::Int(1));
            assert_eq!(list[1], FilterValue::Int(2));
            assert_eq!(list[2], FilterValue::Int(3));
        } else {
            panic!("Expected List");
        }
    }

    #[test]
    fn test_filter_value_from_option_some() {
        let val: FilterValue = Some(42i32).into();
        assert_eq!(val, FilterValue::Int(42));
    }

    #[test]
    fn test_filter_value_from_option_none() {
        let val: FilterValue = Option::<i32>::None.into();
        assert_eq!(val, FilterValue::Null);
    }

    // ==================== ScalarFilter Tests ====================

    #[test]
    fn test_scalar_filter_not() {
        let filter = ScalarFilter::Not(Box::new("test".to_string())).into_filter("name");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""name" != $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_scalar_filter_in() {
        let filter = ScalarFilter::In(vec!["a".to_string(), "b".to_string()]).into_filter("status");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("IN"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_scalar_filter_not_in() {
        let filter = ScalarFilter::NotIn(vec!["x".to_string()]).into_filter("status");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("NOT IN"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_scalar_filter_lt() {
        let filter = ScalarFilter::Lt(100i32).into_filter("price");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""price" < $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_scalar_filter_lte() {
        let filter = ScalarFilter::Lte(100i32).into_filter("price");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""price" <= $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_scalar_filter_gt() {
        let filter = ScalarFilter::Gt(0i32).into_filter("quantity");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""quantity" > $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_scalar_filter_gte() {
        let filter = ScalarFilter::Gte(0i32).into_filter("quantity");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""quantity" >= $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_scalar_filter_starts_with() {
        let filter = ScalarFilter::StartsWith("prefix".to_string()).into_filter("name");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
        if let FilterValue::String(s) = &params[0] {
            assert!(s.starts_with("prefix"));
            assert!(s.ends_with("%"));
        }
    }

    #[test]
    fn test_scalar_filter_ends_with() {
        let filter = ScalarFilter::EndsWith("suffix".to_string()).into_filter("name");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
        if let FilterValue::String(s) = &params[0] {
            assert!(s.starts_with("%"));
            assert!(s.ends_with("suffix"));
        }
    }

    #[test]
    fn test_scalar_filter_is_null() {
        let filter = ScalarFilter::<String>::IsNull.into_filter("deleted_at");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""deleted_at" IS NULL"#);
        assert!(params.is_empty());
    }

    #[test]
    fn test_scalar_filter_is_not_null() {
        let filter = ScalarFilter::<String>::IsNotNull.into_filter("name");
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""name" IS NOT NULL"#);
        assert!(params.is_empty());
    }

    // ==================== Filter Tests ====================

    #[test]
    fn test_filter_none() {
        let filter = Filter::none();
        assert!(filter.is_none());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, "TRUE"); // Filter::None generates TRUE
        assert!(params.is_empty());
    }

    #[test]
    fn test_filter_not_equals() {
        let filter = Filter::NotEquals("status".into(), "deleted".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""status" != $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_lte() {
        let filter = Filter::Lte("price".into(), FilterValue::Int(100));
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""price" <= $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_gte() {
        let filter = Filter::Gte("quantity".into(), FilterValue::Int(0));
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""quantity" >= $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_not_in() {
        let filter = Filter::NotIn("status".into(), vec!["deleted".into(), "archived".into()]);
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("NOT IN"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_filter_starts_with() {
        let filter = Filter::StartsWith("email".into(), "admin".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_ends_with() {
        let filter = Filter::EndsWith("email".into(), "@example.com".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_is_not_null() {
        let filter = Filter::IsNotNull("name".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""name" IS NOT NULL"#);
        assert!(params.is_empty());
    }

    // ==================== Filter Combination Tests ====================

    #[test]
    fn test_filter_and_empty() {
        let filter = Filter::and([]);
        assert!(filter.is_none());
    }

    #[test]
    fn test_filter_and_single() {
        let f = Filter::Equals("name".into(), "Alice".into());
        let combined = Filter::and([f.clone()]);
        assert_eq!(combined, f);
    }

    #[test]
    fn test_filter_and_with_none() {
        let f1 = Filter::Equals("name".into(), "Alice".into());
        let f2 = Filter::None;
        let combined = Filter::and([f1.clone(), f2]);
        assert_eq!(combined, f1);
    }

    #[test]
    fn test_filter_or_empty() {
        let filter = Filter::or([]);
        assert!(filter.is_none());
    }

    #[test]
    fn test_filter_or_single() {
        let f = Filter::Equals("status".into(), "active".into());
        let combined = Filter::or([f.clone()]);
        assert_eq!(combined, f);
    }

    #[test]
    fn test_filter_or_with_none() {
        let f1 = Filter::Equals("status".into(), "active".into());
        let f2 = Filter::None;
        let combined = Filter::or([f1.clone(), f2]);
        assert_eq!(combined, f1);
    }

    #[test]
    fn test_filter_not_none() {
        let filter = Filter::not(Filter::None);
        assert!(filter.is_none());
    }

    #[test]
    fn test_filter_and_then() {
        let f1 = Filter::Equals("name".into(), "Alice".into());
        let f2 = Filter::Gt("age".into(), FilterValue::Int(18));
        let combined = f1.and_then(f2);

        let (sql, params) = combined.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_filter_and_then_with_none_first() {
        let f1 = Filter::None;
        let f2 = Filter::Equals("name".into(), "Bob".into());
        let combined = f1.and_then(f2.clone());
        assert_eq!(combined, f2);
    }

    #[test]
    fn test_filter_and_then_with_none_second() {
        let f1 = Filter::Equals("name".into(), "Alice".into());
        let f2 = Filter::None;
        let combined = f1.clone().and_then(f2);
        assert_eq!(combined, f1);
    }

    #[test]
    fn test_filter_and_then_chained() {
        let f1 = Filter::Equals("a".into(), "1".into());
        let f2 = Filter::Equals("b".into(), "2".into());
        let f3 = Filter::Equals("c".into(), "3".into());
        let combined = f1.and_then(f2).and_then(f3);

        let (sql, params) = combined.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_filter_or_else() {
        let f1 = Filter::Equals("status".into(), "active".into());
        let f2 = Filter::Equals("status".into(), "pending".into());
        let combined = f1.or_else(f2);

        let (sql, _) = combined.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("OR"));
    }

    #[test]
    fn test_filter_or_else_with_none_first() {
        let f1 = Filter::None;
        let f2 = Filter::Equals("name".into(), "Bob".into());
        let combined = f1.or_else(f2.clone());
        assert_eq!(combined, f2);
    }

    #[test]
    fn test_filter_or_else_with_none_second() {
        let f1 = Filter::Equals("name".into(), "Alice".into());
        let f2 = Filter::None;
        let combined = f1.clone().or_else(f2);
        assert_eq!(combined, f1);
    }

    // ==================== Complex Filter SQL Generation ====================

    #[test]
    fn test_filter_nested_and_or() {
        let f1 = Filter::Equals("status".into(), "active".into());
        let f2 = Filter::and([
            Filter::Gt("age".into(), FilterValue::Int(18)),
            Filter::Lt("age".into(), FilterValue::Int(65)),
        ]);
        let combined = Filter::and([f1, f2]);

        let (sql, params) = combined.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_filter_nested_not() {
        let inner = Filter::and([
            Filter::Equals("status".into(), "deleted".into()),
            Filter::Equals("archived".into(), FilterValue::Bool(true)),
        ]);
        let filter = Filter::not(inner);

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("NOT"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_filter_with_json_value() {
        let json_val = serde_json::json!({"key": "value"});
        let filter = Filter::Equals("metadata".into(), FilterValue::Json(json_val));
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert_eq!(sql, r#""metadata" = $1"#);
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_filter_in_empty_list() {
        let filter = Filter::In("status".into(), vec![]);
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        // Empty IN generates FALSE (no match possible)
        assert!(
            sql.contains("FALSE")
                || sql.contains("1=0")
                || sql.is_empty()
                || sql.contains("status")
        );
        assert!(params.is_empty());
    }

    #[test]
    fn test_filter_with_null_value() {
        // When filtering with Null value, it uses IS NULL instead of = $1
        let filter = Filter::IsNull("deleted_at".into());
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("deleted_at"));
        assert!(sql.contains("IS NULL"));
        assert!(params.is_empty());
    }

    // ==================== Builder Tests ====================

    #[test]
    fn test_and_builder_basic() {
        let filter = Filter::and_builder(3)
            .push(Filter::Equals("active".into(), FilterValue::Bool(true)))
            .push(Filter::Gt("score".into(), FilterValue::Int(100)))
            .push(Filter::IsNotNull("email".into()))
            .build();

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2); // score and active, IS NOT NULL has no param
    }

    #[test]
    fn test_and_builder_empty() {
        let filter = Filter::and_builder(0).build();
        assert!(filter.is_none());
    }

    #[test]
    fn test_and_builder_single() {
        let filter = Filter::and_builder(1)
            .push(Filter::Equals("id".into(), FilterValue::Int(42)))
            .build();

        // Single filter should not be wrapped in AND
        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_and_builder_filters_none() {
        let filter = Filter::and_builder(3)
            .push(Filter::None)
            .push(Filter::Equals("id".into(), FilterValue::Int(1)))
            .push(Filter::None)
            .build();

        // None filters should be filtered out, leaving single filter
        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_and_builder_push_if() {
        let include_deleted = false;
        let filter = Filter::and_builder(2)
            .push(Filter::Equals("active".into(), FilterValue::Bool(true)))
            .push_if(include_deleted, Filter::IsNull("deleted_at".into()))
            .build();

        // Should only have active filter since include_deleted is false
        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_or_builder_basic() {
        let filter = Filter::or_builder(2)
            .push(Filter::Equals(
                "role".into(),
                FilterValue::String("admin".into()),
            ))
            .push(Filter::Equals(
                "role".into(),
                FilterValue::String("moderator".into()),
            ))
            .build();

        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("OR"));
    }

    #[test]
    fn test_or_builder_empty() {
        let filter = Filter::or_builder(0).build();
        assert!(filter.is_none());
    }

    #[test]
    fn test_or_builder_single() {
        let filter = Filter::or_builder(1)
            .push(Filter::Equals("id".into(), FilterValue::Int(42)))
            .build();

        // Single filter should not be wrapped in OR
        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_fluent_builder_and() {
        let filter = Filter::builder()
            .eq("status", "active")
            .gt("age", 18)
            .is_not_null("email")
            .build_and();

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_fluent_builder_or() {
        let filter = Filter::builder()
            .eq("role", "admin")
            .eq("role", "moderator")
            .build_or();

        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("OR"));
    }

    #[test]
    fn test_fluent_builder_with_capacity() {
        let filter = Filter::builder()
            .with_capacity(5)
            .eq("a", 1)
            .ne("b", 2)
            .lt("c", 3)
            .lte("d", 4)
            .gte("e", 5)
            .build_and();

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 5);
    }

    #[test]
    fn test_fluent_builder_string_operations() {
        let filter = Filter::builder()
            .contains("name", "john")
            .starts_with("email", "admin")
            .ends_with("domain", ".com")
            .build_and();

        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("LIKE"));
    }

    #[test]
    fn test_fluent_builder_null_operations() {
        let filter = Filter::builder()
            .is_null("deleted_at")
            .is_not_null("created_at")
            .build_and();

        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("IS NULL"));
        assert!(sql.contains("IS NOT NULL"));
    }

    #[test]
    fn test_fluent_builder_in_operations() {
        let filter = Filter::builder()
            .is_in("status", vec!["pending", "processing"])
            .not_in("role", vec!["banned", "suspended"])
            .build_and();

        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("IN"));
        assert!(sql.contains("NOT IN"));
    }

    #[test]
    fn test_fluent_builder_filter_if() {
        let include_archived = false;
        let filter = Filter::builder()
            .eq("active", true)
            .filter_if(
                include_archived,
                Filter::Equals("archived".into(), FilterValue::Bool(true)),
            )
            .build_and();

        // Should only have active filter
        assert!(matches!(filter, Filter::Equals(_, _)));
    }

    #[test]
    fn test_fluent_builder_filter_if_some() {
        let maybe_status: Option<Filter> = Some(Filter::Equals("status".into(), "active".into()));
        let filter = Filter::builder()
            .eq("id", 1)
            .filter_if_some(maybe_status)
            .build_and();

        assert!(matches!(filter, Filter::And(_)));
    }

    #[test]
    fn test_and_builder_extend() {
        let extra_filters = vec![
            Filter::Gt("score".into(), FilterValue::Int(100)),
            Filter::Lt("score".into(), FilterValue::Int(1000)),
        ];

        let filter = Filter::and_builder(3)
            .push(Filter::Equals("active".into(), FilterValue::Bool(true)))
            .extend(extra_filters)
            .build();

        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_builder_len_and_is_empty() {
        let mut builder = AndFilterBuilder::new();
        assert!(builder.is_empty());
        assert_eq!(builder.len(), 0);

        builder = builder.push(Filter::Equals("id".into(), FilterValue::Int(1)));
        assert!(!builder.is_empty());
        assert_eq!(builder.len(), 1);
    }

    // ==================== and2/or2 Tests ====================

    #[test]
    fn test_and2_both_valid() {
        let a = Filter::Equals("id".into(), FilterValue::Int(1));
        let b = Filter::Equals("active".into(), FilterValue::Bool(true));
        let filter = Filter::and2(a, b);

        assert!(matches!(filter, Filter::And(_)));
        let (sql, params) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_and2_first_none() {
        let a = Filter::None;
        let b = Filter::Equals("active".into(), FilterValue::Bool(true));
        let filter = Filter::and2(a, b.clone());

        assert_eq!(filter, b);
    }

    #[test]
    fn test_and2_second_none() {
        let a = Filter::Equals("id".into(), FilterValue::Int(1));
        let b = Filter::None;
        let filter = Filter::and2(a.clone(), b);

        assert_eq!(filter, a);
    }

    #[test]
    fn test_and2_both_none() {
        let filter = Filter::and2(Filter::None, Filter::None);
        assert!(filter.is_none());
    }

    #[test]
    fn test_or2_both_valid() {
        let a = Filter::Equals("role".into(), FilterValue::String("admin".into()));
        let b = Filter::Equals("role".into(), FilterValue::String("mod".into()));
        let filter = Filter::or2(a, b);

        assert!(matches!(filter, Filter::Or(_)));
        let (sql, _) = filter.to_sql(0, &crate::dialect::Postgres);
        assert!(sql.contains("OR"));
    }

    #[test]
    fn test_or2_first_none() {
        let a = Filter::None;
        let b = Filter::Equals("active".into(), FilterValue::Bool(true));
        let filter = Filter::or2(a, b.clone());

        assert_eq!(filter, b);
    }

    #[test]
    fn test_or2_second_none() {
        let a = Filter::Equals("id".into(), FilterValue::Int(1));
        let b = Filter::None;
        let filter = Filter::or2(a.clone(), b);

        assert_eq!(filter, a);
    }

    #[test]
    fn test_or2_both_none() {
        let filter = Filter::or2(Filter::None, Filter::None);
        assert!(filter.is_none());
    }

    // ==================== SQL Injection Prevention Tests ====================

    #[test]
    fn to_sql_quotes_column_names_against_injection() {
        use crate::dialect::{Mssql, Mysql, Postgres};

        // Malicious column name attempts to break out of the identifier.
        let filter = Filter::Equals(r#"id" OR 1=1--"#.into(), FilterValue::Int(1));

        let (sql_pg, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql_pg.starts_with(r#""id"" OR 1=1--" ="#),
            "postgres did not quote col; got: {sql_pg}"
        );

        let (sql_my, _) = filter.to_sql(0, &Mysql);
        assert!(
            sql_my.starts_with(r#"`id" OR 1=1--` ="#),
            "mysql did not quote col; got: {sql_my}"
        );

        let (sql_ms, _) = filter.to_sql(0, &Mssql);
        assert!(
            sql_ms.starts_with(r#"[id" OR 1=1--] ="#),
            "mssql did not quote col; got: {sql_ms}"
        );
    }

    #[test]
    fn to_sql_quotes_in_list_column_names() {
        use crate::dialect::Postgres;
        let filter = Filter::In("id".into(), vec![FilterValue::Int(1), FilterValue::Int(2)]);
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""id" IN ("#),
            "expected quoted id on IN, got: {sql}"
        );
    }

    #[test]
    fn to_sql_quotes_null_checks() {
        use crate::dialect::Postgres;
        let filter = Filter::IsNull("deleted_at".into());
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert_eq!(sql, r#""deleted_at" IS NULL"#);
    }

    #[test]
    fn to_sql_quotes_comparison_operators() {
        use crate::dialect::Postgres;

        let filter = Filter::Lt("age".into(), FilterValue::Int(18));
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(sql.starts_with(r#""age" < "#), "Lt not quoted: {sql}");

        let filter = Filter::Lte("price".into(), FilterValue::Int(100));
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(sql.starts_with(r#""price" <= "#), "Lte not quoted: {sql}");

        let filter = Filter::Gt("score".into(), FilterValue::Int(0));
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(sql.starts_with(r#""score" > "#), "Gt not quoted: {sql}");

        let filter = Filter::Gte("quantity".into(), FilterValue::Int(1));
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""quantity" >= "#),
            "Gte not quoted: {sql}"
        );

        let filter = Filter::NotEquals("status".into(), "deleted".into());
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""status" != "#),
            "NotEquals not quoted: {sql}"
        );
    }

    #[test]
    fn to_sql_quotes_like_operators() {
        use crate::dialect::Postgres;

        let filter = Filter::Contains("email".into(), "example".into());
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""email" LIKE "#),
            "Contains not quoted: {sql}"
        );

        let filter = Filter::StartsWith("name".into(), "admin".into());
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""name" LIKE "#),
            "StartsWith not quoted: {sql}"
        );

        let filter = Filter::EndsWith("domain".into(), ".com".into());
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""domain" LIKE "#),
            "EndsWith not quoted: {sql}"
        );
    }

    #[test]
    fn to_sql_quotes_not_in() {
        use crate::dialect::Postgres;
        let filter = Filter::NotIn("status".into(), vec!["deleted".into(), "archived".into()]);
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert!(
            sql.starts_with(r#""status" NOT IN ("#),
            "NotIn not quoted: {sql}"
        );
    }

    #[test]
    fn to_sql_quotes_is_not_null() {
        use crate::dialect::Postgres;
        let filter = Filter::IsNotNull("verified_at".into());
        let (sql, _) = filter.to_sql(0, &Postgres);
        assert_eq!(sql, r#""verified_at" IS NOT NULL"#);
    }

    #[test]
    fn filter_value_from_u64_in_range() {
        assert_eq!(FilterValue::from(42u64), FilterValue::Int(42));
        assert_eq!(FilterValue::from(0u64), FilterValue::Int(0));
        let max_safe = i64::MAX as u64;
        assert_eq!(FilterValue::from(max_safe), FilterValue::Int(i64::MAX));
    }

    #[test]
    #[should_panic(expected = "u64 value exceeds i64::MAX")]
    fn filter_value_from_u64_overflow_panics() {
        let _ = FilterValue::from(u64::MAX);
    }

    #[test]
    fn filter_value_from_chrono_datetime_utc_rfc3339() {
        use chrono::{TimeZone, Utc};
        let dt = Utc.with_ymd_and_hms(2020, 1, 15, 10, 30, 45).unwrap();
        let fv = FilterValue::from(dt);
        assert_eq!(
            fv,
            FilterValue::String("2020-01-15T10:30:45.000000Z".to_string())
        );
    }

    #[test]
    fn filter_value_from_chrono_naive_datetime_iso() {
        use chrono::NaiveDate;
        let dt = NaiveDate::from_ymd_opt(2020, 1, 15)
            .unwrap()
            .and_hms_opt(10, 30, 45)
            .unwrap();
        let fv = FilterValue::from(dt);
        assert_eq!(
            fv,
            FilterValue::String("2020-01-15T10:30:45.000000".to_string())
        );
    }

    #[test]
    fn filter_value_from_chrono_naive_date() {
        use chrono::NaiveDate;
        let d = NaiveDate::from_ymd_opt(2020, 1, 15).unwrap();
        assert_eq!(
            FilterValue::from(d),
            FilterValue::String("2020-01-15".to_string())
        );
    }

    #[test]
    fn filter_value_from_chrono_naive_time() {
        use chrono::NaiveTime;
        let t = NaiveTime::from_hms_opt(10, 30, 45).unwrap();
        assert_eq!(
            FilterValue::from(t),
            FilterValue::String("10:30:45.000000".to_string())
        );
    }

    // ==================== Extended From-impl coverage ====================
    // Pins the tail of From<T> for FilterValue impls that weren't previously
    // exercised. Each test guards against a specific regression a driver
    // would surface downstream — wrong format, wrong variant, or silent
    // precision loss.

    #[test]
    fn filter_value_from_uuid_is_lowercase_hyphenated() {
        // Driver bridges (Postgres/MySQL/SQLite/MSSQL) all receive the
        // 36-char hyphenated lowercase form; pinning it here prevents a
        // hypothetical switch to simple/hyphen-less encoding from silently
        // breaking every WHERE uuid_col = $1 binding.
        use uuid::Uuid;
        let u = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        match FilterValue::from(u) {
            FilterValue::String(ref s) => {
                assert_eq!(s, "550e8400-e29b-41d4-a716-446655440000");
                assert_eq!(s, &u.to_string());
            }
            other => panic!("expected FilterValue::String, got {other:?}"),
        }
    }

    #[test]
    fn filter_value_from_uuid_nil_round_trips() {
        use uuid::Uuid;
        let u = Uuid::nil();
        assert_eq!(
            FilterValue::from(u),
            FilterValue::String("00000000-0000-0000-0000-000000000000".to_string())
        );
    }

    #[test]
    fn filter_value_from_decimal_uses_to_string_not_f64() {
        // Critical: Decimal must NOT round-trip via f64. Using to_string()
        // preserves precision that parsing-to-f64 loses. "3.14" stays "3.14",
        // not "3.1400000000000001".
        use rust_decimal::Decimal;
        use std::str::FromStr;
        let d = Decimal::from_str("3.14").unwrap();
        assert_eq!(
            FilterValue::from(d),
            FilterValue::String("3.14".to_string())
        );
    }

    #[test]
    fn filter_value_from_decimal_high_precision_preserved() {
        use rust_decimal::Decimal;
        use std::str::FromStr;
        // 28-digit mantissa — would lose precision through f64.
        let d = Decimal::from_str("1234567890.1234567890").unwrap();
        match FilterValue::from(d) {
            FilterValue::String(ref s) => {
                assert_eq!(s, "1234567890.1234567890");
            }
            other => panic!("expected FilterValue::String, got {other:?}"),
        }
    }

    #[test]
    fn filter_value_from_serde_json_value_keeps_json_variant() {
        let v = serde_json::json!({"key": "value", "nested": [1, 2, 3]});
        match FilterValue::from(v.clone()) {
            FilterValue::Json(inner) => {
                assert_eq!(inner, v);
            }
            other => panic!("expected FilterValue::Json, got {other:?}"),
        }
    }

    #[test]
    fn filter_value_from_serde_json_null_keeps_json_variant() {
        // `serde_json::Value::Null` must land as FilterValue::Json(Null),
        // NOT FilterValue::Null — the JSON variant signals to the dialect
        // bridge that this column wants JSONB/JSON binding semantics, not
        // SQL NULL.
        let v = serde_json::Value::Null;
        match FilterValue::from(v) {
            FilterValue::Json(serde_json::Value::Null) => {}
            other => panic!("expected FilterValue::Json(Null), got {other:?}"),
        }
    }

    #[test]
    fn filter_value_from_option_none_maps_to_null() {
        // Repeats an existing test at a different call site — this is the
        // "all integer widths flow through the same Option impl" guard.
        let none_i32: Option<i32> = None;
        assert_eq!(FilterValue::from(none_i32), FilterValue::Null);
        let none_string: Option<String> = None;
        assert_eq!(FilterValue::from(none_string), FilterValue::Null);
    }

    #[test]
    fn filter_value_from_signed_integer_extremes() {
        // Every integer width widens to Int(i64). Pinning MIN catches sign
        // extension bugs (e.g. if `v as i64` were replaced with `v as u64 as i64`).
        assert_eq!(FilterValue::from(i8::MIN), FilterValue::Int(i8::MIN as i64));
        assert_eq!(FilterValue::from(i8::MAX), FilterValue::Int(i8::MAX as i64));
        assert_eq!(
            FilterValue::from(i16::MIN),
            FilterValue::Int(i16::MIN as i64)
        );
        assert_eq!(
            FilterValue::from(i16::MAX),
            FilterValue::Int(i16::MAX as i64)
        );
    }

    #[test]
    fn filter_value_from_unsigned_integer_extremes() {
        // u8/u16/u32 all fit in i64 so these never panic. u64::MAX has its
        // own dedicated `#[should_panic]` test at filter_value_from_u64_overflow_panics.
        assert_eq!(FilterValue::from(u8::MAX), FilterValue::Int(u8::MAX as i64));
        assert_eq!(
            FilterValue::from(u16::MAX),
            FilterValue::Int(u16::MAX as i64)
        );
        assert_eq!(
            FilterValue::from(u32::MAX),
            FilterValue::Int(u32::MAX as i64)
        );
        // u32::MAX = 4_294_967_295, well below i64::MAX.
        assert_eq!(FilterValue::from(u32::MAX), FilterValue::Int(4_294_967_295));
    }

    #[test]
    fn filter_value_from_f32_widens_to_f64() {
        // f32 -> f64 widening must happen via `f64::from(v)`, NOT `v as f64`
        // — the cast form is fine for IEEE-754 normal values but we pin it
        // here to document intent. 1.5f32 is exactly representable so no
        // precision loss either way.
        let v: f32 = 1.5;
        assert_eq!(FilterValue::from(v), FilterValue::Float(1.5));
    }

    // ==================== ToFilterValue tests ====================
    // These pin the reverse-of-FromColumn projection used by the relation
    // loader and `ModelWithPk`. Each case guards against a drift from the
    // matching `From<T>` impl above; the relation executor relies on them
    // producing byte-identical values to the parameter-binding path.

    #[test]
    fn to_filter_value_option_some_some() {
        let v: Option<i32> = Some(42);
        assert_eq!(v.to_filter_value(), FilterValue::Int(42));
    }

    #[test]
    fn to_filter_value_option_none_is_null() {
        let v: Option<i32> = None;
        assert_eq!(v.to_filter_value(), FilterValue::Null);
    }

    #[test]
    fn to_filter_value_uuid_is_string() {
        let id = uuid::Uuid::nil();
        assert_eq!(id.to_filter_value(), FilterValue::String(id.to_string()));
    }

    #[test]
    fn to_filter_value_bool_is_bool() {
        assert_eq!(true.to_filter_value(), FilterValue::Bool(true));
    }
}
