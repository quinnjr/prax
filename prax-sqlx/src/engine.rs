//! SQLx query engine implementation.

use crate::config::{DatabaseBackend, SqlxConfig};
use crate::error::SqlxResult;
use crate::pool::SqlxPool;
use crate::row::SqlxRow;
use crate::types::quote_identifier;
use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use sqlx::Row;
use std::sync::Arc;
use tracing::debug;

/// SQLx-based query engine for Prax.
///
/// This engine provides compile-time checked queries through SQLx,
/// supporting PostgreSQL, MySQL, and SQLite.
///
/// # Example
///
/// ```rust,ignore
/// use prax_sqlx::{SqlxEngine, SqlxConfig};
///
/// let config = SqlxConfig::from_url("postgres://localhost/mydb")?;
/// let engine = SqlxEngine::new(config).await?;
///
/// // Execute queries
/// let count = engine.count_table("users", None).await?;
/// ```
#[derive(Clone)]
pub struct SqlxEngine {
    pool: Arc<SqlxPool>,
    backend: DatabaseBackend,
}

impl SqlxEngine {
    /// Create a new SQLx engine from configuration.
    pub async fn new(config: SqlxConfig) -> SqlxResult<Self> {
        let backend = config.backend;
        let pool = SqlxPool::connect(&config).await?;
        Ok(Self {
            pool: Arc::new(pool),
            backend,
        })
    }

    /// Create a new engine from an existing pool.
    pub fn from_pool(pool: SqlxPool) -> Self {
        let backend = pool.backend();
        Self {
            pool: Arc::new(pool),
            backend,
        }
    }

    /// Get the database backend type.
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// Get the connection pool.
    pub fn pool(&self) -> &SqlxPool {
        &self.pool
    }

    /// Close the engine and all connections.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    // ==================== Low-Level Query Methods ====================

    /// Execute a raw SQL query and return multiple rows.
    pub async fn raw_query_many(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> SqlxResult<Vec<SqlxRow>> {
        debug!(sql = %sql, "Executing raw_query_many");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param);
                }
                let rows = query.fetch_all(pool).await?;
                Ok(rows.into_iter().map(SqlxRow::Postgres).collect())
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param);
                }
                let rows = query.fetch_all(pool).await?;
                Ok(rows.into_iter().map(SqlxRow::MySql).collect())
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param);
                }
                let rows = query.fetch_all(pool).await?;
                Ok(rows.into_iter().map(SqlxRow::Sqlite).collect())
            }
        }
    }

    /// Execute a raw SQL query and return a single row.
    pub async fn raw_query_one(&self, sql: &str, params: &[FilterValue]) -> SqlxResult<SqlxRow> {
        debug!(sql = %sql, "Executing raw_query_one");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param);
                }
                let row = query.fetch_one(pool).await?;
                Ok(SqlxRow::Postgres(row))
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param);
                }
                let row = query.fetch_one(pool).await?;
                Ok(SqlxRow::MySql(row))
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param);
                }
                let row = query.fetch_one(pool).await?;
                Ok(SqlxRow::Sqlite(row))
            }
        }
    }

    /// Execute a raw SQL query and return an optional row.
    pub async fn raw_query_optional(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> SqlxResult<Option<SqlxRow>> {
        debug!(sql = %sql, "Executing raw_query_optional");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param);
                }
                let row = query.fetch_optional(pool).await?;
                Ok(row.map(SqlxRow::Postgres))
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param);
                }
                let row = query.fetch_optional(pool).await?;
                Ok(row.map(SqlxRow::MySql))
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param);
                }
                let row = query.fetch_optional(pool).await?;
                Ok(row.map(SqlxRow::Sqlite))
            }
        }
    }

    /// Execute a SQL statement (INSERT, UPDATE, DELETE) and return affected rows.
    pub async fn raw_execute(&self, sql: &str, params: &[FilterValue]) -> SqlxResult<u64> {
        debug!(sql = %sql, "Executing raw_execute");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param);
                }
                let result = query.execute(pool).await?;
                Ok(result.rows_affected())
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param);
                }
                let result = query.execute(pool).await?;
                Ok(result.rows_affected())
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param);
                }
                let result = query.execute(pool).await?;
                Ok(result.rows_affected())
            }
        }
    }

    /// Count rows in a table with optional filter.
    pub async fn count_table(&self, table: &str, filter: Option<&str>) -> SqlxResult<u64> {
        let table = quote_identifier(self.backend, table);
        let sql = match filter {
            Some(f) => format!("SELECT COUNT(*) as count FROM {} WHERE {}", table, f),
            None => format!("SELECT COUNT(*) as count FROM {}", table),
        };

        let row = self.raw_query_one(&sql, &[]).await?;
        match row {
            #[cfg(feature = "postgres")]
            SqlxRow::Postgres(r) => Ok(r.try_get::<i64, _>("count")? as u64),
            #[cfg(feature = "mysql")]
            SqlxRow::MySql(r) => Ok(r.try_get::<i64, _>("count")? as u64),
            #[cfg(feature = "sqlite")]
            SqlxRow::Sqlite(r) => Ok(r.try_get::<i64, _>("count")? as u64),
        }
    }
}

// ==================== Parameter Binding Helpers ====================

#[cfg(feature = "postgres")]
fn bind_pg_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &'q FilterValue,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match value {
        FilterValue::String(s) => query.bind(s.as_str()),
        FilterValue::Int(i) => query.bind(*i),
        FilterValue::Float(f) => query.bind(*f),
        FilterValue::Bool(b) => query.bind(*b),
        FilterValue::Null => query.bind(Option::<String>::None),
        FilterValue::Json(j) => query.bind(j.clone()),
        FilterValue::List(arr) => {
            // Convert list to JSON for PostgreSQL
            let json = serde_json::to_value(arr).unwrap_or(serde_json::Value::Null);
            query.bind(json)
        }
    }
}

#[cfg(feature = "mysql")]
fn bind_mysql_param<'q>(
    query: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>,
    value: &'q FilterValue,
) -> sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments> {
    match value {
        FilterValue::String(s) => query.bind(s.as_str()),
        FilterValue::Int(i) => query.bind(*i),
        FilterValue::Float(f) => query.bind(*f),
        FilterValue::Bool(b) => query.bind(*b),
        FilterValue::Null => query.bind(Option::<String>::None),
        FilterValue::Json(j) => query.bind(j.to_string()),
        FilterValue::List(arr) => {
            let json = serde_json::to_string(arr).unwrap_or_default();
            query.bind(json)
        }
    }
}

#[cfg(feature = "sqlite")]
fn bind_sqlite_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &'q FilterValue,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match value {
        FilterValue::String(s) => query.bind(s.as_str()),
        FilterValue::Int(i) => query.bind(*i),
        FilterValue::Float(f) => query.bind(*f),
        FilterValue::Bool(b) => query.bind(*b),
        FilterValue::Null => query.bind(Option::<String>::None),
        FilterValue::Json(j) => query.bind(j.to_string()),
        FilterValue::List(arr) => {
            let json = serde_json::to_string(arr).unwrap_or_default();
            query.bind(json)
        }
    }
}

// ==================== QueryEngine Trait Implementation ====================

impl QueryEngine for SqlxEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        match self.backend {
            DatabaseBackend::Postgres => &prax_query::dialect::Postgres,
            DatabaseBackend::MySql => &prax_query::dialect::Mysql,
            DatabaseBackend::Sqlite => &prax_query::dialect::Sqlite,
        }
    }

    fn query_many<T: Model + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_many via QueryEngine");

            let _rows = self
                .raw_query_many(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Placeholder - real implementation would deserialize rows into T
            // For now, return empty vec
            Ok(Vec::new())
        })
    }

    fn query_one<T: Model + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_one via QueryEngine");

            let _row = self.raw_query_one(&sql, &params).await.map_err(|e| {
                let msg = e.to_string();
                if msg.contains("no rows") {
                    prax_query::QueryError::not_found(T::MODEL_NAME)
                } else {
                    prax_query::QueryError::database(msg)
                }
            })?;

            // Placeholder - would deserialize row into T
            Err(prax_query::QueryError::internal(
                "deserialization not yet implemented".to_string(),
            ))
        })
    }

    fn query_optional<T: Model + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_optional via QueryEngine");

            let _row = self
                .raw_query_optional(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Placeholder - return None for now
            Ok(None)
        })
    }

    fn execute_insert<T: Model + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_insert via QueryEngine");

            let _row = self
                .raw_query_one(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Placeholder - would deserialize row into T
            Err(prax_query::QueryError::internal(
                "deserialization not yet implemented".to_string(),
            ))
        })
    }

    fn execute_update<T: Model + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_update via QueryEngine");

            let _rows = self
                .raw_query_many(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            // Placeholder - return empty vec for now
            Ok(Vec::new())
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_delete via QueryEngine");

            let affected = self
                .raw_execute(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(affected)
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_raw via QueryEngine");

            let affected = self
                .raw_execute(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(affected)
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing count via QueryEngine");

            let row = self
                .raw_query_one(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let count = match row {
                #[cfg(feature = "postgres")]
                SqlxRow::Postgres(r) => r
                    .try_get::<i64, _>(0)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                    as u64,
                #[cfg(feature = "mysql")]
                SqlxRow::MySql(r) => r
                    .try_get::<i64, _>(0)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                    as u64,
                #[cfg(feature = "sqlite")]
                SqlxRow::Sqlite(r) => r
                    .try_get::<i64, _>(0)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                    as u64,
            };

            Ok(count)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::placeholder;

    #[test]
    fn test_placeholder_generation() {
        assert_eq!(placeholder(DatabaseBackend::Postgres, 1), "$1");
        assert_eq!(placeholder(DatabaseBackend::MySql, 1), "?");
        assert_eq!(placeholder(DatabaseBackend::Sqlite, 1), "?");
    }

    #[test]
    fn test_quote_identifier() {
        assert_eq!(
            quote_identifier(DatabaseBackend::Postgres, "users"),
            "\"users\""
        );
        assert_eq!(quote_identifier(DatabaseBackend::MySql, "users"), "`users`");
    }
}
