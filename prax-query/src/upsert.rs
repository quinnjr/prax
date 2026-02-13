//! Upsert and conflict resolution support.
//!
//! This module provides types for building upsert operations with conflict
//! resolution across different database backends.
//!
//! # Database Support
//!
//! | Feature          | PostgreSQL     | MySQL              | SQLite         | MSSQL   | MongoDB      |
//! |------------------|----------------|--------------------|----------------|---------|--------------|
//! | ON CONFLICT      | ✅             | ❌                 | ✅             | ❌      | ❌           |
//! | ON DUPLICATE KEY | ❌             | ✅                 | ❌             | ❌      | ❌           |
//! | MERGE statement  | ❌             | ❌                 | ❌             | ✅      | ❌           |
//! | Native upsert    | ❌             | ❌                 | ❌             | ❌      | ✅ upsert:true|
//! | Conflict targets | ✅             | ❌ (implicit PK/UK)| ✅             | ✅      | ✅ filter    |
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::upsert::{Upsert, ConflictTarget, ConflictAction};
//!
//! // PostgreSQL: INSERT ... ON CONFLICT (email) DO UPDATE SET ...
//! let upsert = Upsert::new("users")
//!     .columns(["email", "name", "updated_at"])
//!     .values(["$1", "$2", "NOW()"])
//!     .on_conflict(ConflictTarget::columns(["email"]))
//!     .do_update(["name", "updated_at"]);
//! ```

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

/// An upsert operation (INSERT with conflict handling).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Upsert {
    /// Table name.
    pub table: String,
    /// Columns to insert.
    pub columns: Vec<String>,
    /// Values to insert (expressions or placeholders).
    pub values: Vec<String>,
    /// Conflict target specification.
    pub conflict_target: Option<ConflictTarget>,
    /// Action to take on conflict.
    pub conflict_action: ConflictAction,
    /// WHERE clause for conflict update (PostgreSQL).
    pub where_clause: Option<String>,
    /// RETURNING clause (PostgreSQL).
    pub returning: Option<Vec<String>>,
}

/// What to match on for conflict detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictTarget {
    /// Match on specific columns (unique constraint).
    Columns(Vec<String>),
    /// Match on a named constraint.
    Constraint(String),
    /// Match on index expression (PostgreSQL).
    IndexExpression(String),
    /// No specific target (MySQL ON DUPLICATE KEY).
    Implicit,
}

impl ConflictTarget {
    /// Create a column-based conflict target.
    pub fn columns<I, S>(cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Columns(cols.into_iter().map(Into::into).collect())
    }

    /// Create a constraint-based conflict target.
    pub fn constraint(name: impl Into<String>) -> Self {
        Self::Constraint(name.into())
    }

    /// Create an index expression conflict target.
    pub fn index_expression(expr: impl Into<String>) -> Self {
        Self::IndexExpression(expr.into())
    }

    /// Generate PostgreSQL ON CONFLICT target.
    pub fn to_postgres_sql(&self) -> String {
        match self {
            Self::Columns(cols) => format!("({})", cols.join(", ")),
            Self::Constraint(name) => format!("ON CONSTRAINT {}", name),
            Self::IndexExpression(expr) => format!("({})", expr),
            Self::Implicit => String::new(),
        }
    }

    /// Generate SQLite ON CONFLICT target.
    pub fn to_sqlite_sql(&self) -> String {
        match self {
            Self::Columns(cols) => format!("({})", cols.join(", ")),
            Self::Constraint(_) | Self::IndexExpression(_) => {
                // SQLite doesn't support these directly
                String::new()
            }
            Self::Implicit => String::new(),
        }
    }
}

/// Action to take when a conflict is detected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConflictAction {
    /// Do nothing (ignore the insert).
    DoNothing,
    /// Update specified columns.
    DoUpdate(UpdateSpec),
}

/// Specification for what to update on conflict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateSpec {
    /// Columns to update with their values.
    pub assignments: Vec<Assignment>,
    /// Use EXCLUDED values for columns (PostgreSQL/SQLite).
    pub excluded_columns: Vec<String>,
}

/// A single column assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    /// Column name.
    pub column: String,
    /// Value expression.
    pub value: AssignmentValue,
}

/// Value for an assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssignmentValue {
    /// Use the EXCLUDED/VALUES value.
    Excluded,
    /// Use a literal expression.
    Expression(String),
    /// Use a parameter placeholder.
    Param(usize),
}

impl Upsert {
    /// Create a new upsert for the given table.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            columns: Vec::new(),
            values: Vec::new(),
            conflict_target: None,
            conflict_action: ConflictAction::DoNothing,
            where_clause: None,
            returning: None,
        }
    }

    /// Create an upsert builder.
    pub fn builder(table: impl Into<String>) -> UpsertBuilder {
        UpsertBuilder::new(table)
    }

    /// Set the columns to insert.
    pub fn columns<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = cols.into_iter().map(Into::into).collect();
        self
    }

    /// Set the values to insert.
    pub fn values<I, S>(mut self, vals: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.values = vals.into_iter().map(Into::into).collect();
        self
    }

    /// Set the conflict target.
    pub fn on_conflict(mut self, target: ConflictTarget) -> Self {
        self.conflict_target = Some(target);
        self
    }

    /// Set conflict action to DO NOTHING.
    pub fn do_nothing(mut self) -> Self {
        self.conflict_action = ConflictAction::DoNothing;
        self
    }

    /// Set conflict action to DO UPDATE for specified columns (using EXCLUDED).
    pub fn do_update<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.conflict_action = ConflictAction::DoUpdate(UpdateSpec {
            assignments: Vec::new(),
            excluded_columns: cols.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// Set conflict action to DO UPDATE with specific assignments.
    pub fn do_update_set(mut self, assignments: Vec<Assignment>) -> Self {
        self.conflict_action = ConflictAction::DoUpdate(UpdateSpec {
            assignments,
            excluded_columns: Vec::new(),
        });
        self
    }

    /// Add a WHERE clause for the update (PostgreSQL).
    pub fn where_clause(mut self, condition: impl Into<String>) -> Self {
        self.where_clause = Some(condition.into());
        self
    }

    /// Add RETURNING clause (PostgreSQL).
    pub fn returning<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.returning = Some(cols.into_iter().map(Into::into).collect());
        self
    }

    /// Generate PostgreSQL INSERT ... ON CONFLICT SQL.
    pub fn to_postgres_sql(&self) -> String {
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table,
            self.columns.join(", "),
            self.values.join(", ")
        );

        sql.push_str(" ON CONFLICT ");

        if let Some(ref target) = self.conflict_target {
            sql.push_str(&target.to_postgres_sql());
            sql.push(' ');
        }

        match &self.conflict_action {
            ConflictAction::DoNothing => {
                sql.push_str("DO NOTHING");
            }
            ConflictAction::DoUpdate(spec) => {
                sql.push_str("DO UPDATE SET ");
                let assignments: Vec<String> = if !spec.excluded_columns.is_empty() {
                    spec.excluded_columns
                        .iter()
                        .map(|c| format!("{} = EXCLUDED.{}", c, c))
                        .collect()
                } else {
                    spec.assignments
                        .iter()
                        .map(|a| {
                            let value = match &a.value {
                                AssignmentValue::Excluded => format!("EXCLUDED.{}", a.column),
                                AssignmentValue::Expression(expr) => expr.clone(),
                                AssignmentValue::Param(n) => format!("${}", n),
                            };
                            format!("{} = {}", a.column, value)
                        })
                        .collect()
                };
                sql.push_str(&assignments.join(", "));

                if let Some(ref where_clause) = self.where_clause {
                    sql.push_str(" WHERE ");
                    sql.push_str(where_clause);
                }
            }
        }

        if let Some(ref returning) = self.returning {
            sql.push_str(" RETURNING ");
            sql.push_str(&returning.join(", "));
        }

        sql
    }

    /// Generate MySQL INSERT ... ON DUPLICATE KEY UPDATE SQL.
    pub fn to_mysql_sql(&self) -> String {
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table,
            self.columns.join(", "),
            self.values.join(", ")
        );

        match &self.conflict_action {
            ConflictAction::DoNothing => {
                // MySQL doesn't have DO NOTHING, use INSERT IGNORE
                sql = format!(
                    "INSERT IGNORE INTO {} ({}) VALUES ({})",
                    self.table,
                    self.columns.join(", "),
                    self.values.join(", ")
                );
            }
            ConflictAction::DoUpdate(spec) => {
                sql.push_str(" ON DUPLICATE KEY UPDATE ");
                let assignments: Vec<String> = if !spec.excluded_columns.is_empty() {
                    spec.excluded_columns
                        .iter()
                        .map(|c| format!("{} = VALUES({})", c, c))
                        .collect()
                } else {
                    spec.assignments
                        .iter()
                        .map(|a| {
                            let value = match &a.value {
                                AssignmentValue::Excluded => format!("VALUES({})", a.column),
                                AssignmentValue::Expression(expr) => expr.clone(),
                                AssignmentValue::Param(_n) => "?".to_string(),
                            };
                            format!("{} = {}", a.column, value)
                        })
                        .collect()
                };
                sql.push_str(&assignments.join(", "));
            }
        }

        sql
    }

    /// Generate SQLite INSERT ... ON CONFLICT SQL.
    pub fn to_sqlite_sql(&self) -> String {
        let mut sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table,
            self.columns.join(", "),
            self.values.join(", ")
        );

        sql.push_str(" ON CONFLICT");

        if let Some(ref target) = self.conflict_target {
            let target_sql = target.to_sqlite_sql();
            if !target_sql.is_empty() {
                sql.push(' ');
                sql.push_str(&target_sql);
            }
        }

        match &self.conflict_action {
            ConflictAction::DoNothing => {
                sql.push_str(" DO NOTHING");
            }
            ConflictAction::DoUpdate(spec) => {
                sql.push_str(" DO UPDATE SET ");
                let assignments: Vec<String> = if !spec.excluded_columns.is_empty() {
                    spec.excluded_columns
                        .iter()
                        .map(|c| format!("{} = excluded.{}", c, c))
                        .collect()
                } else {
                    spec.assignments
                        .iter()
                        .map(|a| {
                            let value = match &a.value {
                                AssignmentValue::Excluded => format!("excluded.{}", a.column),
                                AssignmentValue::Expression(expr) => expr.clone(),
                                AssignmentValue::Param(_n) => "?".to_string(),
                            };
                            format!("{} = {}", a.column, value)
                        })
                        .collect()
                };
                sql.push_str(&assignments.join(", "));

                if let Some(ref where_clause) = self.where_clause {
                    sql.push_str(" WHERE ");
                    sql.push_str(where_clause);
                }
            }
        }

        if let Some(ref returning) = self.returning {
            sql.push_str(" RETURNING ");
            sql.push_str(&returning.join(", "));
        }

        sql
    }

    /// Generate MSSQL MERGE statement.
    pub fn to_mssql_sql(&self) -> String {
        let target = self
            .conflict_target
            .as_ref()
            .and_then(|t| match t {
                ConflictTarget::Columns(cols) => Some(cols.clone()),
                _ => None,
            })
            .unwrap_or_else(|| vec![self.columns.first().cloned().unwrap_or_default()]);

        let source_cols: Vec<String> = self
            .columns
            .iter()
            .zip(&self.values)
            .map(|(c, v)| format!("{} AS {}", v, c))
            .collect();

        let match_conditions: Vec<String> = target
            .iter()
            .map(|c| format!("target.{} = source.{}", c, c))
            .collect();

        let mut sql = format!(
            "MERGE INTO {} AS target USING (SELECT {}) AS source ON {}",
            self.table,
            source_cols.join(", "),
            match_conditions.join(" AND ")
        );

        match &self.conflict_action {
            ConflictAction::DoNothing => {
                // MSSQL MERGE requires at least one action
                sql.push_str(" WHEN NOT MATCHED THEN INSERT (");
                sql.push_str(&self.columns.join(", "));
                sql.push_str(") VALUES (");
                let source_refs: Vec<String> = self
                    .columns
                    .iter()
                    .map(|c| format!("source.{}", c))
                    .collect();
                sql.push_str(&source_refs.join(", "));
                sql.push(')');
            }
            ConflictAction::DoUpdate(spec) => {
                sql.push_str(" WHEN MATCHED THEN UPDATE SET ");

                let update_cols = if !spec.excluded_columns.is_empty() {
                    &spec.excluded_columns
                } else {
                    &self.columns
                };

                let assignments: Vec<String> = update_cols
                    .iter()
                    .filter(|c| !target.contains(c))
                    .map(|c| format!("target.{} = source.{}", c, c))
                    .collect();

                if assignments.is_empty() {
                    // Need at least one assignment, use first non-key column
                    let first_non_key = self.columns.iter().find(|c| !target.contains(*c));
                    if let Some(col) = first_non_key {
                        sql.push_str(&format!("target.{} = source.{}", col, col));
                    } else {
                        sql.push_str(&format!(
                            "target.{} = source.{}",
                            self.columns[0], self.columns[0]
                        ));
                    }
                } else {
                    sql.push_str(&assignments.join(", "));
                }

                sql.push_str(" WHEN NOT MATCHED THEN INSERT (");
                sql.push_str(&self.columns.join(", "));
                sql.push_str(") VALUES (");
                let source_refs: Vec<String> = self
                    .columns
                    .iter()
                    .map(|c| format!("source.{}", c))
                    .collect();
                sql.push_str(&source_refs.join(", "));
                sql.push(')');
            }
        }

        sql.push(';');
        sql
    }

    /// Generate SQL for the specified database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_sql(),
            DatabaseType::MySQL => self.to_mysql_sql(),
            DatabaseType::SQLite => self.to_sqlite_sql(),
            DatabaseType::MSSQL => self.to_mssql_sql(),
        }
    }
}

/// Builder for upsert operations.
#[derive(Debug, Clone, Default)]
pub struct UpsertBuilder {
    table: String,
    columns: Vec<String>,
    values: Vec<String>,
    conflict_target: Option<ConflictTarget>,
    conflict_action: Option<ConflictAction>,
    where_clause: Option<String>,
    returning: Option<Vec<String>>,
}

impl UpsertBuilder {
    /// Create a new builder.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            ..Default::default()
        }
    }

    /// Add columns to insert.
    pub fn columns<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = cols.into_iter().map(Into::into).collect();
        self
    }

    /// Add values to insert.
    pub fn values<I, S>(mut self, vals: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.values = vals.into_iter().map(Into::into).collect();
        self
    }

    /// Set conflict target columns.
    pub fn on_conflict_columns<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.conflict_target = Some(ConflictTarget::columns(cols));
        self
    }

    /// Set conflict target constraint.
    pub fn on_conflict_constraint(mut self, name: impl Into<String>) -> Self {
        self.conflict_target = Some(ConflictTarget::Constraint(name.into()));
        self
    }

    /// Set action to DO NOTHING.
    pub fn do_nothing(mut self) -> Self {
        self.conflict_action = Some(ConflictAction::DoNothing);
        self
    }

    /// Set action to DO UPDATE with excluded columns.
    pub fn do_update<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.conflict_action = Some(ConflictAction::DoUpdate(UpdateSpec {
            assignments: Vec::new(),
            excluded_columns: cols.into_iter().map(Into::into).collect(),
        }));
        self
    }

    /// Set action to DO UPDATE with assignments.
    pub fn do_update_assignments(mut self, assignments: Vec<Assignment>) -> Self {
        self.conflict_action = Some(ConflictAction::DoUpdate(UpdateSpec {
            assignments,
            excluded_columns: Vec::new(),
        }));
        self
    }

    /// Add WHERE clause for update.
    pub fn where_clause(mut self, condition: impl Into<String>) -> Self {
        self.where_clause = Some(condition.into());
        self
    }

    /// Add RETURNING clause.
    pub fn returning<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.returning = Some(cols.into_iter().map(Into::into).collect());
        self
    }

    /// Build the upsert.
    pub fn build(self) -> QueryResult<Upsert> {
        if self.columns.is_empty() {
            return Err(QueryError::invalid_input(
                "columns",
                "Upsert requires at least one column",
            ));
        }
        if self.values.is_empty() {
            return Err(QueryError::invalid_input(
                "values",
                "Upsert requires at least one value",
            ));
        }

        Ok(Upsert {
            table: self.table,
            columns: self.columns,
            values: self.values,
            conflict_target: self.conflict_target,
            conflict_action: self.conflict_action.unwrap_or(ConflictAction::DoNothing),
            where_clause: self.where_clause,
            returning: self.returning,
        })
    }
}

/// MongoDB upsert operations.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    /// MongoDB upsert operation builder.
    #[derive(Debug, Clone, Default)]
    pub struct MongoUpsert {
        /// Filter to find existing document.
        pub filter: serde_json::Map<String, JsonValue>,
        /// Update operations or replacement document.
        pub update: JsonValue,
        /// Insert-only fields ($setOnInsert).
        pub set_on_insert: Option<serde_json::Map<String, JsonValue>>,
        /// Array filters for updates.
        pub array_filters: Option<Vec<JsonValue>>,
    }

    impl MongoUpsert {
        /// Create a new upsert with filter.
        pub fn new() -> MongoUpsertBuilder {
            MongoUpsertBuilder::default()
        }

        /// Convert to updateOne options.
        pub fn to_update_one(&self) -> JsonValue {
            let mut options = serde_json::Map::new();
            options.insert("upsert".to_string(), JsonValue::Bool(true));

            if let Some(ref filters) = self.array_filters {
                options.insert(
                    "arrayFilters".to_string(),
                    JsonValue::Array(filters.clone()),
                );
            }

            serde_json::json!({
                "filter": self.filter,
                "update": self.update,
                "options": options
            })
        }

        /// Convert to findOneAndUpdate options.
        pub fn to_find_one_and_update(&self, return_new: bool) -> JsonValue {
            let mut options = serde_json::Map::new();
            options.insert("upsert".to_string(), JsonValue::Bool(true));
            options.insert(
                "returnDocument".to_string(),
                JsonValue::String(if return_new { "after" } else { "before" }.to_string()),
            );

            if let Some(ref filters) = self.array_filters {
                options.insert(
                    "arrayFilters".to_string(),
                    JsonValue::Array(filters.clone()),
                );
            }

            serde_json::json!({
                "filter": self.filter,
                "update": self.update,
                "options": options
            })
        }

        /// Convert to replaceOne options.
        pub fn to_replace_one(&self, replacement: JsonValue) -> JsonValue {
            serde_json::json!({
                "filter": self.filter,
                "replacement": replacement,
                "options": { "upsert": true }
            })
        }
    }

    /// Builder for MongoDB upsert.
    #[derive(Debug, Clone, Default)]
    #[allow(dead_code)]
    pub struct MongoUpsertBuilder {
        filter: serde_json::Map<String, JsonValue>,
        set: serde_json::Map<String, JsonValue>,
        set_on_insert: serde_json::Map<String, JsonValue>,
        inc: serde_json::Map<String, JsonValue>,
        unset: Vec<String>,
        push: serde_json::Map<String, JsonValue>,
        pull: serde_json::Map<String, JsonValue>,
        add_to_set: serde_json::Map<String, JsonValue>,
        array_filters: Option<Vec<JsonValue>>,
    }

    impl MongoUpsertBuilder {
        /// Set filter field equality.
        pub fn filter_eq(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.filter.insert(field.into(), value.into());
            self
        }

        /// Set filter with raw document.
        pub fn filter(mut self, filter: serde_json::Map<String, JsonValue>) -> Self {
            self.filter = filter;
            self
        }

        /// Add $set field.
        pub fn set(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.set.insert(field.into(), value.into());
            self
        }

        /// Add $setOnInsert field (only on insert).
        pub fn set_on_insert(
            mut self,
            field: impl Into<String>,
            value: impl Into<JsonValue>,
        ) -> Self {
            self.set_on_insert.insert(field.into(), value.into());
            self
        }

        /// Add $inc field.
        pub fn inc(mut self, field: impl Into<String>, amount: impl Into<JsonValue>) -> Self {
            self.inc.insert(field.into(), amount.into());
            self
        }

        /// Add $unset field.
        pub fn unset(mut self, field: impl Into<String>) -> Self {
            self.unset.push(field.into());
            self
        }

        /// Add $push field.
        pub fn push(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.push.insert(field.into(), value.into());
            self
        }

        /// Add $addToSet field.
        pub fn add_to_set(mut self, field: impl Into<String>, value: impl Into<JsonValue>) -> Self {
            self.add_to_set.insert(field.into(), value.into());
            self
        }

        /// Add array filters.
        pub fn array_filter(mut self, filter: JsonValue) -> Self {
            self.array_filters.get_or_insert_with(Vec::new).push(filter);
            self
        }

        /// Build the upsert.
        pub fn build(self) -> MongoUpsert {
            let mut update = serde_json::Map::new();

            if !self.set.is_empty() {
                update.insert("$set".to_string(), JsonValue::Object(self.set));
            }

            if !self.set_on_insert.is_empty() {
                update.insert(
                    "$setOnInsert".to_string(),
                    JsonValue::Object(self.set_on_insert.clone()),
                );
            }

            if !self.inc.is_empty() {
                update.insert("$inc".to_string(), JsonValue::Object(self.inc));
            }

            if !self.unset.is_empty() {
                let unset_obj: serde_json::Map<String, JsonValue> = self
                    .unset
                    .into_iter()
                    .map(|f| (f, JsonValue::String(String::new())))
                    .collect();
                update.insert("$unset".to_string(), JsonValue::Object(unset_obj));
            }

            if !self.push.is_empty() {
                update.insert("$push".to_string(), JsonValue::Object(self.push));
            }

            if !self.add_to_set.is_empty() {
                update.insert("$addToSet".to_string(), JsonValue::Object(self.add_to_set));
            }

            MongoUpsert {
                filter: self.filter,
                update: JsonValue::Object(update),
                set_on_insert: if self.set_on_insert.is_empty() {
                    None
                } else {
                    Some(self.set_on_insert)
                },
                array_filters: self.array_filters,
            }
        }
    }

    /// Bulk upsert operation.
    #[derive(Debug, Clone, Default)]
    pub struct BulkUpsert {
        /// Operations to perform.
        pub operations: Vec<BulkUpsertOp>,
        /// Whether operations are ordered.
        pub ordered: bool,
    }

    /// A single bulk upsert operation.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BulkUpsertOp {
        /// Filter to match document.
        pub filter: serde_json::Map<String, JsonValue>,
        /// Update document.
        pub update: JsonValue,
    }

    impl BulkUpsert {
        /// Create a new bulk upsert.
        pub fn new() -> Self {
            Self::default()
        }

        /// Set ordered mode.
        pub fn ordered(mut self, ordered: bool) -> Self {
            self.ordered = ordered;
            self
        }

        /// Add an upsert operation.
        pub fn add(
            mut self,
            filter: serde_json::Map<String, JsonValue>,
            update: JsonValue,
        ) -> Self {
            self.operations.push(BulkUpsertOp { filter, update });
            self
        }

        /// Convert to bulkWrite operations.
        pub fn to_bulk_write(&self) -> JsonValue {
            let ops: Vec<JsonValue> = self
                .operations
                .iter()
                .map(|op| {
                    serde_json::json!({
                        "updateOne": {
                            "filter": op.filter,
                            "update": op.update,
                            "upsert": true
                        }
                    })
                })
                .collect();

            serde_json::json!({
                "operations": ops,
                "options": { "ordered": self.ordered }
            })
        }
    }

    /// Helper to create a MongoDB upsert.
    pub fn upsert() -> MongoUpsertBuilder {
        MongoUpsertBuilder::default()
    }

    /// Helper to create a bulk upsert.
    pub fn bulk_upsert() -> BulkUpsert {
        BulkUpsert::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postgres_on_conflict_do_nothing() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["$1", "$2"])
            .on_conflict(ConflictTarget::columns(["email"]))
            .do_nothing();

        let sql = upsert.to_postgres_sql();
        assert!(sql.contains("INSERT INTO users"));
        assert!(sql.contains("ON CONFLICT (email) DO NOTHING"));
    }

    #[test]
    fn test_postgres_on_conflict_do_update() {
        let upsert = Upsert::new("users")
            .columns(["email", "name", "updated_at"])
            .values(["$1", "$2", "NOW()"])
            .on_conflict(ConflictTarget::columns(["email"]))
            .do_update(["name", "updated_at"]);

        let sql = upsert.to_postgres_sql();
        assert!(sql.contains("ON CONFLICT (email) DO UPDATE SET"));
        assert!(sql.contains("name = EXCLUDED.name"));
        assert!(sql.contains("updated_at = EXCLUDED.updated_at"));
    }

    #[test]
    fn test_postgres_with_where() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["$1", "$2"])
            .on_conflict(ConflictTarget::columns(["email"]))
            .do_update(["name"])
            .where_clause("users.active = true");

        let sql = upsert.to_postgres_sql();
        assert!(sql.contains("WHERE users.active = true"));
    }

    #[test]
    fn test_postgres_with_returning() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["$1", "$2"])
            .on_conflict(ConflictTarget::columns(["email"]))
            .do_update(["name"])
            .returning(["id", "email"]);

        let sql = upsert.to_postgres_sql();
        assert!(sql.contains("RETURNING id, email"));
    }

    #[test]
    fn test_mysql_on_duplicate_key() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["?", "?"])
            .do_update(["name"]);

        let sql = upsert.to_mysql_sql();
        assert!(sql.contains("INSERT INTO users"));
        assert!(sql.contains("ON DUPLICATE KEY UPDATE"));
        assert!(sql.contains("name = VALUES(name)"));
    }

    #[test]
    fn test_mysql_insert_ignore() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["?", "?"])
            .do_nothing();

        let sql = upsert.to_mysql_sql();
        assert!(sql.contains("INSERT IGNORE INTO users"));
    }

    #[test]
    fn test_sqlite_on_conflict() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["?", "?"])
            .on_conflict(ConflictTarget::columns(["email"]))
            .do_update(["name"]);

        let sql = upsert.to_sqlite_sql();
        assert!(sql.contains("ON CONFLICT (email) DO UPDATE SET"));
        assert!(sql.contains("name = excluded.name"));
    }

    #[test]
    fn test_mssql_merge() {
        let upsert = Upsert::new("users")
            .columns(["email", "name"])
            .values(["@P1", "@P2"])
            .on_conflict(ConflictTarget::columns(["email"]))
            .do_update(["name"]);

        let sql = upsert.to_mssql_sql();
        assert!(sql.contains("MERGE INTO users AS target"));
        assert!(sql.contains("USING (SELECT"));
        assert!(sql.contains("WHEN MATCHED THEN UPDATE SET"));
        assert!(sql.contains("WHEN NOT MATCHED THEN INSERT"));
    }

    #[test]
    fn test_upsert_builder() {
        let upsert = UpsertBuilder::new("users")
            .columns(["email", "name"])
            .values(["$1", "$2"])
            .on_conflict_columns(["email"])
            .do_update(["name"])
            .returning(["id"])
            .build()
            .unwrap();

        assert_eq!(upsert.table, "users");
        assert_eq!(upsert.columns, vec!["email", "name"]);
    }

    #[test]
    fn test_conflict_target_constraint() {
        let target = ConflictTarget::constraint("users_email_key");
        assert_eq!(target.to_postgres_sql(), "ON CONSTRAINT users_email_key");
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_simple_upsert() {
            let upsert = upsert()
                .filter_eq("email", "test@example.com")
                .set("name", "John")
                .set("updated_at", serde_json::json!({"$date": "2024-01-01"}))
                .set_on_insert("created_at", serde_json::json!({"$date": "2024-01-01"}))
                .build();

            let doc = upsert.to_update_one();
            assert!(doc["options"]["upsert"].as_bool().unwrap());
            assert!(doc["update"]["$set"]["name"].is_string());
            assert!(doc["update"]["$setOnInsert"].is_object());
        }

        #[test]
        fn test_upsert_with_inc() {
            let upsert = upsert()
                .filter_eq("_id", "doc1")
                .inc("visits", 1)
                .set("last_visit", "2024-01-01")
                .build();

            let doc = upsert.to_update_one();
            assert_eq!(doc["update"]["$inc"]["visits"], 1);
        }

        #[test]
        fn test_find_one_and_update() {
            let upsert = upsert()
                .filter_eq("email", "test@example.com")
                .set("name", "Updated")
                .build();

            let doc = upsert.to_find_one_and_update(true);
            assert_eq!(doc["options"]["returnDocument"], "after");
            assert!(doc["options"]["upsert"].as_bool().unwrap());
        }

        #[test]
        fn test_bulk_upsert() {
            let mut filter1 = serde_json::Map::new();
            filter1.insert("email".to_string(), serde_json::json!("a@b.com"));

            let mut filter2 = serde_json::Map::new();
            filter2.insert("email".to_string(), serde_json::json!("c@d.com"));

            let bulk = bulk_upsert()
                .ordered(false)
                .add(filter1, serde_json::json!({"$set": {"name": "A"}}))
                .add(filter2, serde_json::json!({"$set": {"name": "B"}}));

            let doc = bulk.to_bulk_write();
            assert!(!doc["options"]["ordered"].as_bool().unwrap());
            assert_eq!(doc["operations"].as_array().unwrap().len(), 2);
        }
    }
}
