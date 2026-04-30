//! Procedure migrations - Version control for stored procedures, functions, and triggers.
//!
//! This module provides functionality to manage database procedures through migrations:
//! - Track procedure definitions in the schema
//! - Detect changes between schema and database
//! - Generate CREATE/ALTER/DROP statements
//! - Support for PostgreSQL, MySQL, SQLite (UDFs), and MSSQL
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_migrate::procedure::{ProcedureDiff, ProcedureMigration};
//!
//! // Define a procedure in schema
//! let proc = ProcedureDefinition::new("calculate_tax")
//!     .language(ProcedureLanguage::PlPgSql)
//!     .parameters(vec![
//!         Parameter::new("amount", "DECIMAL"),
//!         Parameter::new("rate", "DECIMAL"),
//!     ])
//!     .returns("DECIMAL")
//!     .body("RETURN amount * rate;");
//!
//! // Generate migration
//! let migration = ProcedureMigration::create(&proc);
//! println!("{}", migration.up_sql());
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ============================================================================
// Procedure Definition
// ============================================================================

/// A stored procedure or function definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureDefinition {
    /// Procedure name.
    pub name: String,
    /// Schema/namespace.
    pub schema: Option<String>,
    /// Whether this is a function (returns value) or procedure.
    pub is_function: bool,
    /// Parameters.
    pub parameters: Vec<ProcedureParameter>,
    /// Return type (for functions).
    pub return_type: Option<String>,
    /// Returns a set/table (SETOF, TABLE).
    pub returns_set: bool,
    /// Table columns for table-returning functions.
    pub return_columns: Vec<ReturnColumn>,
    /// Procedure language.
    pub language: ProcedureLanguage,
    /// Procedure body.
    pub body: String,
    /// Volatility (VOLATILE, STABLE, IMMUTABLE).
    pub volatility: Volatility,
    /// Security definer (runs as owner vs caller).
    pub security_definer: bool,
    /// Cost estimate.
    pub cost: Option<i32>,
    /// Rows estimate (for set-returning functions).
    pub rows: Option<i32>,
    /// Parallel safety.
    pub parallel: ParallelSafety,
    /// Whether to replace if exists.
    pub or_replace: bool,
    /// Comment/description.
    pub comment: Option<String>,
    /// Checksum of the body for change detection.
    pub checksum: Option<String>,
    /// Version number for manual versioning.
    pub version: Option<i32>,
}

impl Default for ProcedureDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            schema: None,
            is_function: true,
            parameters: Vec::new(),
            return_type: None,
            returns_set: false,
            return_columns: Vec::new(),
            language: ProcedureLanguage::Sql,
            body: String::new(),
            volatility: Volatility::Volatile,
            security_definer: false,
            cost: None,
            rows: None,
            parallel: ParallelSafety::Unsafe,
            or_replace: true,
            comment: None,
            checksum: None,
            version: None,
        }
    }
}

impl ProcedureDefinition {
    /// Create a new procedure definition.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Create a new function definition.
    pub fn function(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_function: true,
            ..Default::default()
        }
    }

    /// Create a new stored procedure definition.
    pub fn procedure(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_function: false,
            ..Default::default()
        }
    }

    /// Set the schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Add a parameter.
    pub fn param(mut self, name: impl Into<String>, data_type: impl Into<String>) -> Self {
        self.parameters.push(ProcedureParameter {
            name: name.into(),
            data_type: data_type.into(),
            mode: ParameterMode::In,
            default: None,
        });
        self
    }

    /// Add an OUT parameter.
    pub fn out_param(mut self, name: impl Into<String>, data_type: impl Into<String>) -> Self {
        self.parameters.push(ProcedureParameter {
            name: name.into(),
            data_type: data_type.into(),
            mode: ParameterMode::Out,
            default: None,
        });
        self
    }

    /// Add an INOUT parameter.
    pub fn inout_param(mut self, name: impl Into<String>, data_type: impl Into<String>) -> Self {
        self.parameters.push(ProcedureParameter {
            name: name.into(),
            data_type: data_type.into(),
            mode: ParameterMode::InOut,
            default: None,
        });
        self
    }

    /// Set return type.
    pub fn returns(mut self, return_type: impl Into<String>) -> Self {
        self.return_type = Some(return_type.into());
        self
    }

    /// Set returns SETOF type.
    pub fn returns_setof(mut self, return_type: impl Into<String>) -> Self {
        self.return_type = Some(return_type.into());
        self.returns_set = true;
        self
    }

    /// Set returns TABLE.
    pub fn returns_table(mut self, columns: Vec<ReturnColumn>) -> Self {
        self.returns_set = true;
        self.return_columns = columns;
        self
    }

    /// Set the language.
    pub fn language(mut self, language: ProcedureLanguage) -> Self {
        self.language = language;
        self
    }

    /// Set the body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self.update_checksum();
        self
    }

    /// Set volatility.
    pub fn volatility(mut self, volatility: Volatility) -> Self {
        self.volatility = volatility;
        self
    }

    /// Mark as IMMUTABLE.
    pub fn immutable(mut self) -> Self {
        self.volatility = Volatility::Immutable;
        self
    }

    /// Mark as STABLE.
    pub fn stable(mut self) -> Self {
        self.volatility = Volatility::Stable;
        self
    }

    /// Mark as security definer.
    pub fn security_definer(mut self) -> Self {
        self.security_definer = true;
        self
    }

    /// Set cost.
    pub fn cost(mut self, cost: i32) -> Self {
        self.cost = Some(cost);
        self
    }

    /// Set parallel safety.
    pub fn parallel(mut self, parallel: ParallelSafety) -> Self {
        self.parallel = parallel;
        self
    }

    /// Set comment.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Update the checksum based on the body.
    fn update_checksum(&mut self) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.body.hash(&mut hasher);
        self.checksum = Some(format!("{:016x}", hasher.finish()));
    }

    /// Get the fully qualified name.
    pub fn qualified_name(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", schema, self.name),
            None => self.name.clone(),
        }
    }

    /// Check if the procedure has changed compared to another.
    pub fn has_changed(&self, other: &ProcedureDefinition) -> bool {
        // Compare checksums if available
        if let (Some(a), Some(b)) = (&self.checksum, &other.checksum)
            && a != b
        {
            return true;
        }

        // Compare key properties
        self.body != other.body
            || self.parameters != other.parameters
            || self.return_type != other.return_type
            || self.returns_set != other.returns_set
            || self.language != other.language
            || self.volatility != other.volatility
            || self.security_definer != other.security_definer
    }
}

/// Procedure parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureParameter {
    /// Parameter name.
    pub name: String,
    /// Data type.
    pub data_type: String,
    /// Parameter mode.
    pub mode: ParameterMode,
    /// Default value.
    pub default: Option<String>,
}

/// Parameter mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ParameterMode {
    #[default]
    In,
    Out,
    InOut,
    Variadic,
}

/// Return column for table-returning functions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnColumn {
    /// Column name.
    pub name: String,
    /// Data type.
    pub data_type: String,
}

impl ReturnColumn {
    /// Create a new return column.
    pub fn new(name: impl Into<String>, data_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data_type: data_type.into(),
        }
    }
}

/// Procedure language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ProcedureLanguage {
    #[default]
    Sql,
    PlPgSql,
    PlPython,
    PlPerl,
    PlTcl,
    PlV8,
    C,
}

impl ProcedureLanguage {
    /// Get SQL language name.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Sql => "SQL",
            Self::PlPgSql => "plpgsql",
            Self::PlPython => "plpython3u",
            Self::PlPerl => "plperl",
            Self::PlTcl => "pltcl",
            Self::PlV8 => "plv8",
            Self::C => "C",
        }
    }
}

/// Function volatility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Volatility {
    #[default]
    Volatile,
    Stable,
    Immutable,
}

impl Volatility {
    /// Get SQL volatility string.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Volatile => "VOLATILE",
            Self::Stable => "STABLE",
            Self::Immutable => "IMMUTABLE",
        }
    }
}

/// Parallel safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ParallelSafety {
    #[default]
    Unsafe,
    Restricted,
    Safe,
}

impl ParallelSafety {
    /// Get SQL parallel string.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Unsafe => "PARALLEL UNSAFE",
            Self::Restricted => "PARALLEL RESTRICTED",
            Self::Safe => "PARALLEL SAFE",
        }
    }
}

// ============================================================================
// Trigger Definition
// ============================================================================

/// A database trigger definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDefinition {
    /// Trigger name.
    pub name: String,
    /// Schema/namespace.
    pub schema: Option<String>,
    /// Table the trigger is on.
    pub table: String,
    /// Trigger timing.
    pub timing: TriggerTiming,
    /// Events that fire the trigger.
    pub events: Vec<TriggerEvent>,
    /// Row or statement level.
    pub level: TriggerLevel,
    /// WHEN condition.
    pub condition: Option<String>,
    /// Function to execute.
    pub function: String,
    /// Function arguments.
    pub function_args: Vec<String>,
    /// Whether to replace if exists.
    pub or_replace: bool,
    /// Comment/description.
    pub comment: Option<String>,
    /// Checksum.
    pub checksum: Option<String>,
}

impl Default for TriggerDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            schema: None,
            table: String::new(),
            timing: TriggerTiming::Before,
            events: vec![TriggerEvent::Insert],
            level: TriggerLevel::Row,
            condition: None,
            function: String::new(),
            function_args: Vec::new(),
            or_replace: true,
            comment: None,
            checksum: None,
        }
    }
}

impl TriggerDefinition {
    /// Create a new trigger definition.
    pub fn new(name: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            table: table.into(),
            ..Default::default()
        }
    }

    /// Set timing to BEFORE.
    pub fn before(mut self) -> Self {
        self.timing = TriggerTiming::Before;
        self
    }

    /// Set timing to AFTER.
    pub fn after(mut self) -> Self {
        self.timing = TriggerTiming::After;
        self
    }

    /// Set timing to INSTEAD OF.
    pub fn instead_of(mut self) -> Self {
        self.timing = TriggerTiming::InsteadOf;
        self
    }

    /// Set events.
    pub fn on(mut self, events: Vec<TriggerEvent>) -> Self {
        self.events = events;
        self
    }

    /// Set as row-level trigger.
    pub fn for_each_row(mut self) -> Self {
        self.level = TriggerLevel::Row;
        self
    }

    /// Set as statement-level trigger.
    pub fn for_each_statement(mut self) -> Self {
        self.level = TriggerLevel::Statement;
        self
    }

    /// Set WHEN condition.
    pub fn when(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    /// Set the function to execute.
    pub fn execute(mut self, function: impl Into<String>) -> Self {
        self.function = function.into();
        self
    }

    /// Get the fully qualified name.
    pub fn qualified_name(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", schema, self.name),
            None => self.name.clone(),
        }
    }
}

/// Trigger timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TriggerTiming {
    #[default]
    Before,
    After,
    InsteadOf,
}

impl TriggerTiming {
    /// Get SQL timing string.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Before => "BEFORE",
            Self::After => "AFTER",
            Self::InsteadOf => "INSTEAD OF",
        }
    }
}

/// Trigger event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
    Truncate,
}

impl TriggerEvent {
    /// Get SQL event string.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
        }
    }
}

/// Trigger level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TriggerLevel {
    #[default]
    Row,
    Statement,
}

impl TriggerLevel {
    /// Get SQL level string.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Row => "FOR EACH ROW",
            Self::Statement => "FOR EACH STATEMENT",
        }
    }
}

// ============================================================================
// Procedure Diff
// ============================================================================

/// Differences in procedures between two states.
#[derive(Debug, Clone, Default)]
pub struct ProcedureDiff {
    /// Procedures to create.
    pub create: Vec<ProcedureDefinition>,
    /// Procedures to drop.
    pub drop: Vec<String>,
    /// Procedures to alter (replace).
    pub alter: Vec<ProcedureAlterDiff>,
    /// Triggers to create.
    pub create_triggers: Vec<TriggerDefinition>,
    /// Triggers to drop.
    pub drop_triggers: Vec<String>,
    /// Triggers to alter.
    pub alter_triggers: Vec<TriggerAlterDiff>,
}

/// A procedure alter diff.
#[derive(Debug, Clone)]
pub struct ProcedureAlterDiff {
    /// Old procedure definition.
    pub old: ProcedureDefinition,
    /// New procedure definition.
    pub new: ProcedureDefinition,
    /// What changed.
    pub changes: Vec<ProcedureChange>,
}

/// What changed in a procedure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcedureChange {
    Body,
    Parameters,
    ReturnType,
    Language,
    Volatility,
    SecurityDefiner,
    Cost,
    Parallel,
}

/// A trigger alter diff.
#[derive(Debug, Clone)]
pub struct TriggerAlterDiff {
    /// Old trigger definition.
    pub old: TriggerDefinition,
    /// New trigger definition.
    pub new: TriggerDefinition,
}

impl ProcedureDiff {
    /// Check if there are any differences.
    pub fn is_empty(&self) -> bool {
        self.create.is_empty()
            && self.drop.is_empty()
            && self.alter.is_empty()
            && self.create_triggers.is_empty()
            && self.drop_triggers.is_empty()
            && self.alter_triggers.is_empty()
    }

    /// Get a summary of the diff.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if !self.create.is_empty() {
            parts.push(format!("Create {} procedures", self.create.len()));
        }
        if !self.drop.is_empty() {
            parts.push(format!("Drop {} procedures", self.drop.len()));
        }
        if !self.alter.is_empty() {
            parts.push(format!("Alter {} procedures", self.alter.len()));
        }
        if !self.create_triggers.is_empty() {
            parts.push(format!("Create {} triggers", self.create_triggers.len()));
        }
        if !self.drop_triggers.is_empty() {
            parts.push(format!("Drop {} triggers", self.drop_triggers.len()));
        }
        if !self.alter_triggers.is_empty() {
            parts.push(format!("Alter {} triggers", self.alter_triggers.len()));
        }

        if parts.is_empty() {
            "No changes".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Differ for procedures.
pub struct ProcedureDiffer;

impl ProcedureDiffer {
    /// Compute the diff between two sets of procedures.
    pub fn diff(from: &[ProcedureDefinition], to: &[ProcedureDefinition]) -> ProcedureDiff {
        let mut diff = ProcedureDiff::default();

        let from_map: HashMap<_, _> = from.iter().map(|p| (p.qualified_name(), p)).collect();
        let to_map: HashMap<_, _> = to.iter().map(|p| (p.qualified_name(), p)).collect();

        // Find procedures to create
        for (name, proc) in &to_map {
            if !from_map.contains_key(name) {
                diff.create.push((*proc).clone());
            }
        }

        // Find procedures to drop
        for name in from_map.keys() {
            if !to_map.contains_key(name) {
                diff.drop.push(name.clone());
            }
        }

        // Find procedures to alter
        for (name, new_proc) in &to_map {
            if let Some(old_proc) = from_map.get(name)
                && old_proc.has_changed(new_proc)
            {
                let changes = detect_procedure_changes(old_proc, new_proc);
                diff.alter.push(ProcedureAlterDiff {
                    old: (*old_proc).clone(),
                    new: (*new_proc).clone(),
                    changes,
                });
            }
        }

        diff
    }

    /// Compute trigger diff.
    pub fn diff_triggers(from: &[TriggerDefinition], to: &[TriggerDefinition]) -> ProcedureDiff {
        let mut diff = ProcedureDiff::default();

        let from_map: HashMap<_, _> = from.iter().map(|t| (t.qualified_name(), t)).collect();
        let to_map: HashMap<_, _> = to.iter().map(|t| (t.qualified_name(), t)).collect();

        // Find triggers to create
        for (name, trigger) in &to_map {
            if !from_map.contains_key(name) {
                diff.create_triggers.push((*trigger).clone());
            }
        }

        // Find triggers to drop
        for name in from_map.keys() {
            if !to_map.contains_key(name) {
                diff.drop_triggers.push(name.clone());
            }
        }

        // Find triggers to alter
        for (name, new_trigger) in &to_map {
            if let Some(old_trigger) = from_map.get(name)
                && old_trigger != new_trigger
            {
                diff.alter_triggers.push(TriggerAlterDiff {
                    old: (*old_trigger).clone(),
                    new: (*new_trigger).clone(),
                });
            }
        }

        diff
    }
}

fn detect_procedure_changes(
    old: &ProcedureDefinition,
    new: &ProcedureDefinition,
) -> Vec<ProcedureChange> {
    let mut changes = Vec::new();

    if old.body != new.body {
        changes.push(ProcedureChange::Body);
    }
    if old.parameters != new.parameters {
        changes.push(ProcedureChange::Parameters);
    }
    if old.return_type != new.return_type || old.returns_set != new.returns_set {
        changes.push(ProcedureChange::ReturnType);
    }
    if old.language != new.language {
        changes.push(ProcedureChange::Language);
    }
    if old.volatility != new.volatility {
        changes.push(ProcedureChange::Volatility);
    }
    if old.security_definer != new.security_definer {
        changes.push(ProcedureChange::SecurityDefiner);
    }
    if old.cost != new.cost {
        changes.push(ProcedureChange::Cost);
    }
    if old.parallel != new.parallel {
        changes.push(ProcedureChange::Parallel);
    }

    changes
}

// ============================================================================
// SQL Generation
// ============================================================================

/// Generate SQL for procedure migrations.
pub struct ProcedureSqlGenerator {
    /// Database type.
    pub db_type: DatabaseType,
}

/// Database type for SQL generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    PostgreSQL,
    MySQL,
    SQLite,
    MSSQL,
}

impl ProcedureSqlGenerator {
    /// Create a new generator.
    pub fn new(db_type: DatabaseType) -> Self {
        Self { db_type }
    }

    /// Generate CREATE FUNCTION/PROCEDURE SQL.
    pub fn create_procedure(&self, proc: &ProcedureDefinition) -> String {
        match self.db_type {
            DatabaseType::PostgreSQL => self.create_postgres_procedure(proc),
            DatabaseType::MySQL => self.create_mysql_procedure(proc),
            DatabaseType::SQLite => self.create_sqlite_udf(proc),
            DatabaseType::MSSQL => self.create_mssql_procedure(proc),
        }
    }

    /// Generate DROP FUNCTION/PROCEDURE SQL.
    pub fn drop_procedure(&self, proc: &ProcedureDefinition) -> String {
        let obj_type = if proc.is_function {
            "FUNCTION"
        } else {
            "PROCEDURE"
        };
        let name = proc.qualified_name();

        match self.db_type {
            DatabaseType::PostgreSQL => {
                // PostgreSQL requires parameter types for drop
                let params = proc
                    .parameters
                    .iter()
                    .map(|p| p.data_type.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("DROP {} IF EXISTS {}({});", obj_type, name, params)
            }
            DatabaseType::MySQL => {
                format!("DROP {} IF EXISTS {};", obj_type, name)
            }
            DatabaseType::SQLite => {
                // SQLite doesn't have stored procedures
                format!("-- SQLite: Remove UDF registration for {}", name)
            }
            DatabaseType::MSSQL => {
                format!(
                    "IF OBJECT_ID('{}', '{}') IS NOT NULL DROP {} {};",
                    name,
                    if proc.is_function { "FN" } else { "P" },
                    obj_type,
                    name
                )
            }
        }
    }

    /// Generate ALTER FUNCTION/PROCEDURE SQL (usually CREATE OR REPLACE).
    pub fn alter_procedure(&self, diff: &ProcedureAlterDiff) -> String {
        // For most databases, alter means drop + create or CREATE OR REPLACE
        match self.db_type {
            DatabaseType::PostgreSQL => {
                // PostgreSQL supports CREATE OR REPLACE
                self.create_postgres_procedure(&diff.new)
            }
            DatabaseType::MySQL => {
                // MySQL requires DROP + CREATE
                format!(
                    "{}\n{}",
                    self.drop_procedure(&diff.old),
                    self.create_mysql_procedure(&diff.new)
                )
            }
            DatabaseType::SQLite => self.create_sqlite_udf(&diff.new),
            DatabaseType::MSSQL => {
                // MSSQL uses ALTER
                self.alter_mssql_procedure(&diff.new)
            }
        }
    }

    /// Generate CREATE TRIGGER SQL.
    pub fn create_trigger(&self, trigger: &TriggerDefinition) -> String {
        match self.db_type {
            DatabaseType::PostgreSQL => self.create_postgres_trigger(trigger),
            DatabaseType::MySQL => self.create_mysql_trigger(trigger),
            DatabaseType::SQLite => self.create_sqlite_trigger(trigger),
            DatabaseType::MSSQL => self.create_mssql_trigger(trigger),
        }
    }

    /// Generate DROP TRIGGER SQL.
    pub fn drop_trigger(&self, trigger: &TriggerDefinition) -> String {
        match self.db_type {
            DatabaseType::PostgreSQL => {
                format!(
                    "DROP TRIGGER IF EXISTS {} ON {};",
                    trigger.name, trigger.table
                )
            }
            DatabaseType::MySQL => {
                format!("DROP TRIGGER IF EXISTS {};", trigger.name)
            }
            DatabaseType::SQLite => {
                format!("DROP TRIGGER IF EXISTS {};", trigger.name)
            }
            DatabaseType::MSSQL => {
                format!("DROP TRIGGER IF EXISTS {};", trigger.qualified_name())
            }
        }
    }

    // PostgreSQL implementations
    fn create_postgres_procedure(&self, proc: &ProcedureDefinition) -> String {
        let mut sql = String::new();

        let obj_type = if proc.is_function {
            "FUNCTION"
        } else {
            "PROCEDURE"
        };
        let or_replace = if proc.or_replace { "OR REPLACE " } else { "" };

        sql.push_str(&format!(
            "CREATE {}{}  {} (",
            or_replace,
            obj_type,
            proc.qualified_name()
        ));

        // Parameters
        let params: Vec<String> = proc
            .parameters
            .iter()
            .map(|p| {
                let mode = match p.mode {
                    ParameterMode::In => "",
                    ParameterMode::Out => "OUT ",
                    ParameterMode::InOut => "INOUT ",
                    ParameterMode::Variadic => "VARIADIC ",
                };
                let default = p
                    .default
                    .as_ref()
                    .map(|d| format!(" DEFAULT {}", d))
                    .unwrap_or_default();
                format!("{}{} {}{}", mode, p.name, p.data_type, default)
            })
            .collect();
        sql.push_str(&params.join(", "));
        sql.push_str(")\n");

        // Return type
        if let Some(ref ret) = proc.return_type {
            if proc.returns_set {
                sql.push_str(&format!("RETURNS SETOF {}\n", ret));
            } else {
                sql.push_str(&format!("RETURNS {}\n", ret));
            }
        } else if !proc.return_columns.is_empty() {
            let cols: Vec<String> = proc
                .return_columns
                .iter()
                .map(|c| format!("{} {}", c.name, c.data_type))
                .collect();
            sql.push_str(&format!("RETURNS TABLE ({})\n", cols.join(", ")));
        } else if proc.is_function {
            sql.push_str("RETURNS void\n");
        }

        // Language
        sql.push_str(&format!("LANGUAGE {}\n", proc.language.to_sql()));

        // Volatility
        sql.push_str(&format!("{}\n", proc.volatility.to_sql()));

        // Security
        if proc.security_definer {
            sql.push_str("SECURITY DEFINER\n");
        }

        // Cost
        if let Some(cost) = proc.cost {
            sql.push_str(&format!("COST {}\n", cost));
        }

        // Parallel
        if proc.parallel != ParallelSafety::Unsafe {
            sql.push_str(&format!("{}\n", proc.parallel.to_sql()));
        }

        // Body
        sql.push_str(&format!("AS $$\n{}\n$$;", proc.body));

        // Comment
        if let Some(ref comment) = proc.comment {
            sql.push_str(&format!(
                "\n\nCOMMENT ON {} {} IS '{}';",
                obj_type,
                proc.qualified_name(),
                comment.replace('\'', "''")
            ));
        }

        sql
    }

    fn create_postgres_trigger(&self, trigger: &TriggerDefinition) -> String {
        let mut sql = String::new();

        let or_replace = if trigger.or_replace {
            "OR REPLACE "
        } else {
            ""
        };

        sql.push_str(&format!("CREATE {}TRIGGER {}\n", or_replace, trigger.name));

        // Timing
        sql.push_str(&format!("{} ", trigger.timing.to_sql()));

        // Events
        let events: Vec<&str> = trigger.events.iter().map(|e| e.to_sql()).collect();
        sql.push_str(&events.join(" OR "));

        // Table
        sql.push_str(&format!("\nON {}\n", trigger.table));

        // Level
        sql.push_str(&format!("{}\n", trigger.level.to_sql()));

        // Condition
        if let Some(ref cond) = trigger.condition {
            sql.push_str(&format!("WHEN ({})\n", cond));
        }

        // Function
        let args = if trigger.function_args.is_empty() {
            String::new()
        } else {
            trigger.function_args.join(", ")
        };
        sql.push_str(&format!("EXECUTE FUNCTION {}({});", trigger.function, args));

        sql
    }

    // MySQL implementations
    fn create_mysql_procedure(&self, proc: &ProcedureDefinition) -> String {
        let mut sql = String::new();

        let obj_type = if proc.is_function {
            "FUNCTION"
        } else {
            "PROCEDURE"
        };

        // MySQL doesn't support CREATE OR REPLACE for procedures
        sql.push_str(&format!("CREATE {} {} (", obj_type, proc.qualified_name()));

        // Parameters
        let params: Vec<String> = proc
            .parameters
            .iter()
            .map(|p| {
                let mode = match p.mode {
                    ParameterMode::In => "IN ",
                    ParameterMode::Out => "OUT ",
                    ParameterMode::InOut => "INOUT ",
                    ParameterMode::Variadic => "",
                };
                format!("{}{} {}", mode, p.name, p.data_type)
            })
            .collect();
        sql.push_str(&params.join(", "));
        sql.push_str(")\n");

        // Return type (functions only)
        if proc.is_function
            && let Some(ref ret) = proc.return_type
        {
            sql.push_str(&format!("RETURNS {}\n", ret));
        }

        // Characteristics
        if proc.volatility == Volatility::Immutable {
            sql.push_str("DETERMINISTIC\n");
        } else {
            sql.push_str("NOT DETERMINISTIC\n");
        }

        if proc.security_definer {
            sql.push_str("SQL SECURITY DEFINER\n");
        }

        // Body
        sql.push_str(&format!("BEGIN\n{}\nEND;", proc.body));

        sql
    }

    fn create_mysql_trigger(&self, trigger: &TriggerDefinition) -> String {
        let mut sql = String::new();

        sql.push_str(&format!("CREATE TRIGGER {}\n", trigger.name));

        // Timing
        sql.push_str(&format!("{} ", trigger.timing.to_sql()));

        // Event (MySQL only supports one)
        if let Some(event) = trigger.events.first() {
            sql.push_str(&format!("{}\n", event.to_sql()));
        }

        // Table
        sql.push_str(&format!("ON {}\n", trigger.table));

        // Level
        sql.push_str(&format!("{}\n", trigger.level.to_sql()));

        // Body (MySQL uses the function body directly)
        sql.push_str(&format!("BEGIN\n    CALL {}();\nEND;", trigger.function));

        sql
    }

    // SQLite implementations
    fn create_sqlite_udf(&self, proc: &ProcedureDefinition) -> String {
        // SQLite UDFs are registered in code, not SQL
        format!(
            "-- SQLite UDF: {} must be registered via rusqlite::create_scalar_function\n\
             -- Parameters: {}\n\
             -- Body:\n-- {}",
            proc.name,
            proc.parameters
                .iter()
                .map(|p| format!("{}: {}", p.name, p.data_type))
                .collect::<Vec<_>>()
                .join(", "),
            proc.body.replace('\n', "\n-- ")
        )
    }

    fn create_sqlite_trigger(&self, trigger: &TriggerDefinition) -> String {
        let mut sql = String::new();

        sql.push_str(&format!("CREATE TRIGGER IF NOT EXISTS {}\n", trigger.name));

        // Timing
        sql.push_str(&format!("{} ", trigger.timing.to_sql()));

        // Events
        let events: Vec<&str> = trigger.events.iter().map(|e| e.to_sql()).collect();
        sql.push_str(&events.join(" OR "));

        // Table
        sql.push_str(&format!("\nON {}\n", trigger.table));

        // Level (SQLite only supports row-level)
        sql.push_str("FOR EACH ROW\n");

        // Condition
        if let Some(ref cond) = trigger.condition {
            sql.push_str(&format!("WHEN {}\n", cond));
        }

        // Body (inline for SQLite)
        sql.push_str(&format!("BEGIN\n    SELECT {}();\nEND;", trigger.function));

        sql
    }

    // MSSQL implementations
    fn create_mssql_procedure(&self, proc: &ProcedureDefinition) -> String {
        let mut sql = String::new();

        let obj_type = if proc.is_function {
            "FUNCTION"
        } else {
            "PROCEDURE"
        };

        sql.push_str(&format!("CREATE {} {} (", obj_type, proc.qualified_name()));

        // Parameters
        let params: Vec<String> = proc
            .parameters
            .iter()
            .map(|p| {
                let output = if p.mode == ParameterMode::Out || p.mode == ParameterMode::InOut {
                    " OUTPUT"
                } else {
                    ""
                };
                format!("@{} {}{}", p.name, p.data_type, output)
            })
            .collect();
        sql.push_str(&params.join(", "));
        sql.push_str(")\n");

        // Return type (functions only)
        if proc.is_function
            && let Some(ref ret) = proc.return_type
        {
            sql.push_str(&format!("RETURNS {}\n", ret));
        }

        sql.push_str("AS\nBEGIN\n");
        sql.push_str(&proc.body);
        sql.push_str("\nEND;");

        sql
    }

    fn alter_mssql_procedure(&self, proc: &ProcedureDefinition) -> String {
        // Change CREATE to ALTER
        self.create_mssql_procedure(proc)
            .replacen("CREATE", "ALTER", 1)
    }

    fn create_mssql_trigger(&self, trigger: &TriggerDefinition) -> String {
        let mut sql = String::new();

        sql.push_str(&format!(
            "CREATE TRIGGER {}\nON {}\n",
            trigger.qualified_name(),
            trigger.table
        ));

        // Timing
        sql.push_str(&format!("{} ", trigger.timing.to_sql()));

        // Events
        let events: Vec<&str> = trigger.events.iter().map(|e| e.to_sql()).collect();
        sql.push_str(&events.join(", "));

        sql.push_str("\nAS\nBEGIN\n");
        sql.push_str(&format!("    EXEC {};\n", trigger.function));
        sql.push_str("END;");

        sql
    }

    /// Generate full migration SQL for a procedure diff.
    pub fn generate_migration(&self, diff: &ProcedureDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();

        // Create procedures
        for proc in &diff.create {
            up.push(self.create_procedure(proc));
            down.push(self.drop_procedure(proc));
        }

        // Drop procedures
        for name in &diff.drop {
            // For down, we'd need the original definition
            up.push(format!("DROP FUNCTION IF EXISTS {};", name));
            // down would need to recreate, but we don't have the original
            down.push(format!("-- Recreate {} (original definition needed)", name));
        }

        // Alter procedures
        for alter in &diff.alter {
            up.push(self.alter_procedure(alter));
            // Down would restore the old version
            down.push(self.create_procedure(&alter.old));
        }

        // Create triggers
        for trigger in &diff.create_triggers {
            up.push(self.create_trigger(trigger));
            down.push(self.drop_trigger(trigger));
        }

        // Drop triggers
        for name in &diff.drop_triggers {
            up.push(format!("DROP TRIGGER IF EXISTS {};", name));
            down.push(format!(
                "-- Recreate trigger {} (original definition needed)",
                name
            ));
        }

        // Alter triggers
        for alter in &diff.alter_triggers {
            up.push(self.drop_trigger(&alter.old));
            up.push(self.create_trigger(&alter.new));
            down.push(self.drop_trigger(&alter.new));
            down.push(self.create_trigger(&alter.old));
        }

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
        }
    }
}

/// Generated migration SQL.
#[derive(Debug, Clone)]
pub struct MigrationSql {
    /// Up migration SQL.
    pub up: String,
    /// Down migration SQL (rollback).
    pub down: String,
}

// ============================================================================
// Procedure Store
// ============================================================================

/// Storage for procedure definitions with version tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcedureStore {
    /// Stored procedures and functions.
    pub procedures: HashMap<String, ProcedureDefinition>,
    /// Triggers.
    pub triggers: HashMap<String, TriggerDefinition>,
    /// Scheduled events.
    pub events: HashMap<String, ScheduledEvent>,
    /// History of changes.
    pub history: Vec<ProcedureHistoryEntry>,
}

/// A history entry for procedure changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureHistoryEntry {
    /// Timestamp.
    pub timestamp: String,
    /// Migration ID.
    pub migration_id: String,
    /// Change type.
    pub change_type: ChangeType,
    /// Object name.
    pub name: String,
    /// Old checksum.
    pub old_checksum: Option<String>,
    /// New checksum.
    pub new_checksum: Option<String>,
}

/// Type of change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeType {
    Create,
    Alter,
    Drop,
}

impl ProcedureStore {
    /// Create a new store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a procedure.
    pub fn add_procedure(&mut self, proc: ProcedureDefinition) {
        self.procedures.insert(proc.qualified_name(), proc);
    }

    /// Add a trigger.
    pub fn add_trigger(&mut self, trigger: TriggerDefinition) {
        self.triggers.insert(trigger.qualified_name(), trigger);
    }

    /// Get all procedures as a list.
    pub fn procedures_list(&self) -> Vec<&ProcedureDefinition> {
        self.procedures.values().collect()
    }

    /// Get all triggers as a list.
    pub fn triggers_list(&self) -> Vec<&TriggerDefinition> {
        self.triggers.values().collect()
    }

    /// Save to file as TOML.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let content = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, content)
    }

    /// Load from file as TOML.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(std::io::Error::other)
    }

    /// Add an event.
    pub fn add_event(&mut self, event: ScheduledEvent) {
        self.events.insert(event.name.clone(), event);
    }

    /// Get all events as a list.
    pub fn events_list(&self) -> Vec<&ScheduledEvent> {
        self.events.values().collect()
    }
}

// ============================================================================
// Event Scheduler (MySQL) / SQL Agent Jobs (MSSQL)
// ============================================================================

/// A scheduled event definition (MySQL EVENT / MSSQL SQL Agent Job).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    /// Event name.
    pub name: String,
    /// Schema/database.
    pub schema: Option<String>,
    /// Schedule definition.
    pub schedule: EventSchedule,
    /// SQL body to execute.
    pub body: String,
    /// Whether the event is enabled.
    pub enabled: bool,
    /// Event preservation after expiration.
    pub on_completion: OnCompletion,
    /// Comment/description.
    pub comment: Option<String>,
    /// Start time (optional).
    pub starts: Option<String>,
    /// End time (optional).
    pub ends: Option<String>,
    /// Definer (MySQL) or owner (MSSQL).
    pub definer: Option<String>,
}

impl Default for ScheduledEvent {
    fn default() -> Self {
        Self {
            name: String::new(),
            schema: None,
            schedule: EventSchedule::Once,
            body: String::new(),
            enabled: true,
            on_completion: OnCompletion::Drop,
            comment: None,
            starts: None,
            ends: None,
            definer: None,
        }
    }
}

impl ScheduledEvent {
    /// Create a new scheduled event.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set the schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set to run once at a specific time.
    pub fn at(mut self, datetime: impl Into<String>) -> Self {
        self.schedule = EventSchedule::At(datetime.into());
        self
    }

    /// Set to run every interval.
    pub fn every(mut self, interval: EventInterval) -> Self {
        self.schedule = EventSchedule::Every(interval);
        self
    }

    /// Set the SQL body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Disable the event.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Preserve the event after completion.
    pub fn preserve(mut self) -> Self {
        self.on_completion = OnCompletion::Preserve;
        self
    }

    /// Set start time.
    pub fn starts(mut self, datetime: impl Into<String>) -> Self {
        self.starts = Some(datetime.into());
        self
    }

    /// Set end time.
    pub fn ends(mut self, datetime: impl Into<String>) -> Self {
        self.ends = Some(datetime.into());
        self
    }

    /// Set comment.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Get the fully qualified name.
    pub fn qualified_name(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{}", schema, self.name),
            None => self.name.clone(),
        }
    }
}

/// Event schedule type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventSchedule {
    /// Run once (immediately or when triggered).
    Once,
    /// Run at a specific time.
    At(String),
    /// Run every interval.
    Every(EventInterval),
    /// Cron expression (MSSQL).
    Cron(String),
}

/// Event interval for recurring events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventInterval {
    /// Quantity.
    pub quantity: u32,
    /// Unit.
    pub unit: IntervalUnit,
}

impl EventInterval {
    /// Create a new interval.
    pub fn new(quantity: u32, unit: IntervalUnit) -> Self {
        Self { quantity, unit }
    }

    /// Every N seconds.
    pub fn seconds(n: u32) -> Self {
        Self::new(n, IntervalUnit::Second)
    }

    /// Every N minutes.
    pub fn minutes(n: u32) -> Self {
        Self::new(n, IntervalUnit::Minute)
    }

    /// Every N hours.
    pub fn hours(n: u32) -> Self {
        Self::new(n, IntervalUnit::Hour)
    }

    /// Every N days.
    pub fn days(n: u32) -> Self {
        Self::new(n, IntervalUnit::Day)
    }

    /// Every N weeks.
    pub fn weeks(n: u32) -> Self {
        Self::new(n, IntervalUnit::Week)
    }

    /// Every N months.
    pub fn months(n: u32) -> Self {
        Self::new(n, IntervalUnit::Month)
    }

    /// Convert to MySQL interval string.
    pub fn to_mysql(&self) -> String {
        format!("{} {}", self.quantity, self.unit.to_mysql())
    }
}

/// Interval unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntervalUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl IntervalUnit {
    /// Convert to MySQL interval unit.
    pub fn to_mysql(&self) -> &'static str {
        match self {
            Self::Second => "SECOND",
            Self::Minute => "MINUTE",
            Self::Hour => "HOUR",
            Self::Day => "DAY",
            Self::Week => "WEEK",
            Self::Month => "MONTH",
            Self::Quarter => "QUARTER",
            Self::Year => "YEAR",
        }
    }
}

/// What to do when event completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OnCompletion {
    /// Drop the event.
    #[default]
    Drop,
    /// Preserve the event.
    Preserve,
}

// ============================================================================
// MSSQL SQL Agent Job
// ============================================================================

/// SQL Server Agent Job definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlAgentJob {
    /// Job name.
    pub name: String,
    /// Job description.
    pub description: Option<String>,
    /// Job category.
    pub category: Option<String>,
    /// Job owner.
    pub owner: Option<String>,
    /// Whether the job is enabled.
    pub enabled: bool,
    /// Job steps.
    pub steps: Vec<JobStep>,
    /// Job schedules.
    pub schedules: Vec<JobSchedule>,
    /// Notification settings.
    pub notify_level: NotifyLevel,
}

impl Default for SqlAgentJob {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: None,
            category: None,
            owner: None,
            enabled: true,
            steps: Vec::new(),
            schedules: Vec::new(),
            notify_level: NotifyLevel::OnFailure,
        }
    }
}

impl SqlAgentJob {
    /// Create a new SQL Agent job.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add a step.
    pub fn step(mut self, step: JobStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Add a T-SQL step.
    pub fn tsql_step(mut self, name: impl Into<String>, sql: impl Into<String>) -> Self {
        self.steps.push(JobStep {
            name: name.into(),
            step_type: StepType::TSql,
            command: sql.into(),
            database: None,
            on_success: StepAction::GoToNextStep,
            on_failure: StepAction::QuitWithFailure,
            retry_attempts: 0,
            retry_interval: 0,
        });
        self
    }

    /// Add a schedule.
    pub fn schedule(mut self, schedule: JobSchedule) -> Self {
        self.schedules.push(schedule);
        self
    }

    /// Disable the job.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// A step in a SQL Agent job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStep {
    /// Step name.
    pub name: String,
    /// Step type.
    pub step_type: StepType,
    /// Command to execute.
    pub command: String,
    /// Database context.
    pub database: Option<String>,
    /// Action on success.
    pub on_success: StepAction,
    /// Action on failure.
    pub on_failure: StepAction,
    /// Retry attempts.
    pub retry_attempts: u32,
    /// Retry interval in minutes.
    pub retry_interval: u32,
}

/// Job step type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StepType {
    #[default]
    TSql,
    CmdExec,
    PowerShell,
    Ssis,
    Ssas,
    Ssrs,
}

impl StepType {
    /// Get the subsystem name for sp_add_jobstep.
    pub fn subsystem(&self) -> &'static str {
        match self {
            Self::TSql => "TSQL",
            Self::CmdExec => "CmdExec",
            Self::PowerShell => "PowerShell",
            Self::Ssis => "SSIS",
            Self::Ssas => "AnalysisCommand",
            Self::Ssrs => "Reporting Services Command",
        }
    }
}

/// Action after step completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StepAction {
    #[default]
    GoToNextStep,
    GoToStep(u32),
    QuitWithSuccess,
    QuitWithFailure,
}

impl StepAction {
    /// Get the action ID for sp_add_jobstep.
    pub fn action_id(&self) -> u32 {
        match self {
            Self::GoToNextStep => 3,
            Self::GoToStep(_) => 4,
            Self::QuitWithSuccess => 1,
            Self::QuitWithFailure => 2,
        }
    }
}

/// Job schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSchedule {
    /// Schedule name.
    pub name: String,
    /// Frequency type.
    pub frequency: ScheduleFrequency,
    /// Time of day (for daily/weekly/monthly).
    pub active_start_time: Option<String>,
    /// Start date.
    pub start_date: Option<String>,
    /// End date.
    pub end_date: Option<String>,
    /// Whether enabled.
    pub enabled: bool,
}

impl Default for JobSchedule {
    fn default() -> Self {
        Self {
            name: String::new(),
            frequency: ScheduleFrequency::Daily { every_n_days: 1 },
            active_start_time: None,
            start_date: None,
            end_date: None,
            enabled: true,
        }
    }
}

impl JobSchedule {
    /// Create a new schedule.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Run once.
    pub fn once(mut self) -> Self {
        self.frequency = ScheduleFrequency::Once;
        self
    }

    /// Run daily.
    pub fn daily(mut self, every_n_days: u32) -> Self {
        self.frequency = ScheduleFrequency::Daily { every_n_days };
        self
    }

    /// Run weekly.
    pub fn weekly(mut self, days: Vec<Weekday>) -> Self {
        self.frequency = ScheduleFrequency::Weekly {
            every_n_weeks: 1,
            days,
        };
        self
    }

    /// Run monthly.
    pub fn monthly(mut self, day_of_month: u32) -> Self {
        self.frequency = ScheduleFrequency::Monthly {
            every_n_months: 1,
            day_of_month,
        };
        self
    }

    /// Set time of day.
    pub fn at(mut self, time: impl Into<String>) -> Self {
        self.active_start_time = Some(time.into());
        self
    }
}

/// Schedule frequency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleFrequency {
    Once,
    Daily {
        every_n_days: u32,
    },
    Weekly {
        every_n_weeks: u32,
        days: Vec<Weekday>,
    },
    Monthly {
        every_n_months: u32,
        day_of_month: u32,
    },
    OnIdle,
    OnAgentStart,
}

impl ScheduleFrequency {
    /// Get frequency type ID for sp_add_schedule.
    pub fn freq_type(&self) -> u32 {
        match self {
            Self::Once => 1,
            Self::Daily { .. } => 4,
            Self::Weekly { .. } => 8,
            Self::Monthly { .. } => 16,
            Self::OnIdle => 128,
            Self::OnAgentStart => 64,
        }
    }

    /// Get frequency interval.
    pub fn freq_interval(&self) -> u32 {
        match self {
            Self::Once => 0,
            Self::Daily { every_n_days } => *every_n_days,
            Self::Weekly { days, .. } => days.iter().map(|d| d.bitmask()).fold(0, |acc, m| acc | m),
            Self::Monthly { day_of_month, .. } => *day_of_month,
            Self::OnIdle | Self::OnAgentStart => 0,
        }
    }
}

/// Day of week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl Weekday {
    /// Get bitmask for SQL Agent.
    pub fn bitmask(&self) -> u32 {
        match self {
            Self::Sunday => 1,
            Self::Monday => 2,
            Self::Tuesday => 4,
            Self::Wednesday => 8,
            Self::Thursday => 16,
            Self::Friday => 32,
            Self::Saturday => 64,
        }
    }
}

/// Notification level for job completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NotifyLevel {
    Never,
    OnSuccess,
    #[default]
    OnFailure,
    Always,
}

// ============================================================================
// MongoDB Atlas Triggers
// ============================================================================

/// MongoDB Atlas Trigger definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasTrigger {
    /// Trigger name.
    pub name: String,
    /// Trigger type.
    pub trigger_type: AtlasTriggerType,
    /// Whether enabled.
    pub enabled: bool,
    /// Function to execute.
    pub function_name: String,
    /// Match expression for database triggers.
    pub match_expression: Option<String>,
    /// Project expression.
    pub project: Option<String>,
    /// Full document option.
    pub full_document: bool,
    /// Full document before change.
    pub full_document_before_change: bool,
}

impl Default for AtlasTrigger {
    fn default() -> Self {
        Self {
            name: String::new(),
            trigger_type: AtlasTriggerType::Database {
                database: String::new(),
                collection: String::new(),
                operation_types: Vec::new(),
            },
            enabled: true,
            function_name: String::new(),
            match_expression: None,
            project: None,
            full_document: false,
            full_document_before_change: false,
        }
    }
}

impl AtlasTrigger {
    /// Create a new Atlas trigger.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Configure as database trigger.
    pub fn database(
        mut self,
        database: impl Into<String>,
        collection: impl Into<String>,
        operations: Vec<AtlasOperation>,
    ) -> Self {
        self.trigger_type = AtlasTriggerType::Database {
            database: database.into(),
            collection: collection.into(),
            operation_types: operations,
        };
        self
    }

    /// Configure as scheduled trigger.
    pub fn scheduled(mut self, cron: impl Into<String>) -> Self {
        self.trigger_type = AtlasTriggerType::Scheduled {
            schedule: cron.into(),
        };
        self
    }

    /// Configure as authentication trigger.
    pub fn authentication(mut self, operation: AuthOperation) -> Self {
        self.trigger_type = AtlasTriggerType::Authentication { operation };
        self
    }

    /// Set the function to execute.
    pub fn function(mut self, name: impl Into<String>) -> Self {
        self.function_name = name.into();
        self
    }

    /// Enable full document.
    pub fn full_document(mut self) -> Self {
        self.full_document = true;
        self
    }

    /// Disable the trigger.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Atlas trigger type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtlasTriggerType {
    /// Database change trigger.
    Database {
        database: String,
        collection: String,
        operation_types: Vec<AtlasOperation>,
    },
    /// Scheduled trigger (cron).
    Scheduled { schedule: String },
    /// Authentication trigger.
    Authentication { operation: AuthOperation },
}

/// Atlas database operation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtlasOperation {
    Insert,
    Update,
    Replace,
    Delete,
}

impl AtlasOperation {
    /// Get the operation type string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Replace => "REPLACE",
            Self::Delete => "DELETE",
        }
    }
}

/// Authentication operation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthOperation {
    Create,
    Login,
    Delete,
}

// ============================================================================
// Event SQL Generation
// ============================================================================

impl ProcedureSqlGenerator {
    /// Generate CREATE EVENT SQL (MySQL).
    pub fn create_event(&self, event: &ScheduledEvent) -> String {
        match self.db_type {
            DatabaseType::MySQL => self.create_mysql_event(event),
            DatabaseType::MSSQL => {
                // MSSQL uses SQL Agent - return a comment
                format!(
                    "-- Use SqlAgentJob for MSSQL scheduled tasks\n\
                     -- Event: {}\n\
                     -- Schedule: {:?}",
                    event.name, event.schedule
                )
            }
            _ => format!("-- Events not supported for {:?}", self.db_type),
        }
    }

    /// Generate DROP EVENT SQL.
    pub fn drop_event(&self, event: &ScheduledEvent) -> String {
        match self.db_type {
            DatabaseType::MySQL => {
                format!("DROP EVENT IF EXISTS {};", event.qualified_name())
            }
            _ => format!("-- DROP EVENT not supported for {:?}", self.db_type),
        }
    }

    fn create_mysql_event(&self, event: &ScheduledEvent) -> String {
        let mut sql = String::new();

        sql.push_str(&format!(
            "CREATE EVENT IF NOT EXISTS {}\n",
            event.qualified_name()
        ));

        // Schedule
        match &event.schedule {
            EventSchedule::Once => sql.push_str("ON SCHEDULE AT CURRENT_TIMESTAMP\n"),
            EventSchedule::At(datetime) => {
                sql.push_str(&format!("ON SCHEDULE AT '{}'\n", datetime))
            }
            EventSchedule::Every(interval) => {
                sql.push_str(&format!("ON SCHEDULE EVERY {}\n", interval.to_mysql()));
                if let Some(ref starts) = event.starts {
                    sql.push_str(&format!("STARTS '{}'\n", starts));
                }
                if let Some(ref ends) = event.ends {
                    sql.push_str(&format!("ENDS '{}'\n", ends));
                }
            }
            EventSchedule::Cron(_) => {
                // MySQL doesn't support cron directly, use EVERY
                sql.push_str("ON SCHEDULE EVERY 1 DAY\n");
            }
        }

        // Completion
        match event.on_completion {
            OnCompletion::Drop => sql.push_str("ON COMPLETION NOT PRESERVE\n"),
            OnCompletion::Preserve => sql.push_str("ON COMPLETION PRESERVE\n"),
        }

        // Enabled
        if event.enabled {
            sql.push_str("ENABLE\n");
        } else {
            sql.push_str("DISABLE\n");
        }

        // Comment
        if let Some(ref comment) = event.comment {
            sql.push_str(&format!("COMMENT '{}'\n", comment.replace('\'', "''")));
        }

        // Body
        sql.push_str(&format!("DO\n{};", event.body));

        sql
    }

    /// Generate SQL Agent job creation script.
    pub fn create_sql_agent_job(&self, job: &SqlAgentJob) -> String {
        let mut sql = String::new();

        // Add job
        sql.push_str("-- Create SQL Agent Job\n");
        sql.push_str("EXEC msdb.dbo.sp_add_job\n");
        sql.push_str(&format!("    @job_name = N'{}',\n", job.name));

        if let Some(ref desc) = job.description {
            sql.push_str(&format!("    @description = N'{}',\n", desc));
        }

        sql.push_str(&format!(
            "    @enabled = {};\n\n",
            if job.enabled { 1 } else { 0 }
        ));

        // Add steps
        for (i, step) in job.steps.iter().enumerate() {
            sql.push_str(&format!("-- Step {}: {}\n", i + 1, step.name));
            sql.push_str("EXEC msdb.dbo.sp_add_jobstep\n");
            sql.push_str(&format!("    @job_name = N'{}',\n", job.name));
            sql.push_str(&format!("    @step_name = N'{}',\n", step.name));
            sql.push_str(&format!(
                "    @subsystem = N'{}',\n",
                step.step_type.subsystem()
            ));
            sql.push_str(&format!(
                "    @command = N'{}',\n",
                step.command.replace('\'', "''")
            ));

            if let Some(ref db) = step.database {
                sql.push_str(&format!("    @database_name = N'{}',\n", db));
            }

            sql.push_str(&format!(
                "    @on_success_action = {},\n",
                step.on_success.action_id()
            ));
            sql.push_str(&format!(
                "    @on_fail_action = {},\n",
                step.on_failure.action_id()
            ));
            sql.push_str(&format!("    @retry_attempts = {},\n", step.retry_attempts));
            sql.push_str(&format!(
                "    @retry_interval = {};\n\n",
                step.retry_interval
            ));
        }

        // Add schedules
        for schedule in &job.schedules {
            sql.push_str(&format!("-- Schedule: {}\n", schedule.name));
            sql.push_str("EXEC msdb.dbo.sp_add_schedule\n");
            sql.push_str(&format!("    @schedule_name = N'{}',\n", schedule.name));
            sql.push_str(&format!(
                "    @enabled = {},\n",
                if schedule.enabled { 1 } else { 0 }
            ));
            sql.push_str(&format!(
                "    @freq_type = {},\n",
                schedule.frequency.freq_type()
            ));
            sql.push_str(&format!(
                "    @freq_interval = {};\n",
                schedule.frequency.freq_interval()
            ));

            // Attach schedule to job
            sql.push_str("\nEXEC msdb.dbo.sp_attach_schedule\n");
            sql.push_str(&format!("    @job_name = N'{}',\n", job.name));
            sql.push_str(&format!("    @schedule_name = N'{}';\n\n", schedule.name));
        }

        // Add job server
        sql.push_str("EXEC msdb.dbo.sp_add_jobserver\n");
        sql.push_str(&format!("    @job_name = N'{}',\n", job.name));
        sql.push_str("    @server_name = N'(LOCAL)';\n");

        sql
    }

    /// Generate DROP SQL Agent job script.
    pub fn drop_sql_agent_job(&self, job_name: &str) -> String {
        format!(
            "IF EXISTS (SELECT 1 FROM msdb.dbo.sysjobs WHERE name = N'{}')\n\
             BEGIN\n\
                 EXEC msdb.dbo.sp_delete_job @job_name = N'{}';\n\
             END;",
            job_name, job_name
        )
    }
}

// ============================================================================
// Event Diff
// ============================================================================

/// Differences in scheduled events.
#[derive(Debug, Clone, Default)]
pub struct EventDiff {
    /// Events to create.
    pub create: Vec<ScheduledEvent>,
    /// Events to drop.
    pub drop: Vec<String>,
    /// Events to alter.
    pub alter: Vec<EventAlterDiff>,
    /// SQL Agent jobs to create.
    pub create_jobs: Vec<SqlAgentJob>,
    /// SQL Agent jobs to drop.
    pub drop_jobs: Vec<String>,
}

/// Event alter diff.
#[derive(Debug, Clone)]
pub struct EventAlterDiff {
    /// Old event.
    pub old: ScheduledEvent,
    /// New event.
    pub new: ScheduledEvent,
}

impl EventDiff {
    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.create.is_empty()
            && self.drop.is_empty()
            && self.alter.is_empty()
            && self.create_jobs.is_empty()
            && self.drop_jobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_procedure_definition() {
        let proc = ProcedureDefinition::function("calculate_tax")
            .schema("public")
            .param("amount", "DECIMAL(10,2)")
            .param("rate", "DECIMAL(5,4)")
            .returns("DECIMAL(10,2)")
            .language(ProcedureLanguage::PlPgSql)
            .immutable()
            .body("RETURN amount * rate;");

        assert_eq!(proc.name, "calculate_tax");
        assert_eq!(proc.qualified_name(), "public.calculate_tax");
        assert_eq!(proc.parameters.len(), 2);
        assert!(proc.is_function);
    }

    #[test]
    fn test_trigger_definition() {
        let trigger = TriggerDefinition::new("audit_users", "users")
            .after()
            .on(vec![TriggerEvent::Insert, TriggerEvent::Update])
            .for_each_row()
            .execute("audit_trigger_fn");

        assert_eq!(trigger.name, "audit_users");
        assert_eq!(trigger.table, "users");
        assert_eq!(trigger.timing, TriggerTiming::After);
        assert_eq!(trigger.events.len(), 2);
    }

    #[test]
    fn test_procedure_diff() {
        let old = vec![
            ProcedureDefinition::function("fn1").body("v1"),
            ProcedureDefinition::function("fn2").body("v2"),
        ];
        let new = vec![
            ProcedureDefinition::function("fn1").body("v1_updated"),
            ProcedureDefinition::function("fn3").body("v3"),
        ];

        let diff = ProcedureDiffer::diff(&old, &new);

        assert_eq!(diff.create.len(), 1); // fn3
        assert_eq!(diff.drop.len(), 1); // fn2
        assert_eq!(diff.alter.len(), 1); // fn1
    }

    #[test]
    fn test_postgres_sql_generation() {
        let generator = ProcedureSqlGenerator::new(DatabaseType::PostgreSQL);

        let proc = ProcedureDefinition::function("greet")
            .param("name", "TEXT")
            .returns("TEXT")
            .language(ProcedureLanguage::Sql)
            .immutable()
            .body("SELECT 'Hello, ' || name || '!';");

        let sql = generator.create_procedure(&proc);
        assert!(sql.contains("CREATE OR REPLACE"));
        assert!(sql.contains("FUNCTION"));
        assert!(sql.contains("RETURNS TEXT"));
        assert!(sql.contains("IMMUTABLE"));
    }

    #[test]
    fn test_trigger_sql_generation() {
        let generator = ProcedureSqlGenerator::new(DatabaseType::PostgreSQL);

        let trigger = TriggerDefinition::new("update_timestamp", "users")
            .before()
            .on(vec![TriggerEvent::Update])
            .for_each_row()
            .execute("set_updated_at");

        let sql = generator.create_trigger(&trigger);
        assert!(sql.contains("CREATE OR REPLACE TRIGGER"));
        assert!(sql.contains("BEFORE UPDATE"));
        assert!(sql.contains("FOR EACH ROW"));
    }

    #[test]
    fn test_procedure_store() {
        let mut store = ProcedureStore::new();

        store.add_procedure(ProcedureDefinition::function("fn1").body("test"));
        store.add_trigger(TriggerDefinition::new("tr1", "table1"));

        assert_eq!(store.procedures.len(), 1);
        assert_eq!(store.triggers.len(), 1);
    }

    #[test]
    fn test_scheduled_event() {
        let event = ScheduledEvent::new("cleanup_old_data")
            .every(EventInterval::days(1))
            .body("DELETE FROM logs WHERE created_at < NOW() - INTERVAL 30 DAY")
            .preserve()
            .comment("Daily cleanup of old log entries");

        assert_eq!(event.name, "cleanup_old_data");
        assert!(matches!(event.schedule, EventSchedule::Every(_)));
        assert_eq!(event.on_completion, OnCompletion::Preserve);
    }

    #[test]
    fn test_mysql_event_sql() {
        let generator = ProcedureSqlGenerator::new(DatabaseType::MySQL);

        let event = ScheduledEvent::new("hourly_stats")
            .every(EventInterval::hours(1))
            .body("CALL update_statistics()");

        let sql = generator.create_event(&event);
        assert!(sql.contains("CREATE EVENT IF NOT EXISTS"));
        assert!(sql.contains("ON SCHEDULE EVERY 1 HOUR"));
    }

    #[test]
    fn test_sql_agent_job() {
        let job = SqlAgentJob::new("nightly_backup")
            .description("Nightly database backup")
            .tsql_step(
                "Backup",
                "BACKUP DATABASE mydb TO DISK = 'C:\\backups\\mydb.bak'",
            )
            .schedule(JobSchedule::new("daily_2am").daily(1).at("02:00:00"));

        assert_eq!(job.name, "nightly_backup");
        assert_eq!(job.steps.len(), 1);
        assert_eq!(job.schedules.len(), 1);
    }

    #[test]
    fn test_sql_agent_job_sql() {
        let generator = ProcedureSqlGenerator::new(DatabaseType::MSSQL);

        let job = SqlAgentJob::new("test_job").tsql_step("Step1", "SELECT 1");

        let sql = generator.create_sql_agent_job(&job);
        assert!(sql.contains("sp_add_job"));
        assert!(sql.contains("sp_add_jobstep"));
    }

    #[test]
    fn test_atlas_trigger() {
        let trigger = AtlasTrigger::new("on_user_create")
            .database("mydb", "users", vec![AtlasOperation::Insert])
            .function("handleUserCreate")
            .full_document();

        assert_eq!(trigger.name, "on_user_create");
        assert!(trigger.full_document);
        assert!(matches!(
            trigger.trigger_type,
            AtlasTriggerType::Database { .. }
        ));
    }
}
