//! Row-Level Security (RLS) policy definitions for the Prax schema AST.
//!
//! Policies enable fine-grained access control at the row level.
//! They are evaluated for each row that a query accesses and determine whether
//! the row should be visible or modifiable based on the policy expression.
//!
//! ## Supported Databases
//!
//! - **PostgreSQL**: Native RLS with CREATE POLICY
//! - **SQL Server (MSSQL)**: Security Policies with predicate functions
//!
//! ## PostgreSQL RLS
//!
//! PostgreSQL uses `CREATE POLICY` statements with USING (filter) and WITH CHECK
//! (block) expressions evaluated inline.
//!
//! ## SQL Server RLS
//!
//! SQL Server requires:
//! 1. A schema-bound inline table-valued function (predicate function)
//! 2. A security policy binding the function to the table
//!
//! The predicate function returns 1 for rows that should be accessible.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Documentation, Ident, Span};

/// A Row-Level Security (RLS) policy definition.
///
/// Policies provide fine-grained access control at the row level.
/// They are applied to tables and evaluated for each row operation.
///
/// # Example Schema Syntax
///
/// ```text
/// policy UserReadOwnData on User {
///     for     SELECT
///     to      authenticated
///     using   "id = current_user_id()"
/// }
///
/// policy UserModifyOwnData on User {
///     for     [INSERT, UPDATE, DELETE]
///     to      authenticated
///     using   "id = current_user_id()"
///     check   "id = current_user_id()"
/// }
/// ```
///
/// # Database Support
///
/// - PostgreSQL: Full support via CREATE POLICY
/// - SQL Server: Supported via Security Policies with predicate functions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// Policy name (must be unique per table).
    pub name: Ident,
    /// The model/table this policy applies to.
    pub table: Ident,
    /// Policy type: PERMISSIVE (default) or RESTRICTIVE.
    pub policy_type: PolicyType,
    /// Commands this policy applies to (SELECT, INSERT, UPDATE, DELETE, or ALL).
    pub commands: Vec<PolicyCommand>,
    /// Roles this policy applies to (default: PUBLIC).
    pub roles: Vec<SmolStr>,
    /// USING expression - evaluated for existing rows (SELECT, UPDATE, DELETE).
    /// Should return boolean. Row is visible if expression returns true.
    /// In MSSQL, this becomes the FILTER PREDICATE.
    pub using_expr: Option<String>,
    /// WITH CHECK expression - evaluated for new rows (INSERT, UPDATE).
    /// Should return boolean. Row can be inserted/updated if expression returns true.
    /// In MSSQL, this becomes BLOCK PREDICATE(s).
    pub check_expr: Option<String>,
    /// MSSQL-specific: Schema for the predicate function (default: "Security").
    pub mssql_schema: Option<SmolStr>,
    /// MSSQL-specific: Block operations to apply (default: all applicable).
    pub mssql_block_operations: Vec<MssqlBlockOperation>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
}

impl Policy {
    /// Create a new policy.
    pub fn new(name: Ident, table: Ident, span: Span) -> Self {
        Self {
            name,
            table,
            policy_type: PolicyType::Permissive,
            commands: vec![PolicyCommand::All],
            roles: vec![],
            using_expr: None,
            check_expr: None,
            mssql_schema: None,
            mssql_block_operations: vec![],
            documentation: None,
            span,
        }
    }

    /// Get the policy name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Get the table name as a string.
    pub fn table(&self) -> &str {
        self.table.as_str()
    }

    /// Set the policy type.
    pub fn with_type(mut self, policy_type: PolicyType) -> Self {
        self.policy_type = policy_type;
        self
    }

    /// Set the commands this policy applies to.
    pub fn with_commands(mut self, commands: Vec<PolicyCommand>) -> Self {
        self.commands = commands;
        self
    }

    /// Add a command this policy applies to.
    pub fn add_command(&mut self, command: PolicyCommand) {
        self.commands.push(command);
    }

    /// Set the roles this policy applies to.
    pub fn with_roles(mut self, roles: Vec<SmolStr>) -> Self {
        self.roles = roles;
        self
    }

    /// Add a role this policy applies to.
    pub fn add_role(&mut self, role: impl Into<SmolStr>) {
        self.roles.push(role.into());
    }

    /// Set the USING expression.
    pub fn with_using(mut self, expr: impl Into<String>) -> Self {
        self.using_expr = Some(expr.into());
        self
    }

    /// Set the WITH CHECK expression.
    pub fn with_check(mut self, expr: impl Into<String>) -> Self {
        self.check_expr = Some(expr.into());
        self
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: Documentation) -> Self {
        self.documentation = Some(doc);
        self
    }

    /// Set the MSSQL schema for the predicate function.
    pub fn with_mssql_schema(mut self, schema: impl Into<SmolStr>) -> Self {
        self.mssql_schema = Some(schema.into());
        self
    }

    /// Set the MSSQL block operations.
    pub fn with_mssql_block_operations(mut self, operations: Vec<MssqlBlockOperation>) -> Self {
        self.mssql_block_operations = operations;
        self
    }

    /// Add an MSSQL block operation.
    pub fn add_mssql_block_operation(&mut self, operation: MssqlBlockOperation) {
        self.mssql_block_operations.push(operation);
    }

    /// Check if this policy applies to a specific command.
    pub fn applies_to(&self, command: PolicyCommand) -> bool {
        self.commands.contains(&PolicyCommand::All) || self.commands.contains(&command)
    }

    /// Check if this policy is restrictive.
    pub fn is_restrictive(&self) -> bool {
        self.policy_type == PolicyType::Restrictive
    }

    /// Check if this policy is permissive.
    pub fn is_permissive(&self) -> bool {
        self.policy_type == PolicyType::Permissive
    }

    /// Get the effective roles (PUBLIC if none specified).
    pub fn effective_roles(&self) -> Vec<&str> {
        if self.roles.is_empty() {
            vec!["PUBLIC"]
        } else {
            self.roles.iter().map(|r| r.as_str()).collect()
        }
    }

    /// Get the MSSQL schema (default: "Security").
    pub fn mssql_schema(&self) -> &str {
        self.mssql_schema
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("Security")
    }

    /// Get the predicate function name for MSSQL.
    pub fn mssql_predicate_function_name(&self) -> String {
        format!("fn_{}_predicate", self.name())
    }

    /// Generate the PostgreSQL CREATE POLICY statement.
    pub fn to_sql(&self, table_name: &str) -> String {
        self.to_postgres_sql(table_name)
    }

    /// Generate the PostgreSQL CREATE POLICY statement.
    pub fn to_postgres_sql(&self, table_name: &str) -> String {
        let mut sql = format!("CREATE POLICY {} ON {}", self.name(), table_name);

        // AS PERMISSIVE/RESTRICTIVE
        match self.policy_type {
            PolicyType::Permissive => {} // Default, no need to specify
            PolicyType::Restrictive => sql.push_str(" AS RESTRICTIVE"),
        }

        // FOR command
        if !self.commands.is_empty() && !self.commands.contains(&PolicyCommand::All) {
            let cmds: Vec<&str> = self.commands.iter().map(|c| c.as_str()).collect();
            // PostgreSQL only allows a single command, use first one
            sql.push_str(&format!(" FOR {}", cmds[0]));
        }

        // TO roles
        let roles = self.effective_roles();
        sql.push_str(&format!(" TO {}", roles.join(", ")));

        // USING expression
        if let Some(ref using) = self.using_expr {
            sql.push_str(&format!(" USING ({})", using));
        }

        // WITH CHECK expression
        if let Some(ref check) = self.check_expr {
            sql.push_str(&format!(" WITH CHECK ({})", check));
        }

        sql
    }

    /// Generate SQL Server (MSSQL) security policy statements.
    ///
    /// Returns a tuple of:
    /// 1. CREATE FUNCTION statement for the predicate function
    /// 2. CREATE SECURITY POLICY statement
    ///
    /// # Arguments
    ///
    /// * `table_name` - The fully qualified table name (e.g., "dbo.Users")
    /// * `predicate_column` - The column name to use in the predicate (e.g., "UserId")
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (func_sql, policy_sql) = policy.to_mssql_sql("dbo.Users", "UserId");
    /// ```
    pub fn to_mssql_sql(&self, table_name: &str, predicate_column: &str) -> MssqlPolicyStatements {
        let schema = self.mssql_schema();
        let func_name = self.mssql_predicate_function_name();

        // Generate the predicate function
        let filter_expr = self
            .using_expr
            .as_deref()
            .unwrap_or("1 = 1")
            .replace(
                "current_user_id()",
                "CAST(SESSION_CONTEXT(N'UserId') AS INT)",
            )
            .replace("auth.uid()", "CAST(SESSION_CONTEXT(N'UserId') AS INT)")
            .replace(
                "current_setting('app.current_org')",
                "SESSION_CONTEXT(N'OrgId')",
            );

        let function_sql = format!(
            r#"CREATE FUNCTION {schema}.{func_name}(@{predicate_column} AS INT)
    RETURNS TABLE
WITH SCHEMABINDING
AS
    RETURN SELECT 1 AS fn_securitypredicate_result
    WHERE {filter_expr}"#,
            schema = schema,
            func_name = func_name,
            predicate_column = predicate_column,
            filter_expr = filter_expr
        );

        // Generate the security policy
        let mut policy_sql = format!(
            "CREATE SECURITY POLICY {schema}.{policy_name}\n",
            schema = schema,
            policy_name = self.name()
        );

        // Add FILTER PREDICATE if we have a using expression
        if self.using_expr.is_some() {
            policy_sql.push_str(&format!(
                "ADD FILTER PREDICATE {schema}.{func_name}({predicate_column}) ON {table_name}",
                schema = schema,
                func_name = func_name,
                predicate_column = predicate_column,
                table_name = table_name
            ));
        }

        // Add BLOCK PREDICATE(s) if we have a check expression
        if self.check_expr.is_some() {
            let block_ops = if self.mssql_block_operations.is_empty() {
                // Default block operations based on commands
                self.default_mssql_block_operations()
            } else {
                self.mssql_block_operations.clone()
            };

            for (i, op) in block_ops.iter().enumerate() {
                if i > 0 || self.using_expr.is_some() {
                    policy_sql.push_str(",\n");
                }
                policy_sql.push_str(&format!(
                    "ADD BLOCK PREDICATE {schema}.{func_name}({predicate_column}) ON {table_name} {op}",
                    schema = schema,
                    func_name = func_name,
                    predicate_column = predicate_column,
                    table_name = table_name,
                    op = op.as_str()
                ));
            }
        }

        policy_sql.push_str("\nWITH (STATE = ON)");

        MssqlPolicyStatements {
            schema_sql: format!("CREATE SCHEMA {schema}"),
            function_sql,
            policy_sql,
        }
    }

    /// Get default MSSQL block operations based on the policy commands.
    fn default_mssql_block_operations(&self) -> Vec<MssqlBlockOperation> {
        let mut ops = vec![];

        if self.applies_to(PolicyCommand::Insert) {
            ops.push(MssqlBlockOperation::AfterInsert);
        }
        if self.applies_to(PolicyCommand::Update) {
            ops.push(MssqlBlockOperation::AfterUpdate);
            ops.push(MssqlBlockOperation::BeforeUpdate);
        }
        if self.applies_to(PolicyCommand::Delete) {
            ops.push(MssqlBlockOperation::BeforeDelete);
        }

        ops
    }
}

/// PostgreSQL policy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PolicyType {
    /// Permissive policies are combined with OR.
    /// At least one permissive policy must allow access.
    #[default]
    Permissive,
    /// Restrictive policies are combined with AND.
    /// All restrictive policies must allow access.
    Restrictive,
}

impl PolicyType {
    /// Parse a policy type from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "PERMISSIVE" => Some(Self::Permissive),
            "RESTRICTIVE" => Some(Self::Restrictive),
            _ => None,
        }
    }

    /// Get the SQL keyword for this policy type.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Permissive => "PERMISSIVE",
            Self::Restrictive => "RESTRICTIVE",
        }
    }
}

impl std::fmt::Display for PolicyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// PostgreSQL policy command type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PolicyCommand {
    /// Policy applies to all commands (SELECT, INSERT, UPDATE, DELETE).
    All,
    /// Policy applies to SELECT queries.
    Select,
    /// Policy applies to INSERT statements.
    Insert,
    /// Policy applies to UPDATE statements.
    Update,
    /// Policy applies to DELETE statements.
    Delete,
}

impl PolicyCommand {
    /// Parse a policy command from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "ALL" => Some(Self::All),
            "SELECT" => Some(Self::Select),
            "INSERT" => Some(Self::Insert),
            "UPDATE" => Some(Self::Update),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }

    /// Get the SQL keyword for this command.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }

    /// Check if this command requires a USING expression.
    pub fn requires_using(&self) -> bool {
        matches!(self, Self::All | Self::Select | Self::Update | Self::Delete)
    }

    /// Check if this command requires a WITH CHECK expression.
    pub fn requires_check(&self) -> bool {
        matches!(self, Self::All | Self::Insert | Self::Update)
    }
}

impl std::fmt::Display for PolicyCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// MSSQL-specific block operation types.
///
/// SQL Server's BLOCK PREDICATE can be applied at different points
/// in the data modification lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MssqlBlockOperation {
    /// Block predicate evaluated after INSERT.
    /// Prevents inserting rows that don't satisfy the predicate.
    AfterInsert,
    /// Block predicate evaluated after UPDATE.
    /// Prevents updating rows to values that don't satisfy the predicate.
    AfterUpdate,
    /// Block predicate evaluated before UPDATE.
    /// Prevents updating rows that currently don't satisfy the predicate.
    BeforeUpdate,
    /// Block predicate evaluated before DELETE.
    /// Prevents deleting rows that don't satisfy the predicate.
    BeforeDelete,
}

impl MssqlBlockOperation {
    /// Parse a block operation from a string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().replace([' ', '_'], "").as_str() {
            "AFTERINSERT" => Some(Self::AfterInsert),
            "AFTERUPDATE" => Some(Self::AfterUpdate),
            "BEFOREUPDATE" => Some(Self::BeforeUpdate),
            "BEFOREDELETE" => Some(Self::BeforeDelete),
            _ => None,
        }
    }

    /// Get the SQL clause for this block operation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AfterInsert => "AFTER INSERT",
            Self::AfterUpdate => "AFTER UPDATE",
            Self::BeforeUpdate => "BEFORE UPDATE",
            Self::BeforeDelete => "BEFORE DELETE",
        }
    }
}

impl std::fmt::Display for MssqlBlockOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// SQL statements generated for MSSQL security policies.
#[derive(Debug, Clone, PartialEq)]
pub struct MssqlPolicyStatements {
    /// CREATE SCHEMA statement (if the schema doesn't exist).
    pub schema_sql: String,
    /// CREATE FUNCTION statement for the predicate function.
    pub function_sql: String,
    /// CREATE SECURITY POLICY statement.
    pub policy_sql: String,
}

impl MssqlPolicyStatements {
    /// Get all SQL statements in execution order.
    pub fn all_statements(&self) -> Vec<&str> {
        vec![&self.schema_sql, &self.function_sql, &self.policy_sql]
    }

    /// Get all SQL as a single string with separators.
    pub fn to_sql(&self) -> String {
        format!(
            "{schema_sql};\nGO\n\n{function_sql};\nGO\n\n{policy_sql};",
            schema_sql = self.schema_sql,
            function_sql = self.function_sql,
            policy_sql = self.policy_sql
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_span() -> Span {
        Span::new(0, 10)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    // ==================== Policy Tests ====================

    #[test]
    fn test_policy_new() {
        let policy = Policy::new(make_ident("read_own"), make_ident("User"), make_span());

        assert_eq!(policy.name(), "read_own");
        assert_eq!(policy.table(), "User");
        assert_eq!(policy.policy_type, PolicyType::Permissive);
        assert_eq!(policy.commands, vec![PolicyCommand::All]);
        assert!(policy.roles.is_empty());
        assert!(policy.using_expr.is_none());
        assert!(policy.check_expr.is_none());
        assert!(policy.documentation.is_none());
    }

    #[test]
    fn test_policy_with_type() {
        let policy = Policy::new(make_ident("strict"), make_ident("User"), make_span())
            .with_type(PolicyType::Restrictive);

        assert!(policy.is_restrictive());
        assert!(!policy.is_permissive());
    }

    #[test]
    fn test_policy_with_commands() {
        let policy = Policy::new(make_ident("read"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Select]);

        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(!policy.applies_to(PolicyCommand::Insert));
        assert!(!policy.applies_to(PolicyCommand::Update));
        assert!(!policy.applies_to(PolicyCommand::Delete));
    }

    #[test]
    fn test_policy_with_multiple_commands() {
        let policy = Policy::new(make_ident("read_update"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Select, PolicyCommand::Update]);

        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Update));
        assert!(!policy.applies_to(PolicyCommand::Insert));
        assert!(!policy.applies_to(PolicyCommand::Delete));
    }

    #[test]
    fn test_policy_all_command_applies_to_all() {
        let policy = Policy::new(make_ident("all"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::All]);

        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Insert));
        assert!(policy.applies_to(PolicyCommand::Update));
        assert!(policy.applies_to(PolicyCommand::Delete));
        assert!(policy.applies_to(PolicyCommand::All));
    }

    #[test]
    fn test_policy_add_command() {
        let mut policy =
            Policy::new(make_ident("test"), make_ident("User"), make_span()).with_commands(vec![]);

        policy.add_command(PolicyCommand::Select);
        policy.add_command(PolicyCommand::Update);

        assert_eq!(policy.commands.len(), 2);
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Update));
    }

    #[test]
    fn test_policy_with_roles() {
        let policy = Policy::new(make_ident("auth"), make_ident("User"), make_span())
            .with_roles(vec!["authenticated".into(), "admin".into()]);

        assert_eq!(policy.roles.len(), 2);
        let roles = policy.effective_roles();
        assert!(roles.contains(&"authenticated"));
        assert!(roles.contains(&"admin"));
    }

    #[test]
    fn test_policy_add_role() {
        let mut policy = Policy::new(make_ident("test"), make_ident("User"), make_span());

        policy.add_role("user");
        policy.add_role("moderator");

        assert_eq!(policy.roles.len(), 2);
    }

    #[test]
    fn test_policy_effective_roles_default() {
        let policy = Policy::new(make_ident("public"), make_ident("User"), make_span());

        let roles = policy.effective_roles();
        assert_eq!(roles, vec!["PUBLIC"]);
    }

    #[test]
    fn test_policy_with_using() {
        let policy = Policy::new(make_ident("own"), make_ident("User"), make_span())
            .with_using("user_id = current_user_id()");

        assert_eq!(
            policy.using_expr.as_deref(),
            Some("user_id = current_user_id()")
        );
    }

    #[test]
    fn test_policy_with_check() {
        let policy = Policy::new(make_ident("insert"), make_ident("User"), make_span())
            .with_check("user_id = current_user_id()");

        assert_eq!(
            policy.check_expr.as_deref(),
            Some("user_id = current_user_id()")
        );
    }

    #[test]
    fn test_policy_with_documentation() {
        let policy =
            Policy::new(make_ident("doc"), make_ident("User"), make_span()).with_documentation(
                Documentation::new("Users can only see their own data", make_span()),
            );

        assert!(policy.documentation.is_some());
        assert_eq!(
            policy.documentation.unwrap().text,
            "Users can only see their own data"
        );
    }

    #[test]
    fn test_policy_to_sql_simple() {
        let policy = Policy::new(make_ident("read_own"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Select])
            .with_using("id = current_user_id()");

        let sql = policy.to_sql("users");
        assert!(sql.contains("CREATE POLICY read_own ON users"));
        assert!(sql.contains("FOR SELECT"));
        assert!(sql.contains("TO PUBLIC"));
        assert!(sql.contains("USING (id = current_user_id())"));
    }

    #[test]
    fn test_policy_to_sql_with_roles() {
        let policy = Policy::new(make_ident("auth_read"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Select])
            .with_roles(vec!["authenticated".into()])
            .with_using("true");

        let sql = policy.to_sql("users");
        assert!(sql.contains("TO authenticated"));
    }

    #[test]
    fn test_policy_to_sql_restrictive() {
        let policy = Policy::new(make_ident("restrict"), make_ident("User"), make_span())
            .with_type(PolicyType::Restrictive)
            .with_using("org_id = current_org_id()");

        let sql = policy.to_sql("users");
        assert!(sql.contains("AS RESTRICTIVE"));
    }

    #[test]
    fn test_policy_to_sql_with_check() {
        let policy = Policy::new(make_ident("insert_own"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Insert])
            .with_check("id = current_user_id()");

        let sql = policy.to_sql("users");
        assert!(sql.contains("FOR INSERT"));
        assert!(sql.contains("WITH CHECK (id = current_user_id())"));
    }

    #[test]
    fn test_policy_to_sql_both_expressions() {
        let policy = Policy::new(make_ident("update_own"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Update])
            .with_using("id = current_user_id()")
            .with_check("id = current_user_id()");

        let sql = policy.to_sql("users");
        assert!(sql.contains("USING (id = current_user_id())"));
        assert!(sql.contains("WITH CHECK (id = current_user_id())"));
    }

    #[test]
    fn test_policy_equality() {
        let policy1 = Policy::new(make_ident("test"), make_ident("User"), make_span());
        let policy2 = Policy::new(make_ident("test"), make_ident("User"), make_span());

        assert_eq!(policy1, policy2);
    }

    #[test]
    fn test_policy_clone() {
        let policy = Policy::new(make_ident("original"), make_ident("User"), make_span())
            .with_using("id = 1");

        let cloned = policy.clone();
        assert_eq!(cloned.name(), "original");
        assert_eq!(cloned.using_expr, Some("id = 1".to_string()));
    }

    // ==================== PolicyType Tests ====================

    #[test]
    fn test_policy_type_from_str() {
        assert_eq!(
            PolicyType::from_str("PERMISSIVE"),
            Some(PolicyType::Permissive)
        );
        assert_eq!(
            PolicyType::from_str("permissive"),
            Some(PolicyType::Permissive)
        );
        assert_eq!(
            PolicyType::from_str("Permissive"),
            Some(PolicyType::Permissive)
        );
        assert_eq!(
            PolicyType::from_str("RESTRICTIVE"),
            Some(PolicyType::Restrictive)
        );
        assert_eq!(
            PolicyType::from_str("restrictive"),
            Some(PolicyType::Restrictive)
        );
        assert_eq!(PolicyType::from_str("invalid"), None);
    }

    #[test]
    fn test_policy_type_as_str() {
        assert_eq!(PolicyType::Permissive.as_str(), "PERMISSIVE");
        assert_eq!(PolicyType::Restrictive.as_str(), "RESTRICTIVE");
    }

    #[test]
    fn test_policy_type_display() {
        assert_eq!(format!("{}", PolicyType::Permissive), "PERMISSIVE");
        assert_eq!(format!("{}", PolicyType::Restrictive), "RESTRICTIVE");
    }

    #[test]
    fn test_policy_type_default() {
        let policy_type: PolicyType = Default::default();
        assert_eq!(policy_type, PolicyType::Permissive);
    }

    #[test]
    fn test_policy_type_equality() {
        assert_eq!(PolicyType::Permissive, PolicyType::Permissive);
        assert_eq!(PolicyType::Restrictive, PolicyType::Restrictive);
        assert_ne!(PolicyType::Permissive, PolicyType::Restrictive);
    }

    // ==================== PolicyCommand Tests ====================

    #[test]
    fn test_policy_command_from_str() {
        assert_eq!(PolicyCommand::from_str("ALL"), Some(PolicyCommand::All));
        assert_eq!(PolicyCommand::from_str("all"), Some(PolicyCommand::All));
        assert_eq!(
            PolicyCommand::from_str("SELECT"),
            Some(PolicyCommand::Select)
        );
        assert_eq!(
            PolicyCommand::from_str("select"),
            Some(PolicyCommand::Select)
        );
        assert_eq!(
            PolicyCommand::from_str("INSERT"),
            Some(PolicyCommand::Insert)
        );
        assert_eq!(
            PolicyCommand::from_str("UPDATE"),
            Some(PolicyCommand::Update)
        );
        assert_eq!(
            PolicyCommand::from_str("DELETE"),
            Some(PolicyCommand::Delete)
        );
        assert_eq!(PolicyCommand::from_str("invalid"), None);
    }

    #[test]
    fn test_policy_command_as_str() {
        assert_eq!(PolicyCommand::All.as_str(), "ALL");
        assert_eq!(PolicyCommand::Select.as_str(), "SELECT");
        assert_eq!(PolicyCommand::Insert.as_str(), "INSERT");
        assert_eq!(PolicyCommand::Update.as_str(), "UPDATE");
        assert_eq!(PolicyCommand::Delete.as_str(), "DELETE");
    }

    #[test]
    fn test_policy_command_display() {
        assert_eq!(format!("{}", PolicyCommand::All), "ALL");
        assert_eq!(format!("{}", PolicyCommand::Select), "SELECT");
        assert_eq!(format!("{}", PolicyCommand::Insert), "INSERT");
        assert_eq!(format!("{}", PolicyCommand::Update), "UPDATE");
        assert_eq!(format!("{}", PolicyCommand::Delete), "DELETE");
    }

    #[test]
    fn test_policy_command_requires_using() {
        assert!(PolicyCommand::All.requires_using());
        assert!(PolicyCommand::Select.requires_using());
        assert!(PolicyCommand::Update.requires_using());
        assert!(PolicyCommand::Delete.requires_using());
        assert!(!PolicyCommand::Insert.requires_using());
    }

    #[test]
    fn test_policy_command_requires_check() {
        assert!(PolicyCommand::All.requires_check());
        assert!(PolicyCommand::Insert.requires_check());
        assert!(PolicyCommand::Update.requires_check());
        assert!(!PolicyCommand::Select.requires_check());
        assert!(!PolicyCommand::Delete.requires_check());
    }

    #[test]
    fn test_policy_command_equality() {
        assert_eq!(PolicyCommand::Select, PolicyCommand::Select);
        assert_ne!(PolicyCommand::Select, PolicyCommand::Insert);
    }

    // ==================== Full Policy Scenario Tests ====================

    #[test]
    fn test_policy_rls_scenario_user_isolation() {
        // Scenario: Users can only see and modify their own records
        let policy = Policy::new(
            make_ident("user_isolation"),
            make_ident("User"),
            make_span(),
        )
        .with_type(PolicyType::Permissive)
        .with_commands(vec![PolicyCommand::All])
        .with_roles(vec!["authenticated".into()])
        .with_using("id = auth.uid()")
        .with_check("id = auth.uid()");

        assert!(policy.is_permissive());
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Insert));
        assert!(policy.applies_to(PolicyCommand::Update));
        assert!(policy.applies_to(PolicyCommand::Delete));

        let sql = policy.to_sql("users");
        assert!(sql.contains("auth.uid()"));
    }

    #[test]
    fn test_policy_rls_scenario_org_based() {
        // Scenario: Users can only access records in their organization
        let policy = Policy::new(
            make_ident("org_access"),
            make_ident("Document"),
            make_span(),
        )
        .with_type(PolicyType::Restrictive)
        .with_commands(vec![PolicyCommand::Select, PolicyCommand::Update])
        .with_using("org_id = current_setting('app.current_org')::uuid");

        assert!(policy.is_restrictive());
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Update));
        assert!(!policy.applies_to(PolicyCommand::Delete));

        let sql = policy.to_sql("documents");
        assert!(sql.contains("AS RESTRICTIVE"));
        assert!(sql.contains("current_setting"));
    }

    #[test]
    fn test_policy_rls_scenario_public_read() {
        // Scenario: Anyone can read, only owner can modify
        let read_policy = Policy::new(make_ident("public_read"), make_ident("Post"), make_span())
            .with_commands(vec![PolicyCommand::Select])
            .with_using("published = true OR author_id = current_user_id()");

        let write_policy = Policy::new(make_ident("owner_write"), make_ident("Post"), make_span())
            .with_commands(vec![PolicyCommand::Update, PolicyCommand::Delete])
            .with_roles(vec!["authenticated".into()])
            .with_using("author_id = current_user_id()");

        assert_eq!(read_policy.effective_roles(), vec!["PUBLIC"]);
        assert!(write_policy.effective_roles().contains(&"authenticated"));
    }

    // ==================== MSSQL Block Operation Tests ====================

    #[test]
    fn test_mssql_block_operation_from_str() {
        assert_eq!(
            MssqlBlockOperation::from_str("AFTER INSERT"),
            Some(MssqlBlockOperation::AfterInsert)
        );
        assert_eq!(
            MssqlBlockOperation::from_str("after_insert"),
            Some(MssqlBlockOperation::AfterInsert)
        );
        assert_eq!(
            MssqlBlockOperation::from_str("AFTERINSERT"),
            Some(MssqlBlockOperation::AfterInsert)
        );
        assert_eq!(
            MssqlBlockOperation::from_str("AFTER UPDATE"),
            Some(MssqlBlockOperation::AfterUpdate)
        );
        assert_eq!(
            MssqlBlockOperation::from_str("BEFORE UPDATE"),
            Some(MssqlBlockOperation::BeforeUpdate)
        );
        assert_eq!(
            MssqlBlockOperation::from_str("BEFORE DELETE"),
            Some(MssqlBlockOperation::BeforeDelete)
        );
        assert_eq!(MssqlBlockOperation::from_str("invalid"), None);
    }

    #[test]
    fn test_mssql_block_operation_as_str() {
        assert_eq!(MssqlBlockOperation::AfterInsert.as_str(), "AFTER INSERT");
        assert_eq!(MssqlBlockOperation::AfterUpdate.as_str(), "AFTER UPDATE");
        assert_eq!(MssqlBlockOperation::BeforeUpdate.as_str(), "BEFORE UPDATE");
        assert_eq!(MssqlBlockOperation::BeforeDelete.as_str(), "BEFORE DELETE");
    }

    #[test]
    fn test_mssql_block_operation_display() {
        assert_eq!(
            format!("{}", MssqlBlockOperation::AfterInsert),
            "AFTER INSERT"
        );
        assert_eq!(
            format!("{}", MssqlBlockOperation::BeforeDelete),
            "BEFORE DELETE"
        );
    }

    // ==================== MSSQL Policy Tests ====================

    #[test]
    fn test_policy_mssql_schema_default() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span());
        assert_eq!(policy.mssql_schema(), "Security");
    }

    #[test]
    fn test_policy_with_mssql_schema() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_mssql_schema("RLS");
        assert_eq!(policy.mssql_schema(), "RLS");
    }

    #[test]
    fn test_policy_mssql_predicate_function_name() {
        let policy = Policy::new(make_ident("user_filter"), make_ident("User"), make_span());
        assert_eq!(
            policy.mssql_predicate_function_name(),
            "fn_user_filter_predicate"
        );
    }

    #[test]
    fn test_policy_with_mssql_block_operations() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_mssql_block_operations(vec![
                MssqlBlockOperation::AfterInsert,
                MssqlBlockOperation::AfterUpdate,
            ]);

        assert_eq!(policy.mssql_block_operations.len(), 2);
    }

    #[test]
    fn test_policy_add_mssql_block_operation() {
        let mut policy = Policy::new(make_ident("test"), make_ident("User"), make_span());

        policy.add_mssql_block_operation(MssqlBlockOperation::AfterInsert);
        policy.add_mssql_block_operation(MssqlBlockOperation::BeforeDelete);

        assert_eq!(policy.mssql_block_operations.len(), 2);
    }

    #[test]
    fn test_policy_to_mssql_sql_simple() {
        let policy = Policy::new(make_ident("user_filter"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Select])
            .with_using("UserId = @UserId");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        // Check schema creation
        assert!(mssql.schema_sql.contains("CREATE SCHEMA Security"));

        // Check function creation
        assert!(
            mssql
                .function_sql
                .contains("CREATE FUNCTION Security.fn_user_filter_predicate")
        );
        assert!(mssql.function_sql.contains("@UserId AS INT"));
        assert!(mssql.function_sql.contains("WITH SCHEMABINDING"));
        assert!(mssql.function_sql.contains("RETURNS TABLE"));

        // Check policy creation
        assert!(
            mssql
                .policy_sql
                .contains("CREATE SECURITY POLICY Security.user_filter")
        );
        assert!(mssql.policy_sql.contains("FILTER PREDICATE"));
        assert!(mssql.policy_sql.contains("ON dbo.Users"));
        assert!(mssql.policy_sql.contains("WITH (STATE = ON)"));
    }

    #[test]
    fn test_policy_to_mssql_sql_with_check() {
        let policy = Policy::new(make_ident("user_insert"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Insert])
            .with_check("UserId = @UserId");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        assert!(mssql.policy_sql.contains("BLOCK PREDICATE"));
        assert!(mssql.policy_sql.contains("AFTER INSERT"));
    }

    #[test]
    fn test_policy_to_mssql_sql_with_both() {
        let policy = Policy::new(make_ident("user_all"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::All])
            .with_using("UserId = @UserId")
            .with_check("UserId = @UserId");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        assert!(mssql.policy_sql.contains("FILTER PREDICATE"));
        assert!(mssql.policy_sql.contains("BLOCK PREDICATE"));
        assert!(mssql.policy_sql.contains("AFTER INSERT"));
        assert!(mssql.policy_sql.contains("AFTER UPDATE"));
        assert!(mssql.policy_sql.contains("BEFORE UPDATE"));
        assert!(mssql.policy_sql.contains("BEFORE DELETE"));
    }

    #[test]
    fn test_policy_to_mssql_sql_custom_schema() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_mssql_schema("RLS")
            .with_using("UserId = @UserId");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        assert!(mssql.schema_sql.contains("CREATE SCHEMA RLS"));
        assert!(mssql.function_sql.contains("RLS.fn_test_predicate"));
        assert!(mssql.policy_sql.contains("RLS.test"));
    }

    #[test]
    fn test_policy_to_mssql_sql_translates_postgres_functions() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_using("id = current_user_id()");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        assert!(mssql.function_sql.contains("SESSION_CONTEXT(N'UserId')"));
        assert!(!mssql.function_sql.contains("current_user_id"));
    }

    #[test]
    fn test_policy_to_mssql_sql_translates_auth_uid() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_using("id = auth.uid()");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        assert!(mssql.function_sql.contains("SESSION_CONTEXT(N'UserId')"));
        assert!(!mssql.function_sql.contains("auth.uid"));
    }

    #[test]
    fn test_mssql_policy_statements_all_statements() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_using("UserId = @UserId");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");
        let statements = mssql.all_statements();

        assert_eq!(statements.len(), 3);
    }

    #[test]
    fn test_mssql_policy_statements_to_sql() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_using("UserId = @UserId");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");
        let full_sql = mssql.to_sql();

        assert!(full_sql.contains("GO"));
        assert!(full_sql.contains("CREATE SCHEMA"));
        assert!(full_sql.contains("CREATE FUNCTION"));
        assert!(full_sql.contains("CREATE SECURITY POLICY"));
    }

    #[test]
    fn test_policy_default_mssql_block_operations() {
        // Test INSERT only
        let insert_policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Insert])
            .with_check("true");

        let mssql = insert_policy.to_mssql_sql("dbo.Users", "UserId");
        assert!(mssql.policy_sql.contains("AFTER INSERT"));
        assert!(!mssql.policy_sql.contains("BEFORE DELETE"));

        // Test UPDATE only
        let update_policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Update])
            .with_check("true");

        let mssql = update_policy.to_mssql_sql("dbo.Users", "UserId");
        assert!(mssql.policy_sql.contains("AFTER UPDATE"));
        assert!(mssql.policy_sql.contains("BEFORE UPDATE"));
        assert!(!mssql.policy_sql.contains("AFTER INSERT"));

        // Test DELETE only
        let delete_policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::Delete])
            .with_check("true");

        let mssql = delete_policy.to_mssql_sql("dbo.Users", "UserId");
        assert!(mssql.policy_sql.contains("BEFORE DELETE"));
        assert!(!mssql.policy_sql.contains("AFTER INSERT"));
    }

    #[test]
    fn test_policy_mssql_custom_block_operations() {
        let policy = Policy::new(make_ident("test"), make_ident("User"), make_span())
            .with_commands(vec![PolicyCommand::All])
            .with_check("true")
            .with_mssql_block_operations(vec![MssqlBlockOperation::AfterInsert]);

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        // Should only have the custom operation, not all defaults
        assert!(mssql.policy_sql.contains("AFTER INSERT"));
        assert!(!mssql.policy_sql.contains("BEFORE DELETE"));
        assert!(!mssql.policy_sql.contains("AFTER UPDATE"));
    }

    // ==================== MSSQL Real-World Scenario Tests ====================

    #[test]
    fn test_mssql_rls_scenario_user_isolation() {
        let policy = Policy::new(
            make_ident("user_isolation"),
            make_ident("User"),
            make_span(),
        )
        .with_mssql_schema("Security")
        .with_commands(vec![PolicyCommand::All])
        .with_using("UserId = CAST(SESSION_CONTEXT(N'UserId') AS INT)")
        .with_check("UserId = CAST(SESSION_CONTEXT(N'UserId') AS INT)");

        let mssql = policy.to_mssql_sql("dbo.Users", "UserId");

        // Verify complete MSSQL RLS setup
        assert!(mssql.schema_sql.contains("CREATE SCHEMA Security"));
        assert!(mssql.function_sql.contains("fn_user_isolation_predicate"));
        assert!(mssql.policy_sql.contains("user_isolation"));
        assert!(mssql.policy_sql.contains("WITH (STATE = ON)"));
    }

    #[test]
    fn test_mssql_rls_scenario_multi_tenant() {
        let policy = Policy::new(
            make_ident("tenant_isolation"),
            make_ident("Order"),
            make_span(),
        )
        .with_mssql_schema("MultiTenant")
        .with_using("TenantId = CAST(SESSION_CONTEXT(N'TenantId') AS INT)")
        .with_check("TenantId = CAST(SESSION_CONTEXT(N'TenantId') AS INT)");

        let mssql = policy.to_mssql_sql("dbo.Orders", "TenantId");

        assert!(mssql.schema_sql.contains("MultiTenant"));
        assert!(mssql.function_sql.contains("@TenantId AS INT"));
        assert!(mssql.policy_sql.contains("dbo.Orders"));
    }
}
