//! PostgreSQL-specific functionality for SQLx.

use crate::config::DatabaseBackend;
use crate::error::{SqlxError, SqlxResult};
use crate::types::quote_identifier;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

/// Longest channel name PostgreSQL keeps (`NAMEDATALEN` - 1).
const MAX_CHANNEL_LEN: usize = 63;

/// A notification payload must be strictly shorter than this many bytes.
const PAYLOAD_LIMIT: usize = 8000;

/// Client-side checks for `pg_notify` arguments.
fn validate_notify(channel: &str, payload: &str) -> SqlxResult<()> {
    if channel.is_empty() {
        return Err(SqlxError::query(
            "notification channel name must not be empty",
        ));
    }
    if channel.len() > MAX_CHANNEL_LEN {
        return Err(SqlxError::query(format!(
            "notification channel name is {} bytes; PostgreSQL allows at most {MAX_CHANNEL_LEN}",
            channel.len()
        )));
    }
    if channel.contains('\0') {
        return Err(SqlxError::query(
            "notification channel name must not contain NUL",
        ));
    }
    if payload.len() >= PAYLOAD_LIMIT {
        return Err(SqlxError::query(format!(
            "notification payload is {} bytes; PostgreSQL requires fewer than {PAYLOAD_LIMIT}",
            payload.len()
        )));
    }
    Ok(())
}

/// PostgreSQL-specific query helpers.
pub struct PgHelpers;

impl PgHelpers {
    /// Execute a query with RETURNING clause.
    pub async fn query_returning(pool: &PgPool, sql: &str) -> SqlxResult<Vec<PgRow>> {
        let rows = sqlx::query(sql).fetch_all(pool).await?;
        Ok(rows)
    }

    /// Build INSERT ... ON CONFLICT SQL (does not execute; run it via your pool).
    ///
    /// The `pool` parameter is currently unused; it is reserved for a future
    /// executing variant. Identifiers are quoted via `quote_identifier`;
    /// pass trusted identifiers only, not arbitrary user input.
    pub async fn upsert(
        _pool: &PgPool,
        table: &str,
        columns: &[&str],
        conflict_columns: &[&str],
        update_columns: &[&str],
    ) -> SqlxResult<String> {
        let table = quote_identifier(DatabaseBackend::Postgres, table);
        let cols = columns
            .iter()
            .map(|c| quote_identifier(DatabaseBackend::Postgres, c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${}", i)).collect();
        let vals = placeholders.join(", ");
        let conflict = conflict_columns
            .iter()
            .map(|c| quote_identifier(DatabaseBackend::Postgres, c))
            .collect::<Vec<_>>()
            .join(", ");
        let updates: Vec<String> = update_columns
            .iter()
            .map(|c| {
                let col = quote_identifier(DatabaseBackend::Postgres, c);
                format!("{} = EXCLUDED.{}", col, col)
            })
            .collect();
        let update_clause = updates.join(", ");

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {} RETURNING *",
            table, cols, vals, conflict, update_clause
        );

        Ok(sql)
    }

    /// Generate a PostgreSQL array literal.
    ///
    /// Embedded single quotes in values are escaped (`'` -> `''`). Prefer
    /// bound parameters for untrusted values.
    pub fn array_literal<T: std::fmt::Display>(values: &[T]) -> String {
        let items: Vec<String> = values
            .iter()
            .map(|v| format!("'{}'", v.to_string().replace('\'', "''")))
            .collect();
        format!("ARRAY[{}]", items.join(", "))
    }

    /// Generate a PostgreSQL JSON/JSONB path expression.
    ///
    /// The column is quoted via `quote_identifier` and embedded single quotes
    /// in path elements are escaped (`'` -> `''`). Pass trusted identifiers
    /// only, not arbitrary user input.
    pub fn json_path(column: &str, path: &[&str]) -> String {
        let column = quote_identifier(DatabaseBackend::Postgres, column);
        if path.is_empty() {
            column
        } else {
            let path_str: Vec<String> = path
                .iter()
                .map(|p| format!("'{}'", p.replace('\'', "''")))
                .collect();
            format!("{}->>{}", column, path_str.join("->"))
        }
    }

    /// Check if a PostgreSQL extension is available.
    pub async fn has_extension(pool: &PgPool, extension: &str) -> SqlxResult<bool> {
        let sql = "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = $1)";
        let row = sqlx::query(sql).bind(extension).fetch_one(pool).await?;
        let exists: bool = row.try_get(0)?;
        Ok(exists)
    }

    /// Get PostgreSQL version.
    pub async fn version(pool: &PgPool) -> SqlxResult<String> {
        let sql = "SELECT version()";
        let row = sqlx::query(sql).fetch_one(pool).await?;
        let version: String = row.try_get(0)?;
        Ok(version)
    }

    /// Send a notification via `SELECT pg_notify($1, $2)`.
    ///
    /// Both the channel and the payload are bound as parameters. The channel
    /// name must be non-empty, at most 63 bytes and free of NUL, and the
    /// payload shorter than 8000 bytes; both are checked client-side before
    /// anything is sent, so the error names the offending value instead of
    /// surfacing as a server error (or silent truncation of the name).
    ///
    /// To receive notifications use `sqlx::postgres::PgListener`; `LISTEN`
    /// on a pooled connection never delivers them.
    pub async fn notify(pool: &PgPool, channel: &str, payload: &str) -> SqlxResult<()> {
        validate_notify(channel, payload)?;
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(payload)
            .execute(pool)
            .await?;
        Ok(())
    }
}

/// PostgreSQL advisory lock helpers.
pub struct AdvisoryLock;

impl AdvisoryLock {
    /// Acquire an advisory lock (blocking).
    pub async fn acquire(pool: &PgPool, key: i64) -> SqlxResult<()> {
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Try to acquire an advisory lock (non-blocking).
    pub async fn try_acquire(pool: &PgPool, key: i64) -> SqlxResult<bool> {
        let row = sqlx::query("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(pool)
            .await?;
        let acquired: bool = row.try_get(0)?;
        Ok(acquired)
    }

    /// Release an advisory lock.
    pub async fn release(pool: &PgPool, key: i64) -> SqlxResult<()> {
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(key)
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_notify_accepts_boundaries() {
        assert!(validate_notify("a", "").is_ok());
        assert!(validate_notify(&"a".repeat(63), &"p".repeat(7999)).is_ok());
        // Case and quoting are preserved, not rejected.
        assert!(validate_notify("Mixed\"Case", "it's").is_ok());
    }

    #[test]
    fn validate_notify_rejects_bad_channel_and_payload() {
        for (channel, payload, needle) in [
            ("", "x", "empty"),
            ("a\0b", "x", "NUL"),
            (&"a".repeat(64), "x", "64 bytes"),
            ("ok", &"p".repeat(8000), "8000"),
        ] {
            let err = validate_notify(channel, payload).expect_err(needle);
            assert!(matches!(err, SqlxError::Query(_)), "{err}");
            assert!(err.to_string().contains(needle), "{err}");
        }
    }

    #[test]
    fn test_array_literal() {
        assert_eq!(PgHelpers::array_literal(&[1, 2, 3]), "ARRAY['1', '2', '3']");
        assert_eq!(PgHelpers::array_literal(&["a", "b"]), "ARRAY['a', 'b']");
        // Embedded single quotes are escaped.
        assert_eq!(PgHelpers::array_literal(&["a'b"]), "ARRAY['a''b']");
    }

    #[test]
    fn test_json_path() {
        assert_eq!(PgHelpers::json_path("data", &[]), "\"data\"");
        assert_eq!(PgHelpers::json_path("data", &["name"]), "\"data\"->>'name'");
        assert_eq!(
            PgHelpers::json_path("data", &["user", "name"]),
            "\"data\"->>'user'->'name'"
        );
        // Embedded single quotes in path elements are escaped.
        assert_eq!(
            PgHelpers::json_path("data", &["na'me"]),
            "\"data\"->>'na''me'"
        );
    }
}
