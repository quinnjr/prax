# Event Sourced Migration System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign Prax ORM migration system to use event sourcing with auto-generated reversible migrations and comprehensive rollback logging.

**Architecture:** Event sourcing foundation where all migration operations (apply, rollback, failure, resolution) are immutable events in an append-only log. Current state derived by replaying events. Reverse SQL auto-generated from schema diffs with data loss warnings.

**Tech Stack:** Rust, tokio-postgres, serde/serde_json, chrono, async-trait

---

## File Structure

### New Files
- `prax-migrate/src/event.rs` - Event types, EventData, EventType enums
- `prax-migrate/src/event_store.rs` - MigrationEventStore trait and PostgreSQL implementation
- `prax-migrate/src/state.rs` - MigrationState projection from events
- `prax-migrate/src/bootstrap.rs` - V1 to V2 migration logic

### Modified Files
- `prax-migrate/src/sql.rs` - Add warnings to MigrationSql, enhance reverse generation with warnings
- `prax-migrate/src/history.rs` - Add SQL constants for event log table
- `prax-migrate/src/engine.rs` - Refactor to use MigrationEventStore, add dev() and rollback() methods
- `prax-migrate/src/error.rs` - Add new error variants
- `prax-migrate/src/lib.rs` - Re-export new modules
- `prax-cli/src/commands/migrate.rs` - Add rollback reason/user flags, history command

---

## Task 1: Add Warnings to MigrationSql

**Files:**
- Modify: `prax-migrate/src/sql.rs:474-478`
- Test: Unit tests in same file

- [ ] **Step 1: Write failing test for MigrationSql with warnings**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_migration_sql_with_warnings() {
        let sql = MigrationSql {
            up: "CREATE TABLE users (id INT);".to_string(),
            down: "DROP TABLE users;".to_string(),
            warnings: vec![
                "Dropping table 'users' - all data will be lost".to_string(),
            ],
        };
        
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("data will be lost"));
    }
    
    #[test]
    fn test_migration_sql_no_warnings() {
        let sql = MigrationSql {
            up: "CREATE INDEX idx_email ON users(email);".to_string(),
            down: "DROP INDEX idx_email;".to_string(),
            warnings: Vec::new(),
        };
        
        assert!(sql.warnings.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_migration_sql_with_warnings`
Expected: FAIL with "no field `warnings`"

- [ ] **Step 3: Add warnings field to MigrationSql**

```rust
/// SQL for a migration (forward and reverse).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationSql {
    /// SQL to apply the migration.
    pub up: String,
    /// SQL to rollback the migration.
    pub down: String,
    /// Warnings about data loss or irreversible operations.
    pub warnings: Vec<String>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_migration_sql`
Expected: PASS

- [ ] **Step 5: Update all MigrationSql constructors to include empty warnings**

Find all places where `MigrationSql { up, down }` is constructed and add `warnings: Vec::new()`:

```rust
// In PostgresSqlGenerator::generate (line ~96)
MigrationSql {
    up: up.join("\n\n"),
    down: down.join("\n\n"),
    warnings: Vec::new(),
}

// In MySqlGenerator::generate
MigrationSql {
    up: up.join("\n\n"),
    down: down.join("\n\n"),
    warnings: Vec::new(),
}

// In SqliteGenerator::generate
MigrationSql {
    up: up.join("\n\n"),
    down: down.join("\n\n"),
    warnings: Vec::new(),
}

// In MssqlGenerator::generate
MigrationSql {
    up: up.join("\n\n"),
    down: down.join("\n\n"),
    warnings: Vec::new(),
}
```

- [ ] **Step 6: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 7: Commit**

```bash
git add prax-migrate/src/sql.rs
git commit -m "feat(migrate): add warnings field to MigrationSql"
```

---

## Task 2: Add Data Loss Warnings to PostgreSQL Generator

**Files:**
- Modify: `prax-migrate/src/sql.rs:9-473` (PostgresSqlGenerator implementation)

- [ ] **Step 1: Write test for drop table warning**

```rust
#[test]
fn test_postgres_drop_table_generates_warning() {
    let generator = PostgresSqlGenerator;
    let diff = SchemaDiff {
        drop_models: vec!["users".to_string()],
        ..Default::default()
    };
    
    let sql = generator.generate(&diff);
    
    assert!(sql.down.contains("CREATE TABLE"));
    assert_eq!(sql.warnings.len(), 1);
    assert!(sql.warnings[0].contains("users"));
    assert!(sql.warnings[0].contains("data"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_postgres_drop_table_generates_warning`
Expected: FAIL - warnings is empty

- [ ] **Step 3: Update PostgresSqlGenerator::generate to collect warnings**

```rust
impl PostgresSqlGenerator {
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // ... existing code for extensions and enums ...

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // ... rest of existing code ...

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_postgres_drop_table_generates_warning`
Expected: PASS

- [ ] **Step 5: Write test for drop column warning**

```rust
#[test]
fn test_postgres_drop_column_generates_warning() {
    let generator = PostgresSqlGenerator;
    let diff = SchemaDiff {
        alter_models: vec![ModelAlterDiff {
            name: "users".to_string(),
            table_name: "users".to_string(),
            drop_fields: vec!["email".to_string()],
            ..Default::default()
        }],
        ..Default::default()
    };
    
    let sql = generator.generate(&diff);
    
    assert!(sql.warnings.iter().any(|w| 
        w.contains("email") && w.contains("users") && w.contains("data")
    ));
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_postgres_drop_column_generates_warning`
Expected: FAIL - no warning for dropped column

- [ ] **Step 7: Add warning for dropped columns in alter_table**

```rust
// In PostgresSqlGenerator::alter_table method (around line 200)
fn alter_table(&self, alter: &ModelAlterDiff) -> Vec<String> {
    let mut statements = Vec::new();
    
    // Drop fields
    for field in &alter.drop_fields {
        statements.push(format!(
            "ALTER TABLE \"{}\" DROP COLUMN IF EXISTS \"{}\";",
            alter.table_name, field
        ));
        // Note: warnings collected in generate() method
    }
    
    // ... rest of method ...
    
    statements
}

// In generate() method, after alter_models loop:
for alter in &diff.alter_models {
    up.extend(self.alter_table(alter));
    
    // Add warnings for dropped columns
    for field in &alter.drop_fields {
        warnings.push(format!(
            "Dropping column '{}' from table '{}' - data in this column will be lost",
            field, alter.table_name
        ));
    }
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_postgres_drop_column_generates_warning`
Expected: PASS

- [ ] **Step 9: Write test for alter column type warning**

```rust
#[test]
fn test_postgres_alter_column_type_generates_warning() {
    let generator = PostgresSqlGenerator;
    let diff = SchemaDiff {
        alter_models: vec![ModelAlterDiff {
            name: "users".to_string(),
            table_name: "users".to_string(),
            alter_fields: vec![FieldAlterDiff {
                name: "age".to_string(),
                old_type: Some("INTEGER".to_string()),
                new_type: Some("BIGINT".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    
    let sql = generator.generate(&diff);
    
    // Type changes should have warning about potential data loss during reverse
    assert!(sql.warnings.iter().any(|w| 
        w.contains("age") && w.contains("type")
    ));
}
```

- [ ] **Step 10: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_postgres_alter_column_type_generates_warning`
Expected: FAIL - no warning for type change

- [ ] **Step 11: Add warning for column type changes**

```rust
// In generate() method, in the alter_models loop:
for alter in &diff.alter_models {
    up.extend(self.alter_table(alter));
    
    // Add warnings for dropped columns
    for field in &alter.drop_fields {
        warnings.push(format!(
            "Dropping column '{}' from table '{}' - data in this column will be lost",
            field, alter.table_name
        ));
    }
    
    // Add warnings for type changes
    for field_alter in &alter.alter_fields {
        if field_alter.old_type.is_some() && field_alter.new_type.is_some() {
            warnings.push(format!(
                "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                field_alter.name, alter.table_name
            ));
        }
    }
}
```

- [ ] **Step 12: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_postgres_alter_column_type_generates_warning`
Expected: PASS

- [ ] **Step 13: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 14: Commit**

```bash
git add prax-migrate/src/sql.rs
git commit -m "feat(migrate): add data loss warnings to PostgreSQL generator"
```

---

## Task 3: Add Data Loss Warnings to MySQL Generator

**Files:**
- Modify: `prax-migrate/src/sql.rs:489-900` (MySqlGenerator implementation)

- [ ] **Step 1: Write test for MySQL drop table warning**

```rust
#[test]
fn test_mysql_drop_table_generates_warning() {
    let generator = MySqlGenerator;
    let diff = SchemaDiff {
        drop_models: vec!["users".to_string()],
        ..Default::default()
    };
    
    let sql = generator.generate(&diff);
    
    assert_eq!(sql.warnings.len(), 1);
    assert!(sql.warnings[0].contains("users"));
    assert!(sql.warnings[0].contains("data"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_mysql_drop_table_generates_warning`
Expected: FAIL - warnings is empty

- [ ] **Step 3: Update MySqlGenerator::generate to collect warnings**

```rust
impl MySqlGenerator {
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // ... existing create models code ...

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models
        for alter in &diff.alter_models {
            up.extend(self.alter_table(alter));
            
            // Warnings for dropped columns
            for field in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field, alter.table_name
                ));
            }
            
            // Warnings for type changes
            for field_alter in &alter.alter_fields {
                if field_alter.old_type.is_some() && field_alter.new_type.is_some() {
                    warnings.push(format!(
                        "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                        field_alter.name, alter.table_name
                    ));
                }
            }
        }

        // ... rest of existing code ...

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_mysql_drop_table_generates_warning`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add prax-migrate/src/sql.rs
git commit -m "feat(migrate): add data loss warnings to MySQL generator"
```

---

## Task 4: Add Data Loss Warnings to SQLite Generator

**Files:**
- Modify: `prax-migrate/src/sql.rs:902-1280` (SqliteGenerator implementation)

- [ ] **Step 1: Write test for SQLite drop table warning**

```rust
#[test]
fn test_sqlite_drop_table_generates_warning() {
    let generator = SqliteGenerator;
    let diff = SchemaDiff {
        drop_models: vec!["users".to_string()],
        ..Default::default()
    };
    
    let sql = generator.generate(&diff);
    
    assert_eq!(sql.warnings.len(), 1);
    assert!(sql.warnings[0].contains("users"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_sqlite_drop_table_generates_warning`
Expected: FAIL

- [ ] **Step 3: Update SqliteGenerator::generate to collect warnings (same pattern as MySQL)**

```rust
impl SqliteGenerator {
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // ... existing code ...

        // Drop models - add warnings
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models - add warnings
        for alter in &diff.alter_models {
            up.extend(self.alter_table(alter));
            
            for field in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field, alter.table_name
                ));
            }
            
            for field_alter in &alter.alter_fields {
                if field_alter.old_type.is_some() && field_alter.new_type.is_some() {
                    warnings.push(format!(
                        "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                        field_alter.name, alter.table_name
                    ));
                }
            }
        }

        // ... rest of code ...

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_sqlite_drop_table_generates_warning`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/sql.rs
git commit -m "feat(migrate): add data loss warnings to SQLite generator"
```

---

## Task 5: Add Data Loss Warnings to MSSQL Generator

**Files:**
- Modify: `prax-migrate/src/sql.rs:1282-1738` (MssqlGenerator implementation)

- [ ] **Step 1: Write test for MSSQL drop table warning**

```rust
#[test]
fn test_mssql_drop_table_generates_warning() {
    let generator = MssqlGenerator;
    let diff = SchemaDiff {
        drop_models: vec!["users".to_string()],
        ..Default::default()
    };
    
    let sql = generator.generate(&diff);
    
    assert_eq!(sql.warnings.len(), 1);
    assert!(sql.warnings[0].contains("users"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_mssql_drop_table_generates_warning`
Expected: FAIL

- [ ] **Step 3: Update MssqlGenerator::generate to collect warnings (same pattern)**

```rust
impl MssqlGenerator {
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // ... existing code ...

        // Drop models - add warnings
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models - add warnings
        for alter in &diff.alter_models {
            up.extend(self.alter_table(alter));
            
            for field in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field, alter.table_name
                ));
            }
            
            for field_alter in &alter.alter_fields {
                if field_alter.old_type.is_some() && field_alter.new_type.is_some() {
                    warnings.push(format!(
                        "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                        field_alter.name, alter.table_name
                    ));
                }
            }
        }

        // ... rest of code ...

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_mssql_drop_table_generates_warning`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add prax-migrate/src/sql.rs
git commit -m "feat(migrate): add data loss warnings to MSSQL generator"
```

---

## Task 6: Create Event Types Module

**Files:**
- Create: `prax-migrate/src/event.rs`

- [ ] **Step 1: Write test for EventType serialization**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_event_type_serialization() {
        let event_type = EventType::Applied;
        let json = serde_json::to_string(&event_type).unwrap();
        assert_eq!(json, "\"Applied\"");
        
        let deserialized: EventType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, EventType::Applied);
    }
    
    #[test]
    fn test_event_type_as_str() {
        assert_eq!(EventType::Applied.as_str(), "applied");
        assert_eq!(EventType::RolledBack.as_str(), "rolled_back");
        assert_eq!(EventType::Failed.as_str(), "failed");
        assert_eq!(EventType::Resolved.as_str(), "resolved");
    }
    
    #[test]
    fn test_event_type_from_str() {
        assert_eq!(EventType::from_str("applied").unwrap(), EventType::Applied);
        assert_eq!(EventType::from_str("rolled_back").unwrap(), EventType::RolledBack);
        assert!(EventType::from_str("invalid").is_err());
    }
}
```

- [ ] **Step 2: Create file and run test to verify it fails**

```bash
touch prax-migrate/src/event.rs
```

Run: `cargo test --package prax-migrate event_type`
Expected: FAIL - module doesn't exist

- [ ] **Step 3: Implement EventType enum**

```rust
//! Event types for migration event sourcing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::{MigrateError, MigrateResult};

/// Type of migration event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// Migration was applied successfully.
    Applied,
    /// Migration was rolled back.
    RolledBack,
    /// Migration failed to apply.
    Failed,
    /// Migration conflict was resolved.
    Resolved,
}

impl EventType {
    /// Convert to database string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Applied => "applied",
            EventType::RolledBack => "rolled_back",
            EventType::Failed => "failed",
            EventType::Resolved => "resolved",
        }
    }
    
    /// Parse from database string representation.
    pub fn from_str(s: &str) -> MigrateResult<Self> {
        match s {
            "applied" => Ok(EventType::Applied),
            "rolled_back" => Ok(EventType::RolledBack),
            "failed" => Ok(EventType::Failed),
            "resolved" => Ok(EventType::Resolved),
            _ => Err(MigrateError::invalid_migration(format!(
                "Unknown event type: {}",
                s
            ))),
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate event_type`
Expected: PASS

- [ ] **Step 5: Write test for EventData variants**

```rust
#[test]
fn test_event_data_applied_serialization() {
    let data = EventData::Applied {
        checksum: "abc123".to_string(),
        duration_ms: 150,
        applied_by: Some("user@system".to_string()),
        up_sql_preview: Some("CREATE TABLE".to_string()),
        auto_generated: true,
    };
    
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["type"], "applied");
    assert_eq!(json["checksum"], "abc123");
    assert_eq!(json["duration_ms"], 150);
    
    let deserialized: EventData = serde_json::from_value(json).unwrap();
    if let EventData::Applied { checksum, .. } = deserialized {
        assert_eq!(checksum, "abc123");
    } else {
        panic!("Wrong variant");
    }
}

#[test]
fn test_event_data_rolled_back_serialization() {
    let data = EventData::RolledBack {
        checksum: "abc123".to_string(),
        duration_ms: 89,
        rolled_back_by: Some("user@system".to_string()),
        reason: Some("Testing".to_string()),
        parent_event_id: 42,
        down_sql_preview: Some("DROP TABLE".to_string()),
    };
    
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["type"], "rolled_back");
    assert_eq!(json["parent_event_id"], 42);
    assert_eq!(json["reason"], "Testing");
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test --package prax-migrate event_data`
Expected: FAIL - EventData doesn't exist

- [ ] **Step 7: Implement EventData enum**

```rust
/// Event-specific data stored in JSONB column.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventData {
    /// Migration applied successfully.
    Applied {
        checksum: String,
        duration_ms: i64,
        applied_by: Option<String>,
        up_sql_preview: Option<String>,
        auto_generated: bool,
    },
    /// Migration rolled back.
    RolledBack {
        checksum: String,
        duration_ms: i64,
        rolled_back_by: Option<String>,
        reason: Option<String>,
        parent_event_id: i64,
        down_sql_preview: Option<String>,
    },
    /// Migration failed.
    Failed {
        error: String,
        attempted_by: Option<String>,
        sql_preview: Option<String>,
    },
    /// Conflict resolved.
    Resolved {
        resolution_type: String,
        old_checksum: Option<String>,
        new_checksum: Option<String>,
        reason: String,
        resolved_by: Option<String>,
    },
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test --package prax-migrate event_data`
Expected: PASS

- [ ] **Step 9: Write test for MigrationEvent**

```rust
#[test]
fn test_migration_event_creation() {
    let event = MigrationEvent {
        event_id: 1,
        migration_id: "20260425120000".to_string(),
        event_type: EventType::Applied,
        event_data: EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: Some("user".to_string()),
            up_sql_preview: None,
            auto_generated: true,
        },
        created_at: Utc::now(),
    };
    
    assert_eq!(event.event_id, 1);
    assert_eq!(event.migration_id, "20260425120000");
    assert_eq!(event.event_type, EventType::Applied);
}
```

- [ ] **Step 10: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_migration_event_creation`
Expected: FAIL - MigrationEvent doesn't exist

- [ ] **Step 11: Implement MigrationEvent struct**

```rust
/// A migration event in the event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationEvent {
    /// Unique event ID (autoincrement).
    pub event_id: i64,
    /// Migration ID this event belongs to.
    pub migration_id: String,
    /// Type of event.
    pub event_type: EventType,
    /// Event-specific data.
    pub event_data: EventData,
    /// When the event was created.
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 12: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_migration_event_creation`
Expected: PASS

- [ ] **Step 13: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 14: Commit**

```bash
git add prax-migrate/src/event.rs
git commit -m "feat(migrate): add event types for event sourcing"
```

---

## Task 7: Create Migration State Projection Module

**Files:**
- Create: `prax-migrate/src/state.rs`

- [ ] **Step 1: Write test for MigrationStatus creation**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventData, EventType, MigrationEvent};
    use chrono::Utc;
    
    #[test]
    fn test_migration_status_new() {
        let status = MigrationStatus::new("20260425120000".to_string());
        
        assert_eq!(status.migration_id, "20260425120000");
        assert!(status.checksum.is_empty());
        assert!(!status.is_applied);
        assert_eq!(status.apply_count, 0);
        assert_eq!(status.rollback_count, 0);
        assert!(status.last_applied_at.is_none());
        assert!(status.last_rolled_back_at.is_none());
    }
}
```

- [ ] **Step 2: Create file and run test to verify it fails**

```bash
touch prax-migrate/src/state.rs
```

Run: `cargo test --package prax-migrate test_migration_status_new`
Expected: FAIL

- [ ] **Step 3: Implement MigrationStatus struct**

```rust
//! Migration state projection from events.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::event::{EventData, EventType, MigrationEvent};

/// Status of a single migration derived from events.
#[derive(Debug, Clone)]
pub struct MigrationStatus {
    /// Migration ID.
    pub migration_id: String,
    /// Current checksum.
    pub checksum: String,
    /// Whether the migration is currently applied.
    pub is_applied: bool,
    /// Last time it was applied.
    pub last_applied_at: Option<DateTime<Utc>>,
    /// Last time it was rolled back.
    pub last_rolled_back_at: Option<DateTime<Utc>>,
    /// Number of times applied.
    pub apply_count: u32,
    /// Number of times rolled back.
    pub rollback_count: u32,
}

impl MigrationStatus {
    /// Create a new migration status.
    pub fn new(migration_id: String) -> Self {
        Self {
            migration_id,
            checksum: String::new(),
            is_applied: false,
            last_applied_at: None,
            last_rolled_back_at: None,
            apply_count: 0,
            rollback_count: 0,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_migration_status_new`
Expected: PASS

- [ ] **Step 5: Write test for state projection from apply event**

```rust
#[test]
fn test_state_from_apply_event() {
    let events = vec![MigrationEvent {
        event_id: 1,
        migration_id: "20260425120000".to_string(),
        event_type: EventType::Applied,
        event_data: EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
        created_at: Utc::now(),
    }];
    
    let state = MigrationState::from_events(events);
    
    assert!(state.is_applied("20260425120000"));
    let status = state.get_status("20260425120000").unwrap();
    assert_eq!(status.apply_count, 1);
    assert_eq!(status.rollback_count, 0);
    assert_eq!(status.checksum, "abc123");
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_state_from_apply_event`
Expected: FAIL - MigrationState doesn't exist

- [ ] **Step 7: Implement MigrationState struct**

```rust
/// Complete migration state derived from event log.
#[derive(Debug, Clone)]
pub struct MigrationState {
    applied_migrations: HashMap<String, MigrationStatus>,
}

impl MigrationState {
    /// Build state by replaying all events.
    pub fn from_events(events: Vec<MigrationEvent>) -> Self {
        let mut state = HashMap::new();
        
        for event in events {
            let status = state
                .entry(event.migration_id.clone())
                .or_insert_with(|| MigrationStatus::new(event.migration_id.clone()));
            
            match event.event_type {
                EventType::Applied => {
                    status.is_applied = true;
                    status.last_applied_at = Some(event.created_at);
                    status.apply_count += 1;
                    
                    if let EventData::Applied { checksum, .. } = event.event_data {
                        status.checksum = checksum;
                    }
                }
                EventType::RolledBack => {
                    status.is_applied = false;
                    status.last_rolled_back_at = Some(event.created_at);
                    status.rollback_count += 1;
                }
                EventType::Failed => {
                    // Failed attempts don't change applied state
                }
                EventType::Resolved => {
                    // Resolution events update metadata but not state
                    if let EventData::Resolved {
                        new_checksum: Some(checksum),
                        ..
                    } = event.event_data
                    {
                        status.checksum = checksum;
                    }
                }
            }
        }
        
        Self {
            applied_migrations: state,
        }
    }
    
    /// Check if a migration is currently applied.
    pub fn is_applied(&self, migration_id: &str) -> bool {
        self.applied_migrations
            .get(migration_id)
            .map(|s| s.is_applied)
            .unwrap_or(false)
    }
    
    /// Get status for a migration.
    pub fn get_status(&self, migration_id: &str) -> Option<&MigrationStatus> {
        self.applied_migrations.get(migration_id)
    }
    
    /// Get all currently applied migrations.
    pub fn get_applied(&self) -> Vec<&MigrationStatus> {
        self.applied_migrations
            .values()
            .filter(|s| s.is_applied)
            .collect()
    }
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_state_from_apply_event`
Expected: PASS

- [ ] **Step 9: Write test for apply-rollback cycle**

```rust
#[test]
fn test_state_apply_rollback_cycle() {
    let now = Utc::now();
    let events = vec![
        MigrationEvent {
            event_id: 1,
            migration_id: "20260425120000".to_string(),
            event_type: EventType::Applied,
            event_data: EventData::Applied {
                checksum: "abc123".to_string(),
                duration_ms: 150,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
            created_at: now,
        },
        MigrationEvent {
            event_id: 2,
            migration_id: "20260425120000".to_string(),
            event_type: EventType::RolledBack,
            event_data: EventData::RolledBack {
                checksum: "abc123".to_string(),
                duration_ms: 89,
                rolled_back_by: None,
                reason: None,
                parent_event_id: 1,
                down_sql_preview: None,
            },
            created_at: now,
        },
    ];
    
    let state = MigrationState::from_events(events);
    
    assert!(!state.is_applied("20260425120000"));
    let status = state.get_status("20260425120000").unwrap();
    assert_eq!(status.apply_count, 1);
    assert_eq!(status.rollback_count, 1);
}
```

- [ ] **Step 10: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_state_apply_rollback_cycle`
Expected: PASS

- [ ] **Step 11: Write test for multiple apply-rollback cycles**

```rust
#[test]
fn test_state_multiple_cycles() {
    let events = vec![
        // Apply
        create_apply_event(1, "20260425120000"),
        // Rollback
        create_rollback_event(2, "20260425120000", 1),
        // Apply again
        create_apply_event(3, "20260425120000"),
    ];
    
    let state = MigrationState::from_events(events);
    
    // Should be applied (last event wins)
    assert!(state.is_applied("20260425120000"));
    let status = state.get_status("20260425120000").unwrap();
    assert_eq!(status.apply_count, 2);
    assert_eq!(status.rollback_count, 1);
}

fn create_apply_event(event_id: i64, migration_id: &str) -> MigrationEvent {
    MigrationEvent {
        event_id,
        migration_id: migration_id.to_string(),
        event_type: EventType::Applied,
        event_data: EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
        created_at: Utc::now(),
    }
}

fn create_rollback_event(
    event_id: i64,
    migration_id: &str,
    parent_event_id: i64,
) -> MigrationEvent {
    MigrationEvent {
        event_id,
        migration_id: migration_id.to_string(),
        event_type: EventType::RolledBack,
        event_data: EventData::RolledBack {
            checksum: "abc123".to_string(),
            duration_ms: 89,
            rolled_back_by: None,
            reason: None,
            parent_event_id,
            down_sql_preview: None,
        },
        created_at: Utc::now(),
    }
}
```

- [ ] **Step 12: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_state_multiple_cycles`
Expected: PASS

- [ ] **Step 13: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 14: Commit**

```bash
git add prax-migrate/src/state.rs
git commit -m "feat(migrate): add migration state projection from events"
```

---

## Task 8: Update Library Exports

**Files:**
- Modify: `prax-migrate/src/lib.rs`

- [ ] **Step 1: Add module declarations**

```rust
pub mod event;
pub mod state;
```

- [ ] **Step 2: Add re-exports**

```rust
// Add to existing re-exports section
pub use event::{EventData, EventType, MigrationEvent};
pub use state::{MigrationState, MigrationStatus};
```

- [ ] **Step 3: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/lib.rs
git commit -m "feat(migrate): export event and state modules"
```

---

## Task 9: Create Event Store Trait

**Files:**
- Create: `prax-migrate/src/event_store.rs`

- [ ] **Step 1: Write test for trait implementation check**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    // This test just verifies the trait compiles
    #[test]
    fn test_event_store_trait_exists() {
        // If this compiles, the trait is defined correctly
        fn assert_event_store<T: MigrationEventStore>() {}
    }
}
```

- [ ] **Step 2: Create file with trait definition**

```rust
//! Event store for migration events.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::MigrateResult;
use crate::event::{EventData, EventType, MigrationEvent};
use crate::history::MigrationLock;

/// Repository for migration events.
#[async_trait]
pub trait MigrationEventStore: Send + Sync {
    /// Append a new event to the log.
    async fn append_event(
        &self,
        migration_id: &str,
        event_type: EventType,
        event_data: EventData,
    ) -> MigrateResult<i64>;
    
    /// Get all events for a specific migration.
    async fn get_events(&self, migration_id: &str) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Get all events (for replaying full history).
    async fn get_all_events(&self) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Query events by type.
    async fn get_events_by_type(&self, event_type: EventType) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Get events since a specific time.
    async fn get_events_since(&self, since: DateTime<Utc>) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Initialize the event log table.
    async fn initialize(&self) -> MigrateResult<()>;
    
    /// Acquire exclusive lock for migrations.
    async fn acquire_lock(&self) -> MigrateResult<MigrationLock>;
}
```

- [ ] **Step 3: Run test to verify it compiles**

Run: `cargo test --package prax-migrate test_event_store_trait_exists`
Expected: PASS

- [ ] **Step 4: Add event log table SQL constants to history.rs**

```rust
// Add to prax-migrate/src/history.rs after existing POSTGRES_INIT_SQL

/// SQL for creating the event log table (PostgreSQL).
pub const POSTGRES_EVENT_LOG_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS "_prax_migrations" (
    event_id BIGSERIAL PRIMARY KEY,
    migration_id VARCHAR(255) NOT NULL,
    event_type VARCHAR(50) NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    
    CONSTRAINT valid_event_type CHECK (
        event_type IN ('applied', 'rolled_back', 'failed', 'resolved')
    )
);

CREATE INDEX IF NOT EXISTS idx_migrations_migration_id 
    ON "_prax_migrations" (migration_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_migrations_event_type 
    ON "_prax_migrations" (event_type);

CREATE INDEX IF NOT EXISTS idx_migrations_created_at 
    ON "_prax_migrations" (created_at DESC);
"#;
```

- [ ] **Step 5: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add prax-migrate/src/event_store.rs prax-migrate/src/history.rs
git commit -m "feat(migrate): add event store trait and event log SQL"
```

---

## Task 10: Add Event Store to Library Exports

**Files:**
- Modify: `prax-migrate/src/lib.rs`

- [ ] **Step 1: Add module declaration and re-exports**

```rust
pub mod event_store;

// Add to re-exports
pub use event_store::MigrationEventStore;
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add prax-migrate/src/lib.rs
git commit -m "feat(migrate): export event store module"
```

---

## Task 11: Add New Error Variants

**Files:**
- Modify: `prax-migrate/src/error.rs`

- [ ] **Step 1: Write test for new error variants**

```rust
#[test]
fn test_bootstrap_failed_error() {
    let err = MigrationError::BootstrapFailed("Event count mismatch".to_string());
    assert!(err.to_string().contains("Bootstrap failed"));
}

#[test]
fn test_no_migrations_to_rollback_error() {
    let err = MigrationError::NoMigrationsToRollback;
    assert!(err.to_string().contains("No migrations"));
}

#[test]
fn test_parent_event_not_found_error() {
    let err = MigrationError::ParentEventNotFound;
    assert!(err.to_string().contains("Parent event"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --package prax-migrate bootstrap_failed_error`
Expected: FAIL - variant doesn't exist

- [ ] **Step 3: Add new error variants to MigrationError enum**

```rust
// Add these variants to the MigrationError enum in error.rs
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    // ... existing variants ...
    
    #[error("Bootstrap migration failed: {0}")]
    BootstrapFailed(String),
    
    #[error("No migrations to rollback")]
    NoMigrationsToRollback,
    
    #[error("Parent event not found for rollback")]
    ParentEventNotFound,
    
    #[error("Migration file not found: {0}")]
    MigrationFileNotFound(String),
    
    #[error("No down migration available for: {0}")]
    NoDownMigration(String),
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --package prax-migrate error`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/error.rs
git commit -m "feat(migrate): add error variants for event sourcing"
```

---

## Task 12: Implement In-Memory Event Store for Testing

**Files:**
- Modify: `prax-migrate/src/event_store.rs`

- [ ] **Step 1: Write test for in-memory event store**

```rust
#[tokio::test]
async fn test_in_memory_event_store_append_and_get() {
    let store = InMemoryEventStore::new();
    
    let event_id = store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
    ).await.unwrap();
    
    assert!(event_id > 0);
    
    let events = store.get_all_events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].migration_id, "20260425120000");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_in_memory_event_store`
Expected: FAIL - InMemoryEventStore doesn't exist

- [ ] **Step 3: Implement InMemoryEventStore**

```rust
use std::sync::{Arc, Mutex};

/// In-memory event store for testing.
#[derive(Clone)]
pub struct InMemoryEventStore {
    events: Arc<Mutex<Vec<MigrationEvent>>>,
    next_id: Arc<Mutex<i64>>,
}

impl InMemoryEventStore {
    /// Create a new in-memory event store.
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(Mutex::new(1)),
        }
    }
}

#[async_trait]
impl MigrationEventStore for InMemoryEventStore {
    async fn append_event(
        &self,
        migration_id: &str,
        event_type: EventType,
        event_data: EventData,
    ) -> MigrateResult<i64> {
        let mut next_id = self.next_id.lock().unwrap();
        let event_id = *next_id;
        *next_id += 1;
        drop(next_id);
        
        let event = MigrationEvent {
            event_id,
            migration_id: migration_id.to_string(),
            event_type,
            event_data,
            created_at: Utc::now(),
        };
        
        let mut events = self.events.lock().unwrap();
        events.push(event);
        
        Ok(event_id)
    }
    
    async fn get_events(&self, migration_id: &str) -> MigrateResult<Vec<MigrationEvent>> {
        let events = self.events.lock().unwrap();
        Ok(events
            .iter()
            .filter(|e| e.migration_id == migration_id)
            .cloned()
            .collect())
    }
    
    async fn get_all_events(&self) -> MigrateResult<Vec<MigrationEvent>> {
        let events = self.events.lock().unwrap();
        Ok(events.clone())
    }
    
    async fn get_events_by_type(&self, event_type: EventType) -> MigrateResult<Vec<MigrationEvent>> {
        let events = self.events.lock().unwrap();
        Ok(events
            .iter()
            .filter(|e| e.event_type == event_type)
            .cloned()
            .collect())
    }
    
    async fn get_events_since(&self, since: DateTime<Utc>) -> MigrateResult<Vec<MigrationEvent>> {
        let events = self.events.lock().unwrap();
        Ok(events
            .iter()
            .filter(|e| e.created_at >= since)
            .cloned()
            .collect())
    }
    
    async fn initialize(&self) -> MigrateResult<()> {
        Ok(())
    }
    
    async fn acquire_lock(&self) -> MigrateResult<MigrationLock> {
        Ok(MigrationLock::new(1, || {}))
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_in_memory_event_store`
Expected: PASS

- [ ] **Step 5: Write test for event filtering**

```rust
#[tokio::test]
async fn test_in_memory_event_store_filtering() {
    let store = InMemoryEventStore::new();
    
    // Add applied event
    store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
    ).await.unwrap();
    
    // Add rollback event
    store.append_event(
        "20260425120000",
        EventType::RolledBack,
        EventData::RolledBack {
            checksum: "abc123".to_string(),
            duration_ms: 89,
            rolled_back_by: None,
            reason: None,
            parent_event_id: 1,
            down_sql_preview: None,
        },
    ).await.unwrap();
    
    // Filter by type
    let applied = store.get_events_by_type(EventType::Applied).await.unwrap();
    assert_eq!(applied.len(), 1);
    
    let rolled_back = store.get_events_by_type(EventType::RolledBack).await.unwrap();
    assert_eq!(rolled_back.len(), 1);
    
    // Get by migration
    let migration_events = store.get_events("20260425120000").await.unwrap();
    assert_eq!(migration_events.len(), 2);
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_in_memory_event_store_filtering`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add prax-migrate/src/event_store.rs
git commit -m "feat(migrate): add in-memory event store for testing"
```

---

## Task 13: Create Bootstrap Module

**Files:**
- Create: `prax-migrate/src/bootstrap.rs`

- [ ] **Step 1: Write test for V1 format detection**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_detect_v1_format() {
        // This will be integration test with actual DB
        // For now, just check the module compiles
    }
}
```

- [ ] **Step 2: Create bootstrap module**

```rust
//! Bootstrap migration from V1 to V2 event sourcing format.

use crate::error::{MigrateError, MigrateResult};

/// SQL to check if the table uses V1 format.
pub const CHECK_V1_FORMAT_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1 
    FROM information_schema.columns 
    WHERE table_name = '_prax_migrations' 
    AND column_name = 'rolled_back'
    AND data_type = 'boolean'
)
"#;

/// SQL to check if the table uses V2 (event sourcing) format.
pub const CHECK_V2_FORMAT_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1 
    FROM information_schema.columns 
    WHERE table_name = '_prax_migrations' 
    AND column_name = 'event_type'
)
"#;

/// SQL to rename V1 table to backup.
pub const RENAME_V1_TABLE_SQL: &str = r#"
ALTER TABLE "_prax_migrations" RENAME TO "_prax_migrations_v1_backup"
"#;

/// SQL to migrate V1 records to V2 event log.
pub const MIGRATE_V1_TO_V2_SQL: &str = r#"
INSERT INTO "_prax_migrations" 
    (migration_id, event_type, event_data, created_at)
SELECT 
    id,
    CASE 
        WHEN rolled_back THEN 'rolled_back'
        ELSE 'applied'
    END,
    jsonb_build_object(
        'checksum', checksum,
        'duration_ms', COALESCE(duration_ms, 0),
        'migrated_from_v1', true
    ),
    applied_at
FROM "_prax_migrations_v1_backup"
ORDER BY applied_at ASC
"#;

/// SQL to verify migration success.
pub const VERIFY_MIGRATION_SQL: &str = r#"
SELECT 
    (SELECT COUNT(*) FROM "_prax_migrations_v1_backup") as v1_count,
    (SELECT COUNT(*) FROM "_prax_migrations") as v2_count
"#;

/// Bootstrap helper for migration from V1 to V2.
pub struct Bootstrap;

impl Bootstrap {
    /// Generate migration instructions for user.
    pub fn migration_instructions() -> String {
        r#"
╔═══════════════════════════════════════════════════════════════════╗
║  MIGRATION FROM V1 TO EVENT SOURCING (V2)                         ║
╠═══════════════════════════════════════════════════════════════════╣
║                                                                     ║
║  The migration system will:                                        ║
║  1. Rename existing table to _prax_migrations_v1_backup           ║
║  2. Create new event log table                                     ║
║  3. Migrate existing records as events                            ║
║  4. Verify migration success                                       ║
║                                                                     ║
║  Your existing migration history will be preserved in the backup.  ║
║                                                                     ║
╚═══════════════════════════════════════════════════════════════════╝
"#
        .to_string()
    }
    
    /// Check if migration is needed.
    pub fn needs_migration(has_v1: bool, has_v2: bool) -> bool {
        has_v1 && !has_v2
    }
}
```

- [ ] **Step 3: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 4: Add module to lib.rs**

```rust
pub mod bootstrap;
pub use bootstrap::Bootstrap;
```

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/bootstrap.rs prax-migrate/src/lib.rs
git commit -m "feat(migrate): add bootstrap module for V1 to V2 migration"
```

---

## Task 14: Add dev() Method to MigrationEngine

**Files:**
- Modify: `prax-migrate/src/engine.rs`

- [ ] **Step 1: Write test for dev workflow (using in-memory store)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_store::InMemoryEventStore;
    use crate::event::EventType;
    
    #[tokio::test]
    async fn test_migration_engine_dev() {
        let event_store = InMemoryEventStore::new();
        let config = MigrationConfig::default();
        let engine = MigrationEngine::new(config, event_store.clone());
        
        // This is a simplified test - full test needs schema parsing
        // For now, just verify the structure compiles
    }
}
```

- [ ] **Step 2: Update MigrationEngine to be generic over event store**

```rust
// In engine.rs, update the MigrationEngine struct:

/// Migration engine with event sourcing.
pub struct MigrationEngine<S: MigrationEventStore> {
    config: MigrationConfig,
    event_store: S,
    file_manager: MigrationFileManager,
    sql_generator: PostgresSqlGenerator,
    resolutions: ResolutionConfig,
}

impl<S: MigrationEventStore> MigrationEngine<S> {
    /// Create a new migration engine.
    pub fn new(config: MigrationConfig, event_store: S) -> Self {
        let file_manager = MigrationFileManager::new(&config.migrations_dir);
        Self {
            config,
            event_store,
            file_manager,
            sql_generator: PostgresSqlGenerator,
            resolutions: ResolutionConfig::new(),
        }
    }
    
    // ... existing methods ...
}
```

- [ ] **Step 3: Add DevResult type**

```rust
/// Result of a dev workflow operation.
#[derive(Debug)]
pub struct DevResult {
    /// Migration ID that was created.
    pub migration_id: String,
    /// Path to the migration directory.
    pub migration_path: PathBuf,
    /// Event ID of the apply event.
    pub event_id: i64,
    /// Duration in milliseconds.
    pub duration_ms: i64,
    /// Warnings about data loss.
    pub warnings: Vec<String>,
}
```

- [ ] **Step 4: Implement dev() method stub**

```rust
impl<S: MigrationEventStore> MigrationEngine<S> {
    /// Execute the dev workflow: generate and apply migration.
    pub async fn dev(
        &self,
        name: &str,
        schema: &prax_schema::Schema,
        applied_by: Option<String>,
    ) -> MigrateResult<DevResult> {
        // 1. Generate migration ID
        let migration_id = self.file_manager.generate_id();
        
        // 2. TODO: Introspect database to get current schema
        // 3. TODO: Generate diff using SchemaDiffer
        // 4. TODO: Generate SQL using sql_generator
        // 5. TODO: Create migration file
        // 6. TODO: Apply migration
        // 7. TODO: Record event
        
        // Placeholder for now
        Err(MigrationError::NoChanges)
    }
}
```

- [ ] **Step 5: Run full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 6: Commit**

```bash
git add prax-migrate/src/engine.rs
git commit -m "feat(migrate): add dev() method skeleton to MigrationEngine"
```

---

## Task 15: Add rollback() Method to MigrationEngine

**Files:**
- Modify: `prax-migrate/src/engine.rs`

- [ ] **Step 1: Add RollbackResult type**

```rust
/// Result of a rollback operation.
#[derive(Debug)]
pub struct RollbackResult {
    /// Migration ID that was rolled back.
    pub migration_id: String,
    /// Event ID of the rollback event.
    pub event_id: i64,
    /// Duration in milliseconds.
    pub duration_ms: i64,
}
```

- [ ] **Step 2: Write test for rollback**

```rust
#[tokio::test]
async fn test_migration_engine_rollback() {
    let event_store = InMemoryEventStore::new();
    
    // Add an apply event first
    event_store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
    ).await.unwrap();
    
    let config = MigrationConfig::default();
    let engine = MigrationEngine::new(config, event_store.clone());
    
    // This test verifies the structure - full implementation needs DB
}
```

- [ ] **Step 3: Implement rollback() method stub**

```rust
impl<S: MigrationEventStore> MigrationEngine<S> {
    /// Rollback the last applied migration.
    pub async fn rollback(
        &self,
        reason: Option<String>,
        rolled_back_by: Option<String>,
    ) -> MigrateResult<RollbackResult> {
        // 1. Get current state from events
        let events = self.event_store.get_all_events().await?;
        let state = MigrationState::from_events(events.clone());
        
        // 2. Find last applied migration
        let last_applied = state
            .get_applied()
            .iter()
            .max_by_key(|s| s.last_applied_at)
            .ok_or(MigrationError::NoMigrationsToRollback)?;
        
        // 3. Find parent apply event
        let parent_event = events
            .iter()
            .filter(|e| e.migration_id == last_applied.migration_id 
                     && e.event_type == EventType::Applied)
            .max_by_key(|e| e.created_at)
            .ok_or(MigrationError::ParentEventNotFound)?;
        
        // 4. TODO: Load migration file and check for down.sql
        // 5. TODO: Execute down SQL
        // 6. TODO: Record rollback event
        
        // Placeholder
        Err(MigrationError::NoDownMigration(last_applied.migration_id.clone()))
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --package prax-migrate rollback`
Expected: PASS (tests compile)

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/engine.rs
git commit -m "feat(migrate): add rollback() method skeleton to MigrationEngine"
```

---

## Task 16: Add get_migration_history() Method

**Files:**
- Modify: `prax-migrate/src/engine.rs`

- [ ] **Step 1: Add MigrationHistory type**

```rust
/// Complete history for a single migration.
#[derive(Debug)]
pub struct MigrationHistory {
    /// Migration ID.
    pub migration_id: String,
    /// All apply events.
    pub applies: Vec<MigrationEvent>,
    /// All rollback events.
    pub rollbacks: Vec<MigrationEvent>,
    /// All failure events.
    pub failures: Vec<MigrationEvent>,
}
```

- [ ] **Step 2: Write test for history query**

```rust
#[tokio::test]
async fn test_get_migration_history() {
    let event_store = InMemoryEventStore::new();
    
    // Add apply event
    event_store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
    ).await.unwrap();
    
    // Add rollback event
    event_store.append_event(
        "20260425120000",
        EventType::RolledBack,
        EventData::RolledBack {
            checksum: "abc123".to_string(),
            duration_ms: 89,
            rolled_back_by: None,
            reason: Some("Testing".to_string()),
            parent_event_id: 1,
            down_sql_preview: None,
        },
    ).await.unwrap();
    
    let config = MigrationConfig::default();
    let engine = MigrationEngine::new(config, event_store);
    
    let history = engine.get_migration_history("20260425120000").await.unwrap();
    
    assert_eq!(history.applies.len(), 1);
    assert_eq!(history.rollbacks.len(), 1);
    assert_eq!(history.failures.len(), 0);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --package prax-migrate test_get_migration_history`
Expected: FAIL - method doesn't exist

- [ ] **Step 4: Implement get_migration_history() method**

```rust
impl<S: MigrationEventStore> MigrationEngine<S> {
    /// Get complete history for a migration.
    pub async fn get_migration_history(
        &self,
        migration_id: &str,
    ) -> MigrateResult<MigrationHistory> {
        let events = self.event_store.get_events(migration_id).await?;
        
        let mut applies = Vec::new();
        let mut rollbacks = Vec::new();
        let mut failures = Vec::new();
        
        for event in events {
            match event.event_type {
                EventType::Applied => applies.push(event),
                EventType::RolledBack => rollbacks.push(event),
                EventType::Failed => failures.push(event),
                EventType::Resolved => {}
            }
        }
        
        Ok(MigrationHistory {
            migration_id: migration_id.to_string(),
            applies,
            rollbacks,
            failures,
        })
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --package prax-migrate test_get_migration_history`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add prax-migrate/src/engine.rs
git commit -m "feat(migrate): add get_migration_history() method"
```

---

## Task 17: Update CLI Migrate Command Structure

**Files:**
- Modify: `prax-cli/src/commands/migrate.rs`

- [ ] **Step 1: Check current migrate command structure**

Read: `prax-cli/src/commands/migrate.rs` to understand current structure

- [ ] **Step 2: Add rollback subcommand with reason/user flags**

```rust
// Add to the migrate command enum:

#[derive(Subcommand)]
pub enum MigrateCommand {
    // ... existing commands ...
    
    /// Rollback the last applied migration
    Rollback {
        /// Reason for rolling back
        #[arg(long)]
        reason: Option<String>,
        
        /// User performing the rollback
        #[arg(long)]
        user: Option<String>,
        
        /// Rollback to a specific migration ID
        #[arg(long)]
        to: Option<String>,
    },
    
    /// View migration history
    History {
        /// Specific migration ID to view
        #[arg(long)]
        migration: Option<String>,
    },
}
```

- [ ] **Step 3: Add handler stub for rollback command**

```rust
async fn handle_rollback(
    reason: Option<String>,
    user: Option<String>,
    to: Option<String>,
) -> Result<()> {
    println!("🔄 Rolling back migration...");
    
    // TODO: Load engine and call rollback
    // TODO: Format output with warnings
    
    Ok(())
}
```

- [ ] **Step 4: Add handler stub for history command**

```rust
async fn handle_history(migration: Option<String>) -> Result<()> {
    println!("📜 Migration History");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    // TODO: Load engine and get history
    // TODO: Format output with timeline
    
    Ok(())
}
```

- [ ] **Step 5: Run CLI to verify it compiles**

Run: `cargo build --package prax-cli`
Expected: SUCCESS

- [ ] **Step 6: Commit**

```bash
git add prax-cli/src/commands/migrate.rs
git commit -m "feat(cli): add rollback and history subcommands"
```

---

## Task 18: Add Integration Test for Full Event Sourcing Workflow

**Files:**
- Create: `prax-migrate/tests/event_sourcing_workflow.rs`

- [ ] **Step 1: Create integration test file**

```rust
//! Integration tests for event sourcing workflow.

use prax_migrate::*;
use prax_migrate::event_store::InMemoryEventStore;
use prax_migrate::event::{EventType, EventData};

#[tokio::test]
async fn test_full_event_sourcing_workflow() {
    // Create in-memory event store
    let event_store = InMemoryEventStore::new();
    
    // Initialize
    event_store.initialize().await.unwrap();
    
    // Record an apply event
    let event_id = event_store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: Some("test".to_string()),
            up_sql_preview: Some("CREATE TABLE users".to_string()),
            auto_generated: true,
        },
    ).await.unwrap();
    
    assert_eq!(event_id, 1);
    
    // Get all events and build state
    let events = event_store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);
    
    // Verify state
    assert!(state.is_applied("20260425120000"));
    let status = state.get_status("20260425120000").unwrap();
    assert_eq!(status.apply_count, 1);
    assert_eq!(status.checksum, "abc123");
    
    // Record a rollback event
    let rollback_id = event_store.append_event(
        "20260425120000",
        EventType::RolledBack,
        EventData::RolledBack {
            checksum: "abc123".to_string(),
            duration_ms: 89,
            rolled_back_by: Some("test".to_string()),
            reason: Some("Testing rollback".to_string()),
            parent_event_id: event_id,
            down_sql_preview: Some("DROP TABLE users".to_string()),
        },
    ).await.unwrap();
    
    assert_eq!(rollback_id, 2);
    
    // Rebuild state
    let events = event_store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);
    
    // Verify rollback
    assert!(!state.is_applied("20260425120000"));
    let status = state.get_status("20260425120000").unwrap();
    assert_eq!(status.rollback_count, 1);
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test event_sourcing_workflow`
Expected: PASS

- [ ] **Step 3: Add test for multiple migrations**

```rust
#[tokio::test]
async fn test_multiple_migrations_state() {
    let event_store = InMemoryEventStore::new();
    
    // Apply first migration
    event_store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123".to_string(),
            duration_ms: 150,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
    ).await.unwrap();
    
    // Apply second migration
    event_store.append_event(
        "20260425130000",
        EventType::Applied,
        EventData::Applied {
            checksum: "def456".to_string(),
            duration_ms: 200,
            applied_by: None,
            up_sql_preview: None,
            auto_generated: true,
        },
    ).await.unwrap();
    
    // Rollback second migration
    event_store.append_event(
        "20260425130000",
        EventType::RolledBack,
        EventData::RolledBack {
            checksum: "def456".to_string(),
            duration_ms: 100,
            rolled_back_by: None,
            reason: None,
            parent_event_id: 2,
            down_sql_preview: None,
        },
    ).await.unwrap();
    
    // Build state
    let events = event_store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);
    
    // First should still be applied
    assert!(state.is_applied("20260425120000"));
    // Second should be rolled back
    assert!(!state.is_applied("20260425130000"));
    
    let applied = state.get_applied();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].migration_id, "20260425120000");
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --test event_sourcing_workflow test_multiple_migrations_state`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/tests/event_sourcing_workflow.rs
git commit -m "test(migrate): add integration tests for event sourcing workflow"
```

---

## Task 19: Add Property-Based Tests for State Projection

**Files:**
- Create: `prax-migrate/tests/property_tests.rs`

- [ ] **Step 1: Add proptest dependency**

```toml
# Add to prax-migrate/Cargo.toml under [dev-dependencies]
proptest = "1.4"
```

- [ ] **Step 2: Create property test file**

```rust
//! Property-based tests for event replay invariants.

use proptest::prelude::*;
use prax_migrate::*;
use prax_migrate::event::{EventType, EventData, MigrationEvent};
use chrono::Utc;

/// Generate arbitrary migration events.
fn arb_event(migration_id: String, event_id: i64) -> impl Strategy<Value = MigrationEvent> {
    prop_oneof![
        Just(MigrationEvent {
            event_id,
            migration_id: migration_id.clone(),
            event_type: EventType::Applied,
            event_data: EventData::Applied {
                checksum: "abc123".to_string(),
                duration_ms: 150,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
            created_at: Utc::now(),
        }),
        Just(MigrationEvent {
            event_id,
            migration_id: migration_id.clone(),
            event_type: EventType::RolledBack,
            event_data: EventData::RolledBack {
                checksum: "abc123".to_string(),
                duration_ms: 89,
                rolled_back_by: None,
                reason: None,
                parent_event_id: 1,
                down_sql_preview: None,
            },
            created_at: Utc::now(),
        }),
    ]
}

proptest! {
    #[test]
    fn test_event_replay_is_deterministic(event_count in 1..20usize) {
        let migration_id = "20260425120000".to_string();
        let mut events = Vec::new();
        
        for i in 0..event_count {
            events.push(MigrationEvent {
                event_id: i as i64 + 1,
                migration_id: migration_id.clone(),
                event_type: if i % 2 == 0 { EventType::Applied } else { EventType::RolledBack },
                event_data: if i % 2 == 0 {
                    EventData::Applied {
                        checksum: "abc123".to_string(),
                        duration_ms: 150,
                        applied_by: None,
                        up_sql_preview: None,
                        auto_generated: true,
                    }
                } else {
                    EventData::RolledBack {
                        checksum: "abc123".to_string(),
                        duration_ms: 89,
                        rolled_back_by: None,
                        reason: None,
                        parent_event_id: i as i64,
                        down_sql_preview: None,
                    }
                },
                created_at: Utc::now(),
            });
        }
        
        // Replay twice
        let state1 = MigrationState::from_events(events.clone());
        let state2 = MigrationState::from_events(events.clone());
        
        // Should produce same result
        assert_eq!(
            state1.is_applied(&migration_id),
            state2.is_applied(&migration_id)
        );
    }
    
    #[test]
    fn test_apply_then_rollback_is_not_applied(cycle_count in 1..10usize) {
        let migration_id = "20260425120000".to_string();
        let mut events = Vec::new();
        let mut event_id = 1;
        
        for _ in 0..cycle_count {
            // Apply
            events.push(MigrationEvent {
                event_id,
                migration_id: migration_id.clone(),
                event_type: EventType::Applied,
                event_data: EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
                created_at: Utc::now(),
            });
            event_id += 1;
            
            // Rollback
            events.push(MigrationEvent {
                event_id,
                migration_id: migration_id.clone(),
                event_type: EventType::RolledBack,
                event_data: EventData::RolledBack {
                    checksum: "abc123".to_string(),
                    duration_ms: 89,
                    rolled_back_by: None,
                    reason: None,
                    parent_event_id: event_id - 1,
                    down_sql_preview: None,
                },
                created_at: Utc::now(),
            });
            event_id += 1;
        }
        
        let state = MigrationState::from_events(events);
        
        // After all cycles, should NOT be applied
        assert!(!state.is_applied(&migration_id));
        
        let status = state.get_status(&migration_id).unwrap();
        assert_eq!(status.apply_count as usize, cycle_count);
        assert_eq!(status.rollback_count as usize, cycle_count);
    }
}
```

- [ ] **Step 3: Run property tests**

Run: `cargo test --test property_tests`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/Cargo.toml prax-migrate/tests/property_tests.rs
git commit -m "test(migrate): add property-based tests for state projection"
```

---

## Task 20: Add End-to-End Documentation Example

**Files:**
- Create: `prax-migrate/examples/event_sourcing_example.rs`

- [ ] **Step 1: Create example file**

```rust
//! Example demonstrating the event sourcing migration system.

use prax_migrate::*;
use prax_migrate::event_store::InMemoryEventStore;
use prax_migrate::event::{EventType, EventData};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Event Sourcing Migration System Example");
    println!("========================================\n");
    
    // Create an in-memory event store for this example
    let event_store = InMemoryEventStore::new();
    
    // Initialize the event log
    event_store.initialize().await?;
    println!("✓ Event store initialized\n");
    
    // Simulate applying a migration
    println!("Applying migration: 20260425120000_create_users");
    let apply_event_id = event_store.append_event(
        "20260425120000",
        EventType::Applied,
        EventData::Applied {
            checksum: "abc123def456".to_string(),
            duration_ms: 150,
            applied_by: Some("developer".to_string()),
            up_sql_preview: Some("CREATE TABLE users (id INT PRIMARY KEY, email VARCHAR(255))".to_string()),
            auto_generated: true,
        },
    ).await?;
    println!("  ✓ Applied in 150ms (event_id: {})\n", apply_event_id);
    
    // Check current state
    let events = event_store.get_all_events().await?;
    let state = MigrationState::from_events(events.clone());
    
    println!("Current migration state:");
    for status in state.get_applied() {
        println!("  • {} - applied {} time(s)", status.migration_id, status.apply_count);
    }
    println!();
    
    // Simulate rolling back the migration
    println!("Rolling back migration: 20260425120000_create_users");
    let rollback_event_id = event_store.append_event(
        "20260425120000",
        EventType::RolledBack,
        EventData::RolledBack {
            checksum: "abc123def456".to_string(),
            duration_ms: 89,
            rolled_back_by: Some("developer".to_string()),
            reason: Some("Testing schema changes".to_string()),
            parent_event_id: apply_event_id,
            down_sql_preview: Some("DROP TABLE users".to_string()),
        },
    ).await?;
    println!("  ✓ Rolled back in 89ms (event_id: {})", rollback_event_id);
    println!("  Reason: Testing schema changes\n");
    
    // Rebuild state
    let events = event_store.get_all_events().await?;
    let state = MigrationState::from_events(events.clone());
    
    println!("Migration state after rollback:");
    if state.is_applied("20260425120000") {
        println!("  Migration is APPLIED");
    } else {
        println!("  Migration is NOT APPLIED");
    }
    
    if let Some(status) = state.get_status("20260425120000") {
        println!("  Apply count: {}", status.apply_count);
        println!("  Rollback count: {}", status.rollback_count);
    }
    println!();
    
    // Show event history
    println!("Complete event history for 20260425120000:");
    let migration_events = event_store.get_events("20260425120000").await?;
    for event in migration_events {
        match event.event_type {
            EventType::Applied => {
                if let EventData::Applied { duration_ms, applied_by, .. } = event.event_data {
                    println!("  ✓ Applied at {} ({}ms) by {:?}",
                        event.created_at.format("%Y-%m-%d %H:%M:%S"),
                        duration_ms,
                        applied_by.unwrap_or_else(|| "system".to_string())
                    );
                }
            }
            EventType::RolledBack => {
                if let EventData::RolledBack { duration_ms, reason, .. } = event.event_data {
                    println!("  ✗ Rolled back at {} ({}ms)",
                        event.created_at.format("%Y-%m-%d %H:%M:%S"),
                        duration_ms
                    );
                    if let Some(r) = reason {
                        println!("    Reason: {}", r);
                    }
                }
            }
            _ => {}
        }
    }
    
    Ok(())
}
```

- [ ] **Step 2: Run example to verify it works**

Run: `cargo run --package prax-migrate --example event_sourcing_example`
Expected: Output showing event sourcing workflow

- [ ] **Step 3: Commit**

```bash
git add prax-migrate/examples/event_sourcing_example.rs
git commit -m "docs(migrate): add event sourcing workflow example"
```

---

## Task 21: Final Integration and Documentation

**Files:**
- Modify: `prax-migrate/README.md` (if exists)
- Create: `MIGRATION_GUIDE.md` in docs

- [ ] **Step 1: Create migration guide for users**

```markdown
# Migration Guide: V1 to Event Sourcing (V2)

## Overview

Prax ORM v0.7.0 introduces a new event sourcing architecture for migrations. This guide helps you migrate from the V1 system.

## What Changed

### V1 System (Old)
- Single table `_prax_migrations` with columns: `id`, `checksum`, `applied_at`, `duration_ms`, `rolled_back` (boolean)
- Rollbacks marked with boolean flag
- Limited audit trail

### V2 System (New)
- Event log table `_prax_migrations` with columns: `event_id`, `migration_id`, `event_type`, `event_data` (JSONB), `created_at`
- Complete audit trail of all operations (apply, rollback, failure, resolution)
- Rich metadata in JSONB (user, reason, SQL preview)
- Support for multiple apply/rollback cycles

## Migration Process

### Automatic Migration

When you first run `prax migrate dev` or `prax migrate apply` with v0.7.0+, the system will:

1. Detect the V1 format
2. Prompt for confirmation
3. Backup existing table to `_prax_migrations_v1_backup`
4. Create new event log table
5. Migrate existing records as events
6. Verify migration success

### Manual Migration

If you prefer to migrate manually:

```bash
# 1. Backup your database first!
pg_dump -t _prax_migrations mydb > migrations_backup.sql

# 2. Run the bootstrap command
prax migrate bootstrap

# 3. Verify the migration
prax migrate status
```

## New Features

### Rollback with Reason

```bash
prax migrate rollback --reason "Reverting incompatible schema change"
```

### View Migration History

```bash
# All migrations
prax migrate history

# Specific migration
prax migrate history --migration 20260425120000_add_users
```

### Data Loss Warnings

All SQL generators now produce warnings for lossy operations:
- Dropping tables
- Dropping columns
- Changing column types

## Breaking Changes

None! The V1 format is automatically migrated to V2.

## Rollback Plan

If you need to rollback to V1 format:

```sql
-- Drop V2 table
DROP TABLE "_prax_migrations";

-- Restore V1 table
ALTER TABLE "_prax_migrations_v1_backup" RENAME TO "_prax_migrations";
```

Then downgrade to Prax ORM v0.6.x.
```

- [ ] **Step 2: Run final full test suite**

Run: `cargo test --package prax-migrate`
Expected: All tests PASS

- [ ] **Step 3: Run all integration tests**

Run: `cargo test --package prax-migrate --tests`
Expected: All tests PASS

- [ ] **Step 4: Final commit**

```bash
git add docs/MIGRATION_GUIDE.md
git commit -m "docs(migrate): add V1 to V2 migration guide"
```

---

## Plan Completion Checklist

**Spec Coverage Final Check:**
- ✅ Event types and data structures (Tasks 6-7)
- ✅ MigrationSql warnings field (Tasks 1-5)
- ✅ Event store trait and in-memory implementation (Tasks 9-10, 12)
- ✅ State projection logic (Task 7)
- ✅ Bootstrap migration infrastructure (Task 13)
- ✅ Engine refactor with dev(), rollback(), history() (Tasks 14-16)
- ✅ CLI updates for rollback and history (Task 17)
- ✅ Integration tests (Tasks 18-19)
- ✅ Documentation and examples (Tasks 20-21)

**Implementation Status:**
- Phase 1 (Core Event System): ✅ Complete (Tasks 6-10)
- Phase 2 (SQL Warnings): ✅ Complete (Tasks 1-5)
- Phase 3 (Engine Methods): ✅ Skeleton complete (Tasks 14-16)
- Phase 4 (Bootstrap): ✅ Infrastructure complete (Task 13)
- Phase 5 (CLI): ✅ Stubs complete (Task 17)
- Phase 6 (Testing): ✅ Complete (Tasks 18-19)

**Remaining Work:**

The skeleton implementation is complete. The following work remains for full functionality:

1. **PostgreSQL Event Store Implementation** - Implement `MigrationEventStore` for PostgreSQL with actual database operations
2. **Complete dev() Method** - Add schema introspection and diff generation
3. **Complete rollback() Method** - Add SQL execution logic
4. **CLI Handler Implementation** - Wire up CLI commands to engine methods
5. **Database Integration Tests** - Tests with actual PostgreSQL database

These can be implemented incrementally using the TDD approach established in this plan.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-25-event-sourced-migrations.md`.

**Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration. Use the `superpowers:subagent-driven-development` skill.

**2. Inline Execution** - Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints for review.

**Which approach?**
