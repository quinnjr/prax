//! TLS connector selection shared by the pool and the listener.
//!
//! [`select_connector`] is the single place that maps a [`PgConfig`]'s
//! `ssl_mode` to a concrete tokio-postgres connector. The pool hands the
//! result to deadpool's `Manager`; [`crate::listener::PgListener`] matches on
//! it and calls `tokio_postgres::Config::connect` per arm, so the differing
//! `Connection<S, T>` stream types never escape the arm that created them.

use crate::config::{PgConfig, SslMode};
use crate::error::PgResult;

/// The connector a connection is established with.
#[derive(Clone)]
pub(crate) enum Connector {
    /// Plaintext (`tokio_postgres::NoTls`).
    NoTls,
    /// rustls, verifying against webpki roots or `ssl_root_cert`.
    #[cfg(feature = "tls")]
    Rustls(postgres_rustls::MakeTlsConnector),
}

/// Select the connector for `config.ssl_mode`.
///
/// `Disable` is always plaintext; TLS-requiring modes use rustls when the
/// `tls` feature is enabled and fail loudly otherwise (never a silent
/// downgrade). `Prefer` also connects without the feature, since plaintext is
/// within its contract.
pub(crate) fn select_connector(config: &PgConfig) -> PgResult<Connector> {
    #[cfg(feature = "tls")]
    {
        match config.ssl_mode {
            SslMode::Disable => Ok(Connector::NoTls),
            // `Prefer` keeps tokio-postgres's built-in behavior: TLS when the
            // server offers it, plaintext only when the server declines.
            //
            // `ssl_root_cert` replaces the webpki roots when set; the error is
            // raised here rather than at first connect so a bad path or an
            // unreadable bundle fails construction with a message naming the
            // file, instead of surfacing later as a handshake failure.
            _ => Ok(Connector::Rustls(
                crate::tls::make_tls_connector_with_root_cert(config.ssl_root_cert.as_deref())?,
            )),
        }
    }
    #[cfg(not(feature = "tls"))]
    {
        match config.ssl_mode {
            SslMode::Disable => Ok(Connector::NoTls),
            // Plaintext is within `Prefer`'s contract, so a no-tls build
            // still connects — nothing is downgraded below what the mode
            // promises. (Against a server that *offers* TLS the driver
            // still attempts the handshake and fails with the connector's
            // no-TLS error; use `Disable` for such servers.)
            SslMode::Prefer => Ok(Connector::NoTls),
            other => Err(crate::error::PgError::config(format!(
                "ssl_mode {:?} requires TLS support; rebuild prax-postgres \
                 with the `tls` feature (enabled by default)",
                other
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PgConfig;

    #[test]
    fn disable_selects_no_tls() {
        let config = PgConfig::from_url("postgres://u@h/db?sslmode=disable").unwrap();
        assert!(matches!(select_connector(&config), Ok(Connector::NoTls)));
    }

    #[cfg(feature = "tls")]
    #[test]
    fn require_selects_rustls_with_tls_feature() {
        let config = PgConfig::from_url("postgres://u@h/db?sslmode=require").unwrap();
        assert!(matches!(
            select_connector(&config),
            Ok(Connector::Rustls(_))
        ));
    }

    #[cfg(not(feature = "tls"))]
    #[test]
    fn require_is_a_config_error_without_tls_feature() {
        let config = PgConfig::from_url("postgres://u@h/db?sslmode=require").unwrap();
        assert!(matches!(
            select_connector(&config),
            Err(crate::error::PgError::Config(_))
        ));
    }
}
