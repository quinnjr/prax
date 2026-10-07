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
            .field(
                "options",
                &self
                    .options
                    .iter()
                    .map(|(key, value)| {
                        let value = if is_password_key(key) {
                            REDACTED
                        } else {
                            value
                        };
                        (key.as_str(), value)
                    })
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Shown in place of a password in `Debug` output.
const REDACTED: &str = "REDACTED";

/// `url` with its password replaced by [`REDACTED`], for `Debug` output.
/// Only the password inside the user info (`user:password@`) is touched;
/// the rest of the URL is kept exactly as written.
///
/// `password` is the config's password, when known. It is matched right
/// after `user:` first, because the config builder writes it into the URL
/// without percent-encoding, where an `@` or `/` in it would not parse as
/// one URL password. Otherwise the parser locates the password. When it
/// finds none although an `@` follows the scheme and no user name parsed
/// (an unencoded password starting with digits and then `/`, `?` or `#`
/// parses as a port), or the URL does not parse at all, everything between
/// the scheme and its last `@` is redacted. libpq-style `password` query
/// parameters are redacted too.
pub(crate) fn redact_url(url: &str, password: Option<&str>) -> String {
    let Some(start) = url.find("://").map(|i| i + 3) else {
        return url.to_owned();
    };
    let redacted = redact_userinfo(url, start, password);
    redact_query_passwords(&redacted, start)
}

/// The user-info part of [`redact_url`].
fn redact_userinfo(url: &str, start: usize, password: Option<&str>) -> String {
    if let Some(password) = password.filter(|p| !p.is_empty())
        && let Some(redacted) = splice_password(url, start, password)
    {
        return redacted;
    }
    match url::Url::parse(url) {
        Ok(parsed) => match parsed.password() {
            Some(password) => splice_password(url, start, password)
                .unwrap_or_else(|| redact_to_last_at(url, start)),
            None if parsed.username().is_empty() && url[start..].contains('@') => {
                redact_to_last_at(url, start)
            }
            None => url.to_owned(),
        },
        Err(_) => redact_to_last_at(url, start),
    }
}

/// `url` with the value of every `password` query parameter (any case,
/// percent-decoded key) replaced. The query is searched from `start`.
fn redact_query_passwords(url: &str, start: usize) -> String {
    let Some(query_start) = url[start..].find('?').map(|i| start + i + 1) else {
        return url.to_owned();
    };
    let query_end = url[query_start..]
        .find('#')
        .map_or(url.len(), |i| query_start + i);
    let query = url[query_start..query_end]
        .split('&')
        .map(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            if is_password_key(key) {
                format!("{key}={REDACTED}")
            } else {
                pair.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{}{query}{}", &url[..query_start], &url[query_end..])
}

/// Whether a (possibly percent-encoded) query or option key names a
/// password.
fn is_password_key(key: &str) -> bool {
    url::form_urlencoded::parse(key.as_bytes())
        .next()
        .is_some_and(|(key, _)| key.eq_ignore_ascii_case("password"))
}

/// `url` with `password` replaced, when it follows the first `:` of the
/// user info (which starts at `start`) and is followed by `@`.
fn splice_password(url: &str, start: usize, password: &str) -> Option<String> {
    let from = start + url[start..].find(':')? + 1;
    let after = url[from..].strip_prefix(password)?;
    after
        .starts_with('@')
        .then(|| format!("{}{REDACTED}{after}", &url[..from]))
}

/// `url` with everything from `start` to its last `@` replaced.
fn redact_to_last_at(url: &str, start: usize) -> String {
    match url[start..].rfind('@') {
        Some(at) => format!("{}{REDACTED}{}", &url[..start], &url[start + at..]),
        None => url.to_owned(),
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

/// How to fix a URL whose password breaks its structure.
const PERCENT_ENCODE_HINT: &str = "percent-encode reserved characters in the password \
     (`@` as %40, `/` as %2F, `?` as %3F, `#` as %23, `:` as %3A)";

/// Reject a URL in which an unencoded `/`, `?` or `#` in the password ended
/// the authority early, so the rest of the password and the real host
/// landed in the path, query or fragment (`postgres://u:8/x@host/db` parses
/// as host `u`, port 8 and database `x@host/db`). The telltale is an `@`
/// after the authority: in the path, the fragment, or a query key (a query
/// *value* such as `application_name=me@work` is fine). An `@` in a
/// database name must be written `%40`. The error never repeats the URL.
fn reject_unencoded_userinfo(parsed: &url::Url) -> PgResult<()> {
    let in_query_key = parsed.query().is_some_and(|query| {
        query.split('&').any(|pair| {
            pair.split_once('=')
                .map_or(pair, |(key, _)| key)
                .contains('@')
        })
    });
    if parsed.path().contains('@')
        || parsed
            .fragment()
            .is_some_and(|fragment| fragment.contains('@'))
        || in_query_key
    {
        return Err(PgError::config(format!(
            "invalid database URL: an `@` follows the host, so the password \
             seems to contain unencoded reserved characters; {PERCENT_ENCODE_HINT}"
        )));
    }
    Ok(())
}

/// Percent-decode a URL component (`what` names it in the error), as libpq
/// does for the user, password and database name.
fn percent_decode(component: &str, what: &str) -> PgResult<String> {
    percent_encoding::percent_decode_str(component)
        .decode_utf8()
        .map(|decoded| decoded.into_owned())
        .map_err(|_| {
            PgError::config(format!(
                "invalid database URL: the {what} is not valid UTF-8 once percent-decoded"
            ))
        })
}

impl PgConfig {
    /// Create a new configuration from a database URL.
    ///
    /// The user name, password and database name are percent-decoded, as
    /// libpq does: write reserved characters in them (`@`, `/`, `?`, `#`,
    /// `:`) percent-encoded. A URL whose password evidently contains them
    /// unencoded (an `@` after the host) is rejected with a configuration
    /// error rather than parsed into the wrong host, port or database.
    pub fn from_url(url: impl Into<String>) -> PgResult<Self> {
        let url = url.into();
        let parsed = url::Url::parse(&url).map_err(|e| {
            // An unencoded `/`, `?` or `#` in the password ends the authority
            // early, which often leaves an invalid port behind.
            let hint = if url.contains('@') {
                format!("; {PERCENT_ENCODE_HINT}")
            } else {
                String::new()
            };
            PgError::config(format!("invalid database URL: {e}{hint}"))
        })?;
        reject_unencoded_userinfo(&parsed)?;

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

        let database = percent_decode(parsed.path().trim_start_matches('/'), "database name")?;

        if database.is_empty() {
            return Err(PgError::config("missing database name in URL"));
        }

        let user = if parsed.username().is_empty() {
            "postgres".to_string()
        } else {
            percent_decode(parsed.username(), "user name")?
        };

        let password = parsed
            .password()
            .map(|password| percent_decode(password, "password"))
            .transpose()?;

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
    fn url_redaction_touches_only_the_password() {
        // A password equal to the user, host or database name (or a single
        // letter of them) must not be replaced anywhere else.
        assert_eq!(
            redact_url(
                "postgres://postgres:postgres@localhost:5432/postgres",
                Some("postgres")
            ),
            "postgres://postgres:REDACTED@localhost:5432/postgres"
        );
        assert_eq!(
            redact_url(
                "postgres://app:a@db.internal:5432/app_db?sslmode=disable",
                Some("a")
            ),
            "postgres://app:REDACTED@db.internal:5432/app_db?sslmode=disable"
        );
        // Without a hint the parser finds the password; the rest of the URL
        // is kept as written.
        assert_eq!(
            redact_url("postgres://app:a@db.internal/app_db", None),
            "postgres://app:REDACTED@db.internal/app_db"
        );
        let config =
            PgConfig::from_url("postgres://postgres:postgres@localhost:5432/postgres").unwrap();
        let debug = format!("{config:?}");
        assert!(
            debug.contains("\"postgres://postgres:REDACTED@localhost:5432/postgres\""),
            "{debug}"
        );
        assert!(debug.contains("database: \"postgres\""), "{debug}");
    }

    #[test]
    fn url_redaction_without_a_password_hint() {
        // An unencoded password starting with digits and then `/`, `?` or
        // `#` parses as a port, so the parser reports no password.
        for url in [
            format!("postgres://u:8/{SENTINEL}@host/db"),
            format!("postgres://u:123#{SENTINEL}@host/db"),
            format!("postgres://u:123?{SENTINEL}@host/db"),
        ] {
            let redacted = redact_url(&url, None);
            assert!(!redacted.contains(SENTINEL), "{url} -> {redacted}");
            assert_eq!(redacted, "postgres://REDACTED@host/db", "{url}");
        }
        // libpq-style `?password=` (key case and encoding ignored).
        for url in [
            format!("postgres://u@host/db?password={SENTINEL}&sslmode=disable"),
            format!("postgres://u@host/db?sslmode=disable&PASSWORD={SENTINEL}#frag"),
            format!("postgres://u@host/db?pass%77ord={SENTINEL}"),
        ] {
            let redacted = redact_url(&url, None);
            assert!(!redacted.contains(SENTINEL), "{url} -> {redacted}");
            assert!(redacted.starts_with("postgres://u@host/db?"), "{redacted}");
        }
        assert_eq!(
            redact_url(
                &format!("postgres://u@host/db?password={SENTINEL}&sslmode=disable"),
                None
            ),
            "postgres://u@host/db?password=REDACTED&sslmode=disable"
        );
        // A user name without a password stays.
        assert_eq!(
            redact_url("postgres://u@host/db", None),
            "postgres://u@host/db"
        );
    }

    #[test]
    fn debug_redacts_password_options_and_query() {
        let url = format!("postgres://u@db.internal/prax?password={SENTINEL}&search_path=app");
        let config = PgConfig::from_url(&url).unwrap();
        // `from_url` keeps the parameter in `options`.
        assert!(config.options.iter().any(|(k, _)| k == "password"));
        let debug = format!("{config:?}");
        assert!(!debug.contains(SENTINEL), "{debug}");
        assert!(debug.contains("search_path"), "{debug}");

        let builder = crate::pool::PgPoolBuilder::new().url(&url);
        let debug = format!("{builder:?}");
        assert!(!debug.contains(SENTINEL), "{debug}");

        let builder = crate::pool::PgPoolBuilder::new()
            .url(format!("postgres://u:8/{SENTINEL}@db.internal/prax"));
        let debug = format!("{builder:?}");
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
    fn from_url_rejects_passwords_with_unencoded_reserved_characters() {
        for url in [
            "postgres://u:8/Hx@host/db",
            "postgres://u:123#456@host/db",
            "postgres://u:123?x@host/db",
            "postgres://u:a@b/c@host/db",
            "postgres://u:8/x:y@host:5432/db?sslmode=disable",
        ] {
            match PgConfig::from_url(url) {
                Err(PgError::Config(message)) => {
                    assert!(message.contains("percent-encode"), "{url}: {message}");
                    // The error never repeats the URL (or the password).
                    assert!(
                        !message.contains("Hx") && !message.contains("456"),
                        "{message}"
                    );
                }
                other => panic!("{url}: expected a config error, got {other:?}"),
            }
        }
    }

    #[test]
    fn from_url_decodes_percent_encoded_credentials_and_database() {
        let config = PgConfig::from_url("postgres://u:8%2FHx@host/db").unwrap();
        assert_eq!(
            (
                config.user.as_str(),
                config.password.as_deref(),
                config.host.as_str()
            ),
            ("u", Some("8/Hx"), "host")
        );
        assert_eq!((config.port, config.database.as_str()), (5432, "db"));

        let config = PgConfig::from_url("postgres://u:123%23456@host:6543/db").unwrap();
        assert_eq!(config.password.as_deref(), Some("123#456"));
        assert_eq!(config.port, 6543);

        let config =
            PgConfig::from_url("postgres://app%40corp:p%40ss%3Aw%2B@host/my%40db").unwrap();
        assert_eq!(config.user, "app@corp");
        assert_eq!(config.password.as_deref(), Some("p@ss:w+"));
        assert_eq!(config.database, "my@db");
        let pg_config = config.to_pg_config();
        assert_eq!(pg_config.get_password(), Some(&b"p@ss:w+"[..]));
        assert_eq!(pg_config.get_user(), Some("app@corp"));

        // An `@` inside a query value is not mistaken for user info.
        let config =
            PgConfig::from_url("postgres://u:pw@host/db?application_name=me@work").unwrap();
        assert_eq!(config.application_name.as_deref(), Some("me@work"));
        assert_eq!(config.password.as_deref(), Some("pw"));
    }

    #[test]
    fn test_config_invalid_scheme() {
        let result = PgConfig::from_url("mysql://localhost/db");
        assert!(result.is_err());
    }
}
