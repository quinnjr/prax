//! Raw SQL query execution with type-safe parameter interpolation.
//!
//! This module provides a safe way to execute raw SQL queries while still
//! benefiting from parameterized queries to prevent SQL injection.
//!
//! # Creating SQL Queries
//!
//! ```rust
//! use prax_query::Sql;
//!
//! // Simple query
//! let sql = Sql::new("SELECT * FROM users");
//! assert_eq!(sql.sql(), "SELECT * FROM users");
//!
//! // Query with parameters (binding appends placeholder)
//! let sql = Sql::new("SELECT * FROM users WHERE id = ")
//!     .bind(42);
//! assert_eq!(sql.params().len(), 1);
//! ```
//!
//! # Using the raw_query! Macro
//!
//! ```rust
//! use prax_query::raw_query;
//!
//! // Simple query
//! let sql = raw_query!("SELECT 1");
//!
//! // Query with one parameter - {} is replaced with $N placeholder
//! let id = 42;
//! let sql = raw_query!("SELECT * FROM users WHERE id = {}", id);
//! assert_eq!(sql.params().len(), 1);
//! assert!(sql.sql().contains("$1"));
//!
//! // Query with multiple parameters
//! let name = "John";
//! let age = 25;
//! let sql = raw_query!("SELECT * FROM users WHERE name = {} AND age > {}", name, age);
//! assert_eq!(sql.params().len(), 2);
//! ```
//!
//! # Building Queries Incrementally
//!
//! ```rust
//! use prax_query::Sql;
//!
//! // Join multiple conditions
//! let conditions = vec!["active = true", "verified = true"];
//! let sql = Sql::new("SELECT * FROM users WHERE ")
//!     .push(conditions.join(" AND "));
//!
//! assert!(sql.sql().contains("active = true AND verified = true"));
//! ```
//!
//! # Safety
//!
//! All values passed via `raw_query!` are parameterized and never interpolated
//! directly into the SQL string, preventing SQL injection attacks.
//!
//! ```rust
//! use prax_query::raw_query;
//!
//! // This malicious input will NOT cause SQL injection
//! let malicious = "'; DROP TABLE users; --";
//! let sql = raw_query!("SELECT * FROM users WHERE name = {}", malicious);
//!
//! // The malicious string is safely bound as a parameter
//! assert_eq!(sql.params().len(), 1);
//! // The SQL itself doesn't contain the malicious text
//! assert!(!sql.sql().contains("DROP TABLE"));
//! ```

use std::borrow::Cow;
use std::marker::PhantomData;
use tracing::debug;

use crate::dialect::SqlDialect;
use crate::error::QueryResult;
use crate::filter::FilterValue;
use crate::sql::DatabaseType;
use crate::traits::{Model, QueryEngine};

/// A piece of a [`Sql`] query: literal text, or the placeholder of the
/// parameter with this 1-based index (rendered when the query is built).
#[derive(Debug, Clone)]
enum Segment {
    Text(String),
    Param(usize),
}

/// A raw SQL query with parameterized values.
///
/// Placeholders are rendered when the query is built, not when a value is
/// bound: [`build`](Self::build) and [`sql`](Self::sql) use the database
/// type (PostgreSQL `$N` unless [`with_db_type`](Self::with_db_type) set
/// another), while [`build_for_dialect`](Self::build_for_dialect) (used by
/// `PraxClient::query_raw` / `execute_raw`) uses the executing engine's
/// dialect unless `with_db_type` was called.
#[derive(Debug, Clone)]
pub struct Sql {
    /// The SQL text and parameter positions, in order.
    segments: Vec<Segment>,
    /// The parameter values.
    params: Vec<FilterValue>,
    /// The database type for parameter formatting.
    db_type: DatabaseType,
    /// Whether `db_type` was set with [`Sql::with_db_type`].
    db_type_explicit: bool,
}

impl Sql {
    /// Create a new raw SQL query.
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            segments: vec![Segment::Text(sql.into())],
            params: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
            db_type_explicit: false,
        }
    }

    /// Create an empty SQL query.
    pub fn empty() -> Self {
        Self {
            segments: Vec::new(),
            params: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
            db_type_explicit: false,
        }
    }

    /// Set the database type for parameter formatting.
    ///
    /// It applies to every placeholder, including those of values bound
    /// before this call, and takes precedence over the engine's dialect in
    /// [`build_for_dialect`](Self::build_for_dialect).
    pub fn with_db_type(mut self, db_type: DatabaseType) -> Self {
        self.db_type = db_type;
        self.db_type_explicit = true;
        self
    }

    /// Append a literal SQL string.
    pub fn push(mut self, sql: impl Into<String>) -> Self {
        let sql = sql.into();
        match self.segments.last_mut() {
            Some(Segment::Text(last)) => last.push_str(&sql),
            _ => self.segments.push(Segment::Text(sql)),
        }
        self
    }

    /// Bind a parameter value; its placeholder is appended to the query.
    pub fn bind(mut self, value: impl Into<FilterValue>) -> Self {
        self.params.push(value.into());
        self.segments.push(Segment::Param(self.params.len()));
        self
    }

    /// Bind multiple parameter values at once.
    pub fn bind_many(mut self, values: impl IntoIterator<Item = FilterValue>) -> Self {
        for value in values {
            self = self.bind(value);
        }
        self
    }

    /// Append a conditional clause.
    pub fn push_if(self, condition: bool, sql: impl Into<String>) -> Self {
        if condition { self.push(sql) } else { self }
    }

    /// Bind a parameter conditionally.
    pub fn bind_if(self, condition: bool, value: impl Into<FilterValue>) -> Self {
        if condition { self.bind(value) } else { self }
    }

    /// Push SQL and bind a value together.
    pub fn push_bind(self, sql: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.push(sql).bind(value)
    }

    /// Push SQL and bind a value conditionally.
    pub fn push_bind_if(
        self,
        condition: bool,
        sql: impl Into<String>,
        value: impl Into<FilterValue>,
    ) -> Self {
        if condition {
            self.push(sql).bind(value)
        } else {
            self
        }
    }

    /// Add a separator between parts if there are previous parts.
    pub fn separated(self, separator: &str) -> SeparatedSql {
        SeparatedSql {
            sql: self,
            separator: separator.to_string(),
            first: true,
        }
    }

    /// Build the final SQL string and parameters, with the placeholders of
    /// the database type (PostgreSQL `$N` unless
    /// [`with_db_type`](Self::with_db_type) set another).
    pub fn build(self) -> (String, Vec<FilterValue>) {
        let sql = self.sql();
        debug!(sql_len = sql.len(), param_count = self.params.len(), db_type = ?self.db_type, "Sql::build()");
        (sql, self.params)
    }

    /// Build the final SQL string and parameters for an engine with
    /// `dialect`: its placeholders (`$N`, `?`, `?N`, `@PN`) are used unless
    /// [`with_db_type`](Self::with_db_type) was called, which wins. A
    /// dialect that does not emit SQL (MongoDB's) keeps the database
    /// type's placeholders.
    pub fn build_for_dialect(self, dialect: &dyn SqlDialect) -> (String, Vec<FilterValue>) {
        if self.db_type_explicit || !dialect.supports_sql_emission() {
            return self.build();
        }
        let sql = self.render(|i| Cow::Owned(dialect.placeholder(i)));
        debug!(
            sql_len = sql.len(),
            param_count = self.params.len(),
            "Sql::build_for_dialect()"
        );
        (sql, self.params)
    }

    /// Get the SQL string (without consuming), with the placeholders of the
    /// database type.
    pub fn sql(&self) -> String {
        self.render(|i| self.db_type.placeholder(i))
    }

    /// Join the segments, rendering each parameter with `placeholder`.
    fn render<'a>(&self, placeholder: impl Fn(usize) -> Cow<'a, str>) -> String {
        let mut sql = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Text(text) => sql.push_str(text),
                Segment::Param(index) => sql.push_str(&placeholder(*index)),
            }
        }
        sql
    }

    /// Get the parameters (without consuming).
    pub fn params(&self) -> &[FilterValue] {
        &self.params
    }

    /// Get the number of bound parameters.
    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    /// Check if the query is empty.
    pub fn is_empty(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| matches!(segment, Segment::Text(text) if text.is_empty()))
    }
}

impl Default for Sql {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Display for Sql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sql())
    }
}

/// A helper for building SQL with separators between items.
#[derive(Debug, Clone)]
pub struct SeparatedSql {
    sql: Sql,
    separator: String,
    first: bool,
}

impl SeparatedSql {
    /// Push a literal SQL string with separator.
    pub fn push(mut self, sql: impl Into<String>) -> Self {
        if !self.first {
            self.sql = self.sql.push(&self.separator);
        }
        self.sql = self.sql.push(sql);
        self.first = false;
        self
    }

    /// Push SQL and bind a value with separator.
    pub fn push_bind(mut self, sql: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        if !self.first {
            self.sql = self.sql.push(&self.separator);
        }
        self.sql = self.sql.push(sql).bind(value);
        self.first = false;
        self
    }

    /// Push SQL and bind conditionally with separator.
    pub fn push_bind_if(
        mut self,
        condition: bool,
        sql: impl Into<String>,
        value: impl Into<FilterValue>,
    ) -> Self {
        if condition {
            if !self.first {
                self.sql = self.sql.push(&self.separator);
            }
            self.sql = self.sql.push(sql).bind(value);
            self.first = false;
        }
        self
    }

    /// Finish and get the underlying Sql.
    pub fn finish(self) -> Sql {
        self.sql
    }

    /// Build the final SQL string and parameters.
    pub fn build(self) -> (String, Vec<FilterValue>) {
        self.sql.build()
    }
}

/// Raw query operation for executing typed queries.
#[derive(Debug)]
pub struct RawQueryOperation<M, E>
where
    M: Model + Send + 'static,
    E: QueryEngine,
{
    _model: PhantomData<M>,
    engine: E,
    sql: Sql,
}

impl<M, E> RawQueryOperation<M, E>
where
    M: Model + crate::row::FromRow + Send + 'static,
    E: QueryEngine,
{
    /// Create a new raw query operation.
    pub fn new(engine: E, sql: Sql) -> Self {
        Self {
            _model: PhantomData,
            engine,
            sql,
        }
    }

    /// Execute the query and return all matching records.
    pub async fn exec(self) -> QueryResult<Vec<M>> {
        let (sql, params) = self.sql.build_for_dialect(self.engine.dialect());
        self.engine.query_many(&sql, params).await
    }

    /// Execute the query and return a single record.
    pub async fn exec_one(self) -> QueryResult<M> {
        let (sql, params) = self.sql.build_for_dialect(self.engine.dialect());
        self.engine.query_one(&sql, params).await
    }

    /// Execute the query and return an optional record.
    pub async fn exec_optional(self) -> QueryResult<Option<M>> {
        let (sql, params) = self.sql.build_for_dialect(self.engine.dialect());
        self.engine.query_optional(&sql, params).await
    }
}

/// Raw execute operation for mutations.
#[derive(Debug)]
pub struct RawExecuteOperation<E>
where
    E: QueryEngine,
{
    engine: E,
    sql: Sql,
}

impl<E> RawExecuteOperation<E>
where
    E: QueryEngine,
{
    /// Create a new raw execute operation.
    pub fn new(engine: E, sql: Sql) -> Self {
        Self { engine, sql }
    }

    /// Execute the mutation and return the number of affected rows.
    pub async fn exec(self) -> QueryResult<u64> {
        let (sql, params) = self.sql.build_for_dialect(self.engine.dialect());
        self.engine.execute_raw(&sql, params).await
    }
}

/// Helper function to create a raw SQL query from a string.
pub fn sql(query: impl Into<String>) -> Sql {
    Sql::new(query)
}

/// Helper function to create a raw SQL query from parts.
///
/// This is typically used with the `raw_query!` macro.
pub fn sql_with_params(sql_str: impl Into<String>, params: Vec<FilterValue>) -> Sql {
    let mut sql = Sql::new(sql_str);
    sql.params = params;
    sql
}

/// A macro for creating raw SQL queries with inline parameter binding.
///
/// # Example
///
/// ```rust,ignore
/// let sql = raw_query!("SELECT * FROM users WHERE id = {} AND active = {}", user_id, true);
/// ```
///
/// The `{}` placeholders are replaced with database-specific parameter markers ($1, $2, etc.
/// for PostgreSQL, ? for MySQL/SQLite) and the values are bound as parameters.
#[macro_export]
macro_rules! raw_query {
    // Base case: just a string, no parameters
    ($sql:expr) => {
        $crate::raw::Sql::new($sql)
    };

    // With parameters
    ($sql:expr, $($params:expr),+ $(,)?) => {{
        let parts: Vec<&str> = $sql.split("{}").collect();
        let param_values: Vec<$crate::filter::FilterValue> = vec![
            $($params.into()),+
        ];

        let mut sql = $crate::raw::Sql::empty();
        let mut param_iter = param_values.into_iter();

        // Interleave parts and parameters
        for (i, part) in parts.iter().enumerate() {
            if !part.is_empty() {
                sql = sql.push(*part);
            }
            if i < parts.len() - 1 {
                if let Some(param) = param_iter.next() {
                    sql = sql.bind(param);
                }
            }
        }

        sql
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_new() {
        let sql = Sql::new("SELECT * FROM users");
        assert_eq!(sql.sql(), "SELECT * FROM users");
        assert!(sql.params().is_empty());
    }

    #[test]
    fn test_sql_push() {
        let sql = Sql::new("SELECT * FROM users").push(" WHERE id = 1");
        assert_eq!(sql.sql(), "SELECT * FROM users WHERE id = 1");
    }

    #[test]
    fn test_sql_bind() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ").bind(42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_multiple_binds() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .bind(42i32)
            .push(" AND name = ")
            .bind("John".to_string());
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND name = $2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_sql_push_bind() {
        let sql = Sql::new("SELECT * FROM users WHERE").push_bind(" id = ", 42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_push_if() {
        let include_active = true;
        let include_deleted = false;

        let sql = Sql::new("SELECT * FROM users")
            .push_if(include_active, " WHERE active = true")
            .push_if(include_deleted, " AND deleted = false");

        assert_eq!(sql.sql(), "SELECT * FROM users WHERE active = true");
    }

    #[test]
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn test_sql_bind_if() {
        let filter_id = Some(42i32);
        let filter_name: Option<String> = None;

        let sql = Sql::new("SELECT * FROM users WHERE 1=1")
            .push_bind_if(filter_id.is_some(), " AND id = ", filter_id.unwrap_or(0))
            .push_bind_if(filter_name.is_some(), " AND name = ", String::new());

        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE 1=1 AND id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_separated() {
        let columns = vec!["id", "name", "email"];

        let mut sep = Sql::new("SELECT ").separated(", ");

        for col in columns {
            sep = sep.push(col);
        }

        let sql = sep.finish().push(" FROM users");
        assert_eq!(sql.sql(), "SELECT id, name, email FROM users");
    }

    #[test]
    fn test_sql_separated_with_binds() {
        let filters = vec![("id", 1i32), ("active", 1i32)];

        let mut sep = Sql::new("SELECT * FROM users WHERE ").separated(" AND ");

        for (col, val) in filters {
            sep = sep.push_bind(format!("{} = ", col), val);
        }

        let (query, params) = sep.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND active = $2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_sql_mysql() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::MySQL)
            .bind(42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = ?");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_sqlite() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::SQLite)
            .bind(42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = ?");
        assert_eq!(params.len(), 1);
    }

    fn two_binds() -> Sql {
        Sql::new("SELECT * FROM users WHERE id = ")
            .bind(42i32)
            .push(" AND name = ")
            .bind("John")
    }

    #[test]
    fn same_sql_renders_each_dialects_placeholders() {
        use crate::dialect::{Mssql, Mysql, Postgres, Sqlite};
        let cases: [(&dyn SqlDialect, &str); 4] = [
            (&Postgres, "SELECT * FROM users WHERE id = $1 AND name = $2"),
            (&Mysql, "SELECT * FROM users WHERE id = ? AND name = ?"),
            (&Sqlite, "SELECT * FROM users WHERE id = ?1 AND name = ?2"),
            (&Mssql, "SELECT * FROM users WHERE id = @P1 AND name = @P2"),
        ];
        for (dialect, expected) in cases {
            let (query, params) = two_binds().build_for_dialect(dialect);
            assert_eq!(query, expected);
            assert_eq!(params.len(), 2);
        }
        // `build` keeps the default Postgres placeholders.
        assert_eq!(
            two_binds().build().0,
            "SELECT * FROM users WHERE id = $1 AND name = $2"
        );
    }

    #[test]
    fn explicit_db_type_wins_over_the_dialect() {
        use crate::dialect::{Mssql, Mysql};
        let (query, _) = two_binds()
            .with_db_type(DatabaseType::PostgreSQL)
            .build_for_dialect(&Mysql);
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND name = $2");
        let (query, _) = two_binds()
            .with_db_type(DatabaseType::MySQL)
            .build_for_dialect(&Mssql);
        assert_eq!(query, "SELECT * FROM users WHERE id = ? AND name = ?");
    }

    #[test]
    fn db_type_applies_to_parameters_bound_before_it() {
        let (query, _) = two_binds().with_db_type(DatabaseType::MSSQL).build();
        assert_eq!(query, "SELECT * FROM users WHERE id = @P1 AND name = @P2");
    }

    #[test]
    fn non_sql_dialect_keeps_the_default_placeholders() {
        let (query, _) = two_binds().build_for_dialect(&crate::dialect::NotSql);
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND name = $2");
    }

    #[test]
    fn binds_after_sql_with_params_continue_the_numbering() {
        let sql = sql_with_params("SELECT * FROM t WHERE a = $1", vec![FilterValue::Int(1)])
            .push(" AND b = ")
            .bind(2i32);
        let (query, params) = sql.build_for_dialect(&crate::dialect::Mssql);
        assert_eq!(query, "SELECT * FROM t WHERE a = $1 AND b = @P2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_sql_is_empty() {
        assert!(Sql::empty().is_empty());
        assert!(!Sql::new("SELECT 1").is_empty());
    }

    #[test]
    fn test_sql_display() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ").bind(42i32);
        assert_eq!(format!("{}", sql), "SELECT * FROM users WHERE id = $1");
    }

    #[test]
    fn test_raw_query_macro_no_params() {
        let sql = raw_query!("SELECT * FROM users");
        assert_eq!(sql.sql(), "SELECT * FROM users");
        assert!(sql.params().is_empty());
    }

    #[test]
    fn test_raw_query_macro_with_params() {
        let sql = raw_query!(
            "SELECT * FROM users WHERE id = {} AND active = {}",
            42i32,
            true
        );
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND active = $2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_raw_query_macro_string_params() {
        let name = "John".to_string();
        let sql = raw_query!("SELECT * FROM users WHERE name = {}", name);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE name = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_bind_many() {
        let values: Vec<FilterValue> = vec![
            FilterValue::Int(1),
            FilterValue::Int(2),
            FilterValue::Int(3),
        ];

        let sql = Sql::new("SELECT * FROM users WHERE id IN (")
            .bind_many(values)
            .push(")");

        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id IN ($1$2$3)");
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_build_in_clause() {
        let ids = vec![1, 2, 3];

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${}", i)).collect();

        let sql = Sql::new(format!(
            "SELECT * FROM users WHERE id IN ({})",
            placeholders.join(", ")
        ));

        let params: Vec<FilterValue> = ids.into_iter().map(FilterValue::Int).collect();
        let sql = sql_with_params(sql.sql(), params);

        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id IN ($1, $2, $3)");
        assert_eq!(params.len(), 3);
    }
}
