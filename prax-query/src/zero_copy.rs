//! Zero-copy types for performance-critical operations.
//!
//! This module provides borrowed/reference-based versions of common types
//! to avoid unnecessary allocations in hot paths.
//!
//! # Types
//!
//! | Owned Type | Zero-Copy Type | Use Case |
//! |------------|----------------|----------|
//! | `JsonPath` | `JsonPathRef<'a>` | JSON path queries with borrowed strings |
//! | `WindowSpec` | `WindowSpecRef<'a>` | Window function specs with borrowed columns |
//! | `Cte` | `CteRef<'a>` | CTE definitions with borrowed column slices |
//!
//! # Performance Benefits
//!
//! - Avoids `String` allocations when using string literals or borrowed data
//! - Reduces memory copies in query building hot paths
//! - Enables zero-copy deserialization patterns
//!
//! # Example
//!
//! ```rust
//! use prax_query::zero_copy::{JsonPathRef, PathSegmentRef, WindowSpecRef};
//! use prax_query::sql::DatabaseType;
//!
//! // Zero-copy JSON path - no allocations!
//! let path = JsonPathRef::new("metadata")
//!     .field("role")
//!     .field("permissions");
//!
//! let sql = path.to_sql(DatabaseType::PostgreSQL);
//!
//! // Zero-copy window spec
//! let spec = WindowSpecRef::new()
//!     .partition_by(&["dept", "team"])
//!     .order_by_asc("salary");
//!
//! let sql = spec.to_sql(DatabaseType::PostgreSQL);
//! ```

use smallvec::SmallVec;
use std::borrow::Cow;

use crate::sql::DatabaseType;
use crate::types::SortOrder;

// ==============================================================================
// Zero-Copy JSON Path
// ==============================================================================

/// A zero-copy JSON path expression that borrows strings where possible.
///
/// This is a more efficient alternative to `JsonPath` when you're working
/// with string literals or borrowed data and don't need to store the path.
///
/// # Example
///
/// ```rust
/// use prax_query::zero_copy::{JsonPathRef, PathSegmentRef};
/// use prax_query::sql::DatabaseType;
///
/// // All string data is borrowed - no allocations
/// let path = JsonPathRef::new("data")
///     .field("user")
///     .field("profile")
///     .index(0);
///
/// // Generate SQL without owning the strings
/// let sql = path.to_sql(DatabaseType::PostgreSQL);
/// assert!(sql.contains("data"));
/// ```
#[derive(Debug, Clone)]
pub struct JsonPathRef<'a> {
    /// The column name containing JSON (borrowed).
    pub column: Cow<'a, str>,
    /// Path segments (may contain borrowed or owned strings).
    pub segments: SmallVec<[PathSegmentRef<'a>; 8]>,
    /// Whether to return text (::text in PostgreSQL).
    pub as_text: bool,
}

/// A segment in a zero-copy JSON path.
#[derive(Debug, Clone, PartialEq)]
pub enum PathSegmentRef<'a> {
    /// Field access with borrowed name.
    Field(Cow<'a, str>),
    /// Array index access.
    Index(i64),
    /// Array wildcard.
    Wildcard,
    /// Recursive descent.
    RecursiveDescent,
}

impl<'a> JsonPathRef<'a> {
    /// Create a new JSON path from a borrowed column name.
    #[inline]
    pub fn new(column: &'a str) -> Self {
        Self {
            column: Cow::Borrowed(column),
            segments: SmallVec::new(),
            as_text: false,
        }
    }

    /// Create a new JSON path from an owned column name.
    #[inline]
    pub fn owned(column: String) -> Self {
        Self {
            column: Cow::Owned(column),
            segments: SmallVec::new(),
            as_text: false,
        }
    }

    /// Create from a JSONPath string (e.g., "$.user.name").
    ///
    /// Note: This may allocate for parsed field names.
    pub fn from_path(column: &'a str, path: &str) -> Self {
        let mut json_path = Self::new(column);

        let path = path.trim_start_matches('$').trim_start_matches('.');

        for segment in path.split('.') {
            if segment.is_empty() {
                continue;
            }

            if let Some(bracket_pos) = segment.find('[') {
                let field_name = &segment[..bracket_pos];
                if !field_name.is_empty() {
                    json_path
                        .segments
                        .push(PathSegmentRef::Field(Cow::Owned(field_name.to_string())));
                }

                if let Some(end_pos) = segment.find(']') {
                    let idx_str = &segment[bracket_pos + 1..end_pos];
                    if idx_str == "*" {
                        json_path.segments.push(PathSegmentRef::Wildcard);
                    } else if let Ok(i) = idx_str.parse::<i64>() {
                        json_path.segments.push(PathSegmentRef::Index(i));
                    }
                }
            } else {
                json_path
                    .segments
                    .push(PathSegmentRef::Field(Cow::Owned(segment.to_string())));
            }
        }

        json_path
    }

    /// Add a field access segment (borrowed).
    #[inline]
    pub fn field(mut self, name: &'a str) -> Self {
        self.segments
            .push(PathSegmentRef::Field(Cow::Borrowed(name)));
        self
    }

    /// Add a field access segment (owned).
    #[inline]
    pub fn field_owned(mut self, name: String) -> Self {
        self.segments.push(PathSegmentRef::Field(Cow::Owned(name)));
        self
    }

    /// Add an array index segment.
    #[inline]
    pub fn index(mut self, idx: i64) -> Self {
        self.segments.push(PathSegmentRef::Index(idx));
        self
    }

    /// Add an array wildcard segment.
    #[inline]
    pub fn all(mut self) -> Self {
        self.segments.push(PathSegmentRef::Wildcard);
        self
    }

    /// Return the value as text instead of JSON.
    #[inline]
    pub fn text(mut self) -> Self {
        self.as_text = true;
        self
    }

    /// Check if this path uses only borrowed data (no allocations).
    pub fn is_zero_copy(&self) -> bool {
        matches!(self.column, Cow::Borrowed(_))
            && self.segments.iter().all(|s| match s {
                PathSegmentRef::Field(cow) => matches!(cow, Cow::Borrowed(_)),
                _ => true,
            })
    }

    /// Get the number of segments.
    #[inline]
    pub fn depth(&self) -> usize {
        self.segments.len()
    }

    /// Generate SQL for this path.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_sql(),
            DatabaseType::MySQL => self.to_mysql_sql(),
            DatabaseType::SQLite => self.to_sqlite_sql(),
            DatabaseType::MSSQL => self.to_mssql_sql(),
        }
    }

    fn to_postgres_sql(&self) -> String {
        let mut sql = String::with_capacity(self.column.len() + self.segments.len() * 16);
        sql.push_str(&self.column);

        let last_idx = self.segments.len().saturating_sub(1);
        for (i, segment) in self.segments.iter().enumerate() {
            match segment {
                PathSegmentRef::Field(name) => {
                    if self.as_text && i == last_idx {
                        sql.push_str(" ->> '");
                    } else {
                        sql.push_str(" -> '");
                    }
                    sql.push_str(name);
                    sql.push('\'');
                }
                PathSegmentRef::Index(idx) => {
                    if self.as_text && i == last_idx {
                        sql.push_str(" ->> ");
                    } else {
                        sql.push_str(" -> ");
                    }
                    sql.push_str(&idx.to_string());
                }
                PathSegmentRef::Wildcard => {
                    sql.push_str(" -> '*'");
                }
                PathSegmentRef::RecursiveDescent => {
                    // PostgreSQL doesn't have native recursive descent
                    sql.push_str(" #> '{}'");
                }
            }
        }

        sql
    }

    fn to_mysql_sql(&self) -> String {
        let mut sql = String::with_capacity(self.column.len() + self.segments.len() * 16);
        sql.push_str(&self.column);

        let last_idx = self.segments.len().saturating_sub(1);
        for (i, segment) in self.segments.iter().enumerate() {
            match segment {
                PathSegmentRef::Field(name) => {
                    if self.as_text && i == last_idx {
                        sql.push_str(" ->> '$.");
                    } else {
                        sql.push_str(" -> '$.");
                    }
                    sql.push_str(name);
                    sql.push('\'');
                }
                PathSegmentRef::Index(idx) => {
                    sql.push_str(" -> '$[");
                    sql.push_str(&idx.to_string());
                    sql.push_str("]'");
                }
                PathSegmentRef::Wildcard => {
                    sql.push_str(" -> '$[*]'");
                }
                PathSegmentRef::RecursiveDescent => {
                    sql.push_str(" -> '$**'");
                }
            }
        }

        sql
    }

    fn to_sqlite_sql(&self) -> String {
        // SQLite uses json_extract function
        let mut path = String::from("$");
        for segment in &self.segments {
            match segment {
                PathSegmentRef::Field(name) => {
                    path.push('.');
                    path.push_str(name);
                }
                PathSegmentRef::Index(idx) => {
                    path.push('[');
                    path.push_str(&idx.to_string());
                    path.push(']');
                }
                PathSegmentRef::Wildcard => {
                    path.push_str("[*]");
                }
                PathSegmentRef::RecursiveDescent => {
                    path.push_str("..");
                }
            }
        }

        format!("json_extract({}, '{}')", self.column, path)
    }

    fn to_mssql_sql(&self) -> String {
        // MSSQL uses JSON_VALUE or JSON_QUERY
        let mut path = String::from("$");
        for segment in &self.segments {
            match segment {
                PathSegmentRef::Field(name) => {
                    path.push('.');
                    path.push_str(name);
                }
                PathSegmentRef::Index(idx) => {
                    path.push('[');
                    path.push_str(&idx.to_string());
                    path.push(']');
                }
                PathSegmentRef::Wildcard | PathSegmentRef::RecursiveDescent => {
                    // MSSQL doesn't support wildcards directly
                    path.push_str("[0]");
                }
            }
        }

        if self.as_text {
            format!("JSON_VALUE({}, '{}')", self.column, path)
        } else {
            format!("JSON_QUERY({}, '{}')", self.column, path)
        }
    }

    /// Convert to owned JsonPath.
    pub fn to_owned(&self) -> crate::json::JsonPath {
        crate::json::JsonPath {
            column: self.column.to_string(),
            segments: self
                .segments
                .iter()
                .map(|s| match s {
                    PathSegmentRef::Field(cow) => crate::json::PathSegment::Field(cow.to_string()),
                    PathSegmentRef::Index(i) => crate::json::PathSegment::Index(*i),
                    PathSegmentRef::Wildcard => crate::json::PathSegment::Wildcard,
                    PathSegmentRef::RecursiveDescent => crate::json::PathSegment::RecursiveDescent,
                })
                .collect(),
            as_text: self.as_text,
        }
    }
}

impl<'a> From<&'a crate::json::JsonPath> for JsonPathRef<'a> {
    fn from(path: &'a crate::json::JsonPath) -> Self {
        Self {
            column: Cow::Borrowed(&path.column),
            segments: path
                .segments
                .iter()
                .map(|s| match s {
                    crate::json::PathSegment::Field(name) => {
                        PathSegmentRef::Field(Cow::Borrowed(name))
                    }
                    crate::json::PathSegment::Index(i) => PathSegmentRef::Index(*i),
                    crate::json::PathSegment::Wildcard => PathSegmentRef::Wildcard,
                    crate::json::PathSegment::RecursiveDescent => PathSegmentRef::RecursiveDescent,
                })
                .collect(),
            as_text: path.as_text,
        }
    }
}

// ==============================================================================
// Zero-Copy Window Spec
// ==============================================================================

/// A zero-copy window specification using borrowed column references.
///
/// This is more efficient than `WindowSpec` when working with string literals.
///
/// # Example
///
/// ```rust
/// use prax_query::zero_copy::WindowSpecRef;
/// use prax_query::sql::DatabaseType;
///
/// // All column names are borrowed - no allocations
/// let spec = WindowSpecRef::new()
///     .partition_by(&["dept", "team"])
///     .order_by_asc("salary")
///     .rows_unbounded_preceding();
///
/// let sql = spec.to_sql(DatabaseType::PostgreSQL);
/// ```
#[derive(Debug, Clone, Default)]
pub struct WindowSpecRef<'a> {
    /// Partition by columns (borrowed slice or owned vec).
    pub partition_by: SmallVec<[Cow<'a, str>; 4]>,
    /// Order by columns with direction.
    pub order_by: SmallVec<[(Cow<'a, str>, SortOrder); 4]>,
    /// Frame clause.
    pub frame: Option<FrameRef<'a>>,
    /// Reference to named window.
    pub window_ref: Option<Cow<'a, str>>,
}

/// A frame clause for window specifications.
#[derive(Debug, Clone)]
pub struct FrameRef<'a> {
    /// Frame type (ROWS, RANGE, GROUPS).
    pub frame_type: FrameTypeRef,
    /// Start bound.
    pub start: FrameBoundRef<'a>,
    /// End bound.
    pub end: Option<FrameBoundRef<'a>>,
}

/// Frame type for window functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameTypeRef {
    Rows,
    Range,
    Groups,
}

/// Frame bound specification.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameBoundRef<'a> {
    UnboundedPreceding,
    Preceding(u32),
    CurrentRow,
    Following(u32),
    UnboundedFollowing,
    /// Expression-based bound (may be borrowed or owned).
    Expr(Cow<'a, str>),
}

impl<'a> WindowSpecRef<'a> {
    /// Create a new empty window spec.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add partition by columns from a slice (zero-copy).
    #[inline]
    pub fn partition_by(mut self, columns: &[&'a str]) -> Self {
        self.partition_by
            .extend(columns.iter().map(|&s| Cow::Borrowed(s)));
        self
    }

    /// Add partition by columns (owned).
    #[inline]
    pub fn partition_by_owned<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.partition_by
            .extend(columns.into_iter().map(|s| Cow::Owned(s.into())));
        self
    }

    /// Add a single partition column (borrowed).
    #[inline]
    pub fn partition_by_col(mut self, column: &'a str) -> Self {
        self.partition_by.push(Cow::Borrowed(column));
        self
    }

    /// Add order by column ascending (borrowed).
    #[inline]
    pub fn order_by_asc(mut self, column: &'a str) -> Self {
        self.order_by.push((Cow::Borrowed(column), SortOrder::Asc));
        self
    }

    /// Add order by column descending (borrowed).
    #[inline]
    pub fn order_by_desc(mut self, column: &'a str) -> Self {
        self.order_by.push((Cow::Borrowed(column), SortOrder::Desc));
        self
    }

    /// Add order by column with direction (borrowed).
    #[inline]
    pub fn order_by(mut self, column: &'a str, order: SortOrder) -> Self {
        self.order_by.push((Cow::Borrowed(column), order));
        self
    }

    /// Add order by column (owned).
    #[inline]
    pub fn order_by_owned(mut self, column: String, order: SortOrder) -> Self {
        self.order_by.push((Cow::Owned(column), order));
        self
    }

    /// Set ROWS frame.
    #[inline]
    pub fn rows(mut self, start: FrameBoundRef<'a>, end: Option<FrameBoundRef<'a>>) -> Self {
        self.frame = Some(FrameRef {
            frame_type: FrameTypeRef::Rows,
            start,
            end,
        });
        self
    }

    /// Set RANGE frame.
    #[inline]
    pub fn range(mut self, start: FrameBoundRef<'a>, end: Option<FrameBoundRef<'a>>) -> Self {
        self.frame = Some(FrameRef {
            frame_type: FrameTypeRef::Range,
            start,
            end,
        });
        self
    }

    /// Set ROWS UNBOUNDED PRECEDING frame (common for running totals).
    #[inline]
    pub fn rows_unbounded_preceding(self) -> Self {
        self.rows(
            FrameBoundRef::UnboundedPreceding,
            Some(FrameBoundRef::CurrentRow),
        )
    }

    /// Set reference to named window.
    #[inline]
    pub fn window_name(mut self, name: &'a str) -> Self {
        self.window_ref = Some(Cow::Borrowed(name));
        self
    }

    /// Check if this spec uses only borrowed data.
    pub fn is_zero_copy(&self) -> bool {
        self.partition_by
            .iter()
            .all(|c| matches!(c, Cow::Borrowed(_)))
            && self
                .order_by
                .iter()
                .all(|(c, _)| matches!(c, Cow::Borrowed(_)))
            && self
                .window_ref
                .as_ref()
                .map(|w| matches!(w, Cow::Borrowed(_)))
                .unwrap_or(true)
    }

    /// Generate SQL for the OVER clause.
    pub fn to_sql(&self, _db_type: DatabaseType) -> String {
        // Window reference
        if let Some(ref name) = self.window_ref {
            return format!("OVER {}", name);
        }

        let mut parts: SmallVec<[String; 4]> = SmallVec::new();

        // PARTITION BY
        if !self.partition_by.is_empty() {
            let cols: Vec<&str> = self.partition_by.iter().map(|c| c.as_ref()).collect();
            parts.push(format!("PARTITION BY {}", cols.join(", ")));
        }

        // ORDER BY
        if !self.order_by.is_empty() {
            let cols: Vec<String> = self
                .order_by
                .iter()
                .map(|(col, order)| {
                    format!(
                        "{} {}",
                        col,
                        match order {
                            SortOrder::Asc => "ASC",
                            SortOrder::Desc => "DESC",
                        }
                    )
                })
                .collect();
            parts.push(format!("ORDER BY {}", cols.join(", ")));
        }

        // Frame
        if let Some(ref frame) = self.frame {
            let frame_type = match frame.frame_type {
                FrameTypeRef::Rows => "ROWS",
                FrameTypeRef::Range => "RANGE",
                FrameTypeRef::Groups => "GROUPS",
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

fn frame_bound_to_sql(bound: &FrameBoundRef<'_>) -> String {
    match bound {
        FrameBoundRef::UnboundedPreceding => "UNBOUNDED PRECEDING".to_string(),
        FrameBoundRef::Preceding(n) => format!("{} PRECEDING", n),
        FrameBoundRef::CurrentRow => "CURRENT ROW".to_string(),
        FrameBoundRef::Following(n) => format!("{} FOLLOWING", n),
        FrameBoundRef::UnboundedFollowing => "UNBOUNDED FOLLOWING".to_string(),
        FrameBoundRef::Expr(expr) => expr.to_string(),
    }
}

// ==============================================================================
// Zero-Copy CTE
// ==============================================================================

/// A zero-copy CTE definition that accepts column slices.
///
/// This is more efficient than `Cte` when working with static column lists.
///
/// # Example
///
/// ```rust
/// use prax_query::zero_copy::CteRef;
/// use prax_query::sql::DatabaseType;
///
/// // Column names are borrowed from a static slice
/// let cte = CteRef::new("active_users")
///     .columns(&["id", "name", "email"])
///     .query("SELECT id, name, email FROM users WHERE active = true");
///
/// let sql = cte.to_sql(DatabaseType::PostgreSQL);
/// ```
#[derive(Debug, Clone, Default)]
pub struct CteRef<'a> {
    /// CTE name.
    pub name: Cow<'a, str>,
    /// Column aliases (borrowed from slice).
    pub columns: SmallVec<[Cow<'a, str>; 8]>,
    /// The defining query.
    pub query: Cow<'a, str>,
    /// Whether this is recursive.
    pub recursive: bool,
    /// Materialization hint (PostgreSQL).
    pub materialized: Option<bool>,
}

impl<'a> CteRef<'a> {
    /// Create a new CTE with a borrowed name.
    #[inline]
    pub fn new(name: &'a str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            columns: SmallVec::new(),
            query: Cow::Borrowed(""),
            recursive: false,
            materialized: None,
        }
    }

    /// Create a new CTE with an owned name.
    #[inline]
    pub fn owned(name: String) -> Self {
        Self {
            name: Cow::Owned(name),
            columns: SmallVec::new(),
            query: Cow::Borrowed(""),
            recursive: false,
            materialized: None,
        }
    }

    /// Set column aliases from a slice (zero-copy).
    #[inline]
    pub fn columns(mut self, cols: &[&'a str]) -> Self {
        self.columns.clear();
        self.columns.extend(cols.iter().map(|&s| Cow::Borrowed(s)));
        self
    }

    /// Set column aliases (owned).
    #[inline]
    pub fn columns_owned<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns.clear();
        self.columns
            .extend(cols.into_iter().map(|s| Cow::Owned(s.into())));
        self
    }

    /// Add a single column (borrowed).
    #[inline]
    pub fn column(mut self, col: &'a str) -> Self {
        self.columns.push(Cow::Borrowed(col));
        self
    }

    /// Set the defining query (borrowed).
    #[inline]
    pub fn query(mut self, q: &'a str) -> Self {
        self.query = Cow::Borrowed(q);
        self
    }

    /// Set the defining query (owned).
    #[inline]
    pub fn query_owned(mut self, q: String) -> Self {
        self.query = Cow::Owned(q);
        self
    }

    /// Mark as recursive CTE.
    #[inline]
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Set materialization hint (PostgreSQL).
    #[inline]
    pub fn materialized(mut self, mat: bool) -> Self {
        self.materialized = Some(mat);
        self
    }

    /// Check if this CTE uses only borrowed data.
    pub fn is_zero_copy(&self) -> bool {
        matches!(self.name, Cow::Borrowed(_))
            && matches!(self.query, Cow::Borrowed(_))
            && self.columns.iter().all(|c| matches!(c, Cow::Borrowed(_)))
    }

    /// Generate the CTE definition SQL.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        let mut sql = String::with_capacity(64 + self.query.len());

        sql.push_str(&self.name);

        // Column list
        if !self.columns.is_empty() {
            sql.push_str(" (");
            let cols: Vec<&str> = self.columns.iter().map(|c| c.as_ref()).collect();
            sql.push_str(&cols.join(", "));
            sql.push(')');
        }

        sql.push_str(" AS ");

        // Materialization hint (PostgreSQL only)
        if matches!(db_type, DatabaseType::PostgreSQL) {
            if let Some(mat) = self.materialized {
                if mat {
                    sql.push_str("MATERIALIZED ");
                } else {
                    sql.push_str("NOT MATERIALIZED ");
                }
            }
        }

        sql.push('(');
        sql.push_str(&self.query);
        sql.push(')');

        sql
    }

    /// Convert to owned Cte.
    pub fn to_owned_cte(&self) -> crate::cte::Cte {
        crate::cte::Cte {
            name: self.name.to_string(),
            columns: self.columns.iter().map(|c| c.to_string()).collect(),
            query: self.query.to_string(),
            recursive: self.recursive,
            materialized: self.materialized.map(|m| {
                if m {
                    crate::cte::Materialized::Yes
                } else {
                    crate::cte::Materialized::No
                }
            }),
            search: None,
            cycle: None,
        }
    }
}

// ==============================================================================
// Zero-Copy WITH Clause Builder
// ==============================================================================

/// A builder for WITH clauses using zero-copy CTEs.
///
/// # Example
///
/// ```rust
/// use prax_query::zero_copy::{CteRef, WithClauseRef};
/// use prax_query::sql::DatabaseType;
///
/// let with = WithClauseRef::new()
///     .cte(CteRef::new("active_users")
///         .columns(&["id", "name"])
///         .query("SELECT id, name FROM users WHERE active = true"))
///     .cte(CteRef::new("recent_orders")
///         .columns(&["user_id", "total"])
///         .query("SELECT user_id, SUM(amount) FROM orders GROUP BY user_id"));
///
/// let sql = with.build_select(&["*"], "active_users", DatabaseType::PostgreSQL);
/// ```
#[derive(Debug, Clone, Default)]
pub struct WithClauseRef<'a> {
    /// CTEs in this WITH clause.
    pub ctes: SmallVec<[CteRef<'a>; 4]>,
    /// Whether this is a recursive WITH.
    pub recursive: bool,
}

impl<'a> WithClauseRef<'a> {
    /// Create a new empty WITH clause.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a CTE.
    #[inline]
    pub fn cte(mut self, cte: CteRef<'a>) -> Self {
        if cte.recursive {
            self.recursive = true;
        }
        self.ctes.push(cte);
        self
    }

    /// Mark as recursive.
    #[inline]
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Build the WITH clause SQL.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        if self.ctes.is_empty() {
            return String::new();
        }

        let mut sql = String::with_capacity(256);

        sql.push_str("WITH ");
        if self.recursive {
            sql.push_str("RECURSIVE ");
        }

        let cte_sqls: Vec<String> = self.ctes.iter().map(|c| c.to_sql(db_type)).collect();
        sql.push_str(&cte_sqls.join(", "));

        sql
    }

    /// Build a complete SELECT query with this WITH clause.
    pub fn build_select(&self, columns: &[&str], from: &str, db_type: DatabaseType) -> String {
        let with_sql = self.to_sql(db_type);
        let cols = if columns.is_empty() || columns == ["*"] {
            "*".to_string()
        } else {
            columns.join(", ")
        };

        if with_sql.is_empty() {
            format!("SELECT {} FROM {}", cols, from)
        } else {
            format!("{} SELECT {} FROM {}", with_sql, cols, from)
        }
    }
}

// ==============================================================================
// Tests
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_path_ref_zero_copy() {
        let path = JsonPathRef::new("data").field("user").field("name");

        assert!(path.is_zero_copy());
        assert_eq!(path.depth(), 2);

        let sql = path.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("data"));
        assert!(sql.contains("user"));
        assert!(sql.contains("name"));
    }

    #[test]
    fn test_json_path_ref_with_index() {
        let path = JsonPathRef::new("items")
            .field("products")
            .index(0)
            .field("name");

        let sql = path.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("products"));
        assert!(sql.contains("0"));
        assert!(sql.contains("name"));
    }

    #[test]
    fn test_json_path_ref_as_text() {
        let path = JsonPathRef::new("data").field("name").text();

        let pg_sql = path.to_sql(DatabaseType::PostgreSQL);
        assert!(pg_sql.contains("->>")); // Text extraction
    }

    #[test]
    fn test_window_spec_ref_zero_copy() {
        let spec = WindowSpecRef::new()
            .partition_by(&["dept", "team"])
            .order_by_asc("salary");

        assert!(spec.is_zero_copy());

        let sql = spec.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("PARTITION BY dept, team"));
        assert!(sql.contains("ORDER BY salary ASC"));
    }

    #[test]
    fn test_window_spec_ref_with_frame() {
        let spec = WindowSpecRef::new()
            .order_by_asc("date")
            .rows_unbounded_preceding();

        let sql = spec.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW"));
    }

    #[test]
    fn test_cte_ref_zero_copy() {
        let cte = CteRef::new("active_users")
            .columns(&["id", "name", "email"])
            .query("SELECT id, name, email FROM users WHERE active = true");

        assert!(cte.is_zero_copy());

        let sql = cte.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("active_users"));
        assert!(sql.contains("id, name, email"));
        assert!(sql.contains("SELECT id, name, email FROM users"));
    }

    #[test]
    fn test_cte_ref_recursive() {
        let cte = CteRef::new("tree")
            .columns(&["id", "parent_id", "level"])
            .query("SELECT id, parent_id, 1 FROM items WHERE parent_id IS NULL")
            .recursive();

        assert!(cte.recursive);
        let sql = cte.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("tree"));
    }

    #[test]
    fn test_with_clause_ref() {
        let with = WithClauseRef::new()
            .cte(
                CteRef::new("active_users")
                    .columns(&["id", "name"])
                    .query("SELECT id, name FROM users WHERE active = true"),
            )
            .cte(
                CteRef::new("orders")
                    .columns(&["user_id", "total"])
                    .query("SELECT user_id, SUM(amount) FROM orders GROUP BY user_id"),
            );

        let sql = with.build_select(&["*"], "active_users", DatabaseType::PostgreSQL);
        assert!(sql.contains("WITH"));
        assert!(sql.contains("active_users"));
        assert!(sql.contains("orders"));
        assert!(sql.contains("SELECT * FROM active_users"));
    }

    #[test]
    fn test_json_path_ref_mysql() {
        let path = JsonPathRef::new("data").field("user").field("email");

        let sql = path.to_sql(DatabaseType::MySQL);
        assert!(sql.contains("data"));
        assert!(sql.contains("$.user"));
    }

    #[test]
    fn test_json_path_ref_sqlite() {
        let path = JsonPathRef::new("config").field("settings").field("theme");

        let sql = path.to_sql(DatabaseType::SQLite);
        assert!(sql.contains("json_extract"));
        assert!(sql.contains("$.settings.theme"));
    }
}
