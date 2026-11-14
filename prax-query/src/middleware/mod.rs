#![allow(dead_code)]

//! Middleware system for query interception.
//!
//! This module provides a flexible middleware system that allows intercepting
//! queries before and after execution. Use cases include:
//!
//! - **Logging** - Log all queries and their execution times
//! - **Metrics** - Collect query performance metrics
//! - **Caching** - Cache query results
//! - **Authentication** - Add tenant/user context to queries
//! - **Retry logic** - Automatically retry failed queries
//! - **Circuit breaking** - Prevent cascade failures
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::middleware::{Middleware, MiddlewareStack, LoggingMiddleware};
//!
//! // Create a middleware stack
//! let mut stack = MiddlewareStack::new();
//! stack.push(LoggingMiddleware::new());
//! stack.push(MetricsMiddleware::new());
//!
//! // Use with an engine
//! let engine = engine.with_middleware(stack);
//! ```

#[doc(hidden)]
pub mod backoff;
mod chain;
mod context;
mod logging;
mod metrics;
mod retry;
mod timing;
mod types;

pub use chain::{MiddlewareBuilder, MiddlewareChain, MiddlewareStack};
pub use context::{QueryContext, QueryMetadata, QueryPhase, QueryType};
pub use logging::{LogLevel, LoggingMiddleware};
pub use metrics::{MetricsCollector, MetricsMiddleware, QueryMetrics};
pub use retry::{RetryConfig, RetryMiddleware};
pub use timing::{TimingMiddleware, TimingResult};
pub use types::{BoxFuture, Middleware, MiddlewareResult, Next, QueryResponse};
