//! Migration file management.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{MigrateResult, MigrationError};
use crate::sql::MigrationSql;

/// A migration file on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationFile {
    /// Path to the migration file.
    pub path: PathBuf,
    /// Migration ID (extracted from filename).
    pub id: String,
    /// Migration name (human readable).
    pub name: String,
    /// Up SQL content.
    pub up_sql: String,
    /// Down SQL content.
    pub down_sql: String,
    /// Checksum of the migration content.
    pub checksum: String,
}

impl MigrationFile {
    /// Create a new migration file.
    pub fn new(id: impl Into<String>, name: impl Into<String>, sql: MigrationSql) -> Self {
        let id = id.into();
        let name = name.into();
        let checksum = compute_checksum(&sql.up);

        Self {
            path: PathBuf::new(),
            id,
            name,
            up_sql: sql.up,
            down_sql: sql.down,
            checksum,
        }
    }

    /// Set the path for this migration file.
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = path.into();
        self
    }
}

/// Compute a checksum for migration content.
fn compute_checksum(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Migration file reader/writer.
pub struct MigrationFileManager {
    /// Directory where migrations are stored.
    migrations_dir: PathBuf,
}

impl MigrationFileManager {
    /// Create a new file manager.
    pub fn new(migrations_dir: impl Into<PathBuf>) -> Self {
        Self {
            migrations_dir: migrations_dir.into(),
        }
    }

    /// Get the migrations directory.
    pub fn migrations_dir(&self) -> &Path {
        &self.migrations_dir
    }

    /// Ensure the migrations directory exists.
    pub async fn ensure_dir(&self) -> MigrateResult<()> {
        tokio::fs::create_dir_all(&self.migrations_dir)
            .await
            .map_err(MigrationError::Io)?;
        Ok(())
    }

    /// List all migration files in order.
    pub async fn list_migrations(&self) -> MigrateResult<Vec<MigrationFile>> {
        let mut migrations = Vec::new();

        if !self.migrations_dir.exists() {
            return Ok(migrations);
        }

        let mut entries = tokio::fs::read_dir(&self.migrations_dir)
            .await
            .map_err(MigrationError::Io)?;

        let mut paths = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(MigrationError::Io)? {
            let path = entry.path();
            if path.is_dir() && is_migration_dir(&path) {
                paths.push(path);
            }
        }

        // Sort by name (which should be timestamp-prefixed)
        paths.sort();

        for path in paths {
            if let Ok(migration) = self.read_migration(&path).await {
                migrations.push(migration);
            }
        }

        Ok(migrations)
    }

    /// Read a migration from a directory.
    async fn read_migration(&self, path: &Path) -> MigrateResult<MigrationFile> {
        let dir_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| MigrationError::InvalidMigration("Invalid path".to_string()))?;

        let (id, name) = parse_migration_name(dir_name)?;

        let up_path = path.join("up.sql");
        let down_path = path.join("down.sql");

        let up_sql = tokio::fs::read_to_string(&up_path)
            .await
            .map_err(MigrationError::Io)?;

        let down_sql = if down_path.exists() {
            tokio::fs::read_to_string(&down_path)
                .await
                .map_err(MigrationError::Io)?
        } else {
            String::new()
        };

        let checksum = compute_checksum(&up_sql);

        Ok(MigrationFile {
            path: path.to_path_buf(),
            id,
            name,
            up_sql,
            down_sql,
            checksum,
        })
    }

    /// Write a migration to disk.
    pub async fn write_migration(&self, migration: &MigrationFile) -> MigrateResult<PathBuf> {
        self.ensure_dir().await?;

        let timestamp = Utc::now().format("%Y%m%d%H%M%S");
        let dir_name = format!("{}_{}", timestamp, migration.name);
        let migration_dir = self.migrations_dir.join(&dir_name);

        tokio::fs::create_dir_all(&migration_dir)
            .await
            .map_err(MigrationError::Io)?;

        let up_path = migration_dir.join("up.sql");
        let down_path = migration_dir.join("down.sql");

        tokio::fs::write(&up_path, &migration.up_sql)
            .await
            .map_err(MigrationError::Io)?;

        if !migration.down_sql.is_empty() {
            tokio::fs::write(&down_path, &migration.down_sql)
                .await
                .map_err(MigrationError::Io)?;
        }

        Ok(migration_dir)
    }

    /// Generate a new migration ID.
    pub fn generate_id(&self) -> String {
        Utc::now().format("%Y%m%d%H%M%S").to_string()
    }
}

/// Check if a path is a migration directory.
fn is_migration_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    // Must have an up.sql file
    path.join("up.sql").exists()
}

/// Parse a migration directory name into (id, name).
fn parse_migration_name(name: &str) -> MigrateResult<(String, String)> {
    // Expected format: YYYYMMDDHHMMSS_name
    let parts: Vec<&str> = name.splitn(2, '_').collect();

    if parts.len() != 2 {
        return Err(MigrationError::InvalidMigration(format!(
            "Invalid migration name format: {}",
            name
        )));
    }

    let id = parts[0].to_string();
    let name = parts[1].to_string();

    // Validate ID looks like a timestamp
    if id.len() != 14 || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(MigrationError::InvalidMigration(format!(
            "Invalid migration ID (expected timestamp): {}",
            id
        )));
    }

    Ok((id, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_migration_name() {
        let (id, name) = parse_migration_name("20231215120000_create_users").unwrap();
        assert_eq!(id, "20231215120000");
        assert_eq!(name, "create_users");
    }

    #[test]
    fn test_parse_migration_name_invalid() {
        assert!(parse_migration_name("invalid").is_err());
        assert!(parse_migration_name("abc_test").is_err());
    }

    #[test]
    fn test_compute_checksum() {
        let checksum1 = compute_checksum("CREATE TABLE users();");
        let checksum2 = compute_checksum("CREATE TABLE users();");
        let checksum3 = compute_checksum("DROP TABLE users;");

        assert_eq!(checksum1, checksum2);
        assert_ne!(checksum1, checksum3);
    }

    #[test]
    fn test_migration_file_new() {
        let sql = MigrationSql {
            up: "CREATE TABLE users();".to_string(),
            down: "DROP TABLE users;".to_string(),
            warnings: Vec::new(),
        };

        let migration = MigrationFile::new("20231215120000", "create_users", sql);
        assert_eq!(migration.id, "20231215120000");
        assert_eq!(migration.name, "create_users");
        assert!(!migration.checksum.is_empty());
    }
}
