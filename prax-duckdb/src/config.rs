//! DuckDB configuration.

use std::path::{Path, PathBuf};

use crate::error::{DuckDbError, DuckDbResult};

/// DuckDB database path.
#[derive(Debug, Clone)]
pub enum DatabasePath {
    /// In-memory database.
    InMemory,
    /// File-based database.
    File(PathBuf),
}

impl DatabasePath {
    /// Get the path string for DuckDB.
    pub fn as_str(&self) -> &str {
        match self {
            Self::InMemory => ":memory:",
            Self::File(path) => path.to_str().unwrap_or(":memory:"),
        }
    }
}

/// Database access mode.
#[derive(Debug, Clone, Copy, Default)]
pub enum AccessMode {
    /// Read-write access (default).
    #[default]
    ReadWrite,
    /// Read-only access.
    ReadOnly,
}

impl AccessMode {
    /// Convert to DuckDB config value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadWrite => "read_write",
            Self::ReadOnly => "read_only",
        }
    }
}

/// Thread safety mode.
#[derive(Debug, Clone, Copy, Default)]
pub enum ThreadMode {
    /// Use multiple threads (default).
    #[default]
    MultiThreaded,
    /// Single-threaded mode.
    SingleThreaded,
}

/// DuckDB configuration.
#[derive(Debug, Clone)]
pub struct DuckDbConfig {
    /// Database path.
    pub path: DatabasePath,
    /// Access mode.
    pub access_mode: AccessMode,
    /// Number of threads for parallel execution.
    pub threads: Option<usize>,
    /// Memory limit (e.g., "4GB").
    pub memory_limit: Option<String>,
    /// Enable external access (file system, network).
    pub enable_external_access: bool,
    /// Enable object cache.
    pub enable_object_cache: bool,
    /// Maximum memory for aggregation (before spilling to disk).
    pub max_memory: Option<String>,
    /// Temporary directory for spilling.
    pub temp_directory: Option<PathBuf>,
    /// Default null order (NULLS FIRST or NULLS LAST).
    pub default_null_order: Option<String>,
    /// Default order type (ASC or DESC).
    pub default_order: Option<String>,
    /// Enable progress bar for long queries.
    pub enable_progress_bar: bool,
}

impl Default for DuckDbConfig {
    fn default() -> Self {
        Self {
            path: DatabasePath::InMemory,
            access_mode: AccessMode::ReadWrite,
            threads: None,
            memory_limit: None,
            enable_external_access: true,
            enable_object_cache: true,
            max_memory: None,
            temp_directory: None,
            default_null_order: None,
            default_order: None,
            enable_progress_bar: false,
        }
    }
}

impl DuckDbConfig {
    /// Create a new in-memory configuration.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Create a configuration from a file path.
    pub fn from_path(path: impl AsRef<Path>) -> DuckDbResult<Self> {
        let path = path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() && !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        Ok(Self {
            path: DatabasePath::File(path.to_path_buf()),
            ..Self::default()
        })
    }

    /// Parse a connection URL.
    ///
    /// Supported formats:
    /// - `duckdb://` or `duckdb://:memory:` - In-memory database
    /// - `duckdb:///path/to/file.duckdb` - File-based database
    /// - `duckdb:///path/to/file.duckdb?threads=4&memory_limit=4GB`
    pub fn from_url(url: &str) -> DuckDbResult<Self> {
        let url = url.trim();

        // Check scheme
        if !url.starts_with("duckdb://") {
            return Err(DuckDbError::config(format!(
                "Invalid URL scheme, expected 'duckdb://', got: {}",
                url
            )));
        }

        let rest = &url[9..]; // Remove "duckdb://"

        // Parse path and query string
        let (path_str, query) = if let Some(idx) = rest.find('?') {
            (&rest[..idx], Some(&rest[idx + 1..]))
        } else {
            (rest, None)
        };

        // Determine database path
        let path = if path_str.is_empty() || path_str == ":memory:" {
            DatabasePath::InMemory
        } else {
            DatabasePath::File(PathBuf::from(path_str))
        };

        let mut config = Self {
            path,
            ..Self::default()
        };

        // Parse query parameters
        if let Some(query) = query {
            for param in query.split('&') {
                if let Some((key, value)) = param.split_once('=') {
                    match key {
                        "threads" => {
                            config.threads = value.parse().ok();
                        }
                        "memory_limit" => {
                            config.memory_limit = Some(value.to_string());
                        }
                        "max_memory" => {
                            config.max_memory = Some(value.to_string());
                        }
                        "access_mode" | "mode" => {
                            config.access_mode = match value {
                                "read_only" | "readonly" | "ro" => AccessMode::ReadOnly,
                                _ => AccessMode::ReadWrite,
                            };
                        }
                        "external_access" => {
                            config.enable_external_access = value == "true" || value == "1";
                        }
                        "object_cache" => {
                            config.enable_object_cache = value == "true" || value == "1";
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(config)
    }

    /// Create a builder for more complex configurations.
    pub fn builder() -> DuckDbConfigBuilder {
        DuckDbConfigBuilder::default()
    }

    /// Get the database path string.
    pub fn path_str(&self) -> &str {
        self.path.as_str()
    }

    /// Check if this is an in-memory database.
    pub fn is_in_memory(&self) -> bool {
        matches!(self.path, DatabasePath::InMemory)
    }

    /// Check if this is a read-only configuration.
    pub fn is_read_only(&self) -> bool {
        matches!(self.access_mode, AccessMode::ReadOnly)
    }
}

/// Builder for DuckDB configuration.
#[derive(Debug, Clone, Default)]
pub struct DuckDbConfigBuilder {
    config: DuckDbConfig,
}

impl DuckDbConfigBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the database path.
    pub fn path(mut self, path: impl AsRef<Path>) -> Self {
        self.config.path = DatabasePath::File(path.as_ref().to_path_buf());
        self
    }

    /// Use an in-memory database.
    pub fn in_memory(mut self) -> Self {
        self.config.path = DatabasePath::InMemory;
        self
    }

    /// Set the access mode.
    pub fn access_mode(mut self, mode: AccessMode) -> Self {
        self.config.access_mode = mode;
        self
    }

    /// Set read-only mode.
    pub fn read_only(mut self) -> Self {
        self.config.access_mode = AccessMode::ReadOnly;
        self
    }

    /// Set the number of threads.
    pub fn threads(mut self, threads: usize) -> Self {
        self.config.threads = Some(threads);
        self
    }

    /// Set the memory limit.
    pub fn memory_limit(mut self, limit: impl Into<String>) -> Self {
        self.config.memory_limit = Some(limit.into());
        self
    }

    /// Set the max memory for aggregation.
    pub fn max_memory(mut self, max: impl Into<String>) -> Self {
        self.config.max_memory = Some(max.into());
        self
    }

    /// Enable or disable external access.
    pub fn external_access(mut self, enable: bool) -> Self {
        self.config.enable_external_access = enable;
        self
    }

    /// Enable or disable object cache.
    pub fn object_cache(mut self, enable: bool) -> Self {
        self.config.enable_object_cache = enable;
        self
    }

    /// Set the temporary directory.
    pub fn temp_directory(mut self, path: impl AsRef<Path>) -> Self {
        self.config.temp_directory = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set the default null order.
    pub fn default_null_order(mut self, order: impl Into<String>) -> Self {
        self.config.default_null_order = Some(order.into());
        self
    }

    /// Set the default order.
    pub fn default_order(mut self, order: impl Into<String>) -> Self {
        self.config.default_order = Some(order.into());
        self
    }

    /// Enable or disable progress bar.
    pub fn progress_bar(mut self, enable: bool) -> Self {
        self.config.enable_progress_bar = enable;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> DuckDbConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_config() {
        let config = DuckDbConfig::in_memory();
        assert!(config.is_in_memory());
        assert_eq!(config.path_str(), ":memory:");
    }

    #[test]
    fn test_url_parsing_memory() {
        let config = DuckDbConfig::from_url("duckdb://").unwrap();
        assert!(config.is_in_memory());

        let config = DuckDbConfig::from_url("duckdb://:memory:").unwrap();
        assert!(config.is_in_memory());
    }

    #[test]
    fn test_url_parsing_file() {
        let config = DuckDbConfig::from_url("duckdb:///tmp/test.duckdb").unwrap();
        assert!(!config.is_in_memory());
        assert!(config.path_str().contains("test.duckdb"));
    }

    #[test]
    fn test_url_parsing_params() {
        let config =
            DuckDbConfig::from_url("duckdb://:memory:?threads=4&memory_limit=4GB").unwrap();
        assert!(config.is_in_memory());
        assert_eq!(config.threads, Some(4));
        assert_eq!(config.memory_limit, Some("4GB".to_string()));
    }

    #[test]
    fn test_builder() {
        let config = DuckDbConfig::builder()
            .in_memory()
            .threads(8)
            .memory_limit("8GB")
            .read_only()
            .build();

        assert!(config.is_in_memory());
        assert_eq!(config.threads, Some(8));
        assert_eq!(config.memory_limit, Some("8GB".to_string()));
        assert!(config.is_read_only());
    }
}
