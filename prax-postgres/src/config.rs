//! PostgreSQL connection configuration.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::{PgError, PgResult};

/// PostgreSQL connection configuration.
///
/// `Debug` output redacts [`password`](Self::password) and any password in
/// [`url`](Self::url).
#[non_exhaustive]
#[derive(Clone)]
pub struct PgConfig {
    /// Database URL.
    pub url: String,
    /// Host (extracted from URL or explicit).
    pub host: String,
    /// Port (default: 5432).
    pub port: u16,
    /// Database name.
    pub database: String,
    /// Username.
    pub user: String,
    /// Password.
    pub password: Option<String>,
    /// SSL mode.
    ///
    /// With the default `tls` cargo feature, TLS connections are established
    /// via rustls with certificates verified against the Mozilla root store
    /// (chain + hostname). Without the feature, any TLS-requiring mode fails
    /// at pool build time with a clear error — it is never silently
    /// downgraded to plaintext.
    pub ssl_mode: SslMode,
    /// Path to a PEM file of root certificates to verify the server against,
    /// from the libpq-compatible `sslrootcert` URL parameter.
    ///
    /// When set, these certificates *replace* the Mozilla root store rather
    /// than adding to it, matching libpq: a pool talks to one server, and
    /// "trust exactly this bundle" is both the stricter and the more
    /// predictable reading.
    ///
    /// The case this exists for is a server whose CA is deliberately not
    /// publicly trusted — Amazon RDS being the common one, since its
    /// `rds-ca-*` authorities are Amazon-operated and absent from the Mozilla
    /// store, so the default configuration cannot verify them at all.
    pub ssl_root_cert: Option<PathBuf>,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Statement timeout.
    pub statement_timeout: Option<Duration>,
    /// Application name (shown in pg_stat_activity).
    pub application_name: Option<String>,
    /// Additional options.
    pub options: Vec<(String, String)>,
}

impl fmt::Debug for PgConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgConfig")
            .field("url", &redact_url(&self.url, self.password.as_deref()))
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| REDACTED))
            .field("ssl_mode", &self.ssl_mode)
            .field("ssl_root_cert", &self.ssl_root_cert)
            .field("connect_timeout", &self.connect_timeout)
            .field("statement_timeout", &self.statement_timeout)
            .field("application_name", &self.application_name)
            .field("options", &self.options)
            .finish()
    }
}

/// Shown in place of a password in `Debug` output.
const REDACTED: &str = "REDACTED";

/// `url` with its password replaced by [`REDACTED`], for `Debug` output.
///
/// The literal `password` is replaced first: the config builder writes it
/// into the URL without percent-encoding, where an `@` or `/` in it would
/// not parse as one URL password. Then any URL password is replaced through
/// the parser; a URL that does not parse loses everything between the
/// scheme and its last `@`.
pub(crate) fn redact_url(url: &str, password: Option<&str>) -> String {
    let url = match password {
        Some(password) if !password.is_empty() => url.replace(password, REDACTED),
        _ => url.to_owned(),
    };
    match url::Url::parse(&url) {
        Ok(mut parsed) => {
            if parsed.password().is_some() && parsed.set_password(Some(REDACTED)).is_ok() {
                parsed.to_string()
            } else {
                url
            }
        }
        Err(_) => {
            let Some(start) = url.find("://").map(|i| i + 3) else {
                return url;
            };
            match url[start..].rfind('@') {
                Some(at) => format!("{}{REDACTED}{}", &url[..start], &url[start + at..]),
                None => url,
            }
        }
    }
}

/// SSL mode for connections.
///
/// With the default `tls` cargo feature, `Require`/`VerifyCa`/`VerifyFull`
/// establish rustls-encrypted connections verified against the Mozilla root
/// store. `Prefer` uses TLS when the server offers it and falls back to
/// plaintext only when the server declines TLS (note: stricter than libpq —
/// a certificate verification failure fails the connection rather than
/// retrying plaintext). Without the `tls` feature, TLS-requiring modes fail
/// at pool build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SslMode {
    /// Disable SSL.
    Disable,
    /// Prefer SSL but allow non-SSL when the server declines TLS.
    #[default]
    Prefer,
    /// Require SSL. Certificates are verified (chain + hostname) — stricter
    /// than libpq's `require`, which skips verification.
    Require,
    /// Require SSL and verify the certificate chain. Currently also verifies
    /// the hostname (i.e. behaves as `VerifyFull`; libpq's hostname-less
    /// `verify-ca` is not yet distinguished).
    VerifyCa,
    /// Require SSL and verify the certificate chain and hostname.
    VerifyFull,
}

/// Validate a GUC name destined for the `options` startup parameter.
/// Postgres GUC names match `^[A-Za-z_][A-Za-z0-9_.]*$` (the `.`
/// separates extension namespaces, e.g. `pg_trgm.similarity_threshold`).
fn is_valid_guc_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Reject values that could break out of the space-joined `options`
/// startup parameter: whitespace terminates the assignment and starts a
/// new token, and `\` / `'` are libpq's quoting characters in that
/// string. A percent-decoded URL value could otherwise smuggle extra
/// `-c key=value` assignments (e.g. `?x=1%20-c%20search_path%3Devil`).
fn is_safe_guc_value(value: &str) -> bool {
    !value
        .chars()
        .any(|c| c.is_whitespace() || c == '\\' || c == '\'')
}

impl PgConfig {
    /// Create a new configuration from a database URL.
    pub fn from_url(url: impl Into<String>) -> PgResult<Self> {
        let url = url.into();
        let parsed = url::Url::parse(&url)
            .map_err(|e| PgError::config(format!("invalid database URL: {}", e)))?;

        if parsed.scheme() != "postgresql" && parsed.scheme() != "postgres" {
            return Err(PgError::config(format!(
                "invalid scheme: expected 'postgresql' or 'postgres', got '{}'",
                parsed.scheme()
            )));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| PgError::config("missing host in URL"))?
            .to_string();

        let port = parsed.port().unwrap_or(5432);

        let database = parsed.path().trim_start_matches('/').to_string();

        if database.is_empty() {
            return Err(PgError::config("missing database name in URL"));
        }

        let user = if parsed.username().is_empty() {
            "postgres".to_string()
        } else {
            parsed.username().to_string()
        };

        let password = parsed.password().map(String::from);

        // Parse query parameters
        let mut ssl_mode = SslMode::Prefer;
        let mut connect_timeout = Duration::from_secs(30);
        let mut statement_timeout = None;
        let mut application_name = None;
        let mut ssl_root_cert = None;
        let mut options = Vec::new();

        for (key, value) in parsed.query_pairs() {
            let key_str: &str = &key;
            let value_str: &str = &value;
            match key_str {
                "sslmode" => {
                    ssl_mode = match value_str {
                        "disable" => SslMode::Disable,
                        "prefer" => SslMode::Prefer,
                        "require" => SslMode::Require,
                        "verify-ca" => SslMode::VerifyCa,
                        "verify-full" => SslMode::VerifyFull,
                        other => {
                            return Err(PgError::config(format!("invalid sslmode: {}", other)));
                        }
                    };
                }
                "connect_timeout" => {
                    let secs: u64 = value_str
                        .parse()
                        .map_err(|_| PgError::config("invalid connect_timeout"))?;
                    connect_timeout = Duration::from_secs(secs);
                }
                "statement_timeout" => {
                    let ms: u64 = value_str
                        .parse()
                        .map_err(|_| PgError::config("invalid statement_timeout"))?;
                    statement_timeout = Some(Duration::from_millis(ms));
                }
                "application_name" => {
                    application_name = Some(value_str.to_string());
                }
                "sslrootcert" => {
                    ssl_root_cert = Some(PathBuf::from(value_str));
                }
                _ => {
                    options.push((key_str.to_string(), value_str.to_string()));
                }
            }
        }

        Ok(Self {
            url,
            host,
            port,
            database,
            user,
            password,
            ssl_mode,
            ssl_root_cert,
            connect_timeout,
            statement_timeout,
            application_name,
            options,
        })
    }

    /// Create a builder for configuration.
    pub fn builder() -> PgConfigBuilder {
        PgConfigBuilder::new()
    }

    /// Convert to tokio-postgres config.
    ///
    /// Applies everything `tokio_postgres::Config` can express without a TLS
    /// connector: host/port/dbname/user/password, `application_name`,
    /// `connect_timeout`, the driver's [`tokio_postgres::config::SslMode`],
    /// plus `statement_timeout` and any extra `options` (passed via the
    /// libpq-style `options` startup parameter as space-separated
    /// `-c key=value` pairs).
    ///
    /// Option pairs whose key is not a valid GUC name, or whose value
    /// contains whitespace / `\` / `'`, are dropped with a warning:
    /// percent-decoded values could otherwise smuggle extra `-c`
    /// assignments into the space-joined string.
    ///
    /// `Require`/`VerifyCa`/`VerifyFull` all map to the driver's
    /// `SslMode::Require`; the pool supplies the rustls connector (with
    /// webpki certificate verification) that makes the mode satisfiable.
    pub fn to_pg_config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.dbname(&self.database);
        config.user(&self.user);

        if let Some(ref password) = self.password {
            config.password(password);
        }

        if let Some(ref app_name) = self.application_name {
            config.application_name(app_name);
        }

        config.connect_timeout(self.connect_timeout);

        let driver_ssl_mode = match self.ssl_mode {
            SslMode::Disable => tokio_postgres::config::SslMode::Disable,
            SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                tokio_postgres::config::SslMode::Require
            }
        };
        config.ssl_mode(driver_ssl_mode);

        // `statement_timeout` and arbitrary GUC options ride the libpq-style
        // `options` startup parameter as space-separated `-c key=value` pairs.
        let mut options = Vec::new();
        if let Some(timeout) = self.statement_timeout {
            // A bare integer is interpreted as milliseconds by PostgreSQL.
            options.push(format!("-c statement_timeout={}", timeout.as_millis()));
        }
        for (key, value) in &self.options {
            if !is_valid_guc_key(key) {
                tracing::warn!(key = %key, "dropping connection option with invalid GUC name");
                continue;
            }
            if !is_safe_guc_value(value) {
                // Name the key but never the value, so a malicious
                // payload doesn't land in the logs.
                tracing::warn!(
                    key = %key,
                    "dropping connection option whose value contains whitespace or quoting characters"
                );
                continue;
            }
            options.push(format!("-c {}={}", key, value));
        }
        if !options.is_empty() {
            config.options(options.join(" "));
        }

        config
    }
}

/// Builder for PostgreSQL configuration.
///
/// `Debug` output redacts the password, also inside the URL.
#[derive(Default)]
pub struct PgConfigBuilder {
    url: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    database: Option<String>,
    user: Option<String>,
    password: Option<String>,
    ssl_mode: Option<SslMode>,
    ssl_root_cert: Option<PathBuf>,
    connect_timeout: Option<Duration>,
    statement_timeout: Option<Duration>,
    application_name: Option<String>,
}

impl fmt::Debug for PgConfigBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgConfigBuilder")
            .field(
                "url",
                &self
                    .url
                    .as_deref()
                    .map(|url| redact_url(url, self.password.as_deref())),
            )
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| REDACTED))
            .field("ssl_mode", &self.ssl_mode)
            .field("ssl_root_cert", &self.ssl_root_cert)
            .field("connect_timeout", &self.connect_timeout)
            .field("statement_timeout", &self.statement_timeout)
            .field("application_name", &self.application_name)
            .finish()
    }
}

impl PgConfigBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the database URL (parses all connection parameters).
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the host.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Set the port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the database name.
    pub fn database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// Set the username.
    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set the password.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the SSL mode.
    pub fn ssl_mode(mut self, mode: SslMode) -> Self {
        self.ssl_mode = Some(mode);
        self
    }

    /// Set a PEM bundle of root certificates to verify the server against,
    /// replacing the Mozilla root store. Equivalent to the `sslrootcert` URL
    /// parameter.
    pub fn ssl_root_cert(mut self, path: impl Into<PathBuf>) -> Self {
        self.ssl_root_cert = Some(path.into());
        self
    }

    /// Set the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Set the statement timeout.
    pub fn statement_timeout(mut self, timeout: Duration) -> Self {
        self.statement_timeout = Some(timeout);
        self
    }

    /// Set the application name.
    pub fn application_name(mut self, name: impl Into<String>) -> Self {
        self.application_name = Some(name.into());
        self
    }

    /// Build the configuration.
    pub fn build(self) -> PgResult<PgConfig> {
        if let Some(url) = self.url {
            let mut config = PgConfig::from_url(url)?;

            // Override with explicit values
            if let Some(host) = self.host {
                config.host = host;
            }
            if let Some(port) = self.port {
                config.port = port;
            }
            if let Some(database) = self.database {
                config.database = database;
            }
            if let Some(user) = self.user {
                config.user = user;
            }
            if let Some(password) = self.password {
                config.password = Some(password);
            }
            if let Some(ssl_root_cert) = self.ssl_root_cert {
                config.ssl_root_cert = Some(ssl_root_cert);
            }
            if let Some(ssl_mode) = self.ssl_mode {
                config.ssl_mode = ssl_mode;
            }
            if let Some(timeout) = self.connect_timeout {
                config.connect_timeout = timeout;
            }
            if let Some(timeout) = self.statement_timeout {
                config.statement_timeout = Some(timeout);
            }
            if let Some(name) = self.application_name {
                config.application_name = Some(name);
            }

            Ok(config)
        } else {
            // Build from individual components
            let host = self.host.unwrap_or_else(|| "localhost".to_string());
            let port = self.port.unwrap_or(5432);
            let database = self
                .database
                .ok_or_else(|| PgError::config("database name is required"))?;
            let user = self.user.unwrap_or_else(|| "postgres".to_string());

            let url = format!(
                "postgresql://{}{}@{}:{}/{}",
                user,
                self.password
                    .as_ref()
                    .map(|p| format!(":{}", p))
                    .unwrap_or_default(),
                host,
                port,
                database
            );

            Ok(PgConfig {
                url,
                host,
                port,
                database,
                user,
                password: self.password,
                ssl_mode: self.ssl_mode.unwrap_or_default(),
                ssl_root_cert: self.ssl_root_cert,
                connect_timeout: self.connect_timeout.unwrap_or(Duration::from_secs(30)),
                statement_timeout: self.statement_timeout,
                application_name: self.application_name,
                options: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_url() {
        let config = PgConfig::from_url("postgresql://user:pass@localhost:5432/mydb").unwrap();
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 5432);
        assert_eq!(config.database, "mydb");
        assert_eq!(config.user, "user");
        assert_eq!(config.password, Some("pass".to_string()));
    }

    #[test]
    fn test_config_from_url_with_params() {
        let config =
            PgConfig::from_url("postgresql://localhost/mydb?sslmode=require&application_name=prax")
                .unwrap();
        assert_eq!(config.ssl_mode, SslMode::Require);
        assert_eq!(config.application_name, Some("prax".to_string()));
    }

    #[test]
    fn test_to_pg_config_applies_statement_timeout_and_options() {
        let config = PgConfig::from_url(
            "postgresql://localhost/mydb?statement_timeout=5000&search_path=public",
        )
        .unwrap();
        let pg_config = config.to_pg_config();
        assert_eq!(
            pg_config.get_options(),
            Some("-c statement_timeout=5000 -c search_path=public")
        );
    }

    #[test]
    fn test_to_pg_config_without_timeouts_or_options_sets_none() {
        let config = PgConfig::from_url("postgresql://localhost/mydb").unwrap();
        let pg_config = config.to_pg_config();
        assert_eq!(pg_config.get_options(), None);
    }

    #[test]
    fn test_to_pg_config_drops_option_with_smuggled_value() {
        // Percent-decoded whitespace in a value must not survive into
        // the space-joined `options` string, where it would smuggle in
        // extra `-c key=value` assignments.
        let config =
            PgConfig::from_url("postgresql://localhost/mydb?x=1%20-c%20search_path%3Devil")
                .unwrap();
        let pg_config = config.to_pg_config();
        assert_eq!(pg_config.get_options(), None);
    }

    #[test]
    fn test_to_pg_config_drops_option_with_invalid_key() {
        let config =
            PgConfig::from_url("postgresql://localhost/mydb?bad%20key=1&search_path=public")
                .unwrap();
        let pg_config = config.to_pg_config();
        // The invalid key is dropped; the valid option is kept.
        assert_eq!(pg_config.get_options(), Some("-c search_path=public"));
    }

    #[test]
    fn test_to_pg_config_drops_option_with_quoting_chars() {
        // `\` and `'` are libpq's quoting characters inside `options`.
        let config = PgConfig::from_url("postgresql://localhost/mydb?a=b%5Cc&d=e%27f").unwrap();
        let pg_config = config.to_pg_config();
        assert_eq!(pg_config.get_options(), None);
    }

    #[test]
    fn parses_sslrootcert_from_the_url() {
        let config = PgConfig::from_url(
            "postgresql://localhost/mydb?sslmode=verify-full&sslrootcert=/etc/ssl/rds.pem",
        )
        .unwrap();
        assert_eq!(
            config.ssl_root_cert,
            Some(std::path::PathBuf::from("/etc/ssl/rds.pem"))
        );
        // It must not fall through into the generic `options` bag, which would
        // send it to the server as a GUC.
        assert!(!config.options.iter().any(|(k, _)| k == "sslrootcert"));
    }

    #[test]
    fn sslrootcert_defaults_to_none() {
        let config = PgConfig::from_url("postgresql://localhost/mydb").unwrap();
        assert_eq!(config.ssl_root_cert, None);
    }

    #[test]
    fn builder_sets_sslrootcert() {
        let config = PgConfig::builder()
            .url("postgresql://localhost/mydb")
            .ssl_root_cert("/etc/ssl/override.pem")
            .build()
            .unwrap();
        assert_eq!(
            config.ssl_root_cert,
            Some(std::path::PathBuf::from("/etc/ssl/override.pem"))
        );
    }

    #[test]
    fn test_to_pg_config_maps_sslmode_require() {
        // TLS is supported via rustls: `require` maps to the driver's
        // `Require` (never downgraded).
        let config = PgConfig::from_url("postgresql://localhost/mydb?sslmode=require").unwrap();
        let pg_config = config.to_pg_config();
        assert_eq!(
            pg_config.get_ssl_mode(),
            tokio_postgres::config::SslMode::Require
        );
    }

    #[test]
    fn test_from_url_parses_verify_modes() {
        let config = PgConfig::from_url("postgresql://localhost/mydb?sslmode=verify-ca").unwrap();
        assert_eq!(config.ssl_mode, SslMode::VerifyCa);
        let pg_config = config.to_pg_config();
        assert_eq!(
            pg_config.get_ssl_mode(),
            tokio_postgres::config::SslMode::Require
        );

        let config = PgConfig::from_url("postgresql://localhost/mydb?sslmode=verify-full").unwrap();
        assert_eq!(config.ssl_mode, SslMode::VerifyFull);

        assert!(PgConfig::from_url("postgresql://localhost/mydb?sslmode=bogus").is_err());
    }

    #[test]
    fn test_to_pg_config_maps_sslmode_disable() {
        let config = PgConfig::from_url("postgresql://localhost/mydb?sslmode=disable").unwrap();
        let pg_config = config.to_pg_config();
        assert_eq!(
            pg_config.get_ssl_mode(),
            tokio_postgres::config::SslMode::Disable
        );
    }

    #[test]
    fn test_config_builder() {
        let config = PgConfig::builder()
            .host("localhost")
            .port(5432)
            .database("mydb")
            .user("postgres")
            .build()
            .unwrap();

        assert_eq!(config.host, "localhost");
        assert_eq!(config.database, "mydb");
    }

    const SENTINEL: &str = "hunter2-sentinel";

    #[test]
    fn debug_redacts_the_password_and_the_url_password() {
        let config = PgConfig::from_url(format!(
            "postgres://prax:{SENTINEL}@db.internal:5432/prax?application_name=app"
        ))
        .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains(SENTINEL), "{debug}");
        // Everything else stays visible.
        for visible in [
            "db.internal",
            "5432",
            "prax",
            "application_name=app",
            "REDACTED",
        ] {
            assert!(debug.contains(visible), "{visible}: {debug}");
        }
        let alternate = format!("{config:#?}");
        assert!(!alternate.contains(SENTINEL), "{alternate}");
    }

    #[test]
    fn debug_redacts_unencoded_builder_passwords() {
        // The builder writes the password into `url` verbatim, so `@` and
        // `/` in it do not parse as one URL password.
        let config = PgConfig::builder()
            .host("db.internal")
            .database("prax")
            .user("prax")
            .password(format!("{SENTINEL}@x/y"))
            .build()
            .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(!debug.contains("x/y"), "{debug}");

        // A URL that does not parse is redacted up to its last `@`.
        let mut config = config;
        config.url = format!("postgres://prax:{SENTINEL}@bad host/prax");
        let debug = format!("{config:?}");
        assert!(!debug.contains(SENTINEL), "{debug}");
    }

    #[test]
    fn debug_keeps_urls_without_a_password() {
        let config = PgConfig::from_url("postgres://prax@db.internal/prax").unwrap();
        let debug = format!("{config:?}");
        assert!(
            debug.contains("postgres://prax@db.internal/prax"),
            "{debug}"
        );
        assert!(debug.contains("password: None"), "{debug}");
    }

    #[test]
    fn config_builder_debug_redacts_secrets() {
        let builder = PgConfig::builder()
            .url(format!("postgres://prax:{SENTINEL}@db.internal/prax"))
            .password(SENTINEL);
        let debug = format!("{builder:?}");
        assert!(!debug.contains(SENTINEL), "{debug}");
        assert!(debug.contains("db.internal"), "{debug}");
    }

    #[test]
    fn test_config_invalid_scheme() {
        let result = PgConfig::from_url("mysql://localhost/db");
        assert!(result.is_err());
    }
}
