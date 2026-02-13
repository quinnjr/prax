//! CLI configuration handling.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::CliResult;

/// Default config file name (lives in project root)
pub const CONFIG_FILE_NAME: &str = "prax.toml";

/// Default Prax directory name
pub const PRAX_DIR: &str = "prax";

/// Default schema file name (relative to prax directory)
pub const SCHEMA_FILE_NAME: &str = "schema.prax";

/// Default schema file path (relative to project root)
pub const SCHEMA_FILE_PATH: &str = "prax/schema.prax";

/// Default migrations directory (relative to project root)
pub const MIGRATIONS_DIR: &str = "prax/migrations";

/// Default seeds directory (relative to project root)
pub const SEEDS_DIR: &str = "prax/seeds";

/// Prax CLI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Database configuration
    pub database: DatabaseConfig,

    /// Generator configuration
    pub generator: GeneratorConfig,

    /// Migration configuration
    pub migrations: MigrationConfig,

    /// Seed configuration
    pub seed: SeedConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
            generator: GeneratorConfig::default(),
            migrations: MigrationConfig::default(),
            seed: SeedConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from a file
    pub fn load(path: &Path) -> CliResult<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save configuration to a file
    pub fn save(&self, path: &Path) -> CliResult<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Create a default config for a specific provider
    pub fn default_for_provider(provider: &str) -> Self {
        let mut config = Self::default();
        config.database.provider = provider.to_string();
        config
    }
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    /// Database provider (postgresql, mysql, sqlite)
    pub provider: String,

    /// Database connection URL
    pub url: Option<String>,

    /// Shadow database URL (for safe migrations)
    pub shadow_url: Option<String>,

    /// Direct database URL (bypasses connection pooling)
    pub direct_url: Option<String>,

    /// Path to seed file
    pub seed_path: Option<PathBuf>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            provider: "postgresql".to_string(),
            url: None,
            shadow_url: None,
            direct_url: None,
            seed_path: None,
        }
    }
}

/// Generator configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratorConfig {
    /// Output directory for generated code
    pub output: String,

    /// Features to enable (serde, graphql, etc.)
    pub features: Option<Vec<String>>,

    /// Custom prelude imports
    pub prelude: Option<Vec<String>>,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            output: "./src/generated".to_string(),
            features: None,
            prelude: None,
        }
    }
}

/// Migration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MigrationConfig {
    /// Directory for migration files
    pub directory: String,

    /// Migration table name
    pub table_name: String,

    /// Schema for migration table (PostgreSQL only)
    pub schema: Option<String>,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            directory: MIGRATIONS_DIR.to_string(),
            table_name: "_prax_migrations".to_string(),
            schema: None,
        }
    }
}

/// Seed configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SeedConfig {
    /// Directory for seed files
    pub directory: String,

    /// Path to seed script (relative to seeds directory or absolute)
    pub script: Option<PathBuf>,

    /// Run seed automatically after migrations
    pub auto_seed: bool,

    /// Environment-specific seeding
    /// Key: environment name, Value: whether to seed in that environment
    pub environments: std::collections::HashMap<String, bool>,
}

impl Default for SeedConfig {
    fn default() -> Self {
        let mut environments = std::collections::HashMap::new();
        environments.insert("development".to_string(), true);
        environments.insert("test".to_string(), true);
        environments.insert("staging".to_string(), false);
        environments.insert("production".to_string(), false);

        Self {
            directory: SEEDS_DIR.to_string(),
            script: None,
            auto_seed: false,
            environments,
        }
    }
}

impl SeedConfig {
    /// Check if seeding should run for the given environment
    pub fn should_seed(&self, environment: &str) -> bool {
        self.environments.get(environment).copied().unwrap_or(false)
    }
}
