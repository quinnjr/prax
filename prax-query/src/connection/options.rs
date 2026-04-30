//! Connection and pool options.

use std::collections::HashMap;
use std::time::Duration;

/// SSL/TLS mode for connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SslMode {
    /// Disable SSL.
    Disable,
    /// Allow SSL but don't require it.
    Allow,
    /// Prefer SSL but allow non-SSL.
    #[default]
    Prefer,
    /// Require SSL.
    Require,
    /// Require SSL and verify the server certificate.
    VerifyCa,
    /// Require SSL and verify the server certificate and hostname.
    VerifyFull,
}

impl SslMode {
    /// Parse from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "disable" | "false" | "0" => Some(Self::Disable),
            "allow" => Some(Self::Allow),
            "prefer" => Some(Self::Prefer),
            "require" | "true" | "1" => Some(Self::Require),
            "verify-ca" | "verify_ca" => Some(Self::VerifyCa),
            "verify-full" | "verify_full" => Some(Self::VerifyFull),
            _ => None,
        }
    }

    /// Convert to string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Allow => "allow",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

/// SSL/TLS configuration.
#[derive(Debug, Clone, Default)]
pub struct SslConfig {
    /// SSL mode.
    pub mode: SslMode,
    /// Path to CA certificate.
    pub ca_cert: Option<String>,
    /// Path to client certificate.
    pub client_cert: Option<String>,
    /// Path to client key.
    pub client_key: Option<String>,
    /// Server name for SNI.
    pub server_name: Option<String>,
}

impl SslConfig {
    /// Create a new SSL config.
    pub fn new(mode: SslMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    /// Require SSL.
    pub fn require() -> Self {
        Self::new(SslMode::Require)
    }

    /// Set CA certificate path.
    pub fn with_ca_cert(mut self, path: impl Into<String>) -> Self {
        self.ca_cert = Some(path.into());
        self
    }

    /// Set client certificate path.
    pub fn with_client_cert(mut self, path: impl Into<String>) -> Self {
        self.client_cert = Some(path.into());
        self
    }

    /// Set client key path.
    pub fn with_client_key(mut self, path: impl Into<String>) -> Self {
        self.client_key = Some(path.into());
        self
    }
}

/// Common connection options.
#[derive(Debug, Clone)]
pub struct ConnectionOptions {
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Read timeout.
    pub read_timeout: Option<Duration>,
    /// Write timeout.
    pub write_timeout: Option<Duration>,
    /// SSL configuration.
    pub ssl: SslConfig,
    /// Application name.
    pub application_name: Option<String>,
    /// Schema/database to use after connecting.
    pub schema: Option<String>,
    /// Additional options as key-value pairs.
    pub extra: HashMap<String, String>,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            read_timeout: None,
            write_timeout: None,
            ssl: SslConfig::default(),
            application_name: None,
            schema: None,
            extra: HashMap::new(),
        }
    }
}

impl ConnectionOptions {
    /// Create new connection options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set read timeout.
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }

    /// Set write timeout.
    pub fn write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = Some(timeout);
        self
    }

    /// Set SSL mode.
    pub fn ssl_mode(mut self, mode: SslMode) -> Self {
        self.ssl.mode = mode;
        self
    }

    /// Set SSL configuration.
    pub fn ssl(mut self, config: SslConfig) -> Self {
        self.ssl = config;
        self
    }

    /// Set application name.
    pub fn application_name(mut self, name: impl Into<String>) -> Self {
        self.application_name = Some(name.into());
        self
    }

    /// Set schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Add extra option.
    pub fn option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Parse options from URL query parameters.
    pub fn from_params(params: &HashMap<String, String>) -> Self {
        let mut opts = Self::default();

        if let Some(timeout) = params.get("connect_timeout")
            && let Ok(secs) = timeout.parse::<u64>()
        {
            opts.connect_timeout = Duration::from_secs(secs);
        }

        if let Some(timeout) = params.get("read_timeout")
            && let Ok(secs) = timeout.parse::<u64>()
        {
            opts.read_timeout = Some(Duration::from_secs(secs));
        }

        if let Some(timeout) = params.get("write_timeout")
            && let Ok(secs) = timeout.parse::<u64>()
        {
            opts.write_timeout = Some(Duration::from_secs(secs));
        }

        if let Some(ssl) = params.get("sslmode").or_else(|| params.get("ssl"))
            && let Some(mode) = SslMode::parse(ssl)
        {
            opts.ssl.mode = mode;
        }

        if let Some(name) = params.get("application_name") {
            opts.application_name = Some(name.clone());
        }

        if let Some(schema) = params.get("schema").or_else(|| params.get("search_path")) {
            opts.schema = Some(schema.clone());
        }

        // Copy remaining params as extra options
        for (key, value) in params {
            if !matches!(
                key.as_str(),
                "connect_timeout"
                    | "read_timeout"
                    | "write_timeout"
                    | "sslmode"
                    | "ssl"
                    | "application_name"
                    | "schema"
                    | "search_path"
            ) {
                opts.extra.insert(key.clone(), value.clone());
            }
        }

        opts
    }
}

/// Pool options.
#[derive(Debug, Clone)]
pub struct PoolOptions {
    /// Maximum number of connections.
    pub max_connections: u32,
    /// Minimum number of connections to keep idle.
    pub min_connections: u32,
    /// Maximum time to wait for a connection.
    pub acquire_timeout: Duration,
    /// Maximum idle time before closing a connection.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection.
    pub max_lifetime: Option<Duration>,
    /// Test connections before returning them.
    pub test_before_acquire: bool,
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(600)),
            max_lifetime: Some(Duration::from_secs(1800)),
            test_before_acquire: true,
        }
    }
}

impl PoolOptions {
    /// Create new pool options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max connections.
    pub fn max_connections(mut self, n: u32) -> Self {
        self.max_connections = n;
        self
    }

    /// Set min connections.
    pub fn min_connections(mut self, n: u32) -> Self {
        self.min_connections = n;
        self
    }

    /// Set acquire timeout.
    pub fn acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = timeout;
        self
    }

    /// Set idle timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// Disable idle timeout.
    pub fn no_idle_timeout(mut self) -> Self {
        self.idle_timeout = None;
        self
    }

    /// Set max lifetime.
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = Some(lifetime);
        self
    }

    /// Disable max lifetime.
    pub fn no_max_lifetime(mut self) -> Self {
        self.max_lifetime = None;
        self
    }

    /// Enable/disable test before acquire.
    pub fn test_before_acquire(mut self, enabled: bool) -> Self {
        self.test_before_acquire = enabled;
        self
    }
}

/// PostgreSQL-specific options.
#[derive(Debug, Clone, Default)]
pub struct PostgresOptions {
    /// Statement cache capacity.
    pub statement_cache_capacity: usize,
    /// Enable prepared statements.
    pub prepared_statements: bool,
    /// Channel binding mode.
    pub channel_binding: Option<String>,
    /// Target session attributes.
    pub target_session_attrs: Option<String>,
}

impl PostgresOptions {
    /// Create new PostgreSQL options.
    pub fn new() -> Self {
        Self {
            statement_cache_capacity: 100,
            prepared_statements: true,
            channel_binding: None,
            target_session_attrs: None,
        }
    }

    /// Set statement cache capacity.
    pub fn statement_cache(mut self, capacity: usize) -> Self {
        self.statement_cache_capacity = capacity;
        self
    }

    /// Enable/disable prepared statements.
    pub fn prepared_statements(mut self, enabled: bool) -> Self {
        self.prepared_statements = enabled;
        self
    }
}

/// MySQL-specific options.
#[derive(Debug, Clone, Default)]
pub struct MySqlOptions {
    /// Enable compression.
    pub compression: bool,
    /// Character set.
    pub charset: Option<String>,
    /// Collation.
    pub collation: Option<String>,
    /// SQL mode.
    pub sql_mode: Option<String>,
    /// Timezone.
    pub timezone: Option<String>,
}

impl MySqlOptions {
    /// Create new MySQL options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable compression.
    pub fn compression(mut self, enabled: bool) -> Self {
        self.compression = enabled;
        self
    }

    /// Set character set.
    pub fn charset(mut self, charset: impl Into<String>) -> Self {
        self.charset = Some(charset.into());
        self
    }

    /// Set SQL mode.
    pub fn sql_mode(mut self, mode: impl Into<String>) -> Self {
        self.sql_mode = Some(mode.into());
        self
    }
}

/// SQLite-specific options.
#[derive(Debug, Clone)]
pub struct SqliteOptions {
    /// Journal mode.
    pub journal_mode: SqliteJournalMode,
    /// Synchronous mode.
    pub synchronous: SqliteSynchronous,
    /// Foreign keys enforcement.
    pub foreign_keys: bool,
    /// Busy timeout in milliseconds.
    pub busy_timeout: u32,
    /// Cache size in pages (negative for KB).
    pub cache_size: i32,
}

impl Default for SqliteOptions {
    fn default() -> Self {
        Self {
            journal_mode: SqliteJournalMode::Wal,
            synchronous: SqliteSynchronous::Normal,
            foreign_keys: true,
            busy_timeout: 5000,
            cache_size: -2000, // 2MB
        }
    }
}

impl SqliteOptions {
    /// Create new SQLite options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set journal mode.
    pub fn journal_mode(mut self, mode: SqliteJournalMode) -> Self {
        self.journal_mode = mode;
        self
    }

    /// Set synchronous mode.
    pub fn synchronous(mut self, mode: SqliteSynchronous) -> Self {
        self.synchronous = mode;
        self
    }

    /// Enable/disable foreign keys.
    pub fn foreign_keys(mut self, enabled: bool) -> Self {
        self.foreign_keys = enabled;
        self
    }

    /// Set busy timeout in milliseconds.
    pub fn busy_timeout(mut self, ms: u32) -> Self {
        self.busy_timeout = ms;
        self
    }

    /// Generate PRAGMA statements.
    pub fn to_pragmas(&self) -> Vec<String> {
        vec![
            format!("PRAGMA journal_mode = {};", self.journal_mode.as_str()),
            format!("PRAGMA synchronous = {};", self.synchronous.as_str()),
            format!(
                "PRAGMA foreign_keys = {};",
                if self.foreign_keys { "ON" } else { "OFF" }
            ),
            format!("PRAGMA busy_timeout = {};", self.busy_timeout),
            format!("PRAGMA cache_size = {};", self.cache_size),
        ]
    }
}

/// SQLite journal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqliteJournalMode {
    /// Delete journal after transaction.
    Delete,
    /// Truncate journal.
    Truncate,
    /// Persist journal.
    Persist,
    /// In-memory journal.
    Memory,
    /// Write-ahead logging (recommended).
    #[default]
    Wal,
    /// Disable journaling.
    Off,
}

impl SqliteJournalMode {
    /// Get the SQL string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::Persist => "PERSIST",
            Self::Memory => "MEMORY",
            Self::Wal => "WAL",
            Self::Off => "OFF",
        }
    }
}

/// SQLite synchronous mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqliteSynchronous {
    /// No synchronization.
    Off,
    /// Normal synchronization.
    #[default]
    Normal,
    /// Full synchronization.
    Full,
    /// Extra synchronization.
    Extra,
}

impl SqliteSynchronous {
    /// Get the SQL string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
            Self::Extra => "EXTRA",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssl_mode_parse() {
        assert_eq!(SslMode::parse("disable"), Some(SslMode::Disable));
        assert_eq!(SslMode::parse("require"), Some(SslMode::Require));
        assert_eq!(SslMode::parse("verify-full"), Some(SslMode::VerifyFull));
        assert_eq!(SslMode::parse("invalid"), None);
    }

    #[test]
    fn test_connection_options_builder() {
        let opts = ConnectionOptions::new()
            .connect_timeout(Duration::from_secs(10))
            .ssl_mode(SslMode::Require)
            .application_name("test-app");

        assert_eq!(opts.connect_timeout, Duration::from_secs(10));
        assert_eq!(opts.ssl.mode, SslMode::Require);
        assert_eq!(opts.application_name, Some("test-app".to_string()));
    }

    #[test]
    fn test_pool_options_builder() {
        let opts = PoolOptions::new()
            .max_connections(20)
            .min_connections(5)
            .no_idle_timeout();

        assert_eq!(opts.max_connections, 20);
        assert_eq!(opts.min_connections, 5);
        assert_eq!(opts.idle_timeout, None);
    }

    #[test]
    fn test_sqlite_options_pragmas() {
        let opts = SqliteOptions::new()
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pragmas = opts.to_pragmas();
        assert!(pragmas.iter().any(|p| p.contains("journal_mode = WAL")));
        assert!(pragmas.iter().any(|p| p.contains("foreign_keys = ON")));
    }

    #[test]
    fn test_options_from_params() {
        let mut params = HashMap::new();
        params.insert("connect_timeout".to_string(), "10".to_string());
        params.insert("sslmode".to_string(), "require".to_string());
        params.insert("application_name".to_string(), "myapp".to_string());

        let opts = ConnectionOptions::from_params(&params);
        assert_eq!(opts.connect_timeout, Duration::from_secs(10));
        assert_eq!(opts.ssl.mode, SslMode::Require);
        assert_eq!(opts.application_name, Some("myapp".to_string()));
    }
}
