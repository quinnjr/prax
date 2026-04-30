//! Tenant middleware for automatic query filtering.

use super::config::TenantConfig;
use super::context::TenantContext;
use super::strategy::ColumnType;
use crate::error::{QueryError, QueryResult};
use crate::middleware::{BoxFuture, Middleware, Next, QueryContext, QueryResponse, QueryType};
use std::sync::{Arc, RwLock};

/// Middleware that automatically applies tenant filtering to queries.
pub struct TenantMiddleware {
    config: TenantConfig,
    current_tenant: Arc<RwLock<Option<TenantContext>>>,
}

impl TenantMiddleware {
    /// Create a new tenant middleware with the given config.
    pub fn new(config: TenantConfig) -> Self {
        Self {
            config,
            current_tenant: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the current tenant context.
    pub fn set_tenant(&self, ctx: TenantContext) {
        *self.current_tenant.write().expect("lock poisoned") = Some(ctx);
    }

    /// Clear the current tenant context.
    pub fn clear_tenant(&self) {
        *self.current_tenant.write().expect("lock poisoned") = None;
    }

    /// Get the current tenant context.
    pub fn current_tenant(&self) -> Option<TenantContext> {
        self.current_tenant.read().expect("lock poisoned").clone()
    }

    /// Create a scoped tenant context (automatically clears on drop).
    pub fn scoped(&self, ctx: TenantContext) -> TenantScope {
        self.set_tenant(ctx);
        TenantScope {
            middleware: Arc::new(self.clone()),
        }
    }

    /// Apply row-level filtering to a SQL query.
    fn apply_row_level_filter(&self, sql: &str, tenant_id: &str) -> (String, Vec<String>) {
        let config = match self.config.row_level_config() {
            Some(c) => c,
            None => return (sql.to_string(), vec![]),
        };

        let column = &config.column;
        let tenant_value = match config.column_type {
            ColumnType::String => format!("'{}'", tenant_id.replace('\'', "''")),
            ColumnType::Uuid => format!("'{}'::uuid", tenant_id),
            ColumnType::Integer | ColumnType::BigInt => tenant_id.to_string(),
        };

        // Parse and modify SQL
        let modified_sql = self.inject_tenant_filter(sql, column, &tenant_value);
        (modified_sql, vec![tenant_id.to_string()])
    }

    /// Inject tenant filter into SQL.
    fn inject_tenant_filter(&self, sql: &str, column: &str, value: &str) -> String {
        let sql_upper = sql.to_uppercase();
        let filter = format!("{} = {}", column, value);

        // Handle SELECT queries
        if sql_upper.starts_with("SELECT") {
            if let Some(where_pos) = sql_upper.find("WHERE") {
                // Insert after WHERE
                let (before, after) = sql.split_at(where_pos + 5);
                return format!("{} {} AND {}", before.trim(), filter, after.trim());
            } else if let Some(order_pos) = sql_upper.find("ORDER BY") {
                // Insert before ORDER BY
                let (before, after) = sql.split_at(order_pos);
                return format!("{} WHERE {} {}", before.trim(), filter, after);
            } else if let Some(limit_pos) = sql_upper.find("LIMIT") {
                // Insert before LIMIT
                let (before, after) = sql.split_at(limit_pos);
                return format!("{} WHERE {} {}", before.trim(), filter, after);
            } else {
                // Append WHERE clause
                return format!("{} WHERE {}", sql.trim(), filter);
            }
        }

        // Handle UPDATE queries
        if sql_upper.starts_with("UPDATE") {
            if let Some(where_pos) = sql_upper.find("WHERE") {
                let (before, after) = sql.split_at(where_pos + 5);
                return format!("{} {} AND {}", before.trim(), filter, after.trim());
            } else if let Some(returning_pos) = sql_upper.find("RETURNING") {
                let (before, after) = sql.split_at(returning_pos);
                return format!("{} WHERE {} {}", before.trim(), filter, after);
            } else {
                return format!("{} WHERE {}", sql.trim(), filter);
            }
        }

        // Handle DELETE queries
        if sql_upper.starts_with("DELETE") {
            if let Some(where_pos) = sql_upper.find("WHERE") {
                let (before, after) = sql.split_at(where_pos + 5);
                return format!("{} {} AND {}", before.trim(), filter, after.trim());
            } else if let Some(returning_pos) = sql_upper.find("RETURNING") {
                let (before, after) = sql.split_at(returning_pos);
                return format!("{} WHERE {} {}", before.trim(), filter, after);
            } else {
                return format!("{} WHERE {}", sql.trim(), filter);
            }
        }

        // Handle INSERT queries (add tenant_id column)
        if sql_upper.starts_with("INSERT")
            && self
                .config
                .row_level_config()
                .is_some_and(|c| c.auto_insert)
        {
            // This is simplified - real implementation would parse the INSERT properly
            // For now, we assume tenant_id is included in the data
        }

        sql.to_string()
    }

    /// Apply schema-based isolation.
    fn apply_schema_isolation(&self, tenant_id: &str) -> Option<String> {
        self.config
            .schema_config()
            .map(|c| c.search_path(tenant_id))
    }
}

impl Clone for TenantMiddleware {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            current_tenant: Arc::clone(&self.current_tenant),
        }
    }
}

impl std::fmt::Debug for TenantMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantMiddleware")
            .field("config", &self.config)
            .field("has_tenant", &self.current_tenant().is_some())
            .finish()
    }
}

impl Middleware for TenantMiddleware {
    fn handle<'a>(
        &'a self,
        mut ctx: QueryContext,
        next: Next<'a>,
    ) -> BoxFuture<'a, QueryResult<QueryResponse>> {
        Box::pin(async move {
            // Get tenant context
            let tenant_ctx = match self.current_tenant() {
                Some(ctx) => ctx,
                None => {
                    // No tenant context
                    if self.config.require_tenant {
                        if let Some(default) = &self.config.default_tenant {
                            TenantContext::new(default.clone())
                        } else {
                            return Err(QueryError::internal(
                                "Tenant context required but not provided",
                            ));
                        }
                    } else {
                        // No tenant filtering
                        return next.run(ctx).await;
                    }
                }
            };

            // Check for bypass
            if self.config.allow_bypass && tenant_ctx.should_bypass() {
                if self.config.log_tenant_context {
                    tracing::debug!(
                        tenant_id = %tenant_ctx.id,
                        bypass = true,
                        "Tenant filter bypassed"
                    );
                }
                return next.run(ctx).await;
            }

            // Apply row-level filtering if configured
            if self.config.strategy.is_row_level() {
                let query_type = ctx.query_type();

                // Validate writes
                if self.config.enforce_on_writes
                    && matches!(
                        query_type,
                        QueryType::Insert | QueryType::Update | QueryType::Delete
                    )
                {
                    // For writes, we need to ensure tenant_id is included
                }

                // Apply filter to query
                let (modified_sql, _extra_params) =
                    self.apply_row_level_filter(ctx.sql(), tenant_ctx.id.as_str());

                // Update context with modified SQL
                ctx = ctx.with_sql(modified_sql);
            }

            // Apply schema-based isolation if configured
            if self.config.strategy.is_schema_based()
                && let Some(search_path) = self.apply_schema_isolation(tenant_ctx.id.as_str())
            {
                // The search_path should be set on the connection
                // This is typically done by the connection manager
                ctx.metadata_mut().set_schema_override(Some(
                    self.config
                        .schema_config()
                        .unwrap()
                        .schema_name(tenant_ctx.id.as_str()),
                ));

                // Log the schema setting
                if self.config.log_tenant_context {
                    tracing::debug!(
                        tenant_id = %tenant_ctx.id,
                        search_path = %search_path,
                        "Setting schema for tenant"
                    );
                }
            }

            // Log tenant context
            if self.config.log_tenant_context {
                tracing::debug!(
                    tenant_id = %tenant_ctx.id,
                    strategy = ?self.config.strategy,
                    sql = %ctx.sql(),
                    "Executing query with tenant context"
                );
            }

            // Set tenant in metadata for downstream middleware
            ctx.metadata_mut().tenant_id = Some(tenant_ctx.id.to_string());

            // Continue with modified query
            next.run(ctx).await
        })
    }

    fn name(&self) -> &'static str {
        "TenantMiddleware"
    }
}

/// A scoped tenant context that clears on drop.
pub struct TenantScope {
    middleware: Arc<TenantMiddleware>,
}

impl Drop for TenantScope {
    fn drop(&mut self) {
        self.middleware.clear_tenant();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_level_filter_select() {
        let config = TenantConfig::row_level("tenant_id");
        let middleware = TenantMiddleware::new(config);

        let (sql, _) = middleware.apply_row_level_filter("SELECT * FROM users", "tenant-123");
        assert!(sql.contains("WHERE tenant_id = 'tenant-123'"));

        let (sql, _) = middleware
            .apply_row_level_filter("SELECT * FROM users WHERE active = true", "tenant-123");
        assert!(sql.contains("tenant_id = 'tenant-123' AND active = true"));
    }

    #[test]
    fn test_row_level_filter_update() {
        let config = TenantConfig::row_level("tenant_id");
        let middleware = TenantMiddleware::new(config);

        let (sql, _) =
            middleware.apply_row_level_filter("UPDATE users SET name = 'Bob'", "tenant-123");
        assert!(sql.contains("WHERE tenant_id = 'tenant-123'"));

        let (sql, _) = middleware
            .apply_row_level_filter("UPDATE users SET name = 'Bob' WHERE id = 1", "tenant-123");
        assert!(sql.contains("tenant_id = 'tenant-123' AND id = 1"));
    }

    #[test]
    fn test_row_level_filter_delete() {
        let config = TenantConfig::row_level("tenant_id");
        let middleware = TenantMiddleware::new(config);

        let (sql, _) = middleware.apply_row_level_filter("DELETE FROM users", "tenant-123");
        assert!(sql.contains("WHERE tenant_id = 'tenant-123'"));
    }

    #[test]
    fn test_tenant_scope() {
        let config = TenantConfig::row_level("tenant_id");
        let middleware = TenantMiddleware::new(config);

        {
            let _scope = middleware.scoped(TenantContext::new("tenant-123"));
            assert!(middleware.current_tenant().is_some());
            assert_eq!(
                middleware.current_tenant().unwrap().id.as_str(),
                "tenant-123"
            );
        }

        // Scope dropped, tenant cleared
        assert!(middleware.current_tenant().is_none());
    }
}
