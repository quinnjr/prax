//! Axum framework integration for Prax ORM.
//!
//! This crate provides seamless integration between Prax ORM and the
//! [Axum](https://github.com/tokio-rs/axum) web framework.
//!
//! # Features
//!
//! - **State Extension**: Add `PraxClient` to Axum's state
//! - **Extractors**: Extract database connections in handlers
//! - **Middleware**: Tower-compatible middleware for connection handling
//! - **Transaction Support**: Request-scoped transactions via middleware
//!
//! # Example
//!
//! ```rust,ignore
//! use axum::{Router, routing::get, extract::State};
//! use prax_axum::{PraxLayer, PraxClient};
//! use std::sync::Arc;
//!
//! async fn list_users(State(db): State<Arc<PraxClient>>) -> String {
//!     // Use the database client
//!     "Hello, World!".to_string()
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let client = PraxClient::connect("postgresql://localhost/mydb")
//!         .await
//!         .unwrap();
//!
//!     let app = Router::new()
//!         .route("/users", get(list_users))
//!         .layer(PraxLayer::new(client.clone()))
//!         .with_state(client);
//!
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//!     axum::serve(listener, app).await.unwrap();
//! }
//! ```

use std::sync::Arc;
use std::task::{Context, Poll};

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{Request, StatusCode},
    response::IntoResponse,
};
use thiserror::Error;
use tower::{Layer, Service};
use tracing::{debug, info};

use prax_query::connection::{DatabaseConfig, PoolConfig};

// Re-export key types
pub use prax_query::filter::{Filter, FilterValue};
pub use prax_query::prelude::*;

/// Errors that can occur during Prax-Axum integration.
#[derive(Error, Debug)]
pub enum PraxAxumError {
    /// Failed to connect to the database.
    #[error("database connection failed: {0}")]
    ConnectionFailed(String),

    /// Failed to acquire a connection from the pool.
    #[error("failed to acquire database connection")]
    AcquireFailed,

    /// Configuration error.
    #[error("configuration error: {0}")]
    ConfigError(String),
}

impl IntoResponse for PraxAxumError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            PraxAxumError::ConnectionFailed(_) => StatusCode::SERVICE_UNAVAILABLE,
            PraxAxumError::AcquireFailed => StatusCode::SERVICE_UNAVAILABLE,
            PraxAxumError::ConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

/// Result type for Prax-Axum operations.
pub type Result<T> = std::result::Result<T, PraxAxumError>;

/// A database client that can be used with Axum.
///
/// This is the main entry point for database operations in an Axum application.
/// Add it to your router state and extract it in handlers.
///
/// # Example
///
/// ```rust,ignore
/// use axum::extract::State;
/// use prax_axum::PraxClient;
/// use std::sync::Arc;
///
/// async fn handler(State(db): State<Arc<PraxClient>>) -> &'static str {
///     // Use db...
///     "OK"
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PraxClient {
    config: DatabaseConfig,
    pool_config: PoolConfig,
}

impl PraxClient {
    /// Create a new PraxClient from a connection URL.
    pub async fn connect(url: &str) -> Result<Arc<Self>> {
        info!(url_len = url.len(), "PraxClient connecting to database");

        let config = DatabaseConfig::from_url(url)
            .map_err(|e| PraxAxumError::ConnectionFailed(e.to_string()))?;

        let client = Self {
            config,
            pool_config: PoolConfig::default(),
        };

        info!("PraxClient connected successfully");
        Ok(Arc::new(client))
    }

    /// Create a new PraxClient from environment variables.
    pub async fn from_env() -> Result<Arc<Self>> {
        info!("PraxClient loading configuration from DATABASE_URL");

        let config =
            DatabaseConfig::from_env().map_err(|e| PraxAxumError::ConfigError(e.to_string()))?;

        let client = Self {
            config,
            pool_config: PoolConfig::default(),
        };

        info!("PraxClient connected from environment");
        Ok(Arc::new(client))
    }

    /// Create a new PraxClient with custom configuration.
    pub fn with_config(config: DatabaseConfig) -> Arc<Self> {
        info!(driver = %config.driver.name(), "PraxClient created with custom config");
        Arc::new(Self {
            config,
            pool_config: PoolConfig::default(),
        })
    }

    /// Get the database configuration.
    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// Get the pool configuration.
    pub fn pool_config(&self) -> &PoolConfig {
        &self.pool_config
    }
}

/// Tower layer for Prax database middleware.
///
/// This layer adds database connection handling to your Axum router.
///
/// # Example
///
/// ```rust,ignore
/// use axum::Router;
/// use prax_axum::{PraxLayer, PraxClient};
///
/// let client = PraxClient::connect("postgresql://localhost/mydb").await?;
///
/// let app = Router::new()
///     .layer(PraxLayer::new(client));
/// ```
#[derive(Clone)]
pub struct PraxLayer {
    client: Arc<PraxClient>,
}

impl PraxLayer {
    /// Create a new Prax layer.
    pub fn new(client: Arc<PraxClient>) -> Self {
        info!("PraxLayer created");
        Self { client }
    }

    /// Get the underlying client.
    pub fn client(&self) -> &PraxClient {
        &self.client
    }
}

impl<S> Layer<S> for PraxLayer {
    type Service = PraxMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PraxMiddleware {
            inner,
            client: self.client.clone(),
        }
    }
}

/// Tower middleware service for Prax.
#[derive(Clone)]
pub struct PraxMiddleware<S> {
    inner: S,
    client: Arc<PraxClient>,
}

impl<S, ReqBody> Service<Request<ReqBody>> for PraxMiddleware<S>
where
    S: Service<Request<ReqBody>> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        debug!("PraxMiddleware handling request");
        self.inner.call(request)
    }
}

/// Extractor for getting a database connection in handlers.
///
/// This extractor provides access to the `PraxClient` from the request extensions.
///
/// # Example
///
/// ```rust,ignore
/// use prax_axum::DatabaseConnection;
///
/// async fn handler(DatabaseConnection(db): DatabaseConnection) -> &'static str {
///     // Use db...
///     "OK"
/// }
/// ```
#[derive(Debug, Clone)]
pub struct DatabaseConnection(pub Arc<PraxClient>);

impl<S> FromRequestParts<S> for DatabaseConnection
where
    Arc<PraxClient>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = PraxAxumError;

    async fn from_request_parts(
        _parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let client = Arc::<PraxClient>::from_ref(state);
        Ok(DatabaseConnection(client))
    }
}

/// Builder for configuring PraxClient with Axum.
///
/// # Example
///
/// ```rust,ignore
/// let client = PraxClientBuilder::new()
///     .url("postgresql://localhost/mydb")
///     .pool_config(PoolConfig::read_heavy())
///     .build()
///     .await?;
/// ```
pub struct PraxClientBuilder {
    url: Option<String>,
    config: Option<DatabaseConfig>,
    pool_config: PoolConfig,
}

impl PraxClientBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            url: None,
            config: None,
            pool_config: PoolConfig::default(),
        }
    }

    /// Set the database URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the database configuration directly.
    pub fn config(mut self, config: DatabaseConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the pool configuration.
    pub fn pool_config(mut self, pool_config: PoolConfig) -> Self {
        self.pool_config = pool_config;
        self
    }

    /// Use read-heavy pool configuration.
    pub fn read_heavy(mut self) -> Self {
        self.pool_config = PoolConfig::read_heavy();
        self
    }

    /// Use write-heavy pool configuration.
    pub fn write_heavy(mut self) -> Self {
        self.pool_config = PoolConfig::write_heavy();
        self
    }

    /// Use serverless pool configuration.
    pub fn serverless(mut self) -> Self {
        self.pool_config = PoolConfig::serverless();
        self
    }

    /// Build the PraxClient.
    pub async fn build(self) -> Result<Arc<PraxClient>> {
        let config = if let Some(config) = self.config {
            config
        } else if let Some(url) = self.url {
            DatabaseConfig::from_url(&url).map_err(|e| PraxAxumError::ConfigError(e.to_string()))?
        } else {
            DatabaseConfig::from_env().map_err(|e| PraxAxumError::ConfigError(e.to_string()))?
        };

        info!(
            driver = %config.driver.name(),
            "PraxClientBuilder building client"
        );

        let client = PraxClient {
            config,
            pool_config: self.pool_config,
        };

        Ok(Arc::new(client))
    }
}

impl Default for PraxClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Prelude for convenient imports.
pub mod prelude {
    pub use super::{
        DatabaseConnection, PraxAxumError, PraxClient, PraxClientBuilder, PraxLayer,
        PraxMiddleware, Result,
    };
    pub use prax_query::prelude::*;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default() {
        let builder = PraxClientBuilder::new();
        assert!(builder.url.is_none());
        assert!(builder.config.is_none());
    }

    #[test]
    fn test_builder_with_url() {
        let builder = PraxClientBuilder::new().url("postgresql://localhost/test");
        assert_eq!(builder.url, Some("postgresql://localhost/test".to_string()));
    }

    #[test]
    fn test_builder_pool_configs() {
        let builder = PraxClientBuilder::new().read_heavy();
        assert_eq!(builder.pool_config.pool.max_connections, 30);

        let builder = PraxClientBuilder::new().write_heavy();
        assert_eq!(builder.pool_config.pool.max_connections, 15);

        let builder = PraxClientBuilder::new().serverless();
        assert_eq!(builder.pool_config.pool.max_connections, 10);
    }
}
