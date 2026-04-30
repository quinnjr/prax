#![allow(dead_code, unused, clippy::type_complexity)]
//! # Middleware Examples
//!
//! This example demonstrates the middleware system in Prax:
//! - Logging middleware
//! - Metrics middleware
//! - Timing middleware
//! - Retry middleware
//! - Custom middleware implementation
//!
//! ## Running this example
//!
//! ```bash
//! cargo run --example middleware
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// Mock types for demonstration
#[derive(Debug, Clone)]
struct QueryContext {
    query_type: QueryType,
    model: String,
    sql: String,
    started_at: Instant,
}

#[derive(Debug, Clone, Copy)]
enum QueryType {
    Select,
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone)]
struct QueryResult {
    rows_affected: u64,
    duration: Duration,
}

// Middleware trait
trait Middleware: Send + Sync {
    fn name(&self) -> &str;

    fn before_query(&self, ctx: &mut QueryContext) {
        let _ = ctx; // Default: do nothing
    }

    fn after_query(&self, ctx: &QueryContext, result: &QueryResult) {
        let _ = (ctx, result); // Default: do nothing
    }

    fn on_error(&self, ctx: &QueryContext, error: &str) {
        let _ = (ctx, error); // Default: do nothing
    }
}

// Logging Middleware
struct LoggingMiddleware {
    log_queries: bool,
    log_params: bool,
    slow_threshold: Duration,
}

impl LoggingMiddleware {
    fn new() -> Self {
        Self {
            log_queries: true,
            log_params: false,
            slow_threshold: Duration::from_secs(1),
        }
    }

    fn log_queries(mut self, enabled: bool) -> Self {
        self.log_queries = enabled;
        self
    }

    fn log_params(mut self, enabled: bool) -> Self {
        self.log_params = enabled;
        self
    }

    fn slow_threshold(mut self, threshold: Duration) -> Self {
        self.slow_threshold = threshold;
        self
    }
}

impl Middleware for LoggingMiddleware {
    fn name(&self) -> &str {
        "logging"
    }

    fn before_query(&self, ctx: &mut QueryContext) {
        if self.log_queries {
            println!("[LOG] Executing {:?} on {}", ctx.query_type, ctx.model);
            if self.log_params {
                println!("[LOG] SQL: {}", ctx.sql);
            }
        }
    }

    fn after_query(&self, ctx: &QueryContext, result: &QueryResult) {
        if self.log_queries {
            let duration = result.duration;
            if duration > self.slow_threshold {
                println!(
                    "[LOG] ⚠️ SLOW QUERY on {}: {:?} (threshold: {:?})",
                    ctx.model, duration, self.slow_threshold
                );
            } else {
                println!(
                    "[LOG] ✓ {:?} on {} completed in {:?}",
                    ctx.query_type, ctx.model, duration
                );
            }
        }
    }

    fn on_error(&self, ctx: &QueryContext, error: &str) {
        println!("[LOG] ✗ Error on {}: {}", ctx.model, error);
    }
}

// Metrics Middleware
struct MetricsMiddleware {
    query_count: AtomicU64,
    error_count: AtomicU64,
    total_duration_ms: AtomicU64,
}

impl MetricsMiddleware {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            query_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            total_duration_ms: AtomicU64::new(0),
        })
    }

    fn query_count(&self) -> u64 {
        self.query_count.load(Ordering::Relaxed)
    }

    fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    fn average_duration(&self) -> Duration {
        let count = self.query_count.load(Ordering::Relaxed);
        let total = self.total_duration_ms.load(Ordering::Relaxed);
        total
            .checked_div(count)
            .map(Duration::from_millis)
            .unwrap_or(Duration::ZERO)
    }
}

impl Middleware for MetricsMiddleware {
    fn name(&self) -> &str {
        "metrics"
    }

    fn after_query(&self, _ctx: &QueryContext, result: &QueryResult) {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        self.total_duration_ms
            .fetch_add(result.duration.as_millis() as u64, Ordering::Relaxed);
    }

    fn on_error(&self, _ctx: &QueryContext, _error: &str) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }
}

// Timing Middleware
struct TimingMiddleware {
    on_slow_query: Option<Box<dyn Fn(&QueryContext, Duration) + Send + Sync>>,
    threshold: Duration,
}

impl TimingMiddleware {
    fn new() -> Self {
        Self {
            on_slow_query: None,
            threshold: Duration::from_secs(1),
        }
    }

    fn threshold(mut self, duration: Duration) -> Self {
        self.threshold = duration;
        self
    }

    fn on_slow<F>(mut self, callback: F) -> Self
    where
        F: Fn(&QueryContext, Duration) + Send + Sync + 'static,
    {
        self.on_slow_query = Some(Box::new(callback));
        self
    }
}

impl Middleware for TimingMiddleware {
    fn name(&self) -> &str {
        "timing"
    }

    fn after_query(&self, ctx: &QueryContext, result: &QueryResult) {
        if result.duration > self.threshold
            && let Some(callback) = &self.on_slow_query
        {
            callback(ctx, result.duration);
        }
    }
}

// Retry Middleware
struct RetryMiddleware {
    max_retries: u32,
    retry_delay: Duration,
    retryable_errors: Vec<String>,
}

impl RetryMiddleware {
    fn new() -> Self {
        Self {
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
            retryable_errors: vec![
                "connection_error".to_string(),
                "timeout".to_string(),
                "deadlock".to_string(),
            ],
        }
    }

    fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    fn delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    fn is_retryable(&self, error: &str) -> bool {
        self.retryable_errors.iter().any(|e| error.contains(e))
    }
}

impl Middleware for RetryMiddleware {
    fn name(&self) -> &str {
        "retry"
    }

    fn on_error(&self, ctx: &QueryContext, error: &str) {
        if self.is_retryable(error) {
            println!(
                "[RETRY] Query on {} failed with retryable error: {}",
                ctx.model, error
            );
            println!(
                "[RETRY] Will retry up to {} times with {:?} delay",
                self.max_retries, self.retry_delay
            );
        }
    }
}

// Custom Audit Middleware
struct AuditMiddleware {
    audit_log: Arc<std::sync::Mutex<Vec<AuditEntry>>>,
}

#[derive(Debug, Clone)]
struct AuditEntry {
    timestamp: Instant,
    query_type: QueryType,
    model: String,
    duration: Duration,
    success: bool,
}

impl AuditMiddleware {
    fn new() -> Self {
        Self {
            audit_log: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn get_entries(&self) -> Vec<AuditEntry> {
        self.audit_log.lock().unwrap().clone()
    }
}

impl Middleware for AuditMiddleware {
    fn name(&self) -> &str {
        "audit"
    }

    fn after_query(&self, ctx: &QueryContext, result: &QueryResult) {
        let entry = AuditEntry {
            timestamp: Instant::now(),
            query_type: ctx.query_type,
            model: ctx.model.clone(),
            duration: result.duration,
            success: true,
        };
        self.audit_log.lock().unwrap().push(entry);
    }

    fn on_error(&self, ctx: &QueryContext, _error: &str) {
        let entry = AuditEntry {
            timestamp: Instant::now(),
            query_type: ctx.query_type,
            model: ctx.model.clone(),
            duration: ctx.started_at.elapsed(),
            success: false,
        };
        self.audit_log.lock().unwrap().push(entry);
    }
}

// Middleware Stack
struct MiddlewareStack {
    middlewares: Vec<Box<dyn Middleware>>,
}

impl MiddlewareStack {
    fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    fn add<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.middlewares.push(Box::new(middleware));
        self
    }

    fn execute_before(&self, ctx: &mut QueryContext) {
        for middleware in &self.middlewares {
            middleware.before_query(ctx);
        }
    }

    fn execute_after(&self, ctx: &QueryContext, result: &QueryResult) {
        // Execute in reverse order for proper unwinding
        for middleware in self.middlewares.iter().rev() {
            middleware.after_query(ctx, result);
        }
    }

    fn execute_error(&self, ctx: &QueryContext, error: &str) {
        for middleware in self.middlewares.iter().rev() {
            middleware.on_error(ctx, error);
        }
    }
}

fn main() {
    println!("=== Prax Middleware Examples ===\n");

    // =========================================================================
    // LOGGING MIDDLEWARE
    // =========================================================================
    println!("--- Logging Middleware ---");

    let logging = LoggingMiddleware::new()
        .log_queries(true)
        .log_params(true)
        .slow_threshold(Duration::from_millis(100));

    let mut ctx = QueryContext {
        query_type: QueryType::Select,
        model: "User".to_string(),
        sql: "SELECT * FROM users WHERE active = true".to_string(),
        started_at: Instant::now(),
    };

    logging.before_query(&mut ctx);
    std::thread::sleep(Duration::from_millis(50));
    logging.after_query(
        &ctx,
        &QueryResult {
            rows_affected: 10,
            duration: Duration::from_millis(50),
        },
    );
    println!();

    // =========================================================================
    // METRICS MIDDLEWARE
    // =========================================================================
    println!("--- Metrics Middleware ---");

    let metrics = MetricsMiddleware::new();

    // Simulate some queries
    for i in 0..5 {
        let ctx = QueryContext {
            query_type: QueryType::Select,
            model: "User".to_string(),
            sql: format!("SELECT * FROM users WHERE id = {}", i),
            started_at: Instant::now(),
        };

        metrics.after_query(
            &ctx,
            &QueryResult {
                rows_affected: 1,
                duration: Duration::from_millis(10 * (i + 1) as u64),
            },
        );
    }

    println!("Total queries: {}", metrics.query_count());
    println!("Error count: {}", metrics.error_count());
    println!("Average duration: {:?}", metrics.average_duration());
    println!();

    // =========================================================================
    // TIMING MIDDLEWARE WITH CALLBACK
    // =========================================================================
    println!("--- Timing Middleware ---");

    let timing = TimingMiddleware::new()
        .threshold(Duration::from_millis(100))
        .on_slow(|ctx, duration| {
            println!("⚠️ Slow query alert: {} took {:?}", ctx.model, duration);
        });

    // Fast query
    let ctx = QueryContext {
        query_type: QueryType::Select,
        model: "Post".to_string(),
        sql: "SELECT * FROM posts LIMIT 10".to_string(),
        started_at: Instant::now(),
    };
    timing.after_query(
        &ctx,
        &QueryResult {
            rows_affected: 10,
            duration: Duration::from_millis(50),
        },
    );

    // Slow query
    let ctx = QueryContext {
        query_type: QueryType::Select,
        model: "Analytics".to_string(),
        sql: "SELECT * FROM analytics GROUP BY ...".to_string(),
        started_at: Instant::now(),
    };
    timing.after_query(
        &ctx,
        &QueryResult {
            rows_affected: 10000,
            duration: Duration::from_millis(500),
        },
    );
    println!();

    // =========================================================================
    // RETRY MIDDLEWARE
    // =========================================================================
    println!("--- Retry Middleware ---");

    let retry = RetryMiddleware::new()
        .max_retries(3)
        .delay(Duration::from_millis(100));

    let ctx = QueryContext {
        query_type: QueryType::Insert,
        model: "User".to_string(),
        sql: "INSERT INTO users ...".to_string(),
        started_at: Instant::now(),
    };

    // Simulate retryable error
    retry.on_error(&ctx, "connection_error: timeout");

    // Simulate non-retryable error
    println!("Non-retryable error:");
    retry.on_error(&ctx, "unique_constraint_violation");
    println!();

    // =========================================================================
    // AUDIT MIDDLEWARE
    // =========================================================================
    println!("--- Audit Middleware ---");

    let audit = AuditMiddleware::new();

    // Log some operations
    let queries = vec![
        (QueryType::Select, "User"),
        (QueryType::Insert, "User"),
        (QueryType::Update, "Post"),
        (QueryType::Delete, "Comment"),
    ];

    for (query_type, model) in queries {
        let ctx = QueryContext {
            query_type,
            model: model.to_string(),
            sql: String::new(),
            started_at: Instant::now(),
        };
        audit.after_query(
            &ctx,
            &QueryResult {
                rows_affected: 1,
                duration: Duration::from_millis(10),
            },
        );
    }

    println!("Audit log entries:");
    for entry in audit.get_entries() {
        println!(
            "  {:?} on {} - success: {}, duration: {:?}",
            entry.query_type, entry.model, entry.success, entry.duration
        );
    }
    println!();

    // =========================================================================
    // MIDDLEWARE STACK
    // =========================================================================
    println!("--- Middleware Stack ---");

    let stack = MiddlewareStack::new()
        .add(LoggingMiddleware::new().log_queries(true))
        .add(RetryMiddleware::new());

    let mut ctx = QueryContext {
        query_type: QueryType::Select,
        model: "User".to_string(),
        sql: "SELECT * FROM users".to_string(),
        started_at: Instant::now(),
    };

    println!("Executing query with middleware stack:");
    stack.execute_before(&mut ctx);
    std::thread::sleep(Duration::from_millis(20));
    stack.execute_after(
        &ctx,
        &QueryResult {
            rows_affected: 5,
            duration: Duration::from_millis(20),
        },
    );
    println!();

    // =========================================================================
    // CONFIGURATION EXAMPLE
    // =========================================================================
    println!("--- Configuration Reference ---");
    println!(
        r#"
Middleware configuration in code:

```rust
use prax_orm::middleware::{{LoggingMiddleware, MetricsMiddleware, RetryMiddleware}};

let client = PraxClient::new(database_url)
    .await?
    .with_middleware(
        MiddlewareStack::new()
            .add(LoggingMiddleware::new()
                .log_queries(true)
                .slow_threshold(Duration::from_millis(100)))
            .add(MetricsMiddleware::new())
            .add(RetryMiddleware::new()
                .max_retries(3)
                .delay(Duration::from_millis(100)))
    );

// All queries now go through the middleware stack
let users = client.user().find_many().exec().await?;
```

Custom middleware:

```rust
struct MyMiddleware;

impl Middleware for MyMiddleware {{
    fn name(&self) -> &str {{
        "my_middleware"
    }}

    fn before_query(&self, ctx: &mut QueryContext) {{
        // Called before each query
    }}

    fn after_query(&self, ctx: &QueryContext, result: &QueryResult) {{
        // Called after successful query
    }}

    fn on_error(&self, ctx: &QueryContext, error: &str) {{
        // Called on query error
    }}
}}
```
"#
    );

    println!("=== All examples completed successfully! ===");
}
