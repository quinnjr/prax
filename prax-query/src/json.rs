//! JSON and document operations support.
//!
//! This module provides types for working with JSON columns and document
//! operations across different database backends.
//!
//! # Supported Features
//!
//! | Feature           | PostgreSQL | MySQL    | SQLite   | MSSQL       | MongoDB     |
//! |-------------------|------------|----------|----------|-------------|-------------|
//! | JSON column type  | ✅ JSONB   | ✅ JSON  | ✅ JSON  | ✅          | ✅ Native   |
//! | JSON path queries | ✅ @>, ->  | ✅ ->, ->>| ✅ ->, ->>| ✅ JSON_VALUE| ✅ Dot     |
//! | JSON indexing     | ✅ GIN     | ✅ Gen cols| ❌      | ✅          | ✅ Native   |
//! | JSON aggregation  | ✅         | ✅       | ✅       | ✅          | ✅          |
//! | Array operations  | ✅         | ✅       | ❌       | ✅          | ✅ Native   |
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::json::{JsonPath, JsonOp, JsonFilter};
//!
//! // Path query
//! let filter = JsonPath::new("metadata")
//!     .field("role")
//!     .equals("admin");
//!
//! // JSON mutation
//! let update = JsonOp::set("metadata", JsonPath::new("$.settings.theme"), "dark");
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::error::{QueryError, QueryResult};
use crate::filter::FilterValue;
use crate::sql::DatabaseType;

/// A JSON path expression for navigating JSON documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonPath {
    /// The column name containing JSON.
    pub column: String,
    /// Path segments (field names or array indices).
    pub segments: Vec<PathSegment>,
    /// Whether to return text (::text in PostgreSQL).
    pub as_text: bool,
}

/// A segment in a JSON path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathSegment {
    /// Field access (e.g., .name).
    Field(String),
    /// Array index access (e.g., [0]).
    Index(i64),
    /// Array wildcard (e.g., [*]).
    Wildcard,
    /// Recursive descent (e.g., ..).
    RecursiveDescent,
}

impl JsonPath {
    /// Create a new JSON path starting from a column.
    pub fn new(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            segments: Vec::new(),
            as_text: false,
        }
    }

    /// Create a path from a JSONPath string (e.g., "$.user.name").
    pub fn from_path(column: impl Into<String>, path: &str) -> Self {
        let mut json_path = Self::new(column);

        // Parse simple JSONPath syntax
        let path = path.trim_start_matches('$').trim_start_matches('.');

        for segment in path.split('.') {
            if segment.is_empty() {
                continue;
            }

            // Check if segment contains array index like "addresses[0]"
            if let Some(bracket_pos) = segment.find('[') {
                // Extract field name before bracket
                let field_name = &segment[..bracket_pos];
                if !field_name.is_empty() {
                    json_path
                        .segments
                        .push(PathSegment::Field(field_name.to_string()));
                }

                // Extract index
                if let Some(end_pos) = segment.find(']') {
                    let idx_str = &segment[bracket_pos + 1..end_pos];
                    if idx_str == "*" {
                        json_path.segments.push(PathSegment::Wildcard);
                    } else if let Ok(i) = idx_str.parse::<i64>() {
                        json_path.segments.push(PathSegment::Index(i));
                    }
                }
            } else {
                json_path
                    .segments
                    .push(PathSegment::Field(segment.to_string()));
            }
        }

        json_path
    }

    /// Add a field access segment.
    pub fn field(mut self, name: impl Into<String>) -> Self {
        self.segments.push(PathSegment::Field(name.into()));
        self
    }

    /// Add an array index segment.
    pub fn index(mut self, idx: i64) -> Self {
        self.segments.push(PathSegment::Index(idx));
        self
    }

    /// Add an array wildcard segment.
    pub fn all(mut self) -> Self {
        self.segments.push(PathSegment::Wildcard);
        self
    }

    /// Return the value as text instead of JSON.
    pub fn text(mut self) -> Self {
        self.as_text = true;
        self
    }

    /// Convert to PostgreSQL JSON path expression.
    pub fn to_postgres_expr(&self) -> String {
        let mut expr = self.column.clone();

        for segment in &self.segments {
            match segment {
                PathSegment::Field(name) => {
                    if self.as_text && self.segments.last() == Some(segment) {
                        expr.push_str(" ->> '");
                    } else {
                        expr.push_str(" -> '");
                    }
                    expr.push_str(name);
                    expr.push('\'');
                }
                PathSegment::Index(idx) => {
                    if self.as_text && self.segments.last() == Some(segment) {
                        expr.push_str(" ->> ");
                    } else {
                        expr.push_str(" -> ");
                    }
                    expr.push_str(&idx.to_string());
                }
                PathSegment::Wildcard => {
                    // PostgreSQL doesn't directly support [*] in -> operators
                    // Use jsonb_array_elements for this
                    expr = format!("jsonb_array_elements({})", expr);
                }
                PathSegment::RecursiveDescent => {
                    // Use jsonb_path_query for recursive descent
                    expr = format!("jsonb_path_query({}, '$.**')", expr);
                }
            }
        }

        expr
    }

    /// Convert to MySQL JSON path expression.
    pub fn to_mysql_expr(&self) -> String {
        let path = self.to_jsonpath_string();

        if self.as_text {
            format!("JSON_UNQUOTE(JSON_EXTRACT({}, '{}'))", self.column, path)
        } else {
            format!("JSON_EXTRACT({}, '{}')", self.column, path)
        }
    }

    /// Convert to SQLite JSON path expression.
    pub fn to_sqlite_expr(&self) -> String {
        let path = self.to_jsonpath_string();

        if self.as_text {
            format!("json_extract({}, '{}')", self.column, path)
        } else {
            format!("json({}, '{}')", self.column, path)
        }
    }

    /// Convert to MSSQL JSON path expression.
    pub fn to_mssql_expr(&self) -> String {
        let path = self.to_jsonpath_string();

        if self.as_text {
            format!("JSON_VALUE({}, '{}')", self.column, path)
        } else {
            format!("JSON_QUERY({}, '{}')", self.column, path)
        }
    }

    /// Convert to standard JSONPath string.
    pub fn to_jsonpath_string(&self) -> String {
        let mut path = String::from("$");

        for segment in &self.segments {
            match segment {
                PathSegment::Field(name) => {
                    path.push('.');
                    path.push_str(name);
                }
                PathSegment::Index(idx) => {
                    path.push('[');
                    path.push_str(&idx.to_string());
                    path.push(']');
                }
                PathSegment::Wildcard => {
                    path.push_str("[*]");
                }
                PathSegment::RecursiveDescent => {
                    path.push_str("..");
                }
            }
        }

        path
    }

    /// Convert to MongoDB dot notation.
    pub fn to_mongodb_path(&self) -> String {
        let mut parts = vec![self.column.clone()];

        for segment in &self.segments {
            match segment {
                PathSegment::Field(name) => parts.push(name.clone()),
                PathSegment::Index(idx) => parts.push(idx.to_string()),
                PathSegment::Wildcard => parts.push("$".to_string()),
                PathSegment::RecursiveDescent => {} // MongoDB uses different syntax
            }
        }

        parts.join(".")
    }

    /// Convert to SQL expression for the specified database.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_expr(),
            DatabaseType::MySQL => self.to_mysql_expr(),
            DatabaseType::SQLite => self.to_sqlite_expr(),
            DatabaseType::MSSQL => self.to_mssql_expr(),
        }
    }
}

/// A JSON filter operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JsonFilter {
    /// Check if path equals a value.
    Equals(JsonPath, JsonValue),
    /// Check if path does not equal a value.
    NotEquals(JsonPath, JsonValue),
    /// Check if JSON contains another JSON value (PostgreSQL @>).
    Contains(String, JsonValue),
    /// Check if JSON is contained by another JSON value (PostgreSQL <@).
    ContainedBy(String, JsonValue),
    /// Check if any of the keys exist (PostgreSQL ?|).
    HasAnyKey(String, Vec<String>),
    /// Check if all of the keys exist (PostgreSQL ?&).
    HasAllKeys(String, Vec<String>),
    /// Check if a key exists (PostgreSQL ?).
    HasKey(String, String),
    /// Check if path value is greater than.
    GreaterThan(JsonPath, JsonValue),
    /// Check if path value is less than.
    LessThan(JsonPath, JsonValue),
    /// Check if path exists.
    Exists(JsonPath),
    /// Check if path is null.
    IsNull(JsonPath),
    /// Check if path is not null.
    IsNotNull(JsonPath),
    /// Check if array contains value.
    ArrayContains(JsonPath, JsonValue),
    /// Check value using JSONPath predicate (PostgreSQL @?).
    PathMatch(String, String),
}

impl JsonFilter {
    /// Create an equals filter.
    pub fn equals(path: JsonPath, value: impl Into<JsonValue>) -> Self {
        Self::Equals(path, value.into())
    }

    /// Create a contains filter.
    pub fn contains(column: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        Self::Contains(column.into(), value.into())
    }

    /// Create a has key filter.
    pub fn has_key(column: impl Into<String>, key: impl Into<String>) -> Self {
        Self::HasKey(column.into(), key.into())
    }

    /// Create an exists filter.
    pub fn exists(path: JsonPath) -> Self {
        Self::Exists(path)
    }

    /// Generate PostgreSQL SQL for this filter.
    pub fn to_postgres_sql(&self) -> (String, Vec<FilterValue>) {
        let mut params = Vec::new();

        let sql = match self {
            Self::Equals(path, value) => {
                let expr = path.to_postgres_expr();
                params.push(FilterValue::Json(value.clone()));
                format!("{} = $1::jsonb", expr)
            }
            Self::NotEquals(path, value) => {
                let expr = path.to_postgres_expr();
                params.push(FilterValue::Json(value.clone()));
                format!("{} <> $1::jsonb", expr)
            }
            Self::Contains(col, value) => {
                params.push(FilterValue::Json(value.clone()));
                format!("{} @> $1::jsonb", col)
            }
            Self::ContainedBy(col, value) => {
                params.push(FilterValue::Json(value.clone()));
                format!("{} <@ $1::jsonb", col)
            }
            Self::HasKey(col, key) => {
                params.push(FilterValue::String(key.clone()));
                format!("{} ? $1", col)
            }
            Self::HasAnyKey(col, keys) => {
                let placeholders: Vec<String> =
                    (1..=keys.len()).map(|i| format!("${}", i)).collect();
                for key in keys {
                    params.push(FilterValue::String(key.clone()));
                }
                format!("{} ?| ARRAY[{}]", col, placeholders.join(", "))
            }
            Self::HasAllKeys(col, keys) => {
                let placeholders: Vec<String> =
                    (1..=keys.len()).map(|i| format!("${}", i)).collect();
                for key in keys {
                    params.push(FilterValue::String(key.clone()));
                }
                format!("{} ?& ARRAY[{}]", col, placeholders.join(", "))
            }
            Self::GreaterThan(path, value) => {
                let expr = path.to_postgres_expr();
                params.push(FilterValue::Json(value.clone()));
                format!("({})::numeric > ($1::jsonb)::numeric", expr)
            }
            Self::LessThan(path, value) => {
                let expr = path.to_postgres_expr();
                params.push(FilterValue::Json(value.clone()));
                format!("({})::numeric < ($1::jsonb)::numeric", expr)
            }
            Self::Exists(path) => {
                format!("{} IS NOT NULL", path.to_postgres_expr())
            }
            Self::IsNull(path) => {
                format!("{} IS NULL", path.to_postgres_expr())
            }
            Self::IsNotNull(path) => {
                format!("{} IS NOT NULL", path.to_postgres_expr())
            }
            Self::ArrayContains(path, value) => {
                params.push(FilterValue::Json(value.clone()));
                format!("{} @> $1::jsonb", path.to_postgres_expr())
            }
            Self::PathMatch(col, predicate) => {
                params.push(FilterValue::String(predicate.clone()));
                format!("{} @? $1::jsonpath", col)
            }
        };

        (sql, params)
    }

    /// Generate MySQL SQL for this filter.
    pub fn to_mysql_sql(&self) -> (String, Vec<FilterValue>) {
        let mut params = Vec::new();

        let sql = match self {
            Self::Equals(path, value) => {
                let expr = path.to_mysql_expr();
                params.push(FilterValue::Json(value.clone()));
                format!("{} = CAST(? AS JSON)", expr)
            }
            Self::NotEquals(path, value) => {
                let expr = path.to_mysql_expr();
                params.push(FilterValue::Json(value.clone()));
                format!("{} <> CAST(? AS JSON)", expr)
            }
            Self::Contains(col, value) => {
                params.push(FilterValue::Json(value.clone()));
                format!("JSON_CONTAINS({}, ?)", col)
            }
            Self::HasKey(col, key) => {
                params.push(FilterValue::String(format!("$.{}", key)));
                format!("JSON_CONTAINS_PATH({}, 'one', ?)", col)
            }
            Self::Exists(path) => {
                format!("{} IS NOT NULL", path.to_mysql_expr())
            }
            Self::IsNull(path) => {
                format!("{} IS NULL", path.to_mysql_expr())
            }
            Self::IsNotNull(path) => {
                format!("{} IS NOT NULL", path.to_mysql_expr())
            }
            Self::ArrayContains(path, value) => {
                params.push(FilterValue::Json(value.clone()));
                format!("JSON_CONTAINS({}, ?)", path.column)
            }
            _ => "1=1".to_string(), // Unsupported filter
        };

        (sql, params)
    }

    /// Generate SQL for the specified database.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<(String, Vec<FilterValue>)> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_sql()),
            DatabaseType::MySQL => Ok(self.to_mysql_sql()),
            DatabaseType::SQLite => {
                // SQLite has limited JSON support
                let (sql, params) = match self {
                    Self::Equals(path, value) => {
                        let expr = path.to_sqlite_expr();
                        (
                            format!("{} = json(?)", expr),
                            vec![FilterValue::Json(value.clone())],
                        )
                    }
                    Self::IsNull(path) => (format!("{} IS NULL", path.to_sqlite_expr()), vec![]),
                    Self::IsNotNull(path) => {
                        (format!("{} IS NOT NULL", path.to_sqlite_expr()), vec![])
                    }
                    _ => {
                        return Err(QueryError::unsupported(
                            "This JSON filter is not supported in SQLite",
                        ));
                    }
                };
                Ok((sql, params))
            }
            DatabaseType::MSSQL => {
                let (sql, params) = match self {
                    Self::Equals(path, value) => {
                        let expr = path.to_mssql_expr();
                        (
                            format!("{} = ?", expr),
                            vec![FilterValue::Json(value.clone())],
                        )
                    }
                    Self::IsNull(path) => (format!("{} IS NULL", path.to_mssql_expr()), vec![]),
                    Self::IsNotNull(path) => {
                        (format!("{} IS NOT NULL", path.to_mssql_expr()), vec![])
                    }
                    _ => {
                        return Err(QueryError::unsupported(
                            "This JSON filter is not supported in MSSQL",
                        ));
                    }
                };
                Ok((sql, params))
            }
        }
    }
}

/// JSON mutation operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JsonOp {
    /// Set a value at a path.
    Set {
        column: String,
        path: String,
        value: JsonValue,
    },
    /// Insert a value at a path (only if not exists).
    Insert {
        column: String,
        path: String,
        value: JsonValue,
    },
    /// Replace a value at a path (only if exists).
    Replace {
        column: String,
        path: String,
        value: JsonValue,
    },
    /// Remove a key/element.
    Remove { column: String, path: String },
    /// Append to an array.
    ArrayAppend {
        column: String,
        path: String,
        value: JsonValue,
    },
    /// Prepend to an array.
    ArrayPrepend {
        column: String,
        path: String,
        value: JsonValue,
    },
    /// Merge two JSON objects.
    Merge { column: String, value: JsonValue },
    /// Increment a numeric value.
    Increment {
        column: String,
        path: String,
        amount: f64,
    },
}

impl JsonOp {
    /// Create a set operation.
    pub fn set(
        column: impl Into<String>,
        path: impl Into<String>,
        value: impl Into<JsonValue>,
    ) -> Self {
        Self::Set {
            column: column.into(),
            path: path.into(),
            value: value.into(),
        }
    }

    /// Create an insert operation.
    pub fn insert(
        column: impl Into<String>,
        path: impl Into<String>,
        value: impl Into<JsonValue>,
    ) -> Self {
        Self::Insert {
            column: column.into(),
            path: path.into(),
            value: value.into(),
        }
    }

    /// Create a remove operation.
    pub fn remove(column: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Remove {
            column: column.into(),
            path: path.into(),
        }
    }

    /// Create an array append operation.
    pub fn array_append(
        column: impl Into<String>,
        path: impl Into<String>,
        value: impl Into<JsonValue>,
    ) -> Self {
        Self::ArrayAppend {
            column: column.into(),
            path: path.into(),
            value: value.into(),
        }
    }

    /// Create a merge operation.
    pub fn merge(column: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        Self::Merge {
            column: column.into(),
            value: value.into(),
        }
    }

    /// Create an increment operation.
    pub fn increment(column: impl Into<String>, path: impl Into<String>, amount: f64) -> Self {
        Self::Increment {
            column: column.into(),
            path: path.into(),
            amount,
        }
    }

    /// Generate PostgreSQL SQL expression.
    pub fn to_postgres_expr(&self) -> (String, Vec<FilterValue>) {
        let mut params = Vec::new();

        let expr = match self {
            Self::Set {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                format!(
                    "jsonb_set({}, '{{{}}}', $1::jsonb)",
                    column,
                    path.replace('.', ",")
                )
            }
            Self::Insert {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                format!(
                    "jsonb_set({}, '{{{}}}', $1::jsonb, true)",
                    column,
                    path.replace('.', ",")
                )
            }
            Self::Replace {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                format!(
                    "jsonb_set({}, '{{{}}}', $1::jsonb, false)",
                    column,
                    path.replace('.', ",")
                )
            }
            Self::Remove { column, path } => {
                format!("{} #- '{{{}}}' ", column, path.replace('.', ","))
            }
            Self::ArrayAppend {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                if path.is_empty() || path == "$" {
                    format!("{} || $1::jsonb", column)
                } else {
                    format!(
                        "jsonb_set({}, '{{{}}}', ({} -> '{}') || $1::jsonb)",
                        column,
                        path.replace('.', ","),
                        column,
                        path
                    )
                }
            }
            Self::ArrayPrepend {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                if path.is_empty() || path == "$" {
                    format!("$1::jsonb || {}", column)
                } else {
                    format!(
                        "jsonb_set({}, '{{{}}}', $1::jsonb || ({} -> '{}'))",
                        column,
                        path.replace('.', ","),
                        column,
                        path
                    )
                }
            }
            Self::Merge { column, value } => {
                params.push(FilterValue::Json(value.clone()));
                format!("{} || $1::jsonb", column)
            }
            Self::Increment {
                column,
                path,
                amount,
            } => {
                params.push(FilterValue::Float(*amount));
                format!(
                    "jsonb_set({}, '{{{}}}', to_jsonb((({} -> '{}')::numeric + $1)))",
                    column,
                    path.replace('.', ","),
                    column,
                    path
                )
            }
        };

        (expr, params)
    }

    /// Generate MySQL SQL expression.
    pub fn to_mysql_expr(&self) -> (String, Vec<FilterValue>) {
        let mut params = Vec::new();

        let expr = match self {
            Self::Set {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                format!("JSON_SET({}, '$.{}', CAST(? AS JSON))", column, path)
            }
            Self::Insert {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                format!("JSON_INSERT({}, '$.{}', CAST(? AS JSON))", column, path)
            }
            Self::Replace {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                format!("JSON_REPLACE({}, '$.{}', CAST(? AS JSON))", column, path)
            }
            Self::Remove { column, path } => {
                format!("JSON_REMOVE({}, '$.{}')", column, path)
            }
            Self::ArrayAppend {
                column,
                path,
                value,
            } => {
                params.push(FilterValue::Json(value.clone()));
                if path.is_empty() || path == "$" {
                    format!("JSON_ARRAY_APPEND({}, '$', CAST(? AS JSON))", column)
                } else {
                    format!(
                        "JSON_ARRAY_APPEND({}, '$.{}', CAST(? AS JSON))",
                        column, path
                    )
                }
            }
            Self::Merge { column, value } => {
                params.push(FilterValue::Json(value.clone()));
                format!("JSON_MERGE_PATCH({}, CAST(? AS JSON))", column)
            }
            _ => column_name_from_op(self).to_string(),
        };

        (expr, params)
    }

    /// Generate SQL for the specified database.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<(String, Vec<FilterValue>)> {
        match db_type {
            DatabaseType::PostgreSQL => Ok(self.to_postgres_expr()),
            DatabaseType::MySQL => Ok(self.to_mysql_expr()),
            DatabaseType::SQLite => match self {
                Self::Set {
                    column,
                    path,
                    value,
                } => Ok((
                    format!("json_set({}, '$.{}', json(?))", column, path),
                    vec![FilterValue::Json(value.clone())],
                )),
                Self::Remove { column, path } => {
                    Ok((format!("json_remove({}, '$.{}')", column, path), vec![]))
                }
                _ => Err(QueryError::unsupported(
                    "This JSON operation is not supported in SQLite",
                )),
            },
            DatabaseType::MSSQL => match self {
                Self::Set {
                    column,
                    path,
                    value,
                } => Ok((
                    format!("JSON_MODIFY({}, '$.{}', JSON_QUERY(?))", column, path),
                    vec![FilterValue::Json(value.clone())],
                )),
                _ => Err(QueryError::unsupported(
                    "This JSON operation is not supported in MSSQL",
                )),
            },
        }
    }
}

fn column_name_from_op(op: &JsonOp) -> &str {
    match op {
        JsonOp::Set { column, .. }
        | JsonOp::Insert { column, .. }
        | JsonOp::Replace { column, .. }
        | JsonOp::Remove { column, .. }
        | JsonOp::ArrayAppend { column, .. }
        | JsonOp::ArrayPrepend { column, .. }
        | JsonOp::Merge { column, .. }
        | JsonOp::Increment { column, .. } => column,
    }
}

/// JSON aggregation operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JsonAgg {
    /// Aggregate rows into a JSON array.
    ArrayAgg {
        column: String,
        distinct: bool,
        order_by: Option<String>,
    },
    /// Aggregate rows into a JSON object.
    ObjectAgg {
        key_column: String,
        value_column: String,
    },
    /// Build a JSON object from key-value pairs.
    BuildObject { pairs: Vec<(String, String)> },
    /// Build a JSON array from expressions.
    BuildArray { elements: Vec<String> },
}

impl JsonAgg {
    /// Create an array aggregation.
    pub fn array_agg(column: impl Into<String>) -> Self {
        Self::ArrayAgg {
            column: column.into(),
            distinct: false,
            order_by: None,
        }
    }

    /// Create an object aggregation.
    pub fn object_agg(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self::ObjectAgg {
            key_column: key.into(),
            value_column: value.into(),
        }
    }

    /// Generate PostgreSQL SQL.
    pub fn to_postgres_sql(&self) -> String {
        match self {
            Self::ArrayAgg {
                column,
                distinct,
                order_by,
            } => {
                let mut sql = String::from("jsonb_agg(");
                if *distinct {
                    sql.push_str("DISTINCT ");
                }
                sql.push_str(column);
                if let Some(order) = order_by {
                    sql.push_str(" ORDER BY ");
                    sql.push_str(order);
                }
                sql.push(')');
                sql
            }
            Self::ObjectAgg {
                key_column,
                value_column,
            } => {
                format!("jsonb_object_agg({}, {})", key_column, value_column)
            }
            Self::BuildObject { pairs } => {
                let args: Vec<String> = pairs
                    .iter()
                    .flat_map(|(k, v)| vec![format!("'{}'", k), v.clone()])
                    .collect();
                format!("jsonb_build_object({})", args.join(", "))
            }
            Self::BuildArray { elements } => {
                format!("jsonb_build_array({})", elements.join(", "))
            }
        }
    }

    /// Generate MySQL SQL.
    pub fn to_mysql_sql(&self) -> String {
        match self {
            Self::ArrayAgg { column, .. } => {
                format!("JSON_ARRAYAGG({})", column)
            }
            Self::ObjectAgg {
                key_column,
                value_column,
            } => {
                format!("JSON_OBJECTAGG({}, {})", key_column, value_column)
            }
            Self::BuildObject { pairs } => {
                let args: Vec<String> = pairs
                    .iter()
                    .flat_map(|(k, v)| vec![format!("'{}'", k), v.clone()])
                    .collect();
                format!("JSON_OBJECT({})", args.join(", "))
            }
            Self::BuildArray { elements } => {
                format!("JSON_ARRAY({})", elements.join(", "))
            }
        }
    }

    /// Generate SQL for the specified database.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_sql(),
            DatabaseType::MySQL => self.to_mysql_sql(),
            DatabaseType::SQLite => match self {
                Self::ArrayAgg { column, .. } => format!("json_group_array({})", column),
                Self::ObjectAgg {
                    key_column,
                    value_column,
                } => {
                    format!("json_group_object({}, {})", key_column, value_column)
                }
                Self::BuildObject { pairs } => {
                    let args: Vec<String> = pairs
                        .iter()
                        .flat_map(|(k, v)| vec![format!("'{}'", k), v.clone()])
                        .collect();
                    format!("json_object({})", args.join(", "))
                }
                Self::BuildArray { elements } => {
                    format!("json_array({})", elements.join(", "))
                }
            },
            DatabaseType::MSSQL => {
                // MSSQL uses FOR JSON
                match self {
                    Self::ArrayAgg { .. } => "-- Use FOR JSON AUTO".to_string(),
                    _ => "-- Use FOR JSON PATH".to_string(),
                }
            }
        }
    }
}

/// MongoDB document operations.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    /// MongoDB update operators for documents.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum UpdateOp {
        /// Set a field value ($set).
        Set(String, JsonValue),
        /// Unset a field ($unset).
        Unset(String),
        /// Increment a numeric field ($inc).
        Inc(String, f64),
        /// Multiply a numeric field ($mul).
        Mul(String, f64),
        /// Rename a field ($rename).
        Rename(String, String),
        /// Set field to current date ($currentDate).
        CurrentDate(String),
        /// Set minimum value ($min).
        Min(String, JsonValue),
        /// Set maximum value ($max).
        Max(String, JsonValue),
        /// Set on insert only ($setOnInsert).
        SetOnInsert(String, JsonValue),
    }

    /// MongoDB array update operators.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum ArrayOp {
        /// Push element to array ($push).
        Push {
            field: String,
            value: JsonValue,
            position: Option<i32>,
        },
        /// Push all elements ($push with $each).
        PushAll {
            field: String,
            values: Vec<JsonValue>,
        },
        /// Pull element from array ($pull).
        Pull { field: String, value: JsonValue },
        /// Pull all matching elements ($pullAll).
        PullAll {
            field: String,
            values: Vec<JsonValue>,
        },
        /// Add to set if not exists ($addToSet).
        AddToSet { field: String, value: JsonValue },
        /// Add all to set ($addToSet with $each).
        AddToSetAll {
            field: String,
            values: Vec<JsonValue>,
        },
        /// Remove first or last element ($pop).
        Pop { field: String, first: bool },
    }

    impl UpdateOp {
        /// Create a $set operation.
        pub fn set(field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            Self::Set(field.into(), value.into())
        }

        /// Create an $unset operation.
        pub fn unset(field: impl Into<String>) -> Self {
            Self::Unset(field.into())
        }

        /// Create an $inc operation.
        pub fn inc(field: impl Into<String>, amount: f64) -> Self {
            Self::Inc(field.into(), amount)
        }

        /// Convert to BSON document.
        pub fn to_bson(&self) -> serde_json::Value {
            match self {
                Self::Set(field, value) => serde_json::json!({ "$set": { field: value } }),
                Self::Unset(field) => serde_json::json!({ "$unset": { field: "" } }),
                Self::Inc(field, amount) => serde_json::json!({ "$inc": { field: amount } }),
                Self::Mul(field, amount) => serde_json::json!({ "$mul": { field: amount } }),
                Self::Rename(old, new) => serde_json::json!({ "$rename": { old: new } }),
                Self::CurrentDate(field) => serde_json::json!({ "$currentDate": { field: true } }),
                Self::Min(field, value) => serde_json::json!({ "$min": { field: value } }),
                Self::Max(field, value) => serde_json::json!({ "$max": { field: value } }),
                Self::SetOnInsert(field, value) => {
                    serde_json::json!({ "$setOnInsert": { field: value } })
                }
            }
        }
    }

    impl ArrayOp {
        /// Create a $push operation.
        pub fn push(field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            Self::Push {
                field: field.into(),
                value: value.into(),
                position: None,
            }
        }

        /// Create a $pull operation.
        pub fn pull(field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            Self::Pull {
                field: field.into(),
                value: value.into(),
            }
        }

        /// Create an $addToSet operation.
        pub fn add_to_set(field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            Self::AddToSet {
                field: field.into(),
                value: value.into(),
            }
        }

        /// Convert to BSON document.
        pub fn to_bson(&self) -> serde_json::Value {
            match self {
                Self::Push {
                    field,
                    value,
                    position,
                } => {
                    if let Some(pos) = position {
                        serde_json::json!({
                            "$push": { field: { "$each": [value], "$position": pos } }
                        })
                    } else {
                        serde_json::json!({ "$push": { field: value } })
                    }
                }
                Self::PushAll { field, values } => {
                    serde_json::json!({ "$push": { field: { "$each": values } } })
                }
                Self::Pull { field, value } => {
                    serde_json::json!({ "$pull": { field: value } })
                }
                Self::PullAll { field, values } => {
                    serde_json::json!({ "$pullAll": { field: values } })
                }
                Self::AddToSet { field, value } => {
                    serde_json::json!({ "$addToSet": { field: value } })
                }
                Self::AddToSetAll { field, values } => {
                    serde_json::json!({ "$addToSet": { field: { "$each": values } } })
                }
                Self::Pop { field, first } => {
                    let direction = if *first { -1 } else { 1 };
                    serde_json::json!({ "$pop": { field: direction } })
                }
            }
        }
    }

    /// Builder for MongoDB update operations.
    #[derive(Debug, Clone, Default)]
    pub struct UpdateBuilder {
        ops: Vec<serde_json::Value>,
    }

    impl UpdateBuilder {
        /// Create a new update builder.
        pub fn new() -> Self {
            Self::default()
        }

        /// Add a $set operation.
        pub fn set(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.ops.push(UpdateOp::set(field, value).to_bson());
            self
        }

        /// Add an $unset operation.
        pub fn unset(mut self, field: impl Into<String>) -> Self {
            self.ops.push(UpdateOp::unset(field).to_bson());
            self
        }

        /// Add an $inc operation.
        pub fn inc(mut self, field: impl Into<String>, amount: f64) -> Self {
            self.ops.push(UpdateOp::inc(field, amount).to_bson());
            self
        }

        /// Add a $push operation.
        pub fn push(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.ops.push(ArrayOp::push(field, value).to_bson());
            self
        }

        /// Add a $pull operation.
        pub fn pull(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.ops.push(ArrayOp::pull(field, value).to_bson());
            self
        }

        /// Add an $addToSet operation.
        pub fn add_to_set(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.ops.push(ArrayOp::add_to_set(field, value).to_bson());
            self
        }

        /// Build the combined update document.
        pub fn build(self) -> serde_json::Value {
            // Merge all operations into a single document
            let mut result = serde_json::Map::new();

            for op in self.ops {
                if let serde_json::Value::Object(map) = op {
                    for (key, value) in map {
                        if let Some(existing) = result.get_mut(&key) {
                            if let (
                                serde_json::Value::Object(existing_map),
                                serde_json::Value::Object(new_map),
                            ) = (existing, value)
                            {
                                for (k, v) in new_map {
                                    existing_map.insert(k, v);
                                }
                            }
                        } else {
                            result.insert(key, value);
                        }
                    }
                }
            }

            serde_json::Value::Object(result)
        }
    }

    /// Helper to create an update builder.
    pub fn update() -> UpdateBuilder {
        UpdateBuilder::new()
    }
}

/// JSON index definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonIndex {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table: String,
    /// JSON column name.
    pub column: String,
    /// JSON path to index.
    pub path: Option<String>,
    /// Whether this is a GIN index (PostgreSQL).
    pub gin: bool,
}

impl JsonIndex {
    /// Create a new JSON index builder.
    pub fn builder(name: impl Into<String>) -> JsonIndexBuilder {
        JsonIndexBuilder::new(name)
    }

    /// Generate PostgreSQL CREATE INDEX SQL.
    pub fn to_postgres_sql(&self) -> String {
        if let Some(ref path) = self.path {
            // Index specific path
            format!(
                "CREATE INDEX {} ON {} USING {} (({} -> '{}'));",
                self.name,
                self.table,
                if self.gin { "GIN" } else { "BTREE" },
                self.column,
                path
            )
        } else {
            // Index whole column
            format!(
                "CREATE INDEX {} ON {} USING GIN ({});",
                self.name, self.table, self.column
            )
        }
    }

    /// Generate MySQL generated column + index SQL.
    pub fn to_mysql_sql(&self) -> Vec<String> {
        if let Some(ref path) = self.path {
            let gen_col = format!("{}_{}_{}", self.table, self.column, path.replace('.', "_"));
            vec![
                format!(
                    "ALTER TABLE {} ADD COLUMN {} VARCHAR(255) GENERATED ALWAYS AS (JSON_UNQUOTE(JSON_EXTRACT({}, '$.{}'))) STORED;",
                    self.table, gen_col, self.column, path
                ),
                format!(
                    "CREATE INDEX {} ON {} ({});",
                    self.name, self.table, gen_col
                ),
            ]
        } else {
            vec!["-- MySQL requires generated columns for JSON indexing".to_string()]
        }
    }
}

/// Builder for JSON indexes.
#[derive(Debug, Clone)]
pub struct JsonIndexBuilder {
    name: String,
    table: Option<String>,
    column: Option<String>,
    path: Option<String>,
    gin: bool,
}

impl JsonIndexBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            table: None,
            column: None,
            path: None,
            gin: true,
        }
    }

    /// Set the table name.
    pub fn on_table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Set the JSON column.
    pub fn column(mut self, column: impl Into<String>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// Set the path to index.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Use GIN index (PostgreSQL).
    pub fn gin(mut self) -> Self {
        self.gin = true;
        self
    }

    /// Use BTREE index.
    pub fn btree(mut self) -> Self {
        self.gin = false;
        self
    }

    /// Build the index definition.
    pub fn build(self) -> QueryResult<JsonIndex> {
        let table = self.table.ok_or_else(|| {
            QueryError::invalid_input("table", "Must specify table with on_table()")
        })?;
        let column = self.column.ok_or_else(|| {
            QueryError::invalid_input("column", "Must specify column with column()")
        })?;

        Ok(JsonIndex {
            name: self.name,
            table,
            column,
            path: self.path,
            gin: self.gin,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_path_basic() {
        let path = JsonPath::new("metadata").field("user").field("name");

        assert_eq!(path.to_jsonpath_string(), "$.user.name");
    }

    #[test]
    fn test_json_path_with_index() {
        let path = JsonPath::new("items").field("tags").index(0);

        assert_eq!(path.to_jsonpath_string(), "$.tags[0]");
    }

    #[test]
    fn test_json_path_from_string() {
        let path = JsonPath::from_path("data", "$.user.addresses[0].city");

        assert_eq!(path.segments.len(), 4);
        assert_eq!(path.to_jsonpath_string(), "$.user.addresses[0].city");
    }

    #[test]
    fn test_postgres_path_expr() {
        let path = JsonPath::new("metadata").field("role").text();

        let expr = path.to_postgres_expr();
        assert!(expr.contains(" ->> "));
    }

    #[test]
    fn test_mysql_path_expr() {
        let path = JsonPath::new("data").field("name").text();

        let expr = path.to_mysql_expr();
        assert!(expr.contains("JSON_UNQUOTE"));
        assert!(expr.contains("JSON_EXTRACT"));
    }

    #[test]
    fn test_mongodb_path() {
        let path = JsonPath::new("address").field("city");

        assert_eq!(path.to_mongodb_path(), "address.city");
    }

    #[test]
    fn test_json_filter_contains() {
        let filter = JsonFilter::contains("metadata", serde_json::json!({"role": "admin"}));
        let (sql, params) = filter.to_postgres_sql();

        assert!(sql.contains("@>"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_json_filter_has_key() {
        let filter = JsonFilter::has_key("settings", "theme");
        let (sql, params) = filter.to_postgres_sql();

        assert!(sql.contains("?"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_json_op_set() {
        let op = JsonOp::set("metadata", "theme", serde_json::json!("dark"));
        let (expr, params) = op.to_postgres_expr();

        assert!(expr.contains("jsonb_set"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_json_op_remove() {
        let op = JsonOp::remove("metadata", "old_field");
        let (expr, _) = op.to_postgres_expr();

        assert!(expr.contains("#-"));
    }

    #[test]
    fn test_json_op_array_append() {
        let op = JsonOp::array_append("tags", "$", serde_json::json!("new_tag"));
        let (expr, params) = op.to_postgres_expr();

        assert!(expr.contains("||"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_json_op_merge() {
        let op = JsonOp::merge("settings", serde_json::json!({"new_key": "value"}));
        let (expr, params) = op.to_postgres_expr();

        assert!(expr.contains("||"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_json_agg_array() {
        let agg = JsonAgg::array_agg("name");
        let sql = agg.to_postgres_sql();

        assert_eq!(sql, "jsonb_agg(name)");
    }

    #[test]
    fn test_json_agg_object() {
        let agg = JsonAgg::object_agg("key", "value");
        let sql = agg.to_postgres_sql();

        assert_eq!(sql, "jsonb_object_agg(key, value)");
    }

    #[test]
    fn test_json_index_postgres() {
        let index = JsonIndex::builder("users_metadata_idx")
            .on_table("users")
            .column("metadata")
            .gin()
            .build()
            .unwrap();

        let sql = index.to_postgres_sql();
        assert!(sql.contains("USING GIN"));
    }

    #[test]
    fn test_json_index_with_path() {
        let index = JsonIndex::builder("users_role_idx")
            .on_table("users")
            .column("metadata")
            .path("role")
            .btree()
            .build()
            .unwrap();

        let sql = index.to_postgres_sql();
        assert!(sql.contains("USING BTREE"));
        assert!(sql.contains("-> 'role'"));
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_update_set() {
            let op = UpdateOp::set("name", "John");
            let bson = op.to_bson();

            assert!(bson["$set"]["name"].is_string());
        }

        #[test]
        fn test_update_inc() {
            let op = UpdateOp::inc("count", 1.0);
            let bson = op.to_bson();

            assert_eq!(bson["$inc"]["count"], 1.0);
        }

        #[test]
        fn test_array_push() {
            let op = ArrayOp::push("tags", "new_tag");
            let bson = op.to_bson();

            assert!(bson["$push"]["tags"].is_string());
        }

        #[test]
        fn test_array_add_to_set() {
            let op = ArrayOp::add_to_set("roles", "admin");
            let bson = op.to_bson();

            assert!(bson["$addToSet"]["roles"].is_string());
        }

        #[test]
        fn test_update_builder() {
            let update = update()
                .set("name", "John")
                .inc("visits", 1.0)
                .push("tags", "active")
                .build();

            assert!(update["$set"].is_object());
            assert!(update["$inc"].is_object());
            assert!(update["$push"].is_object());
        }
    }
}
