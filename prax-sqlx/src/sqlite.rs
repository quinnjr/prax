//! SQLite-specific functionality for SQLx.

use crate::config::DatabaseBackend;
use crate::error::SqlxResult;
use crate::types::quote_identifier;
use sqlx::sqlite::SqlitePool;
use sqlx::{AssertSqlSafe, Row};

/// SQLite-specific query helpers.
pub struct SqliteHelpers;

impl SqliteHelpers {
    /// Execute INSERT OR REPLACE (upsert).
    ///
    /// Identifiers are quoted via `quote_identifier`; pass trusted
    /// identifiers only, not arbitrary user input.
    pub fn upsert_sql(table: &str, columns: &[&str]) -> String {
        let table = quote_identifier(DatabaseBackend::Sqlite, table);
        let cols = columns
            .iter()
            .map(|c| quote_identifier(DatabaseBackend::Sqlite, c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders: Vec<String> = columns.iter().map(|_| "?".to_string()).collect();
        let vals = placeholders.join(", ");

        format!(
            "INSERT OR REPLACE INTO {} ({}) VALUES ({})",
            table, cols, vals
        )
    }

    /// Execute INSERT ... ON CONFLICT (SQLite 3.24+).
    ///
    /// Identifiers are quoted via `quote_identifier`; pass trusted
    /// identifiers only, not arbitrary user input.
    pub fn on_conflict_sql(
        table: &str,
        columns: &[&str],
        conflict_columns: &[&str],
        update_columns: &[&str],
    ) -> String {
        let table = quote_identifier(DatabaseBackend::Sqlite, table);
        let cols = columns
            .iter()
            .map(|c| quote_identifier(DatabaseBackend::Sqlite, c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders: Vec<String> = columns.iter().map(|_| "?".to_string()).collect();
        let vals = placeholders.join(", ");
        let conflict = conflict_columns
            .iter()
            .map(|c| quote_identifier(DatabaseBackend::Sqlite, c))
            .collect::<Vec<_>>()
            .join(", ");
        let updates: Vec<String> = update_columns
            .iter()
            .map(|c| {
                let col = quote_identifier(DatabaseBackend::Sqlite, c);
                format!("{} = excluded.{}", col, col)
            })
            .collect();
        let update_clause = updates.join(", ");

        format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT({}) DO UPDATE SET {}",
            table, cols, vals, conflict, update_clause
        )
    }

    /// Generate SQLite JSON extract expression.
    ///
    /// The column is quoted via `quote_identifier` and embedded single
    /// quotes in the path are escaped (`'` -> `''`). Pass trusted
    /// identifiers only, not arbitrary user input.
    pub fn json_extract(column: &str, path: &str) -> String {
        format!(
            "json_extract({}, '$.{}')",
            quote_identifier(DatabaseBackend::Sqlite, column),
            path.replace('\'', "''")
        )
    }

    /// Get last insert rowid.
    pub async fn last_insert_rowid(pool: &SqlitePool) -> SqlxResult<i64> {
        let row = sqlx::query("SELECT last_insert_rowid()")
            .fetch_one(pool)
            .await?;
        let id: i64 = row.try_get(0)?;
        Ok(id)
    }

    /// Get SQLite version.
    pub async fn version(pool: &SqlitePool) -> SqlxResult<String> {
        let row = sqlx::query("SELECT sqlite_version()")
            .fetch_one(pool)
            .await?;
        let version: String = row.try_get(0)?;
        Ok(version)
    }

    /// Check if a table exists.
    pub async fn table_exists(pool: &SqlitePool, table: &str) -> SqlxResult<bool> {
        let sql = "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?";
        let row = sqlx::query(sql).bind(table).fetch_one(pool).await?;
        let count: i32 = row.try_get(0)?;
        Ok(count > 0)
    }

    /// Get table columns.
    pub async fn get_columns(pool: &SqlitePool, table: &str) -> SqlxResult<Vec<String>> {
        let sql = format!(
            "PRAGMA table_info({})",
            quote_identifier(DatabaseBackend::Sqlite, table)
        );
        let rows = sqlx::query(AssertSqlSafe(sql)).fetch_all(pool).await?;
        let columns: Vec<String> = rows
            .iter()
            .map(|r| r.try_get::<String, _>("name"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(columns)
    }

    /// Enable foreign keys.
    pub async fn enable_foreign_keys(pool: &SqlitePool) -> SqlxResult<()> {
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Set journal mode.
    pub async fn set_journal_mode(pool: &SqlitePool, mode: JournalMode) -> SqlxResult<()> {
        let sql = format!("PRAGMA journal_mode = {}", mode.as_str());
        // `mode` renders a fixed keyword from an enum.
        sqlx::query(AssertSqlSafe(sql)).execute(pool).await?;
        Ok(())
    }

    /// Set synchronous mode.
    pub async fn set_synchronous(pool: &SqlitePool, mode: SynchronousMode) -> SqlxResult<()> {
        let sql = format!("PRAGMA synchronous = {}", mode.as_str());
        // `mode` renders a fixed keyword from an enum.
        sqlx::query(AssertSqlSafe(sql)).execute(pool).await?;
        Ok(())
    }

    /// Vacuum the database.
    pub async fn vacuum(pool: &SqlitePool) -> SqlxResult<()> {
        sqlx::query("VACUUM").execute(pool).await?;
        Ok(())
    }

    /// Analyze the database.
    pub async fn analyze(pool: &SqlitePool) -> SqlxResult<()> {
        sqlx::query("ANALYZE").execute(pool).await?;
        Ok(())
    }

    /// Check database integrity.
    pub async fn integrity_check(pool: &SqlitePool) -> SqlxResult<Vec<String>> {
        let rows = sqlx::query("PRAGMA integrity_check")
            .fetch_all(pool)
            .await?;
        let results: Vec<String> = rows
            .iter()
            .map(|r| r.try_get::<String, _>(0).unwrap_or_default())
            .collect();
        Ok(results)
    }

    /// Generate FTS5 match expression.
    ///
    /// The table is quoted via `quote_identifier` (embedded double quotes
    /// are escaped); pass trusted identifiers only, not arbitrary user
    /// input. `query` is not embedded in the generated SQL: the emitted
    /// statement contains a `?` placeholder, and the caller must bind the
    /// query value positionally.
    pub fn fts5_match(table: &str, _query: &str) -> String {
        format!(
            "{} MATCH ?",
            quote_identifier(DatabaseBackend::Sqlite, table)
        )
    }
}

/// SQLite journal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalMode {
    /// Delete journal after transaction
    Delete,
    /// Truncate journal to zero length
    Truncate,
    /// Persist journal file
    Persist,
    /// In-memory journal
    Memory,
    /// Write-ahead logging (recommended)
    Wal,
    /// Disable journaling
    Off,
}

impl JournalMode {
    /// Get the SQL string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::Persist => "PERSIST",
            Self::Memory => "MEMORY",
            Self::Wal => "WAL",
            Self::Off => "OFF",
        }
    }
}

/// SQLite synchronous mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronousMode {
    /// Full synchronization
    Full,
    /// Normal synchronization
    Normal,
    /// No synchronization
    Off,
    /// Extra synchronization
    Extra,
}

impl SynchronousMode {
    /// Get the SQL string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Normal => "NORMAL",
            Self::Off => "OFF",
            Self::Extra => "EXTRA",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_sql() {
        let sql = SqliteHelpers::upsert_sql("users", &["id", "name", "email"]);
        assert!(sql.contains("INSERT OR REPLACE"));
        assert!(sql.contains("\"users\""));
    }

    #[test]
    fn test_on_conflict_sql() {
        let sql = SqliteHelpers::on_conflict_sql(
            "users",
            &["id", "name", "email"],
            &["id"],
            &["name", "email"],
        );
        assert!(sql.contains("ON CONFLICT(\"id\")"));
        assert!(sql.contains("DO UPDATE SET"));
    }

    #[test]
    fn test_json_extract() {
        assert_eq!(
            SqliteHelpers::json_extract("data", "name"),
            "json_extract(\"data\", '$.name')"
        );
        // Embedded double quotes in identifiers are escaped.
        assert_eq!(
            SqliteHelpers::json_extract("da\"ta", "name"),
            "json_extract(\"da\"\"ta\", '$.name')"
        );
        // Embedded single quotes in the path are escaped.
        assert_eq!(
            SqliteHelpers::json_extract("data", "na'me"),
            "json_extract(\"data\", '$.na''me')"
        );
    }

    #[test]
    fn test_fts5_match() {
        assert_eq!(
            SqliteHelpers::fts5_match("posts", "search"),
            "\"posts\" MATCH ?"
        );
        // Embedded double quotes in the table name are escaped.
        assert_eq!(
            SqliteHelpers::fts5_match("po\"sts", "search"),
            "\"po\"\"sts\" MATCH ?"
        );
    }

    #[test]
    fn test_journal_mode() {
        assert_eq!(JournalMode::Wal.as_str(), "WAL");
        assert_eq!(JournalMode::Delete.as_str(), "DELETE");
    }
}
