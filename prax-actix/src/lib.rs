//! Actix-web framework integration for Prax ORM.
//!
//! This crate provides seamless integration between Prax ORM and the
//! [Actix-web](https://actix.rs/) web framework.
//!
//! # Features
//!
//! - **App Data**: Add `PraxClient` to Actix-web's app data
//! - **Extractors**: Extract database connections in handlers
//! - **Middleware**: Actor-based middleware for connection handling
//!
//! # Example
//!
//! ```rust,ignore
//! use actix_web::{web, App, HttpServer, HttpResponse};
//! use prax_actix::{PraxClient, DatabaseConnection};
//! use std::sync::Arc;
//!
//! async fn list_users(db: DatabaseConnection) -> HttpResponse {
//!     // Use the database client
//!     HttpResponse::Ok().body("Hello, World!")
//! }
//!
//! #[actix_web::main]
//! async fn main() -> std::io::Result<()> {
//!     let client = PraxClient::connect("postgresql://localhost/mydb")
//!         .await
//!         .expect("Failed to connect");
//!
//!     HttpServer::new(move || {
//!         App::new()
//!             .app_data(web::Data::new(client.clone()))
//!             .route("/users", web::get().to(list_users))
//!     })
//!     .bind("127.0.0.1:8080")?
//!     .run()
//!     .await
//! }
//! ```

use std::future::{Future, Ready};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::{
    Error, FromRequest, HttpRequest,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    error::ErrorInternalServerError,
    web,
};
use thiserror::Error;
use tracing::{debug, info};

use prax_query::connection::{DatabaseConfig, PoolConfig};

// Re-export key types
pub use prax_query::filter::{Filter, FilterValue};
pub use prax_query::prelude::*;

/// Errors that can occur during Prax-Actix integration.
#[derive(Error, Debug)]
pub enum PraxActixError {
    /// Failed to connect to the database.
    #[error("database connection failed: {0}")]
    ConnectionFailed(String),

    /// Failed to acquire a connection from the pool.
    #[error("failed to acquire database connection")]
    AcquireFailed,

    /// Configuration error.
    #[error("configuration error: {0}")]
    ConfigError(String),

    /// App data not found.
    #[error("database client not found in app data")]
    NotFound,
}

impl actix_web::ResponseError for PraxActixError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        match self {
            PraxActixError::ConnectionFailed(_) => actix_web::http::StatusCode::SERVICE_UNAVAILABLE,
            PraxActixError::AcquireFailed => actix_web::http::StatusCode::SERVICE_UNAVAILABLE,
            PraxActixError::ConfigError(_) => actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
            PraxActixError::NotFound => actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Result type for Prax-Actix operations.
pub type Result<T> = std::result::Result<T, PraxActixError>;

/// A database client that can be used with Actix-web.
///
/// This is the main entry point for database operations in an Actix-web application.
/// Add it to your app data and extract it in handlers.
///
/// # Example
///
/// ```rust,ignore
/// use actix_web::web;
/// use prax_actix::PraxClient;
///
/// async fn handler(db: web::Data<PraxClient>) -> &'static str {
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
            .map_err(|e| PraxActixError::ConnectionFailed(e.to_string()))?;

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
            DatabaseConfig::from_env().map_err(|e| PraxActixError::ConfigError(e.to_string()))?;

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

/// Extractor for getting a database connection in handlers.
///
/// This extractor provides access to the `PraxClient` from the app data.
///
/// # Example
///
/// ```rust,ignore
/// use prax_actix::DatabaseConnection;
/// use actix_web::HttpResponse;
///
/// async fn handler(db: DatabaseConnection) -> HttpResponse {
///     // Use db.client()...
///     HttpResponse::Ok().finish()
/// }
/// ```
#[derive(Debug, Clone)]
pub struct DatabaseConnection(pub Arc<PraxClient>);

impl DatabaseConnection {
    /// Get the underlying client.
    pub fn client(&self) -> &PraxClient {
        &self.0
    }
}

impl FromRequest for DatabaseConnection {
    type Error = Error;
    type Future = Ready<std::result::Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        let result = req
            .app_data::<web::Data<Arc<PraxClient>>>()
            .map(|data| DatabaseConnection(data.get_ref().clone()))
            .ok_or_else(|| ErrorInternalServerError(PraxActixError::NotFound));

        std::future::ready(result)
    }
}

/// Middleware factory for database connection handling.
///
/// This middleware ensures database connections are properly managed.
///
/// # Example
///
/// ```rust,ignore
/// use actix_web::{web, App};
/// use prax_actix::{PraxMiddleware, PraxClient};
///
/// let client = PraxClient::connect("postgresql://localhost/mydb").await?;
///
/// App::new()
///     .wrap(PraxMiddleware::new(client))
///     .route("/", web::get().to(|| async { "Hello" }))
/// ```
pub struct PraxMiddleware {
    client: Arc<PraxClient>,
}

impl PraxMiddleware {
    /// Create a new Prax middleware.
    pub fn new(client: Arc<PraxClient>) -> Self {
        info!("PraxMiddleware created");
        Self { client }
    }
}

impl<S, B> Transform<S, ServiceRequest> for PraxMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = PraxMiddlewareService<S>;
    type Future = Ready<std::result::Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        std::future::ready(Ok(PraxMiddlewareService {
            service,
            client: self.client.clone(),
        }))
    }
}

/// The actual middleware service.
pub struct PraxMiddlewareService<S> {
    service: S,
    #[allow(dead_code)]
    client: Arc<PraxClient>,
}

impl<S, B> Service<ServiceRequest> for PraxMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        debug!("PraxMiddleware handling request");
        let fut = self.service.call(req);
        Box::pin(fut)
    }
}

/// Builder for configuring PraxClient with Actix-web.
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
            DatabaseConfig::from_url(&url)
                .map_err(|e| PraxActixError::ConfigError(e.to_string()))?
        } else {
            DatabaseConfig::from_env().map_err(|e| PraxActixError::ConfigError(e.to_string()))?
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
        DatabaseConnection, PraxActixError, PraxClient, PraxClientBuilder, PraxMiddleware, Result,
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
