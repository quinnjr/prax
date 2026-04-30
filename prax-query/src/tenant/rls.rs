//! PostgreSQL Row-Level Security (RLS) integration.
//!
//! This module provides high-performance RLS support for multi-tenant applications
//! using PostgreSQL's native row-level security features.
//!
//! # Performance Benefits
//!
//! Using database-level RLS provides:
//! - **Zero application overhead** - Filtering happens in the database engine
//! - **Guaranteed isolation** - Even raw SQL queries are filtered
//! - **Index utilization** - RLS policies can use indexes efficiently
//! - **Prepared statement caching** - Same statements work for all tenants
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::tenant::rls::{RlsManager, RlsPolicy};
//!
//! // Create RLS manager
//! let rls = RlsManager::new("tenant_id", "app.current_tenant");
//!
//! // Generate setup SQL
//! let setup = rls.setup_sql(&["users", "orders", "products"]);
//! conn.execute_batch(&setup).await?;
//!
//! // Set tenant context for session
//! rls.set_tenant_sql("tenant-123");
//! ```

use std::collections::HashSet;
use std::fmt::Write;

/// Configuration for PostgreSQL RLS.
#[derive(Debug, Clone)]
pub struct RlsConfig {
    /// The tenant ID column name.
    pub tenant_column: String,
    /// PostgreSQL setting name for current tenant (e.g., "app.current_tenant").
    pub session_variable: String,
    /// Role to apply policies to.
    pub application_role: Option<String>,
    /// Tables to enable RLS on.
    pub tables: HashSet<String>,
    /// Tables excluded from RLS (e.g., shared lookup tables).
    pub excluded_tables: HashSet<String>,
    /// Whether to use BYPASSRLS for admin operations.
    pub allow_bypass: bool,
    /// Policy name prefix.
    pub policy_prefix: String,
}

impl Default for RlsConfig {
    fn default() -> Self {
        Self {
            tenant_column: "tenant_id".to_string(),
            session_variable: "app.current_tenant".to_string(),
            application_role: None,
            tables: HashSet::new(),
            excluded_tables: HashSet::new(),
            allow_bypass: true,
            policy_prefix: "tenant_isolation".to_string(),
        }
    }
}

impl RlsConfig {
    /// Create a new RLS config with the given tenant column.
    pub fn new(tenant_column: impl Into<String>) -> Self {
        Self {
            tenant_column: tenant_column.into(),
            ..Default::default()
        }
    }

    /// Set the session variable name.
    pub fn with_session_variable(mut self, var: impl Into<String>) -> Self {
        self.session_variable = var.into();
        self
    }

    /// Set the application role.
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.application_role = Some(role.into());
        self
    }

    /// Add a table for RLS.
    pub fn add_table(mut self, table: impl Into<String>) -> Self {
        self.tables.insert(table.into());
        self
    }

    /// Add multiple tables for RLS.
    pub fn add_tables<I, S>(mut self, tables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tables.extend(tables.into_iter().map(Into::into));
        self
    }

    /// Exclude a table from RLS.
    pub fn exclude_table(mut self, table: impl Into<String>) -> Self {
        self.excluded_tables.insert(table.into());
        self
    }

    /// Disable bypass for admin.
    pub fn without_bypass(mut self) -> Self {
        self.allow_bypass = false;
        self
    }

    /// Set the policy prefix.
    pub fn with_policy_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.policy_prefix = prefix.into();
        self
    }
}

/// Manager for PostgreSQL RLS operations.
#[derive(Debug, Clone)]
pub struct RlsManager {
    config: RlsConfig,
}

impl RlsManager {
    /// Create a new RLS manager with the given config.
    pub fn new(config: RlsConfig) -> Self {
        Self { config }
    }

    /// Create with simple defaults.
    pub fn simple(tenant_column: impl Into<String>, session_var: impl Into<String>) -> Self {
        Self::new(RlsConfig::new(tenant_column).with_session_variable(session_var))
    }

    /// Get the config.
    pub fn config(&self) -> &RlsConfig {
        &self.config
    }

    /// Generate SQL to enable RLS on a table.
    pub fn enable_rls_sql(&self, table: &str) -> String {
        format!(
            "ALTER TABLE {} ENABLE ROW LEVEL SECURITY;",
            quote_ident(table)
        )
    }

    /// Generate SQL to force RLS even for table owners.
    pub fn force_rls_sql(&self, table: &str) -> String {
        format!(
            "ALTER TABLE {} FORCE ROW LEVEL SECURITY;",
            quote_ident(table)
        )
    }

    /// Generate SQL for the tenant isolation policy.
    pub fn create_policy_sql(&self, table: &str) -> String {
        let policy_name = format!("{}_{}", self.config.policy_prefix, table);
        let role = self.config.application_role.as_deref().unwrap_or("PUBLIC");

        // Create policy that filters by tenant_id = current_setting('app.current_tenant')
        format!(
            r#"CREATE POLICY {} ON {}
    AS PERMISSIVE
    FOR ALL
    TO {}
    USING ({} = current_setting('{}')::text)
    WITH CHECK ({} = current_setting('{}')::text);"#,
            quote_ident(&policy_name),
            quote_ident(table),
            role,
            quote_ident(&self.config.tenant_column),
            self.config.session_variable,
            quote_ident(&self.config.tenant_column),
            self.config.session_variable,
        )
    }

    /// Generate SQL for UUID tenant columns.
    pub fn create_uuid_policy_sql(&self, table: &str) -> String {
        let policy_name = format!("{}_{}", self.config.policy_prefix, table);
        let role = self.config.application_role.as_deref().unwrap_or("PUBLIC");

        format!(
            r#"CREATE POLICY {} ON {}
    AS PERMISSIVE
    FOR ALL
    TO {}
    USING ({} = current_setting('{}')::uuid)
    WITH CHECK ({} = current_setting('{}')::uuid);"#,
            quote_ident(&policy_name),
            quote_ident(table),
            role,
            quote_ident(&self.config.tenant_column),
            self.config.session_variable,
            quote_ident(&self.config.tenant_column),
            self.config.session_variable,
        )
    }

    /// Generate SQL to drop a policy.
    pub fn drop_policy_sql(&self, table: &str) -> String {
        let policy_name = format!("{}_{}", self.config.policy_prefix, table);
        format!(
            "DROP POLICY IF EXISTS {} ON {};",
            quote_ident(&policy_name),
            quote_ident(table)
        )
    }

    /// Generate SQL to set the current tenant for a session.
    pub fn set_tenant_sql(&self, tenant_id: &str) -> String {
        format!(
            "SET {} = '{}';",
            self.config.session_variable,
            tenant_id.replace('\'', "''")
        )
    }

    /// Generate SQL to set the current tenant locally (transaction only).
    pub fn set_tenant_local_sql(&self, tenant_id: &str) -> String {
        format!(
            "SET LOCAL {} = '{}';",
            self.config.session_variable,
            tenant_id.replace('\'', "''")
        )
    }

    /// Generate SQL to reset the tenant context.
    pub fn reset_tenant_sql(&self) -> String {
        format!("RESET {};", self.config.session_variable)
    }

    /// Generate SQL to check the current tenant.
    pub fn current_tenant_sql(&self) -> String {
        format!(
            "SELECT current_setting('{}', true);",
            self.config.session_variable
        )
    }

    /// Generate complete setup SQL for all configured tables.
    pub fn setup_sql(&self) -> String {
        let mut sql = String::with_capacity(4096);

        // Header
        writeln!(sql, "-- Prax Multi-Tenant RLS Setup").unwrap();
        writeln!(
            sql,
            "-- Generated for column: {}",
            self.config.tenant_column
        )
        .unwrap();
        writeln!(sql, "-- Session variable: {}", self.config.session_variable).unwrap();
        writeln!(sql).unwrap();

        // Create admin role if bypass is enabled
        if self.config.allow_bypass
            && let Some(ref role) = self.config.application_role
        {
            writeln!(sql, "-- Admin role with BYPASSRLS").unwrap();
            writeln!(sql, "DO $$").unwrap();
            writeln!(sql, "BEGIN").unwrap();
            writeln!(sql, "    CREATE ROLE {}_admin WITH BYPASSRLS;", role).unwrap();
            writeln!(sql, "EXCEPTION WHEN duplicate_object THEN NULL;").unwrap();
            writeln!(sql, "END $$;").unwrap();
            writeln!(sql).unwrap();
        }

        // Enable RLS and create policies for each table
        for table in &self.config.tables {
            if self.config.excluded_tables.contains(table) {
                continue;
            }

            writeln!(sql, "-- Table: {}", table).unwrap();
            writeln!(sql, "{}", self.enable_rls_sql(table)).unwrap();
            writeln!(sql, "{}", self.force_rls_sql(table)).unwrap();
            writeln!(sql, "{}", self.drop_policy_sql(table)).unwrap();
            writeln!(sql, "{}", self.create_policy_sql(table)).unwrap();
            writeln!(sql).unwrap();
        }

        sql
    }

    /// Generate migration SQL to add RLS to a new table.
    pub fn migration_up_sql(&self, table: &str) -> String {
        let mut sql = String::with_capacity(512);

        writeln!(sql, "-- Enable RLS on {}", table).unwrap();
        writeln!(sql, "{}", self.enable_rls_sql(table)).unwrap();
        writeln!(sql, "{}", self.force_rls_sql(table)).unwrap();
        writeln!(sql, "{}", self.create_policy_sql(table)).unwrap();

        sql
    }

    /// Generate migration SQL to remove RLS from a table.
    pub fn migration_down_sql(&self, table: &str) -> String {
        let mut sql = String::with_capacity(256);

        writeln!(sql, "-- Disable RLS on {}", table).unwrap();
        writeln!(sql, "{}", self.drop_policy_sql(table)).unwrap();
        writeln!(
            sql,
            "ALTER TABLE {} DISABLE ROW LEVEL SECURITY;",
            quote_ident(table)
        )
        .unwrap();

        sql
    }
}

/// Builder for RLS manager.
#[derive(Default)]
pub struct RlsManagerBuilder {
    config: RlsConfig,
}

impl RlsManagerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the tenant column.
    pub fn tenant_column(mut self, column: impl Into<String>) -> Self {
        self.config.tenant_column = column.into();
        self
    }

    /// Set the session variable.
    pub fn session_variable(mut self, var: impl Into<String>) -> Self {
        self.config.session_variable = var.into();
        self
    }

    /// Set the application role.
    pub fn application_role(mut self, role: impl Into<String>) -> Self {
        self.config.application_role = Some(role.into());
        self
    }

    /// Add tables.
    pub fn tables<I, S>(mut self, tables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config
            .tables
            .extend(tables.into_iter().map(Into::into));
        self
    }

    /// Exclude tables.
    pub fn exclude<I, S>(mut self, tables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config
            .excluded_tables
            .extend(tables.into_iter().map(Into::into));
        self
    }

    /// Set policy prefix.
    pub fn policy_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.config.policy_prefix = prefix.into();
        self
    }

    /// Build the manager.
    pub fn build(self) -> RlsManager {
        RlsManager::new(self.config)
    }
}

/// Represents a custom RLS policy.
#[derive(Debug, Clone)]
pub struct RlsPolicy {
    /// Policy name.
    pub name: String,
    /// Table the policy applies to.
    pub table: String,
    /// Command the policy applies to (ALL, SELECT, INSERT, UPDATE, DELETE).
    pub command: PolicyCommand,
    /// Role the policy applies to.
    pub role: Option<String>,
    /// USING expression (for SELECT, UPDATE, DELETE).
    pub using_expr: Option<String>,
    /// WITH CHECK expression (for INSERT, UPDATE).
    pub with_check_expr: Option<String>,
    /// Whether this is a permissive or restrictive policy.
    pub permissive: bool,
}

/// SQL command that a policy applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCommand {
    All,
    Select,
    Insert,
    Update,
    Delete,
}

impl PolicyCommand {
    fn as_str(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }
}

impl RlsPolicy {
    /// Create a new policy.
    pub fn new(name: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            table: table.into(),
            command: PolicyCommand::All,
            role: None,
            using_expr: None,
            with_check_expr: None,
            permissive: true,
        }
    }

    /// Set the command.
    pub fn command(mut self, cmd: PolicyCommand) -> Self {
        self.command = cmd;
        self
    }

    /// Set the role.
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    /// Set the USING expression.
    pub fn using(mut self, expr: impl Into<String>) -> Self {
        self.using_expr = Some(expr.into());
        self
    }

    /// Set the WITH CHECK expression.
    pub fn with_check(mut self, expr: impl Into<String>) -> Self {
        self.with_check_expr = Some(expr.into());
        self
    }

    /// Make this a restrictive policy.
    pub fn restrictive(mut self) -> Self {
        self.permissive = false;
        self
    }

    /// Generate the CREATE POLICY SQL.
    pub fn to_sql(&self) -> String {
        let mut sql = String::with_capacity(256);

        let policy_type = if self.permissive {
            "PERMISSIVE"
        } else {
            "RESTRICTIVE"
        };

        write!(
            sql,
            "CREATE POLICY {} ON {}\n    AS {}\n    FOR {}\n    TO {}",
            quote_ident(&self.name),
            quote_ident(&self.table),
            policy_type,
            self.command.as_str(),
            self.role.as_deref().unwrap_or("PUBLIC"),
        )
        .unwrap();

        if let Some(ref using) = self.using_expr {
            write!(sql, "\n    USING ({})", using).unwrap();
        }

        if let Some(ref check) = self.with_check_expr {
            write!(sql, "\n    WITH CHECK ({})", check).unwrap();
        }

        sql.push(';');
        sql
    }
}

/// Quote a PostgreSQL identifier.
fn quote_ident(name: &str) -> String {
    // Simple quoting - in production, use proper escaping
    if name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.is_empty()
        && !name.chars().next().unwrap().is_ascii_digit()
    {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Context guard that sets tenant for the duration of its lifetime.
///
/// Uses PostgreSQL's SET LOCAL to ensure the setting only applies to
/// the current transaction.
pub struct TenantGuard {
    reset_sql: String,
}

impl TenantGuard {
    /// Create a new tenant guard.
    ///
    /// The caller should execute `set_sql()` before using the connection.
    pub fn new(session_var: &str, tenant_id: &str) -> (Self, String) {
        let set_sql = format!(
            "SET LOCAL {} = '{}';",
            session_var,
            tenant_id.replace('\'', "''")
        );
        let reset_sql = format!("RESET {};", session_var);

        (Self { reset_sql }, set_sql)
    }

    /// Get the SQL to reset the tenant context.
    pub fn reset_sql(&self) -> &str {
        &self.reset_sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rls_config() {
        let config = RlsConfig::new("org_id")
            .with_session_variable("app.org")
            .with_role("app_user")
            .add_tables(["users", "orders", "products"]);

        assert_eq!(config.tenant_column, "org_id");
        assert_eq!(config.session_variable, "app.org");
        assert!(config.tables.contains("users"));
        assert!(config.tables.contains("orders"));
    }

    #[test]
    fn test_set_tenant_sql() {
        let manager = RlsManager::simple("tenant_id", "app.tenant");

        assert_eq!(
            manager.set_tenant_sql("tenant-123"),
            "SET app.tenant = 'tenant-123';"
        );

        // Test SQL injection prevention
        assert_eq!(
            manager.set_tenant_sql("'; DROP TABLE users; --"),
            "SET app.tenant = '''; DROP TABLE users; --';"
        );
    }

    #[test]
    fn test_create_policy_sql() {
        let manager = RlsManager::simple("tenant_id", "app.current_tenant");

        let sql = manager.create_policy_sql("users");
        assert!(sql.contains("CREATE POLICY"));
        assert!(sql.contains("tenant_id = current_setting('app.current_tenant')"));
    }

    #[test]
    fn test_setup_sql() {
        let config = RlsConfig::new("tenant_id")
            .with_session_variable("app.tenant")
            .add_tables(["users", "orders"]);

        let manager = RlsManager::new(config);
        let sql = manager.setup_sql();

        assert!(sql.contains("ENABLE ROW LEVEL SECURITY"));
        assert!(sql.contains("FORCE ROW LEVEL SECURITY"));
        assert!(sql.contains("CREATE POLICY"));
    }

    #[test]
    fn test_custom_policy() {
        let policy = RlsPolicy::new("owner_access", "documents")
            .command(PolicyCommand::All)
            .role("app_user")
            .using("owner_id = current_user_id()")
            .with_check("owner_id = current_user_id()");

        let sql = policy.to_sql();
        assert!(sql.contains("CREATE POLICY owner_access"));
        assert!(sql.contains("FOR ALL"));
        assert!(sql.contains("USING (owner_id = current_user_id())"));
    }

    #[test]
    fn test_migration_sql() {
        let manager = RlsManager::simple("tenant_id", "app.tenant");

        let up = manager.migration_up_sql("invoices");
        assert!(up.contains("ENABLE ROW LEVEL SECURITY"));
        assert!(up.contains("CREATE POLICY"));

        let down = manager.migration_down_sql("invoices");
        assert!(down.contains("DROP POLICY"));
        assert!(down.contains("DISABLE ROW LEVEL SECURITY"));
    }

    #[test]
    fn test_quote_ident() {
        assert_eq!(quote_ident("users"), "users");
        assert_eq!(quote_ident("user-data"), "\"user-data\"");
        assert_eq!(quote_ident("User"), "\"User\"");
        assert_eq!(quote_ident("table\"name"), "\"table\"\"name\"");
    }
}
