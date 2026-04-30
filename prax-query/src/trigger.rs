//! Database trigger definitions and management.
//!
//! This module provides types for defining, creating, and managing database triggers
//! across different database backends.
//!
//! # Supported Features
//!
//! | Feature                   | PostgreSQL | MySQL | SQLite | MSSQL | MongoDB |
//! |---------------------------|------------|-------|--------|-------|---------|
//! | Row-Level Triggers        | ✅         | ✅    | ✅     | ✅    | ✅*     |
//! | Statement-Level Triggers  | ✅         | ❌    | ❌     | ✅    | ❌      |
//! | INSTEAD OF Triggers       | ✅         | ❌    | ✅     | ✅    | ❌      |
//! | BEFORE Triggers           | ✅         | ✅    | ✅     | ❌    | ❌      |
//! | AFTER Triggers            | ✅         | ✅    | ✅     | ✅    | ✅*     |
//!
//! > *MongoDB uses Change Streams for real-time notifications
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use prax_query::trigger::{Trigger, TriggerTiming, TriggerEvent, TriggerLevel};
//!
//! // Create an audit trigger
//! let trigger = Trigger::builder("audit_user_changes")
//!     .on_table("users")
//!     .timing(TriggerTiming::After)
//!     .events([TriggerEvent::Update, TriggerEvent::Delete])
//!     .level(TriggerLevel::Row)
//!     .execute_function("audit_log_changes")
//!     .build();
//!
//! // Generate SQL
//! let sql = trigger.to_postgres_sql();
//! ```

use std::borrow::Cow;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

/// When the trigger fires relative to the triggering event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerTiming {
    /// Fire before the operation (can modify NEW row).
    Before,
    /// Fire after the operation (cannot modify data).
    After,
    /// Replace the operation entirely (for views).
    InsteadOf,
}

impl TriggerTiming {
    /// Convert to SQL keyword.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Before => "BEFORE",
            Self::After => "AFTER",
            Self::InsteadOf => "INSTEAD OF",
        }
    }
}

/// The DML event that fires the trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerEvent {
    /// INSERT operation.
    Insert,
    /// UPDATE operation.
    Update,
    /// DELETE operation.
    Delete,
    /// TRUNCATE operation (PostgreSQL only).
    Truncate,
}

impl TriggerEvent {
    /// Convert to SQL keyword.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
        }
    }
}

/// Whether the trigger fires once per row or once per statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TriggerLevel {
    /// Fire once for each affected row.
    #[default]
    Row,
    /// Fire once for the entire statement.
    Statement,
}

impl TriggerLevel {
    /// Convert to SQL clause.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Row => "FOR EACH ROW",
            Self::Statement => "FOR EACH STATEMENT",
        }
    }
}

/// A column update specification for UPDATE triggers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOf {
    /// Columns that trigger on update.
    pub columns: Vec<String>,
}

impl UpdateOf {
    /// Create a new UpdateOf specification.
    pub fn new(columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
        }
    }

    /// Convert to SQL clause.
    pub fn to_sql(&self) -> String {
        if self.columns.is_empty() {
            String::new()
        } else {
            format!(" OF {}", self.columns.join(", "))
        }
    }
}

/// A WHEN condition for conditional triggers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCondition {
    /// The SQL expression for the condition.
    pub expression: String,
}

impl TriggerCondition {
    /// Create a new trigger condition.
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
        }
    }

    /// Check if OLD.column differs from NEW.column.
    pub fn column_changed(column: &str) -> Self {
        Self::new(format!("OLD.{} IS DISTINCT FROM NEW.{}", column, column))
    }

    /// Check if NEW.column is not null.
    pub fn new_not_null(column: &str) -> Self {
        Self::new(format!("NEW.{} IS NOT NULL", column))
    }

    /// Check if OLD.column was null.
    pub fn old_was_null(column: &str) -> Self {
        Self::new(format!("OLD.{} IS NULL", column))
    }

    /// Combine conditions with AND.
    pub fn and(self, other: Self) -> Self {
        Self::new(format!("({}) AND ({})", self.expression, other.expression))
    }

    /// Combine conditions with OR.
    pub fn or(self, other: Self) -> Self {
        Self::new(format!("({}) OR ({})", self.expression, other.expression))
    }
}

/// The action to execute when the trigger fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerAction {
    /// Execute a stored function/procedure.
    ExecuteFunction {
        /// Function name (optionally schema-qualified).
        name: String,
        /// Arguments to pass to the function.
        args: Vec<String>,
    },
    /// Execute inline SQL (MySQL style).
    InlineSql {
        /// SQL statements to execute.
        statements: Vec<String>,
    },
    /// Reference an existing trigger function (PostgreSQL).
    FunctionReference {
        /// Function name.
        name: String,
    },
}

impl TriggerAction {
    /// Create an action that executes a function.
    pub fn function(name: impl Into<String>) -> Self {
        Self::ExecuteFunction {
            name: name.into(),
            args: Vec::new(),
        }
    }

    /// Create an action that executes a function with arguments.
    pub fn function_with_args(
        name: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::ExecuteFunction {
            name: name.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Create an action with inline SQL.
    pub fn inline_sql(statements: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::InlineSql {
            statements: statements.into_iter().map(Into::into).collect(),
        }
    }
}

/// A database trigger definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    /// Trigger name.
    pub name: String,
    /// Schema name (optional).
    pub schema: Option<String>,
    /// Table or view the trigger is attached to.
    pub table: String,
    /// When the trigger fires (BEFORE/AFTER/INSTEAD OF).
    pub timing: TriggerTiming,
    /// Events that fire the trigger.
    pub events: HashSet<TriggerEvent>,
    /// Row-level or statement-level.
    pub level: TriggerLevel,
    /// Optional column list for UPDATE OF.
    pub update_of: Option<UpdateOf>,
    /// Optional WHEN condition.
    pub condition: Option<TriggerCondition>,
    /// The action to execute.
    pub action: TriggerAction,
    /// Whether the trigger is enabled.
    pub enabled: bool,
    /// Optional comment/description.
    pub comment: Option<String>,
}

impl Trigger {
    /// Create a new trigger builder.
    pub fn builder(name: impl Into<String>) -> TriggerBuilder {
        TriggerBuilder::new(name)
    }

    /// Get the fully qualified trigger name.
    pub fn qualified_name(&self) -> Cow<'_, str> {
        match &self.schema {
            Some(schema) => Cow::Owned(format!("{}.{}", schema, self.name)),
            None => Cow::Borrowed(&self.name),
        }
    }

    /// Get the fully qualified table name.
    pub fn qualified_table(&self) -> Cow<'_, str> {
        match &self.schema {
            Some(schema) => Cow::Owned(format!("{}.{}", schema, self.table)),
            None => Cow::Borrowed(&self.table),
        }
    }

    /// Generate PostgreSQL CREATE TRIGGER SQL.
    pub fn to_postgres_sql(&self) -> QueryResult<String> {
        let mut sql = String::with_capacity(256);

        sql.push_str("CREATE TRIGGER ");
        sql.push_str(&self.name);
        sql.push('\n');

        // Timing
        sql.push_str("    ");
        sql.push_str(self.timing.to_sql());
        sql.push(' ');

        // Events
        let events: Vec<_> = self.events.iter().collect();
        for (i, event) in events.iter().enumerate() {
            if i > 0 {
                sql.push_str(" OR ");
            }
            sql.push_str(event.to_sql());
            if *event == &TriggerEvent::Update
                && let Some(ref update_of) = self.update_of
            {
                sql.push_str(&update_of.to_sql());
            }
        }

        // Table
        sql.push_str("\n    ON ");
        sql.push_str(&self.qualified_table());
        sql.push('\n');

        // Level
        sql.push_str("    ");
        sql.push_str(self.level.to_sql());
        sql.push('\n');

        // Condition
        if let Some(ref condition) = self.condition {
            sql.push_str("    WHEN (");
            sql.push_str(&condition.expression);
            sql.push_str(")\n");
        }

        // Action
        sql.push_str("    EXECUTE ");
        match &self.action {
            TriggerAction::ExecuteFunction { name, args: _ }
            | TriggerAction::FunctionReference { name } => {
                sql.push_str("FUNCTION ");
                sql.push_str(name);
                sql.push('(');
                if let TriggerAction::ExecuteFunction { args, .. } = &self.action {
                    sql.push_str(&args.join(", "));
                }
                sql.push(')');
            }
            TriggerAction::InlineSql { .. } => {
                return Err(QueryError::unsupported(
                    "PostgreSQL triggers require a function, not inline SQL",
                ));
            }
        }

        sql.push(';');

        Ok(sql)
    }

    /// Generate MySQL CREATE TRIGGER SQL.
    pub fn to_mysql_sql(&self) -> QueryResult<String> {
        // MySQL doesn't support statement-level triggers
        if self.level == TriggerLevel::Statement {
            return Err(QueryError::unsupported(
                "MySQL does not support statement-level triggers",
            ));
        }

        // MySQL doesn't support INSTEAD OF triggers
        if self.timing == TriggerTiming::InsteadOf {
            return Err(QueryError::unsupported(
                "MySQL does not support INSTEAD OF triggers",
            ));
        }

        // MySQL triggers can only have one event
        if self.events.len() != 1 {
            return Err(QueryError::unsupported(
                "MySQL triggers can only have one triggering event. Create separate triggers for each event.",
            ));
        }

        let event = self.events.iter().next().unwrap();

        let mut sql = String::with_capacity(256);

        sql.push_str("CREATE TRIGGER ");
        sql.push_str(&self.name);
        sql.push('\n');

        // Timing and event
        sql.push_str("    ");
        sql.push_str(self.timing.to_sql());
        sql.push(' ');
        sql.push_str(event.to_sql());
        sql.push('\n');

        // Table
        sql.push_str("    ON `");
        sql.push_str(&self.table);
        sql.push_str("`\n");

        // Level (MySQL only supports FOR EACH ROW)
        sql.push_str("    FOR EACH ROW\n");

        // Action
        match &self.action {
            TriggerAction::InlineSql { statements } => {
                if statements.len() == 1 {
                    sql.push_str("    ");
                    sql.push_str(&statements[0]);
                } else {
                    sql.push_str("BEGIN\n");
                    for stmt in statements {
                        sql.push_str("    ");
                        sql.push_str(stmt);
                        sql.push_str(";\n");
                    }
                    sql.push_str("END");
                }
            }
            TriggerAction::ExecuteFunction { name, args } => {
                sql.push_str("    CALL ");
                sql.push_str(name);
                sql.push('(');
                sql.push_str(&args.join(", "));
                sql.push(')');
            }
            TriggerAction::FunctionReference { name } => {
                sql.push_str("    CALL ");
                sql.push_str(name);
                sql.push_str("()");
            }
        }

        sql.push(';');

        Ok(sql)
    }

    /// Generate SQLite CREATE TRIGGER SQL.
    pub fn to_sqlite_sql(&self) -> QueryResult<String> {
        // SQLite doesn't support statement-level triggers
        if self.level == TriggerLevel::Statement {
            return Err(QueryError::unsupported(
                "SQLite does not support statement-level triggers",
            ));
        }

        let mut sql = String::with_capacity(256);

        sql.push_str("CREATE TRIGGER ");
        if self.schema.is_some() {
            return Err(QueryError::unsupported(
                "SQLite does not support schema-qualified trigger names",
            ));
        }
        sql.push_str(&self.name);
        sql.push('\n');

        // Timing
        sql.push_str("    ");
        sql.push_str(self.timing.to_sql());
        sql.push(' ');

        // Events (SQLite only supports one event per trigger)
        if self.events.len() != 1 {
            return Err(QueryError::unsupported(
                "SQLite triggers can only have one triggering event",
            ));
        }

        let event = self.events.iter().next().unwrap();
        sql.push_str(event.to_sql());

        if *event == TriggerEvent::Update
            && let Some(ref update_of) = self.update_of
        {
            sql.push_str(&update_of.to_sql());
        }

        // Table
        sql.push_str("\n    ON `");
        sql.push_str(&self.table);
        sql.push_str("`\n");

        // Level
        sql.push_str("    FOR EACH ROW\n");

        // Condition
        if let Some(ref condition) = self.condition {
            sql.push_str("    WHEN ");
            sql.push_str(&condition.expression);
            sql.push('\n');
        }

        // Action (SQLite uses inline SQL)
        sql.push_str("BEGIN\n");
        match &self.action {
            TriggerAction::InlineSql { statements } => {
                for stmt in statements {
                    sql.push_str("    ");
                    sql.push_str(stmt);
                    sql.push_str(";\n");
                }
            }
            TriggerAction::ExecuteFunction { .. } | TriggerAction::FunctionReference { .. } => {
                return Err(QueryError::unsupported(
                    "SQLite triggers require inline SQL, not function calls",
                ));
            }
        }
        sql.push_str("END;");

        Ok(sql)
    }

    /// Generate MSSQL CREATE TRIGGER SQL.
    pub fn to_mssql_sql(&self) -> QueryResult<String> {
        // MSSQL doesn't support BEFORE triggers
        if self.timing == TriggerTiming::Before {
            return Err(QueryError::unsupported(
                "SQL Server does not support BEFORE triggers. Use INSTEAD OF or AFTER triggers.",
            ));
        }

        let mut sql = String::with_capacity(256);

        sql.push_str("CREATE TRIGGER ");
        sql.push_str(&self.qualified_name());
        sql.push('\n');

        // Table
        sql.push_str("ON ");
        sql.push_str(&self.qualified_table());
        sql.push('\n');

        // Timing
        sql.push_str(self.timing.to_sql());
        sql.push(' ');

        // Events
        let events: Vec<_> = self.events.iter().collect();
        for (i, event) in events.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(event.to_sql());
        }
        sql.push('\n');

        // AS clause
        sql.push_str("AS\n");
        sql.push_str("BEGIN\n");
        sql.push_str("    SET NOCOUNT ON;\n");

        // Action
        match &self.action {
            TriggerAction::InlineSql { statements } => {
                for stmt in statements {
                    sql.push_str("    ");
                    sql.push_str(stmt);
                    sql.push_str(";\n");
                }
            }
            TriggerAction::ExecuteFunction { name, args } => {
                sql.push_str("    EXEC ");
                sql.push_str(name);
                if !args.is_empty() {
                    sql.push(' ');
                    sql.push_str(&args.join(", "));
                }
                sql.push_str(";\n");
            }
            TriggerAction::FunctionReference { name } => {
                sql.push_str("    EXEC ");
                sql.push_str(name);
                sql.push_str(";\n");
            }
        }

        sql.push_str("END;");

        Ok(sql)
    }

    /// Generate SQL for the configured database type.
    pub fn to_sql(&self, db_type: DatabaseType) -> QueryResult<String> {
        match db_type {
            DatabaseType::PostgreSQL => self.to_postgres_sql(),
            DatabaseType::MySQL => self.to_mysql_sql(),
            DatabaseType::SQLite => self.to_sqlite_sql(),
            DatabaseType::MSSQL => self.to_mssql_sql(),
        }
    }

    /// Generate DROP TRIGGER SQL.
    pub fn drop_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                format!(
                    "DROP TRIGGER IF EXISTS {} ON {};",
                    self.name,
                    self.qualified_table()
                )
            }
            DatabaseType::MySQL => {
                format!("DROP TRIGGER IF EXISTS {};", self.name)
            }
            DatabaseType::SQLite => {
                format!("DROP TRIGGER IF EXISTS {};", self.name)
            }
            DatabaseType::MSSQL => {
                format!("DROP TRIGGER IF EXISTS {};", self.qualified_name())
            }
        }
    }
}

/// Builder for creating triggers.
#[derive(Debug, Clone)]
pub struct TriggerBuilder {
    name: String,
    schema: Option<String>,
    table: Option<String>,
    timing: TriggerTiming,
    events: HashSet<TriggerEvent>,
    level: TriggerLevel,
    update_of: Option<UpdateOf>,
    condition: Option<TriggerCondition>,
    action: Option<TriggerAction>,
    enabled: bool,
    comment: Option<String>,
}

impl TriggerBuilder {
    /// Create a new trigger builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            table: None,
            timing: TriggerTiming::After,
            events: HashSet::new(),
            level: TriggerLevel::Row,
            update_of: None,
            condition: None,
            action: None,
            enabled: true,
            comment: None,
        }
    }

    /// Set the schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set the table/view the trigger is on.
    pub fn on_table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Alias for on_table.
    pub fn on_view(self, view: impl Into<String>) -> Self {
        self.on_table(view)
    }

    /// Set the trigger timing.
    pub fn timing(mut self, timing: TriggerTiming) -> Self {
        self.timing = timing;
        self
    }

    /// Set timing to BEFORE.
    pub fn before(self) -> Self {
        self.timing(TriggerTiming::Before)
    }

    /// Set timing to AFTER.
    pub fn after(self) -> Self {
        self.timing(TriggerTiming::After)
    }

    /// Set timing to INSTEAD OF.
    pub fn instead_of(self) -> Self {
        self.timing(TriggerTiming::InsteadOf)
    }

    /// Add a triggering event.
    pub fn event(mut self, event: TriggerEvent) -> Self {
        self.events.insert(event);
        self
    }

    /// Add multiple triggering events.
    pub fn events(mut self, events: impl IntoIterator<Item = TriggerEvent>) -> Self {
        self.events.extend(events);
        self
    }

    /// Trigger on INSERT.
    pub fn on_insert(self) -> Self {
        self.event(TriggerEvent::Insert)
    }

    /// Trigger on UPDATE.
    pub fn on_update(self) -> Self {
        self.event(TriggerEvent::Update)
    }

    /// Trigger on DELETE.
    pub fn on_delete(self) -> Self {
        self.event(TriggerEvent::Delete)
    }

    /// Trigger on TRUNCATE (PostgreSQL only).
    pub fn on_truncate(self) -> Self {
        self.event(TriggerEvent::Truncate)
    }

    /// Set the trigger level.
    pub fn level(mut self, level: TriggerLevel) -> Self {
        self.level = level;
        self
    }

    /// Set to row-level trigger.
    pub fn for_each_row(self) -> Self {
        self.level(TriggerLevel::Row)
    }

    /// Set to statement-level trigger.
    pub fn for_each_statement(self) -> Self {
        self.level(TriggerLevel::Statement)
    }

    /// Specify columns for UPDATE OF.
    pub fn update_of(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.update_of = Some(UpdateOf::new(columns));
        self
    }

    /// Add a WHEN condition.
    pub fn when(mut self, condition: TriggerCondition) -> Self {
        self.condition = Some(condition);
        self
    }

    /// Add a WHEN condition from a raw expression.
    pub fn when_expr(self, expression: impl Into<String>) -> Self {
        self.when(TriggerCondition::new(expression))
    }

    /// Set the action to execute a function.
    pub fn execute_function(mut self, name: impl Into<String>) -> Self {
        self.action = Some(TriggerAction::function(name));
        self
    }

    /// Set the action to execute a function with arguments.
    pub fn execute_function_with_args(
        mut self,
        name: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.action = Some(TriggerAction::function_with_args(name, args));
        self
    }

    /// Set the action to inline SQL.
    pub fn execute_sql(mut self, statements: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.action = Some(TriggerAction::inline_sql(statements));
        self
    }

    /// Set whether the trigger is enabled.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Add a comment/description.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Build the trigger.
    pub fn build(self) -> QueryResult<Trigger> {
        let table = self.table.ok_or_else(|| {
            QueryError::invalid_input("table", "Trigger must specify a table with on_table()")
        })?;

        if self.events.is_empty() {
            return Err(QueryError::invalid_input(
                "events",
                "Trigger must have at least one event (on_insert, on_update, on_delete)",
            ));
        }

        let action = self.action.ok_or_else(|| {
            QueryError::invalid_input(
                "action",
                "Trigger must have an action (execute_function or execute_sql)",
            )
        })?;

        Ok(Trigger {
            name: self.name,
            schema: self.schema,
            table,
            timing: self.timing,
            events: self.events,
            level: self.level,
            update_of: self.update_of,
            condition: self.condition,
            action,
            enabled: self.enabled,
            comment: self.comment,
        })
    }
}

/// Pre-built trigger patterns for common use cases.
pub mod patterns {
    use super::*;

    /// Create an audit log trigger that records changes to a table.
    ///
    /// # PostgreSQL Example
    ///
    /// ```rust,ignore
    /// let trigger = patterns::audit_trigger("users", "audit_log", &["UPDATE", "DELETE"]);
    /// ```
    pub fn audit_trigger(
        table: &str,
        audit_table: &str,
        events: impl IntoIterator<Item = TriggerEvent>,
    ) -> TriggerBuilder {
        // Note: The actual audit logic is handled by the audit_trigger_func function
        // which receives OLD and NEW row data and records changes to the audit table.
        let _ = audit_table; // Used for documentation purposes

        Trigger::builder(format!("{}_audit_trigger", table))
            .on_table(table)
            .after()
            .events(events)
            .for_each_row()
            .execute_function("audit_trigger_func")
    }

    /// Create a soft delete trigger that sets deleted_at instead of deleting.
    pub fn soft_delete_trigger(table: &str, deleted_at_column: &str) -> TriggerBuilder {
        Trigger::builder(format!("{}_soft_delete", table))
            .on_table(table)
            .instead_of()
            .on_delete()
            .for_each_row()
            .execute_sql([format!(
                "UPDATE {} SET {} = NOW() WHERE id = OLD.id",
                table, deleted_at_column
            )])
    }

    /// Create a timestamp update trigger for updated_at column.
    pub fn updated_at_trigger(table: &str, column: &str) -> TriggerBuilder {
        Trigger::builder(format!("{}_updated_at", table))
            .on_table(table)
            .before()
            .on_update()
            .for_each_row()
            .execute_sql([format!("NEW.{} = NOW()", column)])
    }

    /// Create a validation trigger that prevents certain operations.
    pub fn validation_trigger(
        table: &str,
        name: &str,
        condition: &str,
        error_message: &str,
    ) -> TriggerBuilder {
        Trigger::builder(name)
            .on_table(table)
            .before()
            .on_insert()
            .on_update()
            .for_each_row()
            .when_expr(condition)
            .execute_sql([format!("RAISE EXCEPTION '{}'", error_message)])
    }
}

/// MongoDB Change Stream support.
pub mod mongodb {
    use super::*;

    /// The type of change in a Change Stream event.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum ChangeType {
        /// Document was inserted.
        Insert,
        /// Document was updated.
        Update,
        /// Document was replaced.
        Replace,
        /// Document was deleted.
        Delete,
        /// Collection was dropped.
        Drop,
        /// Collection was renamed.
        Rename,
        /// Database was dropped.
        DropDatabase,
        /// Operation was invalidated.
        Invalidate,
    }

    impl ChangeType {
        /// Get the MongoDB operation type string.
        pub fn as_str(&self) -> &'static str {
            match self {
                Self::Insert => "insert",
                Self::Update => "update",
                Self::Replace => "replace",
                Self::Delete => "delete",
                Self::Drop => "drop",
                Self::Rename => "rename",
                Self::DropDatabase => "dropDatabase",
                Self::Invalidate => "invalidate",
            }
        }
    }

    /// Options for a Change Stream.
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct ChangeStreamOptions {
        /// Only receive changes after this token.
        pub resume_after: Option<String>,
        /// Only receive changes after this timestamp.
        pub start_at_operation_time: Option<String>,
        /// Whether to return full document on update.
        pub full_document: FullDocument,
        /// Whether to return full document before the change.
        pub full_document_before_change: FullDocumentBeforeChange,
        /// Maximum time to wait for new changes.
        pub max_await_time_ms: Option<u64>,
        /// Batch size for results.
        pub batch_size: Option<u32>,
    }

    /// Full document return policy for updates.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub enum FullDocument {
        /// Don't return full document.
        #[default]
        Default,
        /// Return the full document after the change.
        UpdateLookup,
        /// Return the full document if available.
        WhenAvailable,
        /// Require the full document.
        Required,
    }

    impl FullDocument {
        /// Get the MongoDB option string.
        pub fn as_str(&self) -> &'static str {
            match self {
                Self::Default => "default",
                Self::UpdateLookup => "updateLookup",
                Self::WhenAvailable => "whenAvailable",
                Self::Required => "required",
            }
        }
    }

    /// Full document before change return policy.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub enum FullDocumentBeforeChange {
        /// Don't return document before change.
        #[default]
        Off,
        /// Return document before change if available.
        WhenAvailable,
        /// Require document before change.
        Required,
    }

    impl FullDocumentBeforeChange {
        /// Get the MongoDB option string.
        pub fn as_str(&self) -> &'static str {
            match self {
                Self::Off => "off",
                Self::WhenAvailable => "whenAvailable",
                Self::Required => "required",
            }
        }
    }

    /// A pipeline stage for filtering Change Stream events.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChangeStreamPipeline {
        /// Pipeline stages.
        pub stages: Vec<PipelineStage>,
    }

    impl ChangeStreamPipeline {
        /// Create a new empty pipeline.
        pub fn new() -> Self {
            Self { stages: Vec::new() }
        }

        /// Add a $match stage to filter events.
        pub fn match_stage(mut self, filter: serde_json::Value) -> Self {
            self.stages.push(PipelineStage::Match(filter));
            self
        }

        /// Filter by operation type(s).
        pub fn operation_types(self, types: &[ChangeType]) -> Self {
            let type_strs: Vec<_> = types.iter().map(|t| t.as_str()).collect();
            self.match_stage(serde_json::json!({
                "operationType": { "$in": type_strs }
            }))
        }

        /// Filter by namespace (database.collection).
        pub fn namespace(self, db: &str, collection: &str) -> Self {
            self.match_stage(serde_json::json!({
                "ns": { "db": db, "coll": collection }
            }))
        }

        /// Add a $project stage.
        pub fn project(mut self, projection: serde_json::Value) -> Self {
            self.stages.push(PipelineStage::Project(projection));
            self
        }
    }

    impl Default for ChangeStreamPipeline {
        fn default() -> Self {
            Self::new()
        }
    }

    /// A pipeline stage.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum PipelineStage {
        /// $match stage.
        Match(serde_json::Value),
        /// $project stage.
        Project(serde_json::Value),
        /// $addFields stage.
        AddFields(serde_json::Value),
        /// $replaceRoot stage.
        ReplaceRoot(serde_json::Value),
        /// $redact stage.
        Redact(serde_json::Value),
    }

    /// Builder for Change Stream configuration.
    #[derive(Debug, Clone, Default)]
    pub struct ChangeStreamBuilder {
        collection: Option<String>,
        database: Option<String>,
        pipeline: ChangeStreamPipeline,
        options: ChangeStreamOptions,
    }

    impl ChangeStreamBuilder {
        /// Create a new Change Stream builder.
        pub fn new() -> Self {
            Self::default()
        }

        /// Watch a specific collection.
        pub fn collection(mut self, name: impl Into<String>) -> Self {
            self.collection = Some(name.into());
            self
        }

        /// Watch a specific database.
        pub fn database(mut self, name: impl Into<String>) -> Self {
            self.database = Some(name.into());
            self
        }

        /// Filter by operation types.
        pub fn operations(mut self, types: &[ChangeType]) -> Self {
            self.pipeline = self.pipeline.operation_types(types);
            self
        }

        /// Add a custom match filter.
        pub fn filter(mut self, filter: serde_json::Value) -> Self {
            self.pipeline = self.pipeline.match_stage(filter);
            self
        }

        /// Request full document on updates.
        pub fn full_document(mut self, policy: FullDocument) -> Self {
            self.options.full_document = policy;
            self
        }

        /// Request full document before change.
        pub fn full_document_before_change(mut self, policy: FullDocumentBeforeChange) -> Self {
            self.options.full_document_before_change = policy;
            self
        }

        /// Resume from a specific token.
        pub fn resume_after(mut self, token: impl Into<String>) -> Self {
            self.options.resume_after = Some(token.into());
            self
        }

        /// Set maximum await time.
        pub fn max_await_time_ms(mut self, ms: u64) -> Self {
            self.options.max_await_time_ms = Some(ms);
            self
        }

        /// Set batch size.
        pub fn batch_size(mut self, size: u32) -> Self {
            self.options.batch_size = Some(size);
            self
        }

        /// Get the pipeline stages.
        pub fn build_pipeline(&self) -> &[PipelineStage] {
            &self.pipeline.stages
        }

        /// Get the options.
        pub fn build_options(&self) -> &ChangeStreamOptions {
            &self.options
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_builder() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .after()
            .on_update()
            .on_delete()
            .for_each_row()
            .execute_function("audit_log_changes")
            .build()
            .unwrap();

        assert_eq!(trigger.name, "audit_users");
        assert_eq!(trigger.table, "users");
        assert_eq!(trigger.timing, TriggerTiming::After);
        assert!(trigger.events.contains(&TriggerEvent::Update));
        assert!(trigger.events.contains(&TriggerEvent::Delete));
        assert_eq!(trigger.level, TriggerLevel::Row);
    }

    #[test]
    fn test_postgres_trigger_sql() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .after()
            .on_insert()
            .on_update()
            .for_each_row()
            .execute_function("audit_func")
            .build()
            .unwrap();

        let sql = trigger.to_postgres_sql().unwrap();
        assert!(sql.contains("CREATE TRIGGER audit_users"));
        assert!(sql.contains("AFTER"));
        assert!(sql.contains("ON users"));
        assert!(sql.contains("FOR EACH ROW"));
        assert!(sql.contains("EXECUTE FUNCTION audit_func()"));
    }

    #[test]
    fn test_mysql_trigger_sql() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .after()
            .on_insert()
            .for_each_row()
            .execute_sql(["INSERT INTO audit_log VALUES (NEW.id, 'INSERT')"])
            .build()
            .unwrap();

        let sql = trigger.to_mysql_sql().unwrap();
        assert!(sql.contains("CREATE TRIGGER audit_users"));
        assert!(sql.contains("AFTER INSERT"));
        assert!(sql.contains("ON `users`"));
        assert!(sql.contains("FOR EACH ROW"));
    }

    #[test]
    fn test_mysql_multiple_events_error() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .after()
            .on_insert()
            .on_update()
            .execute_sql(["SELECT 1"])
            .build()
            .unwrap();

        let result = trigger.to_mysql_sql();
        assert!(result.is_err());
    }

    #[test]
    fn test_sqlite_trigger_sql() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .after()
            .on_delete()
            .for_each_row()
            .when_expr("OLD.important = 1")
            .execute_sql(["INSERT INTO deleted_users SELECT * FROM OLD"])
            .build()
            .unwrap();

        let sql = trigger.to_sqlite_sql().unwrap();
        assert!(sql.contains("CREATE TRIGGER audit_users"));
        assert!(sql.contains("AFTER DELETE"));
        assert!(sql.contains("ON `users`"));
        assert!(sql.contains("WHEN OLD.important = 1"));
        assert!(sql.contains("BEGIN"));
        assert!(sql.contains("END;"));
    }

    #[test]
    fn test_mssql_trigger_sql() {
        let trigger = Trigger::builder("audit_users")
            .schema("dbo")
            .on_table("users")
            .after()
            .on_insert()
            .on_update()
            .execute_sql(["INSERT INTO audit_log SELECT * FROM inserted"])
            .build()
            .unwrap();

        let sql = trigger.to_mssql_sql().unwrap();
        assert!(sql.contains("CREATE TRIGGER dbo.audit_users"));
        assert!(sql.contains("ON dbo.users"));
        assert!(sql.contains("AFTER INSERT, UPDATE") || sql.contains("AFTER UPDATE, INSERT"));
        assert!(sql.contains("SET NOCOUNT ON"));
    }

    #[test]
    fn test_mssql_before_error() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .before()
            .on_insert()
            .execute_sql(["SELECT 1"])
            .build()
            .unwrap();

        let result = trigger.to_mssql_sql();
        assert!(result.is_err());
    }

    #[test]
    fn test_drop_trigger_sql() {
        let trigger = Trigger::builder("audit_users")
            .on_table("users")
            .after()
            .on_insert()
            .execute_function("audit_func")
            .build()
            .unwrap();

        let pg_drop = trigger.drop_sql(DatabaseType::PostgreSQL);
        assert_eq!(pg_drop, "DROP TRIGGER IF EXISTS audit_users ON users;");

        let mysql_drop = trigger.drop_sql(DatabaseType::MySQL);
        assert_eq!(mysql_drop, "DROP TRIGGER IF EXISTS audit_users;");
    }

    #[test]
    fn test_trigger_condition() {
        let cond = TriggerCondition::column_changed("email")
            .and(TriggerCondition::new_not_null("verified"));

        assert!(
            cond.expression
                .contains("OLD.email IS DISTINCT FROM NEW.email")
        );
        assert!(cond.expression.contains("NEW.verified IS NOT NULL"));
    }

    #[test]
    fn test_update_of() {
        let update_of = UpdateOf::new(["email", "password"]);
        assert_eq!(update_of.to_sql(), " OF email, password");
    }

    #[test]
    fn test_trigger_with_update_of() {
        let trigger = Trigger::builder("sensitive_update")
            .on_table("users")
            .before()
            .on_update()
            .update_of(["email", "password"])
            .execute_function("validate_sensitive_update")
            .build()
            .unwrap();

        let sql = trigger.to_postgres_sql().unwrap();
        assert!(sql.contains("UPDATE OF email, password"));
    }

    #[test]
    fn test_instead_of_trigger() {
        let trigger = Trigger::builder("view_insert")
            .on_view("user_view")
            .instead_of()
            .on_insert()
            .execute_function("handle_view_insert")
            .build()
            .unwrap();

        let sql = trigger.to_postgres_sql().unwrap();
        assert!(sql.contains("INSTEAD OF INSERT"));
    }

    #[test]
    fn test_missing_table_error() {
        let result = Trigger::builder("test")
            .on_insert()
            .execute_function("func")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_missing_events_error() {
        let result = Trigger::builder("test")
            .on_table("users")
            .execute_function("func")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_missing_action_error() {
        let result = Trigger::builder("test")
            .on_table("users")
            .on_insert()
            .build();

        assert!(result.is_err());
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_change_stream_builder() {
            let builder = ChangeStreamBuilder::new()
                .collection("users")
                .operations(&[ChangeType::Insert, ChangeType::Update])
                .full_document(FullDocument::UpdateLookup)
                .batch_size(100);

            assert_eq!(
                builder.build_options().full_document,
                FullDocument::UpdateLookup
            );
            assert_eq!(builder.build_options().batch_size, Some(100));
        }

        #[test]
        fn test_change_type() {
            assert_eq!(ChangeType::Insert.as_str(), "insert");
            assert_eq!(ChangeType::Update.as_str(), "update");
            assert_eq!(ChangeType::Delete.as_str(), "delete");
        }

        #[test]
        fn test_full_document_options() {
            assert_eq!(FullDocument::Default.as_str(), "default");
            assert_eq!(FullDocument::UpdateLookup.as_str(), "updateLookup");
            assert_eq!(FullDocumentBeforeChange::Required.as_str(), "required");
        }
    }
}
