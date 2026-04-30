//! Prepared statement caching.

use std::collections::HashMap;
use std::sync::RwLock;

use deadpool_postgres::{Object, Transaction};
use tokio_postgres::Statement;
use tracing::{debug, trace};

use crate::error::PgResult;

/// A cache for prepared statements.
///
/// This cache stores prepared statements by their SQL query string,
/// allowing reuse of statements across multiple queries.
pub struct PreparedStatementCache {
    max_size: usize,
    /// Note: We use a simple HashMap here. In production, you might want
    /// an LRU cache to evict old statements when the cache is full.
    /// However, prepared statements are tied to connections, so this
    /// cache is really just for tracking which statements we've prepared.
    prepared_queries: RwLock<HashMap<String, bool>>,
}

impl PreparedStatementCache {
    /// Create a new statement cache with the given maximum size.
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            prepared_queries: RwLock::new(HashMap::new()),
        }
    }

    /// Get or prepare a statement for the given SQL.
    pub async fn get_or_prepare(&self, client: &Object, sql: &str) -> PgResult<Statement> {
        // Check if we've prepared this statement before
        let is_cached = {
            let cache = self.prepared_queries.read().unwrap();
            cache.contains_key(sql)
        };

        if is_cached {
            trace!(sql = %sql, "Using cached prepared statement");
        } else {
            trace!(sql = %sql, "Preparing new statement");

            // Check cache size and potentially evict
            let mut cache = self.prepared_queries.write().unwrap();
            if cache.len() >= self.max_size {
                // Simple eviction: clear half the cache
                // In production, use an LRU cache
                let to_remove: Vec<_> = cache.keys().take(cache.len() / 2).cloned().collect();
                for key in to_remove {
                    cache.remove(&key);
                }
            }
            cache.insert(sql.to_string(), true);
        }

        // Always prepare - the database will reuse if it's cached server-side
        let stmt = client.prepare_cached(sql).await?;
        Ok(stmt)
    }

    /// Get or prepare a statement within a transaction.
    pub async fn get_or_prepare_in_txn<'a>(
        &self,
        txn: &Transaction<'a>,
        sql: &str,
    ) -> PgResult<Statement> {
        // Similar logic to above, but for transactions
        let is_cached = {
            let cache = self.prepared_queries.read().unwrap();
            cache.contains_key(sql)
        };

        if is_cached {
            trace!(sql = %sql, "Using cached prepared statement (txn)");
        } else {
            trace!(sql = %sql, "Preparing new statement (txn)");

            let mut cache = self.prepared_queries.write().unwrap();
            if cache.len() >= self.max_size {
                let to_remove: Vec<_> = cache.keys().take(cache.len() / 2).cloned().collect();
                for key in to_remove {
                    cache.remove(&key);
                }
            }
            cache.insert(sql.to_string(), true);
        }

        let stmt = txn.prepare_cached(sql).await?;
        Ok(stmt)
    }

    /// Clear all cached statements.
    pub fn clear(&self) {
        let mut cache = self.prepared_queries.write().unwrap();
        cache.clear();
        debug!("Statement cache cleared");
    }

    /// Get the number of cached statement keys.
    pub fn len(&self) -> usize {
        let cache = self.prepared_queries.read().unwrap();
        cache.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the maximum cache size.
    pub fn max_size(&self) -> usize {
        self.max_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_creation() {
        let cache = PreparedStatementCache::new(100);
        assert_eq!(cache.max_size(), 100);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_clear() {
        let cache = PreparedStatementCache::new(100);

        // Manually insert some entries for testing
        {
            let mut inner = cache.prepared_queries.write().unwrap();
            inner.insert("SELECT 1".to_string(), true);
            inner.insert("SELECT 2".to_string(), true);
        }

        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }
}
