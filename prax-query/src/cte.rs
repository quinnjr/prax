//! Common Table Expressions (CTEs) support.
//!
//! This module provides types for building CTEs (WITH clauses) across
//! different database backends.
//!
//! # Supported Features
//!
//! | Feature          | PostgreSQL | MySQL | SQLite | MSSQL | MongoDB        |
//! |------------------|------------|-------|--------|-------|----------------|
//! | Non-recursive    | ✅         | ✅    | ✅     | ✅    | ❌ ($lookup)   |
//! | Recursive        | ✅         | ✅    | ✅     | ✅    | ❌             |
//! | Materialized     | ✅         | ❌    | ❌     | ❌    | ❌             |
//! | Pipeline stages  | ❌         | ❌    | ❌     | ❌    | ✅ $lookup     |
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::cte::{Cte, CteBuilder, WithClause};
//!
//! // Simple CTE
//! let cte = Cte::new("active_users")
//!     .columns(["id", "name", "email"])
//!     .as_query("SELECT * FROM users WHERE active = true");
//!
//! // Build full query with CTE
//! let query = WithClause::new()
//!     .cte(cte)
//!     .select("*")
//!     .from("active_users")
//!     .build();
//! ```

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

/// A Common Table Expression (CTE) definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cte {
    /// Name of the CTE (used in FROM clause).
    pub name: String,
    /// Optional column aliases.
    pub columns: Vec<String>,
    /// The query that defines the CTE.
    pub query: String,
    /// Whether this is a recursive CTE.
    pub recursive: bool,
    /// PostgreSQL: MATERIALIZED / NOT MATERIALIZED hint.
    pub materialized: Option<Materialized>,
    /// Search clause for recursive CTEs (PostgreSQL).
    pub search: Option<SearchClause>,
    /// Cycle detection for recursive CTEs (PostgreSQL).
    pub cycle: Option<CycleClause>,
}

/// Materialization hint for CTEs (PostgreSQL only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Materialized {
    /// Force materialization.
    Yes,
    /// Prevent materialization (inline the CTE).
    No,
}

/// Search clause for recursive CTEs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchClause {
    /// Search method.
    pub method: SearchMethod,
    /// Columns to search by.
    pub columns: Vec<String>,
    /// Column to store the search sequence.
    pub set_column: String,
}

/// Search method for recursive CTEs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchMethod {
    /// Breadth-first search.
    BreadthFirst,
    /// Depth-first search.
    DepthFirst,
}

/// Cycle detection for recursive CTEs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleClause {
    /// Columns to check for cycles.
    pub columns: Vec<String>,
    /// Column to mark cycle detection.
    pub set_column: String,
    /// Column to store the path.
    pub using_column: String,
    /// Value when cycle is detected.
    pub mark_value: Option<String>,
    /// Value when no cycle.
    pub default_value: Option<String>,
}

impl Cte {
    /// Create a new CTE with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
            query: String::new(),
            recursive: false,
            materialized: None,
            search: None,
            cycle: None,
        }
    }

    /// Create a new CTE builder.
    pub fn builder(name: impl Into<String>) -> CteBuilder {
        CteBuilder::new(name)
    }

    /// Set the column aliases.
    pub fn columns<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Set the query that defines this CTE.
    pub fn as_query(mut self, query: impl Into<String>) -> Self {
        self.query = query.into();
        self
    }

    /// Mark this as a recursive CTE.
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Set materialization hint (PostgreSQL only).
    pub fn materialized(mut self, mat: Materialized) -> Self {
        self.materialized = Some(mat);
        self
    }

    /// Generate the CTE definition SQL.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        let mut sql = self.name.clone();

        // Column aliases
        if !self.columns.is_empty() {
            sql.push_str(" (");
            sql.push_str(&self.columns.join(", "));
            sql.push(')');
        }

        sql.push_str(" AS ");

        // Materialization hint (PostgreSQL only)
        if db_type == DatabaseType::PostgreSQL
            && let Some(mat) = self.materialized
        {
            match mat {
                Materialized::Yes => sql.push_str("MATERIALIZED "),
                Materialized::No => sql.push_str("NOT MATERIALIZED "),
            }
        }

        sql.push('(');
        sql.push_str(&self.query);
        sql.push(')');

        // Search clause (PostgreSQL only)
        if db_type == DatabaseType::PostgreSQL {
            if let Some(ref search) = self.search {
                sql.push_str(" SEARCH ");
                sql.push_str(match search.method {
                    SearchMethod::BreadthFirst => "BREADTH FIRST BY ",
                    SearchMethod::DepthFirst => "DEPTH FIRST BY ",
                });
                sql.push_str(&search.columns.join(", "));
                sql.push_str(" SET ");
                sql.push_str(&search.set_column);
            }

            if let Some(ref cycle) = self.cycle {
                sql.push_str(" CYCLE ");
                sql.push_str(&cycle.columns.join(", "));
                sql.push_str(" SET ");
                sql.push_str(&cycle.set_column);
                if let (Some(mark), Some(default)) = (&cycle.mark_value, &cycle.default_value) {
                    sql.push_str(" TO ");
                    sql.push_str(mark);
                    sql.push_str(" DEFAULT ");
                    sql.push_str(default);
                }
                sql.push_str(" USING ");
                sql.push_str(&cycle.using_column);
            }
        }

        sql
    }
}

/// Builder for CTEs.
#[derive(Debug, Clone)]
pub struct CteBuilder {
    name: String,
    columns: Vec<String>,
    query: Option<String>,
    recursive: bool,
    materialized: Option<Materialized>,
    search: Option<SearchClause>,
    cycle: Option<CycleClause>,
}

impl CteBuilder {
    /// Create a new CTE builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
            query: None,
            recursive: false,
            materialized: None,
            search: None,
            cycle: None,
        }
    }

    /// Set the column aliases.
    pub fn columns<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Set the query that defines this CTE.
    pub fn as_query(mut self, query: impl Into<String>) -> Self {
        self.query = Some(query.into());
        self
    }

    /// Mark this as a recursive CTE.
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Set materialization hint (PostgreSQL only).
    pub fn materialized(mut self) -> Self {
        self.materialized = Some(Materialized::Yes);
        self
    }

    /// Prevent materialization (PostgreSQL only).
    pub fn not_materialized(mut self) -> Self {
        self.materialized = Some(Materialized::No);
        self
    }

    /// Add breadth-first search (PostgreSQL only).
    pub fn search_breadth_first<I, S>(mut self, columns: I, set_column: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.search = Some(SearchClause {
            method: SearchMethod::BreadthFirst,
            columns: columns.into_iter().map(Into::into).collect(),
            set_column: set_column.into(),
        });
        self
    }

    /// Add depth-first search (PostgreSQL only).
    pub fn search_depth_first<I, S>(mut self, columns: I, set_column: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.search = Some(SearchClause {
            method: SearchMethod::DepthFirst,
            columns: columns.into_iter().map(Into::into).collect(),
            set_column: set_column.into(),
        });
        self
    }

    /// Add cycle detection (PostgreSQL only).
    pub fn cycle<I, S>(
        mut self,
        columns: I,
        set_column: impl Into<String>,
        using_column: impl Into<String>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.cycle = Some(CycleClause {
            columns: columns.into_iter().map(Into::into).collect(),
            set_column: set_column.into(),
            using_column: using_column.into(),
            mark_value: None,
            default_value: None,
        });
        self
    }

    /// Build the CTE.
    pub fn build(self) -> QueryResult<Cte> {
        let query = self.query.ok_or_else(|| {
            QueryError::invalid_input("query", "CTE requires a query (use as_query())")
        })?;

        Ok(Cte {
            name: self.name,
            columns: self.columns,
            query,
            recursive: self.recursive,
            materialized: self.materialized,
            search: self.search,
            cycle: self.cycle,
        })
    }
}

/// A WITH clause containing one or more CTEs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WithClause {
    /// The CTEs in this WITH clause.
    pub ctes: Vec<Cte>,
    /// Whether any CTE is recursive.
    pub recursive: bool,
    /// The main query that uses the CTEs.
    pub main_query: Option<String>,
}

impl WithClause {
    /// Create a new empty WITH clause.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a CTE to this WITH clause.
    pub fn cte(mut self, cte: Cte) -> Self {
        if cte.recursive {
            self.recursive = true;
        }
        self.ctes.push(cte);
        self
    }

    /// Add multiple CTEs.
    pub fn ctes<I>(mut self, ctes: I) -> Self
    where
        I: IntoIterator<Item = Cte>,
    {
        for cte in ctes {
            self = self.cte(cte);
        }
        self
    }

    /// Set the main query.
    pub fn main_query(mut self, query: impl Into<String>) -> Self {
        self.main_query = Some(query.into());
        self
    }

    /// Convenience: SELECT from a CTE.
    pub fn select(self, columns: impl Into<String>) -> WithQueryBuilder {
        WithQueryBuilder {
            with_clause: self,
            select: columns.into(),
            from: None,
            where_clause: None,
            order_by: None,
            limit: None,
        }
    }

    /// Generate the full SQL.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        if self.ctes.is_empty() {
            return Err(QueryError::invalid_input(
                "ctes",
                "WITH clause requires at least one CTE",
            ));
        }

        let mut sql = String::with_capacity(256);

        sql.push_str("WITH ");
        if self.recursive {
            sql.push_str("RECURSIVE ");
        }

        let cte_sqls: Vec<String> = self.ctes.iter().map(|c| c.to_sql(db_type)).collect();
        sql.push_str(&cte_sqls.join(", "));

        if let Some(ref main) = self.main_query {
            sql.push(' ');
            sql.push_str(main);
        }

        Ok(sql)
    }
}

/// Builder for queries using WITH clause.
#[derive(Debug, Clone)]
pub struct WithQueryBuilder {
    with_clause: WithClause,
    select: String,
    from: Option<String>,
    where_clause: Option<String>,
    order_by: Option<String>,
    limit: Option<u64>,
}

impl WithQueryBuilder {
    /// Set the FROM clause.
    pub fn from(mut self, table: impl Into<String>) -> Self {
        self.from = Some(table.into());
        self
    }

    /// Set the WHERE clause.
    pub fn where_clause(mut self, condition: impl Into<String>) -> Self {
        self.where_clause = Some(condition.into());
        self
    }

    /// Set ORDER BY.
    pub fn order_by(mut self, order: impl Into<String>) -> Self {
        self.order_by = Some(order.into());
        self
    }

    /// Set LIMIT.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Build the full SQL query.
    pub fn build(mut self, db_type: DatabaseType) -> QueryResult<String> {
        // Build main query
        let mut main = format!("SELECT {}", self.select);

        if let Some(from) = self.from {
            main.push_str(" FROM ");
            main.push_str(&from);
        }

        if let Some(where_clause) = self.where_clause {
            main.push_str(" WHERE ");
            main.push_str(&where_clause);
        }

        let has_order_by = self.order_by.is_some();
        if let Some(order) = self.order_by {
            main.push_str(" ORDER BY ");
            main.push_str(&order);
        }

        if let Some(limit) = self.limit {
            match db_type {
                DatabaseType::MSSQL => {
                    // MSSQL uses TOP or OFFSET FETCH
                    if has_order_by {
                        main.push_str(&format!(" OFFSET 0 ROWS FETCH NEXT {} ROWS ONLY", limit));
                    } else {
                        // Need to inject TOP after SELECT
                        main = main.replacen("SELECT ", &format!("SELECT TOP {} ", limit), 1);
                    }
                }
                _ => {
                    main.push_str(&format!(" LIMIT {}", limit));
                }
            }
        }

        self.with_clause.main_query = Some(main);
        self.with_clause.to_sql(db_type)
    }
}

/// Helper functions for common CTE patterns.
pub mod patterns {
    use super::*;

    /// Create a recursive CTE for tree traversal (parent-child hierarchy).
    pub fn tree_traversal(
        cte_name: &str,
        table: &str,
        id_col: &str,
        parent_col: &str,
        root_condition: &str,
    ) -> Cte {
        let base_query = format!(
            "SELECT {id}, {parent}, 1 AS depth FROM {table} WHERE {root}",
            id = id_col,
            parent = parent_col,
            table = table,
            root = root_condition
        );

        let recursive_query = format!(
            "SELECT t.{id}, t.{parent}, c.depth + 1 FROM {table} t \
             INNER JOIN {cte} c ON t.{parent} = c.{id}",
            id = id_col,
            parent = parent_col,
            table = table,
            cte = cte_name
        );

        Cte::new(cte_name)
            .columns([id_col, parent_col, "depth"])
            .as_query(format!("{} UNION ALL {}", base_query, recursive_query))
            .recursive()
    }

    /// Create a recursive CTE for graph path finding.
    pub fn graph_path(
        cte_name: &str,
        edges_table: &str,
        from_col: &str,
        to_col: &str,
        start_node: &str,
    ) -> Cte {
        let base_query = format!(
            "SELECT {from_col}, {to_col}, ARRAY[{from_col}] AS path, 1 AS length \
             FROM {table} WHERE {from_col} = {start}",
            from_col = from_col,
            to_col = to_col,
            table = edges_table,
            start = start_node
        );

        let recursive_query = format!(
            "SELECT e.{from_col}, e.{to_col}, p.path || e.{to_col}, p.length + 1 \
             FROM {table} e \
             INNER JOIN {cte} p ON e.{from_col} = p.{to_col} \
             WHERE NOT e.{to_col} = ANY(p.path)",
            from_col = from_col,
            to_col = to_col,
            table = edges_table,
            cte = cte_name
        );

        Cte::new(cte_name)
            .columns([from_col, to_col, "path", "length"])
            .as_query(format!("{} UNION ALL {}", base_query, recursive_query))
            .recursive()
    }

    /// Create a CTE for pagination (row numbering).
    pub fn paginated(cte_name: &str, query: &str, order_by: &str) -> Cte {
        let paginated_query = format!(
            "SELECT *, ROW_NUMBER() OVER (ORDER BY {}) AS row_num FROM ({})",
            order_by, query
        );

        Cte::new(cte_name).as_query(paginated_query)
    }

    /// Create a CTE for running totals.
    pub fn running_total(
        cte_name: &str,
        table: &str,
        value_col: &str,
        order_col: &str,
        partition_col: Option<&str>,
    ) -> Cte {
        let partition = partition_col
            .map(|p| format!("PARTITION BY {} ", p))
            .unwrap_or_default();

        let query = format!(
            "SELECT *, SUM({value}) OVER ({partition}ORDER BY {order}) AS running_total \
             FROM {table}",
            value = value_col,
            partition = partition,
            order = order_col,
            table = table
        );

        Cte::new(cte_name).as_query(query)
    }
}

/// MongoDB $lookup pipeline support (CTE equivalent).
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    /// A $lookup stage for MongoDB aggregation pipelines.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Lookup {
        /// The foreign collection.
        pub from: String,
        /// Local field to match.
        pub local_field: Option<String>,
        /// Foreign field to match.
        pub foreign_field: Option<String>,
        /// Output array field name.
        pub as_field: String,
        /// Pipeline to run on matched documents.
        pub pipeline: Option<Vec<JsonValue>>,
        /// Variables to pass to pipeline.
        pub let_vars: Option<serde_json::Map<String, JsonValue>>,
    }

    impl Lookup {
        /// Create a simple $lookup (equality match).
        pub fn simple(
            from: impl Into<String>,
            local: impl Into<String>,
            foreign: impl Into<String>,
            as_field: impl Into<String>,
        ) -> Self {
            Self {
                from: from.into(),
                local_field: Some(local.into()),
                foreign_field: Some(foreign.into()),
                as_field: as_field.into(),
                pipeline: None,
                let_vars: None,
            }
        }

        /// Create a $lookup with pipeline (subquery).
        pub fn with_pipeline(
            from: impl Into<String>,
            as_field: impl Into<String>,
        ) -> LookupBuilder {
            LookupBuilder {
                from: from.into(),
                as_field: as_field.into(),
                pipeline: Vec::new(),
                let_vars: serde_json::Map::new(),
            }
        }

        /// Convert to BSON document.
        pub fn to_bson(&self) -> JsonValue {
            let mut lookup = serde_json::Map::new();
            lookup.insert("from".to_string(), JsonValue::String(self.from.clone()));

            if let (Some(local), Some(foreign)) = (&self.local_field, &self.foreign_field) {
                lookup.insert("localField".to_string(), JsonValue::String(local.clone()));
                lookup.insert(
                    "foreignField".to_string(),
                    JsonValue::String(foreign.clone()),
                );
            }

            lookup.insert("as".to_string(), JsonValue::String(self.as_field.clone()));

            if let Some(ref pipeline) = self.pipeline {
                lookup.insert("pipeline".to_string(), JsonValue::Array(pipeline.clone()));
            }

            if let Some(ref vars) = self.let_vars
                && !vars.is_empty()
            {
                lookup.insert("let".to_string(), JsonValue::Object(vars.clone()));
            }

            serde_json::json!({ "$lookup": lookup })
        }
    }

    /// Builder for $lookup with pipeline.
    #[derive(Debug, Clone)]
    pub struct LookupBuilder {
        from: String,
        as_field: String,
        pipeline: Vec<JsonValue>,
        let_vars: serde_json::Map<String, JsonValue>,
    }

    impl LookupBuilder {
        /// Add a variable for the pipeline.
        pub fn let_var(mut self, name: impl Into<String>, expr: impl Into<String>) -> Self {
            self.let_vars
                .insert(name.into(), JsonValue::String(format!("${}", expr.into())));
            self
        }

        /// Add a $match stage to the pipeline.
        pub fn match_expr(mut self, expr: JsonValue) -> Self {
            self.pipeline
                .push(serde_json::json!({ "$match": { "$expr": expr } }));
            self
        }

        /// Add a raw stage to the pipeline.
        pub fn stage(mut self, stage: JsonValue) -> Self {
            self.pipeline.push(stage);
            self
        }

        /// Add a $project stage.
        pub fn project(mut self, fields: JsonValue) -> Self {
            self.pipeline
                .push(serde_json::json!({ "$project": fields }));
            self
        }

        /// Add a $limit stage.
        pub fn limit(mut self, n: u64) -> Self {
            self.pipeline.push(serde_json::json!({ "$limit": n }));
            self
        }

        /// Add a $sort stage.
        pub fn sort(mut self, fields: JsonValue) -> Self {
            self.pipeline.push(serde_json::json!({ "$sort": fields }));
            self
        }

        /// Build the $lookup.
        pub fn build(self) -> Lookup {
            Lookup {
                from: self.from,
                local_field: None,
                foreign_field: None,
                as_field: self.as_field,
                pipeline: if self.pipeline.is_empty() {
                    None
                } else {
                    Some(self.pipeline)
                },
                let_vars: if self.let_vars.is_empty() {
                    None
                } else {
                    Some(self.let_vars)
                },
            }
        }
    }

    /// A $graphLookup stage for recursive lookups.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct GraphLookup {
        /// The collection to search.
        pub from: String,
        /// Starting value expression.
        pub start_with: String,
        /// Field to connect from.
        pub connect_from_field: String,
        /// Field to connect to.
        pub connect_to_field: String,
        /// Output array field.
        pub as_field: String,
        /// Maximum recursion depth.
        pub max_depth: Option<u32>,
        /// Name for depth field.
        pub depth_field: Option<String>,
        /// Filter to apply at each level.
        pub restrict_search_with_match: Option<JsonValue>,
    }

    impl GraphLookup {
        /// Create a new $graphLookup.
        pub fn new(
            from: impl Into<String>,
            start_with: impl Into<String>,
            connect_from: impl Into<String>,
            connect_to: impl Into<String>,
            as_field: impl Into<String>,
        ) -> Self {
            Self {
                from: from.into(),
                start_with: start_with.into(),
                connect_from_field: connect_from.into(),
                connect_to_field: connect_to.into(),
                as_field: as_field.into(),
                max_depth: None,
                depth_field: None,
                restrict_search_with_match: None,
            }
        }

        /// Set maximum recursion depth.
        pub fn max_depth(mut self, depth: u32) -> Self {
            self.max_depth = Some(depth);
            self
        }

        /// Add a depth field to results.
        pub fn depth_field(mut self, field: impl Into<String>) -> Self {
            self.depth_field = Some(field.into());
            self
        }

        /// Add a filter for each recursion level.
        pub fn restrict_search(mut self, filter: JsonValue) -> Self {
            self.restrict_search_with_match = Some(filter);
            self
        }

        /// Convert to BSON document.
        pub fn to_bson(&self) -> JsonValue {
            let mut graph = serde_json::Map::new();
            graph.insert("from".to_string(), JsonValue::String(self.from.clone()));
            graph.insert(
                "startWith".to_string(),
                JsonValue::String(format!("${}", self.start_with)),
            );
            graph.insert(
                "connectFromField".to_string(),
                JsonValue::String(self.connect_from_field.clone()),
            );
            graph.insert(
                "connectToField".to_string(),
                JsonValue::String(self.connect_to_field.clone()),
            );
            graph.insert("as".to_string(), JsonValue::String(self.as_field.clone()));

            if let Some(max) = self.max_depth {
                graph.insert("maxDepth".to_string(), JsonValue::Number(max.into()));
            }

            if let Some(ref field) = self.depth_field {
                graph.insert("depthField".to_string(), JsonValue::String(field.clone()));
            }

            if let Some(ref filter) = self.restrict_search_with_match {
                graph.insert("restrictSearchWithMatch".to_string(), filter.clone());
            }

            serde_json::json!({ "$graphLookup": graph })
        }
    }

    /// A $unionWith stage (similar to UNION ALL).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct UnionWith {
        /// Collection to union with.
        pub coll: String,
        /// Optional pipeline to apply before union.
        pub pipeline: Option<Vec<JsonValue>>,
    }

    impl UnionWith {
        /// Create a simple union with a collection.
        pub fn collection(coll: impl Into<String>) -> Self {
            Self {
                coll: coll.into(),
                pipeline: None,
            }
        }

        /// Create a union with a pipeline.
        pub fn with_pipeline(coll: impl Into<String>, pipeline: Vec<JsonValue>) -> Self {
            Self {
                coll: coll.into(),
                pipeline: Some(pipeline),
            }
        }

        /// Convert to BSON document.
        pub fn to_bson(&self) -> JsonValue {
            if let Some(ref pipeline) = self.pipeline {
                serde_json::json!({
                    "$unionWith": {
                        "coll": self.coll,
                        "pipeline": pipeline
                    }
                })
            } else {
                serde_json::json!({ "$unionWith": self.coll })
            }
        }
    }

    /// Helper to create a simple lookup.
    pub fn lookup(from: &str, local: &str, foreign: &str, as_field: &str) -> Lookup {
        Lookup::simple(from, local, foreign, as_field)
    }

    /// Helper to create a lookup with pipeline.
    pub fn lookup_pipeline(from: &str, as_field: &str) -> LookupBuilder {
        Lookup::with_pipeline(from, as_field)
    }

    /// Helper to create a graph lookup.
    pub fn graph_lookup(
        from: &str,
        start_with: &str,
        connect_from: &str,
        connect_to: &str,
        as_field: &str,
    ) -> GraphLookup {
        GraphLookup::new(from, start_with, connect_from, connect_to, as_field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_cte() {
        let cte = Cte::new("active_users").as_query("SELECT * FROM users WHERE active = true");

        let sql = cte.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("active_users AS"));
        assert!(sql.contains("SELECT * FROM users"));
    }

    #[test]
    fn test_cte_with_columns() {
        let cte = Cte::new("user_stats")
            .columns(["id", "name", "total"])
            .as_query("SELECT id, name, COUNT(*) FROM orders GROUP BY user_id");

        let sql = cte.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("user_stats (id, name, total) AS"));
    }

    #[test]
    fn test_recursive_cte() {
        let cte = Cte::new("subordinates")
            .columns(["id", "name", "manager_id", "depth"])
            .as_query(
                "SELECT id, name, manager_id, 1 FROM employees WHERE manager_id IS NULL \
                 UNION ALL \
                 SELECT e.id, e.name, e.manager_id, s.depth + 1 \
                 FROM employees e JOIN subordinates s ON e.manager_id = s.id",
            )
            .recursive();

        assert!(cte.recursive);
    }

    #[test]
    fn test_materialized_cte() {
        let cte = Cte::new("expensive_query")
            .as_query("SELECT * FROM big_table WHERE complex_condition")
            .materialized(Materialized::Yes);

        let sql = cte.to_sql(DatabaseType::PostgreSQL);
        assert!(sql.contains("MATERIALIZED"));
    }

    #[test]
    fn test_with_clause() {
        let cte1 = Cte::new("cte1").as_query("SELECT 1");
        let cte2 = Cte::new("cte2").as_query("SELECT 2");

        let with = WithClause::new()
            .cte(cte1)
            .cte(cte2)
            .main_query("SELECT * FROM cte1, cte2");

        let sql = with.to_sql(DatabaseType::PostgreSQL).unwrap();
        assert!(sql.starts_with("WITH "));
        assert!(sql.contains("cte1 AS"));
        assert!(sql.contains("cte2 AS"));
        assert!(sql.contains("SELECT * FROM cte1, cte2"));
    }

    #[test]
    fn test_recursive_with_clause() {
        let cte = Cte::new("numbers")
            .as_query("SELECT 1 AS n UNION ALL SELECT n + 1 FROM numbers WHERE n < 10")
            .recursive();

        let with = WithClause::new()
            .cte(cte)
            .main_query("SELECT * FROM numbers");

        let sql = with.to_sql(DatabaseType::PostgreSQL).unwrap();
        assert!(sql.starts_with("WITH RECURSIVE"));
    }

    #[test]
    fn test_with_query_builder() {
        let cte = Cte::new("active").as_query("SELECT * FROM users WHERE active = true");

        let sql = WithClause::new()
            .cte(cte)
            .select("*")
            .from("active")
            .where_clause("role = 'admin'")
            .order_by("name")
            .limit(10)
            .build(DatabaseType::PostgreSQL)
            .unwrap();

        assert!(sql.contains("WITH active AS"));
        assert!(sql.contains("SELECT *"));
        assert!(sql.contains("FROM active"));
        assert!(sql.contains("WHERE role = 'admin'"));
        assert!(sql.contains("ORDER BY name"));
        assert!(sql.contains("LIMIT 10"));
    }

    #[test]
    fn test_mssql_limit() {
        let cte = Cte::new("data").as_query("SELECT * FROM table1");

        let sql = WithClause::new()
            .cte(cte)
            .select("*")
            .from("data")
            .order_by("id")
            .limit(10)
            .build(DatabaseType::MSSQL)
            .unwrap();

        assert!(sql.contains("OFFSET 0 ROWS FETCH NEXT 10 ROWS ONLY"));
    }

    #[test]
    fn test_cte_builder() {
        let cte = CteBuilder::new("stats")
            .columns(["a", "b"])
            .as_query("SELECT 1, 2")
            .materialized()
            .build()
            .unwrap();

        assert_eq!(cte.name, "stats");
        assert_eq!(cte.columns, vec!["a", "b"]);
        assert_eq!(cte.materialized, Some(Materialized::Yes));
    }

    mod pattern_tests {
        use super::super::patterns::*;

        #[test]
        fn test_tree_traversal_pattern() {
            let cte = tree_traversal(
                "org_tree",
                "employees",
                "id",
                "manager_id",
                "manager_id IS NULL",
            );

            assert!(cte.recursive);
            assert!(cte.query.contains("UNION ALL"));
            assert!(cte.query.contains("depth + 1"));
        }

        #[test]
        fn test_running_total_pattern() {
            let cte = running_total(
                "account_balance",
                "transactions",
                "amount",
                "transaction_date",
                Some("account_id"),
            );

            assert!(cte.query.contains("SUM(amount)"));
            assert!(cte.query.contains("PARTITION BY account_id"));
            assert!(cte.query.contains("running_total"));
        }
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_simple_lookup() {
            let lookup = Lookup::simple("orders", "user_id", "_id", "user_orders");
            let bson = lookup.to_bson();

            assert_eq!(bson["$lookup"]["from"], "orders");
            assert_eq!(bson["$lookup"]["localField"], "user_id");
            assert_eq!(bson["$lookup"]["foreignField"], "_id");
            assert_eq!(bson["$lookup"]["as"], "user_orders");
        }

        #[test]
        fn test_lookup_with_pipeline() {
            let lookup = Lookup::with_pipeline("inventory", "stock_items")
                .let_var("order_item", "item")
                .match_expr(serde_json::json!({
                    "$eq": ["$sku", "$$order_item"]
                }))
                .project(serde_json::json!({ "inStock": 1 }))
                .build();

            let bson = lookup.to_bson();
            assert!(bson["$lookup"]["pipeline"].is_array());
            assert!(bson["$lookup"]["let"].is_object());
        }

        #[test]
        fn test_graph_lookup() {
            let lookup = GraphLookup::new(
                "employees",
                "reportsTo",
                "reportsTo",
                "name",
                "reportingHierarchy",
            )
            .max_depth(5)
            .depth_field("level");

            let bson = lookup.to_bson();
            assert_eq!(bson["$graphLookup"]["from"], "employees");
            assert_eq!(bson["$graphLookup"]["maxDepth"], 5);
            assert_eq!(bson["$graphLookup"]["depthField"], "level");
        }

        #[test]
        fn test_union_with() {
            let union = UnionWith::collection("archived_orders");
            let bson = union.to_bson();

            assert_eq!(bson["$unionWith"], "archived_orders");
        }

        #[test]
        fn test_union_with_pipeline() {
            let union = UnionWith::with_pipeline(
                "archive",
                vec![serde_json::json!({ "$match": { "year": 2023 } })],
            );
            let bson = union.to_bson();

            assert!(bson["$unionWith"]["pipeline"].is_array());
        }
    }
}
