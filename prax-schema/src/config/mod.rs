//! Configuration file parsing for `prax.toml`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{SchemaError, SchemaResult};

/// Main configuration structure for `prax.toml`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PraxConfig {
    /// Database configuration.
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Schema file configuration.
    #[serde(default)]
    pub schema: SchemaConfig,

    /// Generator configuration.
    #[serde(default)]
    pub generator: GeneratorConfig,

    /// Migration settings.
    #[serde(default)]
    pub migrations: MigrationConfig,

    /// Seeding configuration.
    #[serde(default)]
    pub seed: SeedConfig,

    /// Debug/logging settings.
    #[serde(default)]
    pub debug: DebugConfig,

    /// Environment-specific overrides.
    #[serde(default)]
    pub environments: HashMap<String, EnvironmentOverride>,
}

impl PraxConfig {
    /// Load configuration from a file path.
    pub fn from_file(path: impl AsRef<Path>) -> SchemaResult<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| SchemaError::IoError {
            path: path.display().to_string(),
            source: e,
        })?;

        Self::from_str(&content)
    }

    /// Parse configuration from a TOML string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(content: &str) -> SchemaResult<Self> {
        // First, expand environment variables
        let expanded = expand_env_vars(content);

        toml::from_str(&expanded).map_err(|e| SchemaError::TomlError { source: e })
    }

    /// Get the database URL, resolving environment variables.
    pub fn database_url(&self) -> Option<&str> {
        self.database.url.as_deref()
    }

    /// Apply environment-specific overrides.
    pub fn with_environment(mut self, env: &str) -> Self {
        if let Some(overrides) = self.environments.remove(env) {
            if let Some(db) = overrides.database {
                if let Some(url) = db.url {
                    self.database.url = Some(url);
                }
                if let Some(pool) = db.pool {
                    self.database.pool = pool;
                }
            }
            if let Some(debug) = overrides.debug {
                if let Some(log_queries) = debug.log_queries {
                    self.debug.log_queries = log_queries;
                }
                if let Some(pretty_sql) = debug.pretty_sql {
                    self.debug.pretty_sql = pretty_sql;
                }
                if let Some(threshold) = debug.slow_query_threshold {
                    self.debug.slow_query_threshold = threshold;
                }
            }
        }
        self
    }
}

/// Database configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Database provider.
    #[serde(default = "default_provider")]
    pub provider: DatabaseProvider,

    /// Connection URL (supports `${ENV_VAR}` interpolation).
    pub url: Option<String>,

    /// Connection pool settings.
    #[serde(default)]
    pub pool: PoolConfig,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            provider: DatabaseProvider::PostgreSql,
            url: None,
            pool: PoolConfig::default(),
        }
    }
}

fn default_provider() -> DatabaseProvider {
    DatabaseProvider::PostgreSql
}

/// Supported database providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseProvider {
    /// PostgreSQL.
    #[serde(alias = "postgres")]
    PostgreSql,
    /// MySQL / MariaDB.
    MySql,
    /// SQLite.
    #[serde(alias = "sqlite3")]
    Sqlite,
    /// MongoDB.
    #[serde(alias = "mongo")]
    MongoDb,
}

impl DatabaseProvider {
    /// Get the provider name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PostgreSql => "postgresql",
            Self::MySql => "mysql",
            Self::Sqlite => "sqlite",
            Self::MongoDb => "mongodb",
        }
    }
}

/// Connection pool configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    /// Minimum number of connections.
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,

    /// Maximum number of connections.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Connection timeout.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: String,

    /// Idle connection timeout.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: String,

    /// Maximum connection lifetime.
    #[serde(default = "default_max_lifetime")]
    pub max_lifetime: String,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_connections: default_min_connections(),
            max_connections: default_max_connections(),
            connect_timeout: default_connect_timeout(),
            idle_timeout: default_idle_timeout(),
            max_lifetime: default_max_lifetime(),
        }
    }
}

fn default_min_connections() -> u32 {
    2
}
fn default_max_connections() -> u32 {
    10
}
fn default_connect_timeout() -> String {
    "30s".to_string()
}
fn default_idle_timeout() -> String {
    "10m".to_string()
}
fn default_max_lifetime() -> String {
    "30m".to_string()
}

/// Schema file configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaConfig {
    /// Path to the schema file.
    #[serde(default = "default_schema_path")]
    pub path: String,
}

impl Default for SchemaConfig {
    fn default() -> Self {
        Self {
            path: default_schema_path(),
        }
    }
}

fn default_schema_path() -> String {
    "schema.prax".to_string()
}

/// Generator configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorConfig {
    /// Client generator settings.
    #[serde(default)]
    pub client: ClientGeneratorConfig,
}

/// Style of model code generation.
///
/// Controls whether models are generated as plain Rust structs or with
/// additional framework-specific derives like async-graphql.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelStyle {
    /// Generate plain Rust models with Serde derives.
    /// This is the default and generates the lightest weight models.
    #[default]
    Standard,

    /// Generate models with async-graphql derives.
    /// Adds `#[derive(SimpleObject)]`, `#[derive(InputObject)]`, etc.
    /// Requires the `async-graphql` crate as a dependency.
    #[serde(alias = "async-graphql")]
    GraphQL,
}

impl ModelStyle {
    /// Returns true if this style requires GraphQL derives.
    pub fn is_graphql(&self) -> bool {
        matches!(self, Self::GraphQL)
    }
}

/// Client generator configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientGeneratorConfig {
    /// Output directory.
    #[serde(default = "default_output")]
    pub output: String,

    /// Generate async client.
    #[serde(default = "default_true")]
    pub async_client: bool,

    /// Enable tracing instrumentation.
    #[serde(default)]
    pub tracing: bool,

    /// Preview features to enable.
    #[serde(default)]
    pub preview_features: Vec<String>,

    /// Model generation style.
    ///
    /// Controls the type of derives and attributes added to generated models:
    /// - `standard`: Plain Rust structs with Serde (default)
    /// - `graphql`: Adds async-graphql derives (SimpleObject, InputObject, etc.)
    #[serde(default)]
    pub model_style: ModelStyle,
}

impl Default for ClientGeneratorConfig {
    fn default() -> Self {
        Self {
            output: default_output(),
            async_client: true,
            tracing: false,
            preview_features: vec![],
            model_style: ModelStyle::default(),
        }
    }
}

fn default_output() -> String {
    "./src/generated".to_string()
}
fn default_true() -> bool {
    true
}

/// Migration configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationConfig {
    /// Migration files directory.
    #[serde(default = "default_migrations_dir")]
    pub directory: String,

    /// Auto-apply migrations in development.
    #[serde(default)]
    pub auto_migrate: bool,

    /// Migration history table name.
    #[serde(default = "default_migrations_table")]
    pub table_name: String,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            directory: default_migrations_dir(),
            auto_migrate: false,
            table_name: default_migrations_table(),
        }
    }
}

fn default_migrations_dir() -> String {
    "./migrations".to_string()
}
fn default_migrations_table() -> String {
    "_prax_migrations".to_string()
}

/// Seed configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeedConfig {
    /// Seed script path.
    pub script: Option<String>,

    /// Run seed after migrations.
    #[serde(default)]
    pub auto_seed: bool,

    /// Environment-specific seeding flags.
    #[serde(default)]
    pub environments: HashMap<String, bool>,
}

/// Debug/logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DebugConfig {
    /// Log all queries.
    #[serde(default)]
    pub log_queries: bool,

    /// Pretty print SQL.
    #[serde(default = "default_true")]
    pub pretty_sql: bool,

    /// Slow query threshold in milliseconds.
    #[serde(default = "default_slow_query_threshold")]
    pub slow_query_threshold: u64,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            log_queries: false,
            pretty_sql: true,
            slow_query_threshold: default_slow_query_threshold(),
        }
    }
}

fn default_slow_query_threshold() -> u64 {
    1000
}

/// Environment-specific configuration overrides.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentOverride {
    /// Database overrides.
    pub database: Option<DatabaseOverride>,

    /// Debug overrides.
    pub debug: Option<DebugOverride>,
}

/// Database configuration overrides.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseOverride {
    /// Override connection URL.
    pub url: Option<String>,

    /// Override pool settings.
    pub pool: Option<PoolConfig>,
}

/// Debug configuration overrides.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DebugOverride {
    /// Override log_queries.
    pub log_queries: Option<bool>,

    /// Override pretty_sql.
    pub pretty_sql: Option<bool>,

    /// Override slow_query_threshold.
    pub slow_query_threshold: Option<u64>,
}

/// Expand environment variables in the format `${VAR_NAME}`.
fn expand_env_vars(content: &str) -> String {
    let mut result = content.to_string();
    // Compile the `${...}` pattern once; `expand_env_vars` runs on every
    // config load and the pattern is constant.
    static PATTERN: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();
    let re = PATTERN.get_or_init(|| regex_lite::Regex::new(r"\$\{([^}]+)\}").unwrap());

    for cap in re.captures_iter(content) {
        let var_name = &cap[1];
        let full_match = &cap[0];

        if let Ok(value) = std::env::var(var_name) {
            result = result.replace(full_match, &value);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== PraxConfig Tests ====================

    #[test]
    fn test_default_config() {
        let config = PraxConfig::default();
        assert_eq!(config.database.provider, DatabaseProvider::PostgreSql);
        assert_eq!(config.schema.path, "schema.prax");
        assert!(config.database.url.is_none());
        assert!(config.environments.is_empty());
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
            [database]
            provider = "postgresql"
            url = "postgres://localhost/test"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(
            config.database.url,
            Some("postgres://localhost/test".to_string())
        );
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
            [database]
            provider = "postgresql"
            url = "postgres://user:pass@localhost:5432/db"

            [database.pool]
            min_connections = 5
            max_connections = 20
            connect_timeout = "60s"
            idle_timeout = "5m"
            max_lifetime = "1h"

            [schema]
            path = "prisma/schema.prax"

            [generator.client]
            output = "./src/db"
            async_client = true
            tracing = true
            preview_features = ["json", "fulltext"]

            [migrations]
            directory = "./db/migrations"
            auto_migrate = true
            table_name = "_migrations"

            [seed]
            script = "./scripts/seed.sh"
            auto_seed = true

            [seed.environments]
            development = true
            test = true
            production = false

            [debug]
            log_queries = true
            pretty_sql = false
            slow_query_threshold = 500
        "#;

        let config = PraxConfig::from_str(toml).unwrap();

        // Database
        assert_eq!(config.database.provider, DatabaseProvider::PostgreSql);
        assert!(config.database.url.is_some());
        assert_eq!(config.database.pool.min_connections, 5);
        assert_eq!(config.database.pool.max_connections, 20);

        // Schema
        assert_eq!(config.schema.path, "prisma/schema.prax");

        // Generator
        assert_eq!(config.generator.client.output, "./src/db");
        assert!(config.generator.client.async_client);
        assert!(config.generator.client.tracing);
        assert_eq!(config.generator.client.preview_features.len(), 2);

        // Migrations
        assert_eq!(config.migrations.directory, "./db/migrations");
        assert!(config.migrations.auto_migrate);
        assert_eq!(config.migrations.table_name, "_migrations");

        // Seed
        assert_eq!(config.seed.script, Some("./scripts/seed.sh".to_string()));
        assert!(config.seed.auto_seed);
        assert!(
            config
                .seed
                .environments
                .get("development")
                .copied()
                .unwrap_or(false)
        );

        // Debug
        assert!(config.debug.log_queries);
        assert!(!config.debug.pretty_sql);
        assert_eq!(config.debug.slow_query_threshold, 500);
    }

    #[test]
    fn test_database_url_method() {
        let config = PraxConfig {
            database: DatabaseConfig {
                url: Some("postgres://localhost/test".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(config.database_url(), Some("postgres://localhost/test"));
    }

    #[test]
    fn test_database_url_method_none() {
        let config = PraxConfig::default();
        assert!(config.database_url().is_none());
    }

    #[test]
    fn test_with_environment_overrides() {
        let toml = r#"
            [database]
            url = "postgres://localhost/dev"

            [debug]
            log_queries = false

            [environments.production]
            [environments.production.database]
            url = "postgres://prod.server/db"

            [environments.production.debug]
            log_queries = true
            slow_query_threshold = 100
        "#;

        let config = PraxConfig::from_str(toml)
            .unwrap()
            .with_environment("production");

        assert_eq!(
            config.database.url,
            Some("postgres://prod.server/db".to_string())
        );
        assert!(config.debug.log_queries);
        assert_eq!(config.debug.slow_query_threshold, 100);
    }

    #[test]
    fn test_with_environment_nonexistent() {
        let config = PraxConfig::default().with_environment("nonexistent");
        // Should not panic and return unchanged config
        assert_eq!(config.database.provider, DatabaseProvider::PostgreSql);
    }

    #[test]
    fn test_parse_invalid_toml() {
        let toml = "this is not valid [[ toml";
        let result = PraxConfig::from_str(toml);
        assert!(result.is_err());
    }

    // ==================== DatabaseProvider Tests ====================

    #[test]
    fn test_database_provider_postgresql() {
        let toml = r#"
            [database]
            provider = "postgresql"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::PostgreSql);
        assert_eq!(config.database.provider.as_str(), "postgresql");
    }

    #[test]
    fn test_database_provider_postgres_alias() {
        let toml = r#"
            [database]
            provider = "postgres"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::PostgreSql);
    }

    #[test]
    fn test_database_provider_mysql() {
        let toml = r#"
            [database]
            provider = "mysql"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::MySql);
        assert_eq!(config.database.provider.as_str(), "mysql");
    }

    #[test]
    fn test_database_provider_sqlite() {
        let toml = r#"
            [database]
            provider = "sqlite"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::Sqlite);
        assert_eq!(config.database.provider.as_str(), "sqlite");
    }

    #[test]
    fn test_database_provider_sqlite3_alias() {
        let toml = r#"
            [database]
            provider = "sqlite3"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::Sqlite);
    }

    #[test]
    fn test_database_provider_mongodb() {
        let toml = r#"
            [database]
            provider = "mongodb"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::MongoDb);
        assert_eq!(config.database.provider.as_str(), "mongodb");
    }

    #[test]
    fn test_database_provider_mongo_alias() {
        let toml = r#"
            [database]
            provider = "mongo"
        "#;
        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.provider, DatabaseProvider::MongoDb);
    }

    // ==================== PoolConfig Tests ====================

    #[test]
    fn test_pool_config_defaults() {
        let config = PoolConfig::default();
        assert_eq!(config.min_connections, 2);
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.connect_timeout, "30s");
        assert_eq!(config.idle_timeout, "10m");
        assert_eq!(config.max_lifetime, "30m");
    }

    #[test]
    fn test_pool_config_custom() {
        let toml = r#"
            [database]
            provider = "postgresql"

            [database.pool]
            min_connections = 1
            max_connections = 50
            connect_timeout = "10s"
            idle_timeout = "30m"
            max_lifetime = "2h"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.database.pool.min_connections, 1);
        assert_eq!(config.database.pool.max_connections, 50);
        assert_eq!(config.database.pool.connect_timeout, "10s");
    }

    // ==================== SchemaConfig Tests ====================

    #[test]
    fn test_schema_config_default() {
        let config = SchemaConfig::default();
        assert_eq!(config.path, "schema.prax");
    }

    #[test]
    fn test_schema_config_custom() {
        let toml = r#"
            [schema]
            path = "db/schema.prax"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.schema.path, "db/schema.prax");
    }

    // ==================== GeneratorConfig Tests ====================

    #[test]
    fn test_generator_config_default() {
        let config = GeneratorConfig::default();
        assert_eq!(config.client.output, "./src/generated");
        assert!(config.client.async_client);
        assert!(!config.client.tracing);
        assert!(config.client.preview_features.is_empty());
        assert_eq!(config.client.model_style, ModelStyle::Standard);
    }

    #[test]
    fn test_generator_config_custom() {
        let toml = r#"
            [generator.client]
            output = "./generated"
            async_client = false
            tracing = true
            preview_features = ["feature1", "feature2"]
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.generator.client.output, "./generated");
        assert!(!config.generator.client.async_client);
        assert!(config.generator.client.tracing);
        assert_eq!(config.generator.client.preview_features.len(), 2);
    }

    #[test]
    fn test_generator_config_graphql_model_style() {
        let toml = r#"
            [generator.client]
            model_style = "graphql"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.generator.client.model_style, ModelStyle::GraphQL);
        assert!(config.generator.client.model_style.is_graphql());
    }

    #[test]
    fn test_generator_config_graphql_model_style_alias() {
        let toml = r#"
            [generator.client]
            model_style = "async-graphql"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.generator.client.model_style, ModelStyle::GraphQL);
    }

    #[test]
    fn test_model_style_standard_is_not_graphql() {
        assert!(!ModelStyle::Standard.is_graphql());
        assert!(ModelStyle::GraphQL.is_graphql());
    }

    // ==================== MigrationConfig Tests ====================

    #[test]
    fn test_migration_config_default() {
        let config = MigrationConfig::default();
        assert_eq!(config.directory, "./migrations");
        assert!(!config.auto_migrate);
        assert_eq!(config.table_name, "_prax_migrations");
    }

    #[test]
    fn test_migration_config_custom() {
        let toml = r#"
            [migrations]
            directory = "./db/migrate"
            auto_migrate = true
            table_name = "schema_migrations"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.migrations.directory, "./db/migrate");
        assert!(config.migrations.auto_migrate);
        assert_eq!(config.migrations.table_name, "schema_migrations");
    }

    // ==================== SeedConfig Tests ====================

    #[test]
    fn test_seed_config_default() {
        let config = SeedConfig::default();
        assert!(config.script.is_none());
        assert!(!config.auto_seed);
        assert!(config.environments.is_empty());
    }

    #[test]
    fn test_seed_config_custom() {
        let toml = r#"
            [seed]
            script = "seed.rs"
            auto_seed = true

            [seed.environments]
            dev = true
            prod = false
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(config.seed.script, Some("seed.rs".to_string()));
        assert!(config.seed.auto_seed);
        assert_eq!(config.seed.environments.get("dev"), Some(&true));
        assert_eq!(config.seed.environments.get("prod"), Some(&false));
    }

    // ==================== DebugConfig Tests ====================

    #[test]
    fn test_debug_config_default() {
        let config = DebugConfig::default();
        assert!(!config.log_queries);
        assert!(config.pretty_sql);
        assert_eq!(config.slow_query_threshold, 1000);
    }

    #[test]
    fn test_debug_config_custom() {
        let toml = r#"
            [debug]
            log_queries = true
            pretty_sql = false
            slow_query_threshold = 200
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert!(config.debug.log_queries);
        assert!(!config.debug.pretty_sql);
        assert_eq!(config.debug.slow_query_threshold, 200);
    }

    // ==================== Environment Variable Tests ====================

    #[test]
    fn test_env_var_expansion() {
        // SAFETY: This test runs single-threaded and we clean up after
        unsafe {
            std::env::set_var("TEST_DB_URL", "postgres://test");
        }
        let expanded = expand_env_vars("url = \"${TEST_DB_URL}\"");
        assert_eq!(expanded, "url = \"postgres://test\"");
        unsafe {
            std::env::remove_var("TEST_DB_URL");
        }
    }

    #[test]
    fn test_env_var_expansion_multiple() {
        unsafe {
            std::env::set_var("TEST_HOST", "localhost");
            std::env::set_var("TEST_PORT", "5432");
        }
        let content = "host = \"${TEST_HOST}\"\nport = \"${TEST_PORT}\"";
        let expanded = expand_env_vars(content);
        assert!(expanded.contains("localhost"));
        assert!(expanded.contains("5432"));
        unsafe {
            std::env::remove_var("TEST_HOST");
            std::env::remove_var("TEST_PORT");
        }
    }

    #[test]
    fn test_env_var_expansion_missing_var() {
        let content = "url = \"${DEFINITELY_NOT_SET_VAR_12345}\"";
        let expanded = expand_env_vars(content);
        // Should not expand missing variables
        assert_eq!(expanded, content);
    }

    #[test]
    fn test_env_var_expansion_in_config() {
        unsafe {
            std::env::set_var("TEST_DATABASE_URL_2", "postgres://user:pass@localhost/db");
        }

        let toml = r#"
            [database]
            url = "${TEST_DATABASE_URL_2}"
        "#;

        let config = PraxConfig::from_str(toml).unwrap();
        assert_eq!(
            config.database.url,
            Some("postgres://user:pass@localhost/db".to_string())
        );

        unsafe {
            std::env::remove_var("TEST_DATABASE_URL_2");
        }
    }

    // ==================== Environment Override Tests ====================

    #[test]
    fn test_environment_override_database_url() {
        let toml = r#"
            [database]
            url = "postgres://localhost/dev"

            [environments.test]
            [environments.test.database]
            url = "postgres://localhost/test_db"
        "#;

        let config = PraxConfig::from_str(toml).unwrap().with_environment("test");

        assert_eq!(
            config.database.url,
            Some("postgres://localhost/test_db".to_string())
        );
    }

    #[test]
    fn test_environment_override_pool() {
        let toml = r#"
            [database.pool]
            max_connections = 10

            [environments.production]
            [environments.production.database.pool]
            max_connections = 100
            min_connections = 10
        "#;

        let config = PraxConfig::from_str(toml)
            .unwrap()
            .with_environment("production");

        assert_eq!(config.database.pool.max_connections, 100);
        assert_eq!(config.database.pool.min_connections, 10);
    }

    #[test]
    fn test_environment_override_debug() {
        let toml = r#"
            [debug]
            log_queries = false
            pretty_sql = true

            [environments.development]
            [environments.development.debug]
            log_queries = true
            pretty_sql = false
            slow_query_threshold = 50
        "#;

        let config = PraxConfig::from_str(toml)
            .unwrap()
            .with_environment("development");

        assert!(config.debug.log_queries);
        assert!(!config.debug.pretty_sql);
        assert_eq!(config.debug.slow_query_threshold, 50);
    }

    // ==================== Serialization Tests ====================

    #[test]
    fn test_config_serialization() {
        let config = PraxConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("[database]"));
    }

    #[test]
    fn test_config_roundtrip() {
        let original = PraxConfig {
            database: DatabaseConfig {
                provider: DatabaseProvider::MySql,
                url: Some("mysql://localhost/test".to_string()),
                pool: PoolConfig::default(),
            },
            ..Default::default()
        };

        let toml_str = toml::to_string(&original).unwrap();
        let parsed: PraxConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.database.provider, original.database.provider);
        assert_eq!(parsed.database.url, original.database.url);
    }

    // ==================== Clone and Debug Tests ====================

    #[test]
    fn test_config_clone() {
        let config = PraxConfig::default();
        let cloned = config.clone();
        assert_eq!(config.database.provider, cloned.database.provider);
    }

    #[test]
    fn test_config_debug() {
        let config = PraxConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("PraxConfig"));
    }

    #[test]
    fn test_provider_equality() {
        assert_eq!(DatabaseProvider::PostgreSql, DatabaseProvider::PostgreSql);
        assert_ne!(DatabaseProvider::PostgreSql, DatabaseProvider::MySql);
    }

    // ==================== Default Function Tests ====================

    #[test]
    fn test_default_functions() {
        assert_eq!(default_provider(), DatabaseProvider::PostgreSql);
        assert_eq!(default_min_connections(), 2);
        assert_eq!(default_max_connections(), 10);
        assert_eq!(default_connect_timeout(), "30s");
        assert_eq!(default_idle_timeout(), "10m");
        assert_eq!(default_max_lifetime(), "30m");
        assert_eq!(default_schema_path(), "schema.prax");
        assert_eq!(default_output(), "./src/generated");
        assert!(default_true());
        assert_eq!(default_migrations_dir(), "./migrations");
        assert_eq!(default_migrations_table(), "_prax_migrations");
        assert_eq!(default_slow_query_threshold(), 1000);
    }
}
