//! Security and access control support.
//!
//! This module provides types for database security features including:
//! - Row-Level Security (RLS) policies
//! - Column-level grants
//! - Role and user management
//! - Connection profiles
//! - Data masking
//!
//! # Database Support
//!
//! | Feature           | PostgreSQL | MySQL         | SQLite | MSSQL | MongoDB        |
//! |-------------------|------------|---------------|--------|-------|----------------|
//! | Row-Level Security| ✅         | ❌            | ❌     | ✅    | ✅ Field-level |
//! | Column Grants     | ✅         | ✅            | ❌     | ✅    | ✅             |
//! | Roles & Users     | ✅         | ✅            | ❌     | ✅    | ✅             |
//! | Connection Profiles| ✅        | ✅            | ❌     | ✅    | ✅             |
//! | Data Masking      | ✅         | ✅ Enterprise | ❌     | ✅    | ✅             |

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};
use crate::sql::DatabaseType;

// ============================================================================
// Row-Level Security (RLS)
// ============================================================================

/// A Row-Level Security policy definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RlsPolicy {
    /// Policy name.
    pub name: String,
    /// Table the policy applies to.
    pub table: String,
    /// Policy command (SELECT, INSERT, UPDATE, DELETE, ALL).
    pub command: PolicyCommand,
    /// Role(s) the policy applies to.
    pub roles: Vec<String>,
    /// USING expression (for SELECT, UPDATE, DELETE).
    pub using: Option<String>,
    /// WITH CHECK expression (for INSERT, UPDATE).
    pub with_check: Option<String>,
    /// Whether this is a permissive or restrictive policy.
    pub permissive: bool,
}

/// Commands a policy applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyCommand {
    /// All commands.
    All,
    /// SELECT only.
    Select,
    /// INSERT only.
    Insert,
    /// UPDATE only.
    Update,
    /// DELETE only.
    Delete,
}

impl PolicyCommand {
    /// Convert to SQL keyword.
    pub fn to_sql(&self) -> &'static str {
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
    /// Create a new RLS policy.
    pub fn new(name: impl Into<String>, table: impl Into<String>) -> RlsPolicyBuilder {
        RlsPolicyBuilder::new(name, table)
    }

    /// Generate PostgreSQL CREATE POLICY SQL.
    pub fn to_postgres_sql(&self) -> String {
        let mut sql = format!(
            "CREATE POLICY {} ON {} AS {} FOR {}",
            self.name,
            self.table,
            if self.permissive {
                "PERMISSIVE"
            } else {
                "RESTRICTIVE"
            },
            self.command.to_sql()
        );

        if !self.roles.is_empty() && self.roles != vec!["PUBLIC"] {
            sql.push_str(" TO ");
            sql.push_str(&self.roles.join(", "));
        }

        if let Some(ref using) = self.using {
            sql.push_str(" USING (");
            sql.push_str(using);
            sql.push(')');
        }

        if let Some(ref check) = self.with_check {
            sql.push_str(" WITH CHECK (");
            sql.push_str(check);
            sql.push(')');
        }

        sql
    }

    /// Generate MSSQL security predicate SQL.
    pub fn to_mssql_sql(&self) -> Vec<String> {
        let mut sqls = Vec::new();

        // Create security function
        let func_name = format!("fn_rls_{}", self.name);
        if let Some(ref using) = self.using {
            sqls.push(format!(
                "CREATE FUNCTION dbo.{fn}(@tenant_id INT) \
                 RETURNS TABLE WITH SCHEMABINDING AS \
                 RETURN SELECT 1 AS result WHERE {expr}",
                fn = func_name,
                expr = using
            ));
        }

        // Create security policy
        sqls.push(format!(
            "CREATE SECURITY POLICY {name}_policy \
             ADD FILTER PREDICATE dbo.{fn}(tenant_id) ON dbo.{table}, \
             ADD BLOCK PREDICATE dbo.{fn}(tenant_id) ON dbo.{table} \
             WITH (STATE = ON)",
            name = self.name,
            fn = func_name,
            table = self.table
        ));

        sqls
    }

    /// Generate DROP POLICY SQL.
    pub fn to_drop_sql(&self, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => {
                format!("DROP POLICY IF EXISTS {} ON {}", self.name, self.table)
            }
            DatabaseType::MSSQL => format!("DROP SECURITY POLICY IF EXISTS {}_policy", self.name),
            _ => String::new(),
        }
    }
}

/// Builder for RLS policies.
#[derive(Debug, Clone)]
pub struct RlsPolicyBuilder {
    name: String,
    table: String,
    command: PolicyCommand,
    roles: Vec<String>,
    using: Option<String>,
    with_check: Option<String>,
    permissive: bool,
}

impl RlsPolicyBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            table: table.into(),
            command: PolicyCommand::All,
            roles: vec!["PUBLIC".to_string()],
            using: None,
            with_check: None,
            permissive: true,
        }
    }

    /// Set the command this policy applies to.
    pub fn for_command(mut self, cmd: PolicyCommand) -> Self {
        self.command = cmd;
        self
    }

    /// For SELECT only.
    pub fn for_select(self) -> Self {
        self.for_command(PolicyCommand::Select)
    }

    /// For INSERT only.
    pub fn for_insert(self) -> Self {
        self.for_command(PolicyCommand::Insert)
    }

    /// For UPDATE only.
    pub fn for_update(self) -> Self {
        self.for_command(PolicyCommand::Update)
    }

    /// For DELETE only.
    pub fn for_delete(self) -> Self {
        self.for_command(PolicyCommand::Delete)
    }

    /// Set the roles this policy applies to.
    pub fn to_roles<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.roles = roles.into_iter().map(Into::into).collect();
        self
    }

    /// Set the USING expression.
    pub fn using(mut self, expr: impl Into<String>) -> Self {
        self.using = Some(expr.into());
        self
    }

    /// Set the WITH CHECK expression.
    pub fn with_check(mut self, expr: impl Into<String>) -> Self {
        self.with_check = Some(expr.into());
        self
    }

    /// Make this a restrictive policy.
    pub fn restrictive(mut self) -> Self {
        self.permissive = false;
        self
    }

    /// Make this a permissive policy (default).
    pub fn permissive(mut self) -> Self {
        self.permissive = true;
        self
    }

    /// Build the policy.
    pub fn build(self) -> RlsPolicy {
        RlsPolicy {
            name: self.name,
            table: self.table,
            command: self.command,
            roles: self.roles,
            using: self.using,
            with_check: self.with_check,
            permissive: self.permissive,
        }
    }
}

/// Multi-tenant RLS configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantPolicy {
    /// Table name.
    pub table: String,
    /// Tenant ID column name.
    pub tenant_column: String,
    /// Session variable or function to get current tenant.
    pub tenant_source: TenantSource,
}

/// Source of current tenant ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TenantSource {
    /// PostgreSQL session variable: current_setting('app.tenant_id').
    SessionVar(String),
    /// MSSQL session context: SESSION_CONTEXT('tenant_id').
    SessionContext(String),
    /// Custom function call.
    Function(String),
}

impl TenantPolicy {
    /// Create a new tenant policy.
    pub fn new(
        table: impl Into<String>,
        tenant_column: impl Into<String>,
        source: TenantSource,
    ) -> Self {
        Self {
            table: table.into(),
            tenant_column: tenant_column.into(),
            tenant_source: source,
        }
    }

    /// Generate PostgreSQL RLS policy for tenant isolation.
    pub fn to_postgres_rls(&self) -> RlsPolicy {
        let tenant_expr = match &self.tenant_source {
            TenantSource::SessionVar(var) => format!("current_setting('{}')", var),
            TenantSource::Function(func) => format!("{}()", func),
            TenantSource::SessionContext(key) => format!("current_setting('{}')", key),
        };

        RlsPolicy::new(format!("{}_tenant_isolation", self.table), &self.table)
            .using(format!("{} = {}::INT", self.tenant_column, tenant_expr))
            .with_check(format!("{} = {}::INT", self.tenant_column, tenant_expr))
            .build()
    }

    /// Generate SQL to set the tenant context.
    pub fn set_tenant_sql(&self, tenant_id: &str, db_type: DatabaseType) -> String {
        match db_type {
            DatabaseType::PostgreSQL => match &self.tenant_source {
                TenantSource::SessionVar(var) => {
                    format!("SET LOCAL {} = '{}'", var, tenant_id)
                }
                _ => format!("SELECT set_config('app.tenant_id', '{}', true)", tenant_id),
            },
            DatabaseType::MSSQL => {
                format!(
                    "EXEC sp_set_session_context @key = N'tenant_id', @value = {}",
                    tenant_id
                )
            }
            _ => String::new(),
        }
    }
}

// ============================================================================
// Roles and Users
// ============================================================================

/// A database role definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Role name.
    pub name: String,
    /// Whether this role can login.
    pub login: bool,
    /// Password (if login enabled).
    pub password: Option<String>,
    /// Roles this role inherits from.
    pub inherit_from: Vec<String>,
    /// Whether this is a superuser.
    pub superuser: bool,
    /// Whether this role can create databases.
    pub createdb: bool,
    /// Whether this role can create other roles.
    pub createrole: bool,
    /// Connection limit.
    pub connection_limit: Option<i32>,
    /// Valid until timestamp.
    pub valid_until: Option<String>,
}

impl Role {
    /// Create a new role.
    pub fn new(name: impl Into<String>) -> RoleBuilder {
        RoleBuilder::new(name)
    }

    /// Generate PostgreSQL CREATE ROLE SQL.
    pub fn to_postgres_sql(&self) -> String {
        let mut sql = format!("CREATE ROLE {}", self.name);
        let mut options = Vec::new();

        if self.login {
            options.push("LOGIN".to_string());
        } else {
            options.push("NOLOGIN".to_string());
        }

        if let Some(ref pwd) = self.password {
            options.push(format!("PASSWORD '{}'", pwd));
        }

        if self.superuser {
            options.push("SUPERUSER".to_string());
        }

        if self.createdb {
            options.push("CREATEDB".to_string());
        }

        if self.createrole {
            options.push("CREATEROLE".to_string());
        }

        if let Some(limit) = self.connection_limit {
            options.push(format!("CONNECTION LIMIT {}", limit));
        }

        if let Some(ref until) = self.valid_until {
            options.push(format!("VALID UNTIL '{}'", until));
        }

        if !self.inherit_from.is_empty() {
            options.push(format!("IN ROLE {}", self.inherit_from.join(", ")));
        }

        if !options.is_empty() {
            sql.push_str(" WITH ");
            sql.push_str(&options.join(" "));
        }

        sql
    }

    /// Generate MySQL CREATE USER/ROLE SQL.
    pub fn to_mysql_sql(&self) -> Vec<String> {
        let mut sqls = Vec::new();

        if self.login {
            let mut sql = format!("CREATE USER '{}'@'%'", self.name);
            if let Some(ref pwd) = self.password {
                sql.push_str(&format!(" IDENTIFIED BY '{}'", pwd));
            }
            sqls.push(sql);
        } else {
            sqls.push(format!("CREATE ROLE '{}'", self.name));
        }

        for parent in &self.inherit_from {
            sqls.push(format!("GRANT '{}' TO '{}'", parent, self.name));
        }

        sqls
    }

    /// Generate MSSQL CREATE LOGIN/USER SQL.
    pub fn to_mssql_sql(&self, database: &str) -> Vec<String> {
        let mut sqls = Vec::new();

        if self.login {
            let mut sql = format!("CREATE LOGIN {} WITH PASSWORD = ", self.name);
            if let Some(ref pwd) = self.password {
                sql.push_str(&format!("'{}'", pwd));
            } else {
                sql.push_str("''");
            }
            sqls.push(sql);
            sqls.push(format!(
                "USE {}; CREATE USER {} FOR LOGIN {}",
                database, self.name, self.name
            ));
        } else {
            sqls.push(format!("USE {}; CREATE ROLE {}", database, self.name));
        }

        for parent in &self.inherit_from {
            sqls.push(format!("ALTER ROLE {} ADD MEMBER {}", parent, self.name));
        }

        sqls
    }
}

/// Builder for roles.
#[derive(Debug, Clone)]
pub struct RoleBuilder {
    name: String,
    login: bool,
    password: Option<String>,
    inherit_from: Vec<String>,
    superuser: bool,
    createdb: bool,
    createrole: bool,
    connection_limit: Option<i32>,
    valid_until: Option<String>,
}

impl RoleBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            login: false,
            password: None,
            inherit_from: Vec::new(),
            superuser: false,
            createdb: false,
            createrole: false,
            connection_limit: None,
            valid_until: None,
        }
    }

    /// Enable login capability (makes this a user).
    pub fn login(mut self) -> Self {
        self.login = true;
        self
    }

    /// Set password.
    pub fn password(mut self, pwd: impl Into<String>) -> Self {
        self.password = Some(pwd.into());
        self.login = true;
        self
    }

    /// Inherit from another role.
    pub fn inherit<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.inherit_from = roles.into_iter().map(Into::into).collect();
        self
    }

    /// Grant superuser privileges.
    pub fn superuser(mut self) -> Self {
        self.superuser = true;
        self
    }

    /// Allow creating databases.
    pub fn createdb(mut self) -> Self {
        self.createdb = true;
        self
    }

    /// Allow creating roles.
    pub fn createrole(mut self) -> Self {
        self.createrole = true;
        self
    }

    /// Set connection limit.
    pub fn connection_limit(mut self, limit: i32) -> Self {
        self.connection_limit = Some(limit);
        self
    }

    /// Set expiration.
    pub fn valid_until(mut self, timestamp: impl Into<String>) -> Self {
        self.valid_until = Some(timestamp.into());
        self
    }

    /// Build the role.
    pub fn build(self) -> Role {
        Role {
            name: self.name,
            login: self.login,
            password: self.password,
            inherit_from: self.inherit_from,
            superuser: self.superuser,
            createdb: self.createdb,
            createrole: self.createrole,
            connection_limit: self.connection_limit,
            valid_until: self.valid_until,
        }
    }
}

// ============================================================================
// Grants (Table, Column, Schema level)
// ============================================================================

/// A grant definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Privileges to grant.
    pub privileges: Vec<Privilege>,
    /// Object type and name.
    pub object: GrantObject,
    /// Grantee (role or user).
    pub grantee: String,
    /// Whether to include GRANT OPTION.
    pub with_grant_option: bool,
}

/// Database privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Privilege {
    /// SELECT privilege.
    Select,
    /// INSERT privilege.
    Insert,
    /// UPDATE privilege.
    Update,
    /// DELETE privilege.
    Delete,
    /// TRUNCATE privilege.
    Truncate,
    /// REFERENCES privilege.
    References,
    /// TRIGGER privilege.
    Trigger,
    /// All privileges.
    All,
    /// EXECUTE privilege (for functions).
    Execute,
    /// USAGE privilege (for schemas, sequences).
    Usage,
    /// CREATE privilege.
    Create,
    /// CONNECT privilege (for databases).
    Connect,
}

impl Privilege {
    /// Convert to SQL keyword.
    pub fn to_sql(&self) -> &'static str {
        match self {
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::References => "REFERENCES",
            Self::Trigger => "TRIGGER",
            Self::All => "ALL PRIVILEGES",
            Self::Execute => "EXECUTE",
            Self::Usage => "USAGE",
            Self::Create => "CREATE",
            Self::Connect => "CONNECT",
        }
    }
}

/// Object that grants apply to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrantObject {
    /// Table with optional column list.
    Table {
        name: String,
        columns: Option<Vec<String>>,
    },
    /// Schema.
    Schema(String),
    /// Database.
    Database(String),
    /// Sequence.
    Sequence(String),
    /// Function with signature.
    Function { name: String, args: String },
    /// All tables in schema.
    AllTablesInSchema(String),
    /// All sequences in schema.
    AllSequencesInSchema(String),
}

impl GrantObject {
    /// Create a table grant object.
    pub fn table(name: impl Into<String>) -> Self {
        Self::Table {
            name: name.into(),
            columns: None,
        }
    }

    /// Create a table grant with specific columns.
    pub fn table_columns<I, S>(name: impl Into<String>, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Table {
            name: name.into(),
            columns: Some(columns.into_iter().map(Into::into).collect()),
        }
    }

    /// Create a schema grant object.
    pub fn schema(name: impl Into<String>) -> Self {
        Self::Schema(name.into())
    }

    /// Convert to SQL object reference.
    pub fn to_sql(&self) -> String {
        match self {
            Self::Table { name, columns } => {
                if let Some(cols) = columns {
                    format!("TABLE {} ({})", name, cols.join(", "))
                } else {
                    format!("TABLE {}", name)
                }
            }
            Self::Schema(name) => format!("SCHEMA {}", name),
            Self::Database(name) => format!("DATABASE {}", name),
            Self::Sequence(name) => format!("SEQUENCE {}", name),
            Self::Function { name, args } => format!("FUNCTION {}({})", name, args),
            Self::AllTablesInSchema(schema) => format!("ALL TABLES IN SCHEMA {}", schema),
            Self::AllSequencesInSchema(schema) => format!("ALL SEQUENCES IN SCHEMA {}", schema),
        }
    }
}

impl Grant {
    /// Create a new grant.
    pub fn new(grantee: impl Into<String>) -> GrantBuilder {
        GrantBuilder::new(grantee)
    }

    /// Generate PostgreSQL GRANT SQL.
    pub fn to_postgres_sql(&self) -> String {
        let privs: Vec<&str> = self.privileges.iter().map(Privilege::to_sql).collect();
        let priv_sql = match &self.object {
            GrantObject::Table {
                columns: Some(cols),
                ..
            } => {
                // Column-level grants need special handling
                privs
                    .iter()
                    .map(|p| format!("{} ({})", p, cols.join(", ")))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            _ => privs.join(", "),
        };

        let obj_sql = match &self.object {
            GrantObject::Table {
                name,
                columns: Some(_),
            } => format!("TABLE {}", name),
            _ => self.object.to_sql(),
        };

        let mut sql = format!("GRANT {} ON {} TO {}", priv_sql, obj_sql, self.grantee);

        if self.with_grant_option {
            sql.push_str(" WITH GRANT OPTION");
        }

        sql
    }

    /// Generate MySQL GRANT SQL.
    pub fn to_mysql_sql(&self) -> String {
        let privs: Vec<&str> = self.privileges.iter().map(Privilege::to_sql).collect();
        let priv_sql = match &self.object {
            GrantObject::Table {
                columns: Some(cols),
                ..
            } => privs
                .iter()
                .map(|p| format!("{} ({})", p, cols.join(", ")))
                .collect::<Vec<_>>()
                .join(", "),
            _ => privs.join(", "),
        };

        let obj = match &self.object {
            GrantObject::Table { name, .. } => name.clone(),
            GrantObject::Database(name) => format!("{}.*", name),
            GrantObject::Schema(name) => format!("{}.*", name),
            _ => "*.*".to_string(),
        };

        let mut sql = format!("GRANT {} ON {} TO '{}'@'%'", priv_sql, obj, self.grantee);

        if self.with_grant_option {
            sql.push_str(" WITH GRANT OPTION");
        }

        sql
    }

    /// Generate MSSQL GRANT SQL.
    pub fn to_mssql_sql(&self) -> String {
        let privs: Vec<&str> = self.privileges.iter().map(Privilege::to_sql).collect();

        let (obj_type, obj_name) = match &self.object {
            GrantObject::Table { name, columns } => {
                if let Some(cols) = columns {
                    return format!(
                        "GRANT {} ({}) ON {} TO {}",
                        privs.join(", "),
                        cols.join(", "),
                        name,
                        self.grantee
                    );
                }
                ("OBJECT", name.clone())
            }
            GrantObject::Schema(name) => ("SCHEMA", name.clone()),
            GrantObject::Database(name) => ("DATABASE", name.clone()),
            _ => ("OBJECT", "".to_string()),
        };

        format!(
            "GRANT {} ON {}::{} TO {}",
            privs.join(", "),
            obj_type,
            obj_name,
            self.grantee
        )
    }
}

/// Builder for grants.
#[derive(Debug, Clone)]
pub struct GrantBuilder {
    grantee: String,
    privileges: Vec<Privilege>,
    object: Option<GrantObject>,
    with_grant_option: bool,
}

impl GrantBuilder {
    /// Create a new builder.
    pub fn new(grantee: impl Into<String>) -> Self {
        Self {
            grantee: grantee.into(),
            privileges: Vec::new(),
            object: None,
            with_grant_option: false,
        }
    }

    /// Add a privilege.
    pub fn privilege(mut self, priv_: Privilege) -> Self {
        self.privileges.push(priv_);
        self
    }

    /// Grant SELECT.
    pub fn select(self) -> Self {
        self.privilege(Privilege::Select)
    }

    /// Grant INSERT.
    pub fn insert(self) -> Self {
        self.privilege(Privilege::Insert)
    }

    /// Grant UPDATE.
    pub fn update(self) -> Self {
        self.privilege(Privilege::Update)
    }

    /// Grant DELETE.
    pub fn delete(self) -> Self {
        self.privilege(Privilege::Delete)
    }

    /// Grant ALL PRIVILEGES.
    pub fn all(self) -> Self {
        self.privilege(Privilege::All)
    }

    /// Set the object.
    pub fn on(mut self, object: GrantObject) -> Self {
        self.object = Some(object);
        self
    }

    /// Grant on a table.
    pub fn on_table(self, table: impl Into<String>) -> Self {
        self.on(GrantObject::table(table))
    }

    /// Grant on specific columns.
    pub fn on_columns<I, S>(self, table: impl Into<String>, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.on(GrantObject::table_columns(table, columns))
    }

    /// Grant on schema.
    pub fn on_schema(self, schema: impl Into<String>) -> Self {
        self.on(GrantObject::Schema(schema.into()))
    }

    /// Include WITH GRANT OPTION.
    pub fn with_grant_option(mut self) -> Self {
        self.with_grant_option = true;
        self
    }

    /// Build the grant.
    pub fn build(self) -> QueryResult<Grant> {
        let object = self.object.ok_or_else(|| {
            QueryError::invalid_input(
                "object",
                "Grant requires an object (use on_table, on_schema, etc.)",
            )
        })?;

        if self.privileges.is_empty() {
            return Err(QueryError::invalid_input(
                "privileges",
                "Grant requires at least one privilege",
            ));
        }

        Ok(Grant {
            privileges: self.privileges,
            object,
            grantee: self.grantee,
            with_grant_option: self.with_grant_option,
        })
    }
}

// ============================================================================
// Data Masking
// ============================================================================

/// Data masking configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataMask {
    /// Table name.
    pub table: String,
    /// Column name.
    pub column: String,
    /// Masking function.
    pub mask_function: MaskFunction,
}

/// Data masking function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaskFunction {
    /// Default masking (type-specific).
    Default,
    /// Email masking (aXXX@XXXX.com).
    Email,
    /// Partial masking (show prefix/suffix).
    Partial {
        prefix: usize,
        padding: String,
        suffix: usize,
    },
    /// Random value.
    Random,
    /// Custom masking function.
    Custom(String),
    /// NULL replacement.
    Null,
}

impl DataMask {
    /// Create a new data mask.
    pub fn new(table: impl Into<String>, column: impl Into<String>, mask: MaskFunction) -> Self {
        Self {
            table: table.into(),
            column: column.into(),
            mask_function: mask,
        }
    }

    /// Generate PostgreSQL view-based masking.
    pub fn to_postgres_view(&self, view_name: &str) -> String {
        let masked_expr = match &self.mask_function {
            MaskFunction::Default => format!(
                "CASE WHEN current_user = 'admin' THEN {} ELSE '****' END",
                self.column
            ),
            MaskFunction::Email => format!(
                "CASE WHEN current_user = 'admin' THEN {} ELSE \
                 CONCAT(LEFT({}, 1), '***@', SPLIT_PART({}, '@', 2)) END",
                self.column, self.column, self.column
            ),
            MaskFunction::Partial {
                prefix,
                padding,
                suffix,
            } => format!(
                "CONCAT(LEFT({}, {}), '{}', RIGHT({}, {}))",
                self.column, prefix, padding, self.column, suffix
            ),
            MaskFunction::Null => "NULL".to_string(),
            MaskFunction::Custom(func) => format!("{}({})", func, self.column),
            MaskFunction::Random => "md5(random()::text)".to_string(),
        };

        format!(
            "CREATE OR REPLACE VIEW {} AS SELECT *, {} AS {}_masked FROM {}",
            view_name, masked_expr, self.column, self.table
        )
    }

    /// Generate MSSQL dynamic data masking.
    pub fn to_mssql_alter(&self) -> String {
        let mask_func = match &self.mask_function {
            MaskFunction::Default => "default()".to_string(),
            MaskFunction::Email => "email()".to_string(),
            MaskFunction::Partial {
                prefix,
                padding,
                suffix,
            } => {
                format!("partial({}, '{}', {})", prefix, padding, suffix)
            }
            MaskFunction::Random => "random(1, 100)".to_string(),
            MaskFunction::Custom(func) => func.clone(),
            MaskFunction::Null => "default()".to_string(),
        };

        format!(
            "ALTER TABLE {} ALTER COLUMN {} ADD MASKED WITH (FUNCTION = '{}')",
            self.table, self.column, mask_func
        )
    }
}

// ============================================================================
// Connection Profiles
// ============================================================================

/// A named connection profile with specific permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionProfile {
    /// Profile name.
    pub name: String,
    /// Database role to use.
    pub role: String,
    /// Schema search path.
    pub search_path: Vec<String>,
    /// Session variables to set.
    pub session_vars: Vec<(String, String)>,
    /// Whether this is read-only.
    pub read_only: bool,
    /// Statement timeout (ms).
    pub statement_timeout: Option<u32>,
    /// Lock timeout (ms).
    pub lock_timeout: Option<u32>,
}

impl ConnectionProfile {
    /// Create a new connection profile.
    pub fn new(name: impl Into<String>, role: impl Into<String>) -> ConnectionProfileBuilder {
        ConnectionProfileBuilder::new(name, role)
    }

    /// Generate PostgreSQL session setup SQL.
    pub fn to_postgres_setup(&self) -> Vec<String> {
        let mut sqls = Vec::new();

        sqls.push(format!("SET ROLE {}", self.role));

        if !self.search_path.is_empty() {
            sqls.push(format!(
                "SET search_path TO {}",
                self.search_path.join(", ")
            ));
        }

        if self.read_only {
            sqls.push("SET default_transaction_read_only = ON".to_string());
        }

        if let Some(timeout) = self.statement_timeout {
            sqls.push(format!("SET statement_timeout = {}", timeout));
        }

        if let Some(timeout) = self.lock_timeout {
            sqls.push(format!("SET lock_timeout = {}", timeout));
        }

        for (key, value) in &self.session_vars {
            sqls.push(format!("SET {} = '{}'", key, value));
        }

        sqls
    }

    /// Generate MySQL session setup SQL.
    pub fn to_mysql_setup(&self) -> Vec<String> {
        let mut sqls = Vec::new();

        // MySQL doesn't have SET ROLE in the same way
        if self.read_only {
            sqls.push("SET SESSION TRANSACTION READ ONLY".to_string());
        }

        for (key, value) in &self.session_vars {
            sqls.push(format!("SET @{} = '{}'", key, value));
        }

        sqls
    }
}

/// Builder for connection profiles.
#[derive(Debug, Clone)]
pub struct ConnectionProfileBuilder {
    name: String,
    role: String,
    search_path: Vec<String>,
    session_vars: Vec<(String, String)>,
    read_only: bool,
    statement_timeout: Option<u32>,
    lock_timeout: Option<u32>,
}

impl ConnectionProfileBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>, role: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            role: role.into(),
            search_path: Vec::new(),
            session_vars: Vec::new(),
            read_only: false,
            statement_timeout: None,
            lock_timeout: None,
        }
    }

    /// Set schema search path.
    pub fn search_path<I, S>(mut self, schemas: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.search_path = schemas.into_iter().map(Into::into).collect();
        self
    }

    /// Add a session variable.
    pub fn session_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.session_vars.push((key.into(), value.into()));
        self
    }

    /// Make read-only.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Set statement timeout.
    pub fn statement_timeout(mut self, ms: u32) -> Self {
        self.statement_timeout = Some(ms);
        self
    }

    /// Set lock timeout.
    pub fn lock_timeout(mut self, ms: u32) -> Self {
        self.lock_timeout = Some(ms);
        self
    }

    /// Build the profile.
    pub fn build(self) -> ConnectionProfile {
        ConnectionProfile {
            name: self.name,
            role: self.role,
            search_path: self.search_path,
            session_vars: self.session_vars,
            read_only: self.read_only,
            statement_timeout: self.statement_timeout,
            lock_timeout: self.lock_timeout,
        }
    }
}

// ============================================================================
// MongoDB Security
// ============================================================================

/// MongoDB security operations.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    /// MongoDB role definition.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MongoRole {
        /// Role name.
        pub role: String,
        /// Database.
        pub db: String,
        /// Privileges.
        pub privileges: Vec<MongoPrivilege>,
        /// Inherited roles.
        pub roles: Vec<InheritedRole>,
    }

    /// A MongoDB privilege.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MongoPrivilege {
        /// Resource specification.
        pub resource: MongoResource,
        /// Actions allowed.
        pub actions: Vec<String>,
    }

    /// MongoDB resource specification.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum MongoResource {
        /// Specific database and collection.
        Collection { db: String, collection: String },
        /// All collections in a database.
        Database { db: String },
        /// Cluster-wide resource.
        Cluster { cluster: bool },
    }

    /// An inherited role reference.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct InheritedRole {
        /// Role name.
        pub role: String,
        /// Database.
        pub db: String,
    }

    impl MongoRole {
        /// Create a new role builder.
        pub fn new(role: impl Into<String>, db: impl Into<String>) -> MongoRoleBuilder {
            MongoRoleBuilder::new(role, db)
        }

        /// Convert to createRole command.
        pub fn to_create_command(&self) -> JsonValue {
            let privileges: Vec<JsonValue> = self
                .privileges
                .iter()
                .map(|p| {
                    let resource = match &p.resource {
                        MongoResource::Collection { db, collection } => {
                            serde_json::json!({ "db": db, "collection": collection })
                        }
                        MongoResource::Database { db } => {
                            serde_json::json!({ "db": db, "collection": "" })
                        }
                        MongoResource::Cluster { .. } => {
                            serde_json::json!({ "cluster": true })
                        }
                    };
                    serde_json::json!({
                        "resource": resource,
                        "actions": p.actions
                    })
                })
                .collect();

            let roles: Vec<JsonValue> = self
                .roles
                .iter()
                .map(|r| serde_json::json!({ "role": r.role, "db": r.db }))
                .collect();

            serde_json::json!({
                "createRole": self.role,
                "privileges": privileges,
                "roles": roles
            })
        }
    }

    /// Builder for MongoDB roles.
    #[derive(Debug, Clone, Default)]
    pub struct MongoRoleBuilder {
        role: String,
        db: String,
        privileges: Vec<MongoPrivilege>,
        roles: Vec<InheritedRole>,
    }

    impl MongoRoleBuilder {
        /// Create a new builder.
        pub fn new(role: impl Into<String>, db: impl Into<String>) -> Self {
            Self {
                role: role.into(),
                db: db.into(),
                privileges: Vec::new(),
                roles: Vec::new(),
            }
        }

        /// Add collection-level privilege.
        pub fn privilege_collection<I, S>(
            mut self,
            collection: impl Into<String>,
            actions: I,
        ) -> Self
        where
            I: IntoIterator<Item = S>,
            S: Into<String>,
        {
            self.privileges.push(MongoPrivilege {
                resource: MongoResource::Collection {
                    db: self.db.clone(),
                    collection: collection.into(),
                },
                actions: actions.into_iter().map(Into::into).collect(),
            });
            self
        }

        /// Add database-level privilege.
        pub fn privilege_database<I, S>(mut self, actions: I) -> Self
        where
            I: IntoIterator<Item = S>,
            S: Into<String>,
        {
            self.privileges.push(MongoPrivilege {
                resource: MongoResource::Database {
                    db: self.db.clone(),
                },
                actions: actions.into_iter().map(Into::into).collect(),
            });
            self
        }

        /// Inherit from another role.
        pub fn inherit(mut self, role: impl Into<String>, db: impl Into<String>) -> Self {
            self.roles.push(InheritedRole {
                role: role.into(),
                db: db.into(),
            });
            self
        }

        /// Build the role.
        pub fn build(self) -> MongoRole {
            MongoRole {
                role: self.role,
                db: self.db,
                privileges: self.privileges,
                roles: self.roles,
            }
        }
    }

    /// Field-level encryption configuration.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct FieldEncryption {
        /// Key vault namespace (db.collection).
        pub key_vault_namespace: String,
        /// KMS providers configuration.
        pub kms_providers: KmsProviders,
        /// Schema map for automatic encryption.
        pub schema_map: serde_json::Map<String, JsonValue>,
    }

    /// KMS provider configuration.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum KmsProviders {
        /// Local key (for development).
        Local { key: String },
        /// AWS KMS.
        Aws {
            access_key_id: String,
            secret_access_key: String,
            region: String,
        },
        /// Azure Key Vault.
        Azure {
            tenant_id: String,
            client_id: String,
            client_secret: String,
        },
        /// Google Cloud KMS.
        Gcp { email: String, private_key: String },
    }

    impl FieldEncryption {
        /// Create a new field encryption config.
        pub fn new(key_vault_namespace: impl Into<String>, kms: KmsProviders) -> Self {
            Self {
                key_vault_namespace: key_vault_namespace.into(),
                kms_providers: kms,
                schema_map: serde_json::Map::new(),
            }
        }

        /// Add encrypted field to schema map.
        pub fn encrypt_field(
            mut self,
            namespace: impl Into<String>,
            field: impl Into<String>,
            algorithm: EncryptionAlgorithm,
            key_id: impl Into<String>,
        ) -> Self {
            let ns = namespace.into();
            let field = field.into();

            let field_spec = serde_json::json!({
                "encrypt": {
                    "bsonType": "string",
                    "algorithm": algorithm.to_str(),
                    "keyId": [{ "$binary": { "base64": key_id.into(), "subType": "04" } }]
                }
            });

            // Build nested structure
            let schema = self.schema_map.entry(ns).or_insert_with(|| {
                serde_json::json!({
                    "bsonType": "object",
                    "properties": {}
                })
            });

            if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
                props.insert(field, field_spec);
            }

            self
        }

        /// Convert to client encryption options.
        pub fn to_options(&self) -> JsonValue {
            let kms = match &self.kms_providers {
                KmsProviders::Local { key } => {
                    serde_json::json!({ "local": { "key": key } })
                }
                KmsProviders::Aws {
                    access_key_id,
                    secret_access_key,
                    region,
                } => {
                    serde_json::json!({
                        "aws": {
                            "accessKeyId": access_key_id,
                            "secretAccessKey": secret_access_key,
                            "region": region
                        }
                    })
                }
                KmsProviders::Azure {
                    tenant_id,
                    client_id,
                    client_secret,
                } => {
                    serde_json::json!({
                        "azure": {
                            "tenantId": tenant_id,
                            "clientId": client_id,
                            "clientSecret": client_secret
                        }
                    })
                }
                KmsProviders::Gcp { email, private_key } => {
                    serde_json::json!({
                        "gcp": {
                            "email": email,
                            "privateKey": private_key
                        }
                    })
                }
            };

            serde_json::json!({
                "keyVaultNamespace": self.key_vault_namespace,
                "kmsProviders": kms,
                "schemaMap": self.schema_map
            })
        }
    }

    /// Field-level encryption algorithm.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum EncryptionAlgorithm {
        /// Deterministic encryption (allows equality queries).
        Deterministic,
        /// Random encryption (more secure, no queries).
        Random,
    }

    impl EncryptionAlgorithm {
        /// Convert to MongoDB algorithm string.
        pub fn to_str(&self) -> &'static str {
            match self {
                Self::Deterministic => "AEAD_AES_256_CBC_HMAC_SHA_512-Deterministic",
                Self::Random => "AEAD_AES_256_CBC_HMAC_SHA_512-Random",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rls_policy_postgres() {
        let policy = RlsPolicy::new("tenant_isolation", "orders")
            .using("tenant_id = current_setting('app.tenant_id')::INT")
            .with_check("tenant_id = current_setting('app.tenant_id')::INT")
            .build();

        let sql = policy.to_postgres_sql();
        assert!(sql.contains("CREATE POLICY tenant_isolation ON orders"));
        assert!(sql.contains("USING (tenant_id ="));
        assert!(sql.contains("WITH CHECK (tenant_id ="));
    }

    #[test]
    fn test_rls_policy_for_select() {
        let policy = RlsPolicy::new("read_own", "documents")
            .for_select()
            .to_roles(["app_user"])
            .using("owner_id = current_user_id()")
            .build();

        let sql = policy.to_postgres_sql();
        assert!(sql.contains("FOR SELECT"));
        assert!(sql.contains("TO app_user"));
    }

    #[test]
    fn test_tenant_policy() {
        let tenant = TenantPolicy::new(
            "orders",
            "tenant_id",
            TenantSource::SessionVar("app.tenant_id".to_string()),
        );

        let policy = tenant.to_postgres_rls();
        assert!(policy.using.is_some());
        assert!(policy.with_check.is_some());

        let set_sql = tenant.set_tenant_sql("123", DatabaseType::PostgreSQL);
        assert!(set_sql.contains("SET LOCAL app.tenant_id"));
    }

    #[test]
    fn test_role_postgres() {
        let role = Role::new("app_reader")
            .login()
            .password("secret")
            .connection_limit(10)
            .build();

        let sql = role.to_postgres_sql();
        assert!(sql.contains("CREATE ROLE app_reader"));
        assert!(sql.contains("LOGIN"));
        assert!(sql.contains("PASSWORD 'secret'"));
        assert!(sql.contains("CONNECTION LIMIT 10"));
    }

    #[test]
    fn test_role_inherit() {
        let role = Role::new("senior_dev")
            .inherit(["developer", "analyst"])
            .build();

        let sql = role.to_postgres_sql();
        assert!(sql.contains("IN ROLE developer, analyst"));
    }

    #[test]
    fn test_grant_table() {
        let grant = Grant::new("app_user")
            .select()
            .insert()
            .update()
            .on_table("users")
            .build()
            .unwrap();

        let sql = grant.to_postgres_sql();
        assert!(sql.contains("GRANT SELECT, INSERT, UPDATE ON TABLE users TO app_user"));
    }

    #[test]
    fn test_grant_columns() {
        let grant = Grant::new("restricted_user")
            .select()
            .on_columns("users", ["id", "name", "email"])
            .build()
            .unwrap();

        let sql = grant.to_postgres_sql();
        assert!(sql.contains("SELECT (id, name, email)"));
    }

    #[test]
    fn test_grant_with_option() {
        let grant = Grant::new("admin")
            .all()
            .on_schema("public")
            .with_grant_option()
            .build()
            .unwrap();

        let sql = grant.to_postgres_sql();
        assert!(sql.contains("WITH GRANT OPTION"));
    }

    #[test]
    fn test_data_mask_email() {
        let mask = DataMask::new("users", "email", MaskFunction::Email);
        let sql = mask.to_mssql_alter();

        assert!(sql.contains("ADD MASKED WITH (FUNCTION = 'email()'"));
    }

    #[test]
    fn test_data_mask_partial() {
        let mask = DataMask::new(
            "users",
            "ssn",
            MaskFunction::Partial {
                prefix: 0,
                padding: "XXX-XX-".to_string(),
                suffix: 4,
            },
        );
        let sql = mask.to_mssql_alter();

        assert!(sql.contains("partial(0, 'XXX-XX-', 4)"));
    }

    #[test]
    fn test_connection_profile() {
        let profile = ConnectionProfile::new("readonly_user", "app_readonly")
            .search_path(["app", "public"])
            .read_only()
            .statement_timeout(5000)
            .build();

        let sqls = profile.to_postgres_setup();
        assert!(sqls.iter().any(|s| s.contains("SET ROLE app_readonly")));
        assert!(
            sqls.iter()
                .any(|s| s.contains("search_path TO app, public"))
        );
        assert!(sqls.iter().any(|s| s.contains("read_only = ON")));
        assert!(sqls.iter().any(|s| s.contains("statement_timeout = 5000")));
    }

    mod mongodb_tests {
        use super::super::mongodb::*;

        #[test]
        fn test_mongo_role() {
            let role = MongoRole::new("app_reader", "mydb")
                .privilege_collection("orders", ["find", "aggregate"])
                .inherit("read", "mydb")
                .build();

            let cmd = role.to_create_command();
            assert_eq!(cmd["createRole"], "app_reader");
            assert!(cmd["privileges"].is_array());
            assert!(cmd["roles"].is_array());
        }

        #[test]
        fn test_field_encryption_local() {
            let enc = FieldEncryption::new(
                "encryption.__keyVault",
                KmsProviders::Local {
                    key: "base64key".to_string(),
                },
            )
            .encrypt_field(
                "mydb.users",
                "ssn",
                EncryptionAlgorithm::Deterministic,
                "keyid",
            );

            let opts = enc.to_options();
            assert!(opts["kmsProviders"]["local"].is_object());
            assert!(opts["schemaMap"]["mydb.users"].is_object());
        }

        #[test]
        fn test_field_encryption_aws() {
            let enc = FieldEncryption::new(
                "encryption.__keyVault",
                KmsProviders::Aws {
                    access_key_id: "AKID".to_string(),
                    secret_access_key: "secret".to_string(),
                    region: "us-east-1".to_string(),
                },
            );

            let opts = enc.to_options();
            assert!(opts["kmsProviders"]["aws"]["accessKeyId"].is_string());
        }
    }
}
