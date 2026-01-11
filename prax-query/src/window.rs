//! Window functions support.
//!
//! This module provides types for building window functions (OVER clauses)
//! across different database backends.
//!
//! # Supported Features
//!
//! | Feature         | PostgreSQL | MySQL | SQLite | MSSQL | MongoDB 5.0+ |
//! |-----------------|------------|-------|--------|-------|--------------|
//! | ROW_NUMBER      | ✅         | ✅    | ✅     | ✅    | ✅           |
//! | RANK/DENSE_RANK | ✅         | ✅    | ✅     | ✅    | ✅           |
//! | LAG/LEAD        | ✅         | ✅    | ✅     | ✅    | ✅           |
//! | Frame clauses   | ✅         | ✅    | ✅     | ✅    | ✅           |
//! | Named windows   | ✅         | ✅    | ✅     | ❌    | ❌           |
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::window::{WindowFunction, WindowSpec, row_number, rank, sum};
//!
//! // ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC)
//! let wf = row_number()
//!     .over(WindowSpec::new()
//!         .partition_by(["dept"])
//!         .order_by("salary", SortOrder::Desc));
//!
//! // Running total
//! let running = sum("amount")
//!     .over(WindowSpec::new()
//!         .order_by("date", SortOrder::Asc)
//!         .rows_unbounded_preceding());
//! ```

use serde::{Deserialize, Serialize};

use crate::sql::DatabaseType;
use crate::types::SortOrder;

/// A window function with its OVER clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowFunction {
    /// The function being called.
    pub function: WindowFn,
    /// The OVER clause specification.
    pub over: WindowSpec,
    /// Optional alias for the result.
    pub alias: Option<String>,
}

/// Available window functions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WindowFn {
    // Ranking functions
    /// ROW_NUMBER() - Sequential row number.
    RowNumber,
    /// RANK() - Rank with gaps.
    Rank,
    /// DENSE_RANK() - Rank without gaps.
    DenseRank,
    /// NTILE(n) - Distribute rows into n buckets.
    Ntile(u32),
    /// PERCENT_RANK() - Relative rank (0 to 1).
    PercentRank,
    /// CUME_DIST() - Cumulative distribution.
    CumeDist,

    // Value functions
    /// LAG(expr, offset, default) - Value from previous row.
    Lag {
        expr: String,
        offset: Option<u32>,
        default: Option<String>,
    },
    /// LEAD(expr, offset, default) - Value from next row.
    Lead {
        expr: String,
        offset: Option<u32>,
        default: Option<String>,
    },
    /// FIRST_VALUE(expr) - First value in frame.
    FirstValue(String),
    /// LAST_VALUE(expr) - Last value in frame.
    LastValue(String),
    /// NTH_VALUE(expr, n) - Nth value in frame.
    NthValue(String, u32),

    // Aggregate functions as window functions
    /// SUM(expr).
    Sum(String),
    /// AVG(expr).
    Avg(String),
    /// COUNT(expr).
    Count(String),
    /// MIN(expr).
    Min(String),
    /// MAX(expr).
    Max(String),
    /// Custom function.
    Custom { name: String, args: Vec<String> },
}

impl WindowFn {
    /// Generate the function SQL.
    pub fn to_sql(&self) -> String {
        match self {
            Self::RowNumber => "ROW_NUMBER()".to_string(),
            Self::Rank => "RANK()".to_string(),
            Self::DenseRank => "DENSE_RANK()".to_string(),
            Self::Ntile(n) => format!("NTILE({})", n),
            Self::PercentRank => "PERCENT_RANK()".to_string(),
            Self::CumeDist => "CUME_DIST()".to_string(),
            Self::Lag {
                expr,
                offset,
                default,
            } => {
                let mut sql = format!("LAG({})", expr);
                if let Some(off) = offset {
                    sql = format!("LAG({}, {})", expr, off);
                    if let Some(def) = default {
                        sql = format!("LAG({}, {}, {})", expr, off, def);
                    }
                }
                sql
            }
            Self::Lead {
                expr,
                offset,
                default,
            } => {
                let mut sql = format!("LEAD({})", expr);
                if let Some(off) = offset {
                    sql = format!("LEAD({}, {})", expr, off);
                    if let Some(def) = default {
                        sql = format!("LEAD({}, {}, {})", expr, off, def);
                    }
                }
                sql
            }
            Self::FirstValue(expr) => format!("FIRST_VALUE({})", expr),
            Self::LastValue(expr) => format!("LAST_VALUE({})", expr),
            Self::NthValue(expr, n) => format!("NTH_VALUE({}, {})", expr, n),
            Self::Sum(expr) => format!("SUM({})", expr),
            Self::Avg(expr) => format!("AVG({})", expr),
            Self::Count(expr) => format!("COUNT({})", expr),
            Self::Min(expr) => format!("MIN({})", expr),
            Self::Max(expr) => format!("MAX({})", expr),
            Self::Custom { name, args } => {
                format!("{}({})", name, args.join(", "))
            }
        }
    }
}

/// Window specification (OVER clause).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSpec {
    /// Reference to a named window.
    pub window_name: Option<String>,
    /// PARTITION BY columns.
    pub partition_by: Vec<String>,
    /// ORDER BY specifications.
    pub order_by: Vec<OrderSpec>,
    /// Frame clause.
    pub frame: Option<FrameClause>,
}

/// Order specification for window functions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSpec {
    /// Column or expression to order by.
    pub expr: String,
    /// Sort direction.
    pub direction: SortOrder,
    /// NULLS FIRST/LAST.
    pub nulls: Option<NullsPosition>,
}

/// Position of NULL values in ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NullsPosition {
    /// NULL values first.
    First,
    /// NULL values last.
    Last,
}

/// Frame clause for window functions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameClause {
    /// Frame type (ROWS, RANGE, GROUPS).
    pub frame_type: FrameType,
    /// Frame start bound.
    pub start: FrameBound,
    /// Frame end bound (if BETWEEN).
    pub end: Option<FrameBound>,
    /// Exclude clause (PostgreSQL).
    pub exclude: Option<FrameExclude>,
}

/// Frame type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameType {
    /// Row-based frame.
    Rows,
    /// Value-based frame.
    Range,
    /// Group-based frame (PostgreSQL).
    Groups,
}

/// Frame boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameBound {
    /// UNBOUNDED PRECEDING.
    UnboundedPreceding,
    /// n PRECEDING.
    Preceding(u32),
    /// CURRENT ROW.
    CurrentRow,
    /// n FOLLOWING.
    Following(u32),
    /// UNBOUNDED FOLLOWING.
    UnboundedFollowing,
}

/// Frame exclusion (PostgreSQL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameExclude {
    /// EXCLUDE CURRENT ROW.
    CurrentRow,
    /// EXCLUDE GROUP.
    Group,
    /// EXCLUDE TIES.
    Ties,
    /// EXCLUDE NO OTHERS.
    NoOthers,
}

impl WindowSpec {
    /// Create a new empty window specification.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reference a named window.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            window_name: Some(name.into()),
            ..Default::default()
        }
    }

    /// Add PARTITION BY columns.
    pub fn partition_by<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.partition_by = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Add an ORDER BY column (ascending).
    pub fn order_by(mut self, column: impl Into<String>, direction: SortOrder) -> Self {
        self.order_by.push(OrderSpec {
            expr: column.into(),
            direction,
            nulls: None,
        });
        self
    }

    /// Add an ORDER BY column with NULLS position.
    pub fn order_by_nulls(
        mut self,
        column: impl Into<String>,
        direction: SortOrder,
        nulls: NullsPosition,
    ) -> Self {
        self.order_by.push(OrderSpec {
            expr: column.into(),
            direction,
            nulls: Some(nulls),
        });
        self
    }

    /// Set ROWS frame.
    pub fn rows(mut self, start: FrameBound, end: Option<FrameBound>) -> Self {
        self.frame = Some(FrameClause {
            frame_type: FrameType::Rows,
            start,
            end,
            exclude: None,
        });
        self
    }

    /// Set RANGE frame.
    pub fn range(mut self, start: FrameBound, end: Option<FrameBound>) -> Self {
        self.frame = Some(FrameClause {
            frame_type: FrameType::Range,
            start,
            end,
            exclude: None,
        });
        self
    }

    /// Set GROUPS frame (PostgreSQL).
    pub fn groups(mut self, start: FrameBound, end: Option<FrameBound>) -> Self {
        self.frame = Some(FrameClause {
            frame_type: FrameType::Groups,
            start,
            end,
            exclude: None,
        });
        self
    }

    /// Common frame: ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW.
    pub fn rows_unbounded_preceding(self) -> Self {
        self.rows(FrameBound::UnboundedPreceding, Some(FrameBound::CurrentRow))
    }

    /// Common frame: ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING.
    pub fn rows_unbounded_following(self) -> Self {
        self.rows(FrameBound::CurrentRow, Some(FrameBound::UnboundedFollowing))
    }

    /// Common frame: ROWS BETWEEN n PRECEDING AND n FOLLOWING.
    pub fn rows_around(self, n: u32) -> Self {
        self.rows(FrameBound::Preceding(n), Some(FrameBound::Following(n)))
    }

    /// Common frame: RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW.
    pub fn range_unbounded_preceding(self) -> Self {
        self.range(FrameBound::UnboundedPreceding, Some(FrameBound::CurrentRow))
    }

    /// Generate the OVER clause SQL.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        if let Some(ref name) = self.window_name {
            return format!("OVER {}", name);
        }

        let mut parts = Vec::new();

        if !self.partition_by.is_empty() {
            parts.push(format!("PARTITION BY {}", self.partition_by.join(", ")));
        }

        if !self.order_by.is_empty() {
            let orders: Vec<String> = self
                .order_by
                .iter()
                .map(|o| {
                    let mut s = format!(
                        "{} {}",
                        o.expr,
                        match o.direction {
                            SortOrder::Asc => "ASC",
                            SortOrder::Desc => "DESC",
                        }
                    );
                    if let Some(nulls) = o.nulls {
                        // MSSQL doesn't support NULLS FIRST/LAST directly
                        if db_type != DatabaseType::MSSQL {
                            s.push_str(match nulls {
                                NullsPosition::First => " NULLS FIRST",
                                NullsPosition::Last => " NULLS LAST",
                            });
                        }
                    }
                    s
                })
                .collect();
            parts.push(format!("ORDER BY {}", orders.join(", ")));
        }

        if let Some(ref frame) = self.frame {
            parts.push(frame.to_sql(db_type));
        }

        if parts.is_empty() {
            "OVER ()".to_string()
        } else {
            format!("OVER ({})", parts.join(" "))
        }
    }
}

impl FrameClause {
    /// Generate frame clause SQL.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        let frame_type = match self.frame_type {
            FrameType::Rows => "ROWS",
            FrameType::Range => "RANGE",
            FrameType::Groups => {
                // GROUPS only supported in PostgreSQL and SQLite
                match db_type {
                    DatabaseType::PostgreSQL | DatabaseType::SQLite => "GROUPS",
                    _ => "ROWS", // Fallback
                }
            }
        };

        let bounds = if let Some(ref end) = self.end {
            format!("BETWEEN {} AND {}", self.start.to_sql(), end.to_sql())
        } else {
            self.start.to_sql()
        };

        let mut sql = format!("{} {}", frame_type, bounds);

        // Exclude clause (PostgreSQL only)
        if db_type == DatabaseType::PostgreSQL {
            if let Some(exclude) = self.exclude {
                sql.push_str(match exclude {
                    FrameExclude::CurrentRow => " EXCLUDE CURRENT ROW",
                    FrameExclude::Group => " EXCLUDE GROUP",
                    FrameExclude::Ties => " EXCLUDE TIES",
                    FrameExclude::NoOthers => " EXCLUDE NO OTHERS",
                });
            }
        }

        sql
    }
}

impl FrameBound {
    /// Generate bound SQL.
    pub fn to_sql(&self) -> String {
        match self {
            Self::UnboundedPreceding => "UNBOUNDED PRECEDING".to_string(),
            Self::Preceding(n) => format!("{} PRECEDING", n),
            Self::CurrentRow => "CURRENT ROW".to_string(),
            Self::Following(n) => format!("{} FOLLOWING", n),
            Self::UnboundedFollowing => "UNBOUNDED FOLLOWING".to_string(),
        }
    }
}

impl WindowFunction {
    /// Create a new window function.
    pub fn new(function: WindowFn) -> WindowFunctionBuilder {
        WindowFunctionBuilder {
            function,
            over: None,
            alias: None,
        }
    }

    /// Set the OVER clause.
    pub fn over(mut self, spec: WindowSpec) -> Self {
        self.over = spec;
        self
    }

    /// Set an alias for the result.
    pub fn alias(mut self, name: impl Into<String>) -> Self {
        self.alias = Some(name.into());
        self
    }

    /// Generate the full SQL expression.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        let mut sql = format!("{} {}", self.function.to_sql(), self.over.to_sql(db_type));
        if let Some(ref alias) = self.alias {
            sql.push_str(" AS ");
            sql.push_str(alias);
        }
        sql
    }
}

/// Builder for window functions.
#[derive(Debug, Clone)]
pub struct WindowFunctionBuilder {
    function: WindowFn,
    over: Option<WindowSpec>,
    alias: Option<String>,
}

impl WindowFunctionBuilder {
    /// Set the OVER clause.
    pub fn over(mut self, spec: WindowSpec) -> Self {
        self.over = Some(spec);
        self
    }

    /// Set an alias.
    pub fn alias(mut self, name: impl Into<String>) -> Self {
        self.alias = Some(name.into());
        self
    }

    /// Build the window function.
    pub fn build(self) -> WindowFunction {
        WindowFunction {
            function: self.function,
            over: self.over.unwrap_or_default(),
            alias: self.alias,
        }
    }
}

/// A named window definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedWindow {
    /// Window name.
    pub name: String,
    /// Window specification.
    pub spec: WindowSpec,
}

impl NamedWindow {
    /// Create a new named window.
    pub fn new(name: impl Into<String>, spec: WindowSpec) -> Self {
        Self {
            name: name.into(),
            spec,
        }
    }

    /// Generate the WINDOW clause definition.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        // Named windows generate just the spec content (without OVER)
        let spec_parts = {
            let mut parts = Vec::new();
            if !self.spec.partition_by.is_empty() {
                parts.push(format!(
                    "PARTITION BY {}",
                    self.spec.partition_by.join(", ")
                ));
            }
            if !self.spec.order_by.is_empty() {
                let orders: Vec<String> = self
                    .spec
                    .order_by
                    .iter()
                    .map(|o| {
                        format!(
                            "{} {}",
                            o.expr,
                            match o.direction {
                                SortOrder::Asc => "ASC",
                                SortOrder::Desc => "DESC",
                            }
                        )
                    })
                    .collect();
                parts.push(format!("ORDER BY {}", orders.join(", ")));
            }
            if let Some(ref frame) = self.spec.frame {
                parts.push(frame.to_sql(db_type));
            }
            parts.join(" ")
        };

        format!("{} AS ({})", self.name, spec_parts)
    }
}

// ============================================================================
// Helper functions for creating window functions
// ============================================================================

/// Create ROW_NUMBER() window function.
pub fn row_number() -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::RowNumber)
}

/// Create RANK() window function.
pub fn rank() -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Rank)
}

/// Create DENSE_RANK() window function.
pub fn dense_rank() -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::DenseRank)
}

/// Create NTILE(n) window function.
pub fn ntile(n: u32) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Ntile(n))
}

/// Create PERCENT_RANK() window function.
pub fn percent_rank() -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::PercentRank)
}

/// Create CUME_DIST() window function.
pub fn cume_dist() -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::CumeDist)
}

/// Create LAG() window function.
pub fn lag(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Lag {
        expr: expr.into(),
        offset: None,
        default: None,
    })
}

/// Create LAG() with offset.
pub fn lag_offset(expr: impl Into<String>, offset: u32) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Lag {
        expr: expr.into(),
        offset: Some(offset),
        default: None,
    })
}

/// Create LAG() with offset and default.
pub fn lag_full(
    expr: impl Into<String>,
    offset: u32,
    default: impl Into<String>,
) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Lag {
        expr: expr.into(),
        offset: Some(offset),
        default: Some(default.into()),
    })
}

/// Create LEAD() window function.
pub fn lead(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Lead {
        expr: expr.into(),
        offset: None,
        default: None,
    })
}

/// Create LEAD() with offset.
pub fn lead_offset(expr: impl Into<String>, offset: u32) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Lead {
        expr: expr.into(),
        offset: Some(offset),
        default: None,
    })
}

/// Create LEAD() with offset and default.
pub fn lead_full(
    expr: impl Into<String>,
    offset: u32,
    default: impl Into<String>,
) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Lead {
        expr: expr.into(),
        offset: Some(offset),
        default: Some(default.into()),
    })
}

/// Create FIRST_VALUE() window function.
pub fn first_value(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::FirstValue(expr.into()))
}

/// Create LAST_VALUE() window function.
pub fn last_value(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::LastValue(expr.into()))
}

/// Create NTH_VALUE() window function.
pub fn nth_value(expr: impl Into<String>, n: u32) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::NthValue(expr.into(), n))
}

/// Create SUM() window function.
pub fn sum(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Sum(expr.into()))
}

/// Create AVG() window function.
pub fn avg(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Avg(expr.into()))
}

/// Create COUNT() window function.
pub fn count(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Count(expr.into()))
}

/// Create MIN() window function.
pub fn min(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Min(expr.into()))
}

/// Create MAX() window function.
pub fn max(expr: impl Into<String>) -> WindowFunctionBuilder {
    WindowFunction::new(WindowFn::Max(expr.into()))
}

/// Create a custom window function.
pub fn custom<I, S>(name: impl Into<String>, args: I) -> WindowFunctionBuilder
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    WindowFunction::new(WindowFn::Custom {
        name: name.into(),
        args: args.into_iter().map(Into::into).collect(),
    })
}

/// MongoDB $setWindowFields support.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    /// A $setWindowFields stage for MongoDB aggregation pipelines.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct SetWindowFields {
        /// PARTITION BY equivalent.
        pub partition_by: Option<JsonValue>,
        /// SORT BY specification.
        pub sort_by: Option<JsonValue>,
        /// Output fields with window functions.
        pub output: serde_json::Map<String, JsonValue>,
    }

    impl SetWindowFields {
        /// Create a new $setWindowFields stage.
        pub fn new() -> SetWindowFieldsBuilder {
            SetWindowFieldsBuilder::default()
        }

        /// Convert to BSON document.
        pub fn to_bson(&self) -> JsonValue {
            let mut stage = serde_json::Map::new();

            if let Some(ref partition) = self.partition_by {
                stage.insert("partitionBy".to_string(), partition.clone());
            }

            if let Some(ref sort) = self.sort_by {
                stage.insert("sortBy".to_string(), sort.clone());
            }

            stage.insert("output".to_string(), JsonValue::Object(self.output.clone()));

            serde_json::json!({ "$setWindowFields": stage })
        }
    }

    impl Default for SetWindowFields {
        fn default() -> Self {
            Self {
                partition_by: None,
                sort_by: None,
                output: serde_json::Map::new(),
            }
        }
    }

    /// Builder for $setWindowFields.
    #[derive(Debug, Clone, Default)]
    pub struct SetWindowFieldsBuilder {
        partition_by: Option<JsonValue>,
        sort_by: Option<JsonValue>,
        output: serde_json::Map<String, JsonValue>,
    }

    impl SetWindowFieldsBuilder {
        /// Set PARTITION BY.
        pub fn partition_by(mut self, expr: impl Into<String>) -> Self {
            self.partition_by = Some(JsonValue::String(format!("${}", expr.into())));
            self
        }

        /// Set PARTITION BY with object expression.
        pub fn partition_by_expr(mut self, expr: JsonValue) -> Self {
            self.partition_by = Some(expr);
            self
        }

        /// Set SORT BY (single field ascending).
        pub fn sort_by(mut self, field: impl Into<String>) -> Self {
            let mut sort = serde_json::Map::new();
            sort.insert(field.into(), JsonValue::Number(1.into()));
            self.sort_by = Some(JsonValue::Object(sort));
            self
        }

        /// Set SORT BY with direction.
        pub fn sort_by_desc(mut self, field: impl Into<String>) -> Self {
            let mut sort = serde_json::Map::new();
            sort.insert(field.into(), JsonValue::Number((-1).into()));
            self.sort_by = Some(JsonValue::Object(sort));
            self
        }

        /// Set SORT BY with multiple fields.
        pub fn sort_by_fields(mut self, fields: Vec<(&str, i32)>) -> Self {
            let mut sort = serde_json::Map::new();
            for (field, dir) in fields {
                sort.insert(field.to_string(), JsonValue::Number(dir.into()));
            }
            self.sort_by = Some(JsonValue::Object(sort));
            self
        }

        /// Add $rowNumber output field.
        pub fn row_number(mut self, output_field: impl Into<String>) -> Self {
            self.output
                .insert(output_field.into(), serde_json::json!({ "$rowNumber": {} }));
            self
        }

        /// Add $rank output field.
        pub fn rank(mut self, output_field: impl Into<String>) -> Self {
            self.output
                .insert(output_field.into(), serde_json::json!({ "$rank": {} }));
            self
        }

        /// Add $denseRank output field.
        pub fn dense_rank(mut self, output_field: impl Into<String>) -> Self {
            self.output
                .insert(output_field.into(), serde_json::json!({ "$denseRank": {} }));
            self
        }

        /// Add $sum with window output field.
        pub fn sum(
            mut self,
            output_field: impl Into<String>,
            input: impl Into<String>,
            window: Option<MongoWindow>,
        ) -> Self {
            let mut spec = serde_json::Map::new();
            spec.insert(
                "$sum".to_string(),
                JsonValue::String(format!("${}", input.into())),
            );
            if let Some(w) = window {
                spec.insert("window".to_string(), w.to_bson());
            }
            self.output
                .insert(output_field.into(), JsonValue::Object(spec));
            self
        }

        /// Add $avg with window output field.
        pub fn avg(
            mut self,
            output_field: impl Into<String>,
            input: impl Into<String>,
            window: Option<MongoWindow>,
        ) -> Self {
            let mut spec = serde_json::Map::new();
            spec.insert(
                "$avg".to_string(),
                JsonValue::String(format!("${}", input.into())),
            );
            if let Some(w) = window {
                spec.insert("window".to_string(), w.to_bson());
            }
            self.output
                .insert(output_field.into(), JsonValue::Object(spec));
            self
        }

        /// Add $first output field.
        pub fn first(mut self, output_field: impl Into<String>, input: impl Into<String>) -> Self {
            self.output.insert(
                output_field.into(),
                serde_json::json!({ "$first": format!("${}", input.into()) }),
            );
            self
        }

        /// Add $last output field.
        pub fn last(mut self, output_field: impl Into<String>, input: impl Into<String>) -> Self {
            self.output.insert(
                output_field.into(),
                serde_json::json!({ "$last": format!("${}", input.into()) }),
            );
            self
        }

        /// Add $shift (LAG/LEAD equivalent) output field.
        pub fn shift(
            mut self,
            output_field: impl Into<String>,
            output: impl Into<String>,
            by: i32,
            default: Option<JsonValue>,
        ) -> Self {
            let mut spec = serde_json::Map::new();
            spec.insert(
                "output".to_string(),
                JsonValue::String(format!("${}", output.into())),
            );
            spec.insert("by".to_string(), JsonValue::Number(by.into()));
            if let Some(def) = default {
                spec.insert("default".to_string(), def);
            }
            self.output
                .insert(output_field.into(), serde_json::json!({ "$shift": spec }));
            self
        }

        /// Add custom window function.
        pub fn output(mut self, field: impl Into<String>, spec: JsonValue) -> Self {
            self.output.insert(field.into(), spec);
            self
        }

        /// Build the stage.
        pub fn build(self) -> SetWindowFields {
            SetWindowFields {
                partition_by: self.partition_by,
                sort_by: self.sort_by,
                output: self.output,
            }
        }
    }

    /// MongoDB window specification.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MongoWindow {
        /// Documents array [start, end].
        pub documents: Option<[WindowBound; 2]>,
        /// Range array [start, end].
        pub range: Option<[WindowBound; 2]>,
        /// Unit for range (day, week, month, etc.).
        pub unit: Option<String>,
    }

    /// Window boundary value.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum WindowBound {
        /// Numeric offset or "unbounded"/"current".
        Number(i64),
        /// String keyword.
        Keyword(String),
    }

    impl MongoWindow {
        /// Documents window (like SQL ROWS).
        pub fn documents(start: i64, end: i64) -> Self {
            Self {
                documents: Some([WindowBound::Number(start), WindowBound::Number(end)]),
                range: None,
                unit: None,
            }
        }

        /// Unbounded documents window.
        pub fn documents_unbounded() -> Self {
            Self {
                documents: Some([
                    WindowBound::Keyword("unbounded".to_string()),
                    WindowBound::Keyword("unbounded".to_string()),
                ]),
                range: None,
                unit: None,
            }
        }

        /// Documents from unbounded preceding to current.
        pub fn documents_to_current() -> Self {
            Self {
                documents: Some([
                    WindowBound::Keyword("unbounded".to_string()),
                    WindowBound::Keyword("current".to_string()),
                ]),
                range: None,
                unit: None,
            }
        }

        /// Range window with unit.
        pub fn range_with_unit(start: i64, end: i64, unit: impl Into<String>) -> Self {
            Self {
                documents: None,
                range: Some([WindowBound::Number(start), WindowBound::Number(end)]),
                unit: Some(unit.into()),
            }
        }

        /// Convert to BSON.
        pub fn to_bson(&self) -> JsonValue {
            let mut window = serde_json::Map::new();

            if let Some(ref docs) = self.documents {
                let arr: Vec<JsonValue> = docs
                    .iter()
                    .map(|b| match b {
                        WindowBound::Number(n) => JsonValue::Number((*n).into()),
                        WindowBound::Keyword(s) => JsonValue::String(s.clone()),
                    })
                    .collect();
                window.insert("documents".to_string(), JsonValue::Array(arr));
            }

            if let Some(ref range) = self.range {
                let arr: Vec<JsonValue> = range
                    .iter()
                    .map(|b| match b {
                        WindowBound::Number(n) => JsonValue::Number((*n).into()),
                        WindowBound::Keyword(s) => JsonValue::String(s.clone()),
                    })
                    .collect();
                window.insert("range".to_string(), JsonValue::Array(arr));
            }

            if let Some(ref unit) = self.unit {
                window.insert("unit".to_string(), JsonValue::String(unit.clone()));
            }

            JsonValue::Object(window)
        }
    }

    /// Helper to create a $setWindowFields stage.
    pub fn set_window_fields() -> SetWindowFieldsBuilder {
        SetWindowFields::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_number() {
        let wf = row_number()
            .over(
                WindowSpec::new()
                    .partition_by(["dept"])
                    .order_by("salary", SortOrder::Desc),
            )
            .build();

        let sql = wf.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("ROW_NUMBER()"));
        assert!(sql.contains("PARTITION BY dept"));
        assert!(sql.contains("ORDER BY salary DESC"));
    }

    #[test]
    fn test_rank_functions() {
        let r = rank()
            .over(WindowSpec::new().order_by("score", SortOrder::Desc))
            .build();
        assert!(r.to_sql(DatabaseType::PostgreSQL).contains("RANK()"));

        let dr = dense_rank()
            .over(WindowSpec::new().order_by("score", SortOrder::Desc))
            .build();
        assert!(dr.to_sql(DatabaseType::PostgreSQL).contains("DENSE_RANK()"));
    }

    #[test]
    fn test_ntile() {
        let wf = ntile(4)
            .over(WindowSpec::new().order_by("value", SortOrder::Asc))
            .build();

        assert!(wf.to_sql(DatabaseType::MySQL).contains("NTILE(4)"));
    }

    #[test]
    fn test_lag_lead() {
        let l = lag("price")
            .over(WindowSpec::new().order_by("date", SortOrder::Asc))
            .build();
        assert!(l.to_sql(DatabaseType::PostgreSQL).contains("LAG(price)"));

        let l2 = lag_offset("price", 2)
            .over(WindowSpec::new().order_by("date", SortOrder::Asc))
            .build();
        assert!(
            l2.to_sql(DatabaseType::PostgreSQL)
                .contains("LAG(price, 2)")
        );

        let l3 = lag_full("price", 1, "0")
            .over(WindowSpec::new().order_by("date", SortOrder::Asc))
            .build();
        assert!(
            l3.to_sql(DatabaseType::PostgreSQL)
                .contains("LAG(price, 1, 0)")
        );

        let ld = lead("price")
            .over(WindowSpec::new().order_by("date", SortOrder::Asc))
            .build();
        assert!(ld.to_sql(DatabaseType::PostgreSQL).contains("LEAD(price)"));
    }

    #[test]
    fn test_aggregate_window() {
        let s = sum("amount")
            .over(
                WindowSpec::new()
                    .partition_by(["account_id"])
                    .order_by("date", SortOrder::Asc)
                    .rows_unbounded_preceding(),
            )
            .alias("running_total")
            .build();

        let sql = s.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("SUM(amount)"));
        assert!(sql.contains("ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW"));
        assert!(sql.contains("AS running_total"));
    }

    #[test]
    fn test_frame_clauses() {
        let spec = WindowSpec::new()
            .order_by("id", SortOrder::Asc)
            .rows(FrameBound::Preceding(3), Some(FrameBound::Following(3)));

        let sql = spec.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("ROWS BETWEEN 3 PRECEDING AND 3 FOLLOWING"));
    }

    #[test]
    fn test_named_window() {
        let nw = NamedWindow::new(
            "w",
            WindowSpec::new()
                .partition_by(["dept"])
                .order_by("salary", SortOrder::Desc),
        );

        let sql = nw.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("w AS ("));
        assert!(sql.contains("PARTITION BY dept"));
    }

    #[test]
    fn test_window_reference() {
        let spec = WindowSpec::named("w");
        assert_eq!(spec.to_sql(DatabaseType::PostgreSQL), "OVER w");
    }

    #[test]
    fn test_nulls_position() {
        let spec = WindowSpec::new().order_by_nulls("value", SortOrder::Desc, NullsPosition::Last);

        let pg_sql = spec.to_sql(DatabaseType::PostgreSQL);
        assert!(pg_sql.contains("NULLS LAST"));

        // MSSQL doesn't support NULLS FIRST/LAST
        let mssql_sql = spec.to_sql(DatabaseType::MSSQL);
        assert!(!mssql_sql.contains("NULLS"));
    }

    #[test]
    fn test_first_last_value() {
        let fv = first_value("salary")
            .over(
                WindowSpec::new()
                    .partition_by(["dept"])
                    .order_by("hire_date", SortOrder::Asc),
            )
            .build();

        assert!(
            fv.to_sql(DatabaseType::PostgreSQL)
                .contains("FIRST_VALUE(salary)")
        );

        let lv = last_value("salary")
            .over(
                WindowSpec::new()
                    .partition_by(["dept"])
                    .order_by("hire_date", SortOrder::Asc)
                    .rows(
                        FrameBound::UnboundedPreceding,
                        Some(FrameBound::UnboundedFollowing),
                    ),
            )
            .build();

        assert!(
            lv.to_sql(DatabaseType::PostgreSQL)
                .contains("LAST_VALUE(salary)")
        );
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_row_number() {
            let stage = set_window_fields()
                .partition_by("state")
                .sort_by_desc("quantity")
                .row_number("rowNumber")
                .build();

            let bson = stage.to_bson();
            assert!(bson["$setWindowFields"]["output"]["rowNumber"]["$rowNumber"].is_object());
        }

        #[test]
        fn test_rank() {
            let stage = set_window_fields()
                .sort_by("score")
                .rank("ranking")
                .dense_rank("denseRanking")
                .build();

            let bson = stage.to_bson();
            assert!(bson["$setWindowFields"]["output"]["ranking"]["$rank"].is_object());
            assert!(bson["$setWindowFields"]["output"]["denseRanking"]["$denseRank"].is_object());
        }

        #[test]
        fn test_running_total() {
            let stage = set_window_fields()
                .partition_by("account")
                .sort_by("date")
                .sum(
                    "runningTotal",
                    "amount",
                    Some(MongoWindow::documents_to_current()),
                )
                .build();

            let bson = stage.to_bson();
            let output = &bson["$setWindowFields"]["output"]["runningTotal"];
            assert!(output["$sum"].is_string());
            assert!(output["window"]["documents"].is_array());
        }

        #[test]
        fn test_shift_lag() {
            let stage = set_window_fields()
                .sort_by("date")
                .shift("prevPrice", "price", -1, Some(serde_json::json!(0)))
                .shift("nextPrice", "price", 1, None)
                .build();

            let bson = stage.to_bson();
            assert!(bson["$setWindowFields"]["output"]["prevPrice"]["$shift"]["by"] == -1);
            assert!(bson["$setWindowFields"]["output"]["nextPrice"]["$shift"]["by"] == 1);
        }

        #[test]
        fn test_window_bounds() {
            let w = MongoWindow::documents(-3, 3);
            let bson = w.to_bson();
            assert_eq!(bson["documents"][0], -3);
            assert_eq!(bson["documents"][1], 3);

            let w2 = MongoWindow::range_with_unit(-7, 0, "day");
            let bson2 = w2.to_bson();
            assert!(bson2["range"].is_array());
            assert_eq!(bson2["unit"], "day");
        }
    }
}
