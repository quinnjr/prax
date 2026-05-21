//! Prepared statement caching.

use std::num::NonZeroUsize;
use std::sync::Mutex;

use deadpool_postgres::{Object, Transaction};
use lru::LruCache;
use tokio_postgres::Statement;
use tracing::{debug, trace};

use crate::error::PgResult;

/// A cache for prepared statements.
///
/// Tracks which SQL strings have been prepared so we emit a `trace!`
/// for hits vs. misses. Eviction is true LRU via [`lru::LruCache`] —
/// when the cache reaches `max_size` the least-recently-used entry is
/// dropped on the next insert.
///
/// The cache is keyed on the SQL string; the actual `Statement` is
/// fetched from `client.prepare_cached` on every call (deadpool reuses
/// its own per-connection cache).
pub struct PreparedStatementCache {
    max_size: usize,
    /// LRU cache of SQL strings we've seen. The value is `()` because
    /// the real `Statement` lives in deadpool-postgres' per-connection
    /// cache; we just need to know whether we've encountered the SQL
    /// before for tracing/metrics. `Mutex` (not `RwLock`) because every
    /// `get_or_prepare` mutates LRU order, so the read-only path
    /// doesn't exist.
    prepared_queries: Mutex<LruCache<String, ()>>,
}

impl PreparedStatementCache {
    /// Create a new statement cache with the given maximum size.
    ///
    /// `max_size` of 0 is treated as 1 to satisfy `NonZeroUsize`.
    pub fn new(max_size: usize) -> Self {
        let cap = NonZeroUsize::new(max_size.max(1)).unwrap();
        Self {
            max_size,
            prepared_queries: Mutex::new(LruCache::new(cap)),
        }
    }

    /// Get or prepare a statement for the given SQL.
    pub async fn get_or_prepare(&self, client: &Object, sql: &str) -> PgResult<Statement> {
        let is_cached = {
            let mut cache = self.prepared_queries.lock().unwrap();
            // `get` updates LRU order. If absent, insert with `put`
            // (which evicts LRU on overflow).
            if cache.get(sql).is_some() {
                true
            } else {
                cache.put(sql.to_string(), ());
                false
            }
        };

        if is_cached {
            trace!(sql = %sql, "Using cached prepared statement");
        } else {
            trace!(sql = %sql, "Preparing new statement");
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
        let is_cached = {
            let mut cache = self.prepared_queries.lock().unwrap();
            if cache.get(sql).is_some() {
                true
            } else {
                cache.put(sql.to_string(), ());
                false
            }
        };

        if is_cached {
            trace!(sql = %sql, "Using cached prepared statement (txn)");
        } else {
            trace!(sql = %sql, "Preparing new statement (txn)");
        }

        let stmt = txn.prepare_cached(sql).await?;
        Ok(stmt)
    }

    /// Clear all cached statements.
    pub fn clear(&self) {
        let mut cache = self.prepared_queries.lock().unwrap();
        cache.clear();
        debug!("Statement cache cleared");
    }

    /// Get the number of cached statement keys.
    pub fn len(&self) -> usize {
        let cache = self.prepared_queries.lock().unwrap();
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
            let mut inner = cache.prepared_queries.lock().unwrap();
            inner.put("SELECT 1".to_string(), ());
            inner.put("SELECT 2".to_string(), ());
        }

        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_lru_eviction() {
        let cache = PreparedStatementCache::new(2);
        {
            let mut inner = cache.prepared_queries.lock().unwrap();
            inner.put("A".to_string(), ());
            inner.put("B".to_string(), ());
            // Touch A so B becomes LRU.
            let _ = inner.get("A");
            inner.put("C".to_string(), ());
        }
        let inner = cache.prepared_queries.lock().unwrap();
        assert_eq!(inner.len(), 2);
        assert!(inner.peek("A").is_some());
        assert!(inner.peek("B").is_none(), "B should have been evicted");
        assert!(inner.peek("C").is_some());
    }
}
