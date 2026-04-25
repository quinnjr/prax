# Event Sourced Migration System Design

**Date:** 2026-04-25  
**Status:** Approved  
**Type:** Architecture Redesign

## Overview

This document specifies the redesign of the Prax ORM migration system to use event sourcing architecture with automatic SQL generation from schema diffs. The new system provides complete audit trails, reversible migrations with data loss warnings, and rich rollback event logging.

## Goals

1. **Auto-generate migrations** from Prax schema changes (up.sql and down.sql)
2. **Event sourcing architecture** for complete audit trail
3. **Enhanced rollback logging** with reason, user, parent event tracking
4. **Backwards compatibility** with existing installations
5. **Maintain familiar CLI** workflow

## Non-Goals

- Real-time schema synchronization (still requires explicit migration commands)
- Support for non-SQL databases in initial implementation
- Automatic conflict resolution (requires manual resolution events)

## Architecture

### Event Sourcing Model

The migration system uses event sourcing as its foundation. All operations (apply, rollback, failure, resolution) are represented as immutable events appended to an event log. Current migration state is derived by replaying the event log.

**Benefits:**
- Complete audit trail of all migration operations
- Support for multiple apply/rollback cycles per migration
- Rich metadata capture (duration, user, reason, SQL preview)
- Natural fit for distributed systems (append-only, no updates)

**Trade-offs:**
- Event log grows continuously (one row per operation)
- Current state queries require event replay or materialized view
- More complex than simple "mark as rolled back" boolean

### Database Schema

#### Event Log Table

```sql
CREATE TABLE "_prax_migrations" (
    event_id BIGSERIAL PRIMARY KEY,
    migration_id VARCHAR(255) NOT NULL,
    event_type VARCHAR(50) NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    
    CONSTRAINT valid_event_type CHECK (
        event_type IN ('applied', 'rolled_back', 'failed', 'resolved')
    )
);

-- Indexes for common queries
CREATE INDEX idx_migrations_migration_id 
    ON "_prax_migrations" (migration_id, created_at DESC);

CREATE INDEX idx_migrations_event_type 
    ON "_prax_migrations" (event_type);

CREATE INDEX idx_migrations_created_at 
    ON "_prax_migrations" (created_at DESC);
```

#### Event Data Structures

Event metadata is stored in the `event_data` JSONB column with type-specific schemas:

**Applied Event:**
```json
{
  "checksum": "abc123...",
  "duration_ms": 150,
  "applied_by": "user@system",
  "up_sql_preview": "CREATE TABLE users...",
  "auto_generated": true
}
```

**Rolled Back Event:**
```json
{
  "checksum": "abc123...",
  "duration_ms": 89,
  "rolled_back_by": "user@system",
  "reason": "Reverting problematic schema change",
  "parent_event_id": 42,
  "down_sql_preview": "DROP TABLE users..."
}
```

**Failed Event:**
```json
{
  "error": "column 'email' already exists",
  "attempted_by": "user@system",
  "sql_preview": "ALTER TABLE..."
}
```

**Resolved Event:**
```json
{
  "resolution_type": "checksum_accepted",
  "old_checksum": "abc...",
  "new_checksum": "xyz...",
  "reason": "Fixed column type",
  "resolved_by": "user@system"
}
```

#### Materialized View (Optional)

For query optimization, provide a materialized view:

```sql
CREATE MATERIALIZED VIEW "_prax_migrations_current_state" AS
SELECT DISTINCT ON (migration_id)
    migration_id,
    event_type,
    event_data->>'checksum' as checksum,
    created_at
FROM "_prax_migrations"
WHERE event_type IN ('applied', 'rolled_back')
ORDER BY migration_id, created_at DESC;

CREATE UNIQUE INDEX idx_current_state_migration_id 
    ON "_prax_migrations_current_state" (migration_id);
```

Refresh after each migration operation:
```sql
REFRESH MATERIALIZED VIEW CONCURRENTLY "_prax_migrations_current_state";
```

### Auto-Generation System

#### Reverse SQL Generation

The system generates down.sql by analyzing the schema diff and producing reverse operations:

| Forward Operation | Reverse Operation | Data Loss? |
|------------------|------------------|------------|
| `CREATE TABLE` | `DROP TABLE` | Yes - all data |
| `DROP TABLE` | `CREATE TABLE` | Yes - unrecoverable |
| `ADD COLUMN` | `DROP COLUMN` | Yes - column data |
| `DROP COLUMN` | `ADD COLUMN` (nullable/default) | Yes - original values lost |
| `ALTER COLUMN TYPE` | `ALTER COLUMN` back | Maybe - depends on types |
| `CREATE INDEX` | `DROP INDEX` | No - safe reversal |
| `ADD CONSTRAINT` | `DROP CONSTRAINT` | No - safe reversal |
| `RENAME TABLE` | `RENAME TABLE` back | No - safe reversal |
| `RENAME COLUMN` | `RENAME COLUMN` back | No - safe reversal |

**Reverse Generation Rules:**

1. **Structural reversal always generated** - even if lossy
2. **Warning comments added** to down.sql for lossy operations
3. **Warnings array returned** to CLI for display to user
4. **Operations reversed in opposite order** (undo last changes first)

**Example Generated down.sql:**

```sql
-- AUTO-GENERATED by Prax ORM
-- Migration: 20260425120000_add_user_roles
-- Generated: 2026-04-25 12:00:00 UTC
--
-- WARNING: This migration drops the 'legacy_users' table.
-- Data loss: All rows in 'legacy_users' will be deleted.
-- The down.sql can recreate the structure but NOT the data.

DROP TABLE IF EXISTS "user_roles";

-- WARNING: Dropping column 'role_id' from 'users'
-- Original data in this column cannot be recovered
ALTER TABLE "users" DROP COLUMN IF EXISTS "role_id";

-- WARNING: Recreating 'legacy_users' - original data is lost
CREATE TABLE "legacy_users" (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255)
);
```

#### Dev Workflow: `prax migrate dev --name <name>`

The primary workflow auto-generates and applies migrations:

1. **Parse schema** - Load `schema.prax` and parse into AST
2. **Introspect database** - Query current database schema
3. **Generate diff** - Use `SchemaDiffer` to compute changes
4. **Generate forward SQL** - Use `PostgresSqlGenerator` for up.sql
5. **Generate reverse SQL** - Use `PostgresReverseSqlGenerator` for down.sql
6. **Create migration directory** - `migrations/YYYYMMDDHHMMSS_<name>/`
7. **Write up.sql** - With auto-generation header and warnings
8. **Write down.sql** - With data loss warnings
9. **Apply immediately** - Execute up.sql in transaction
10. **Record event** - Append 'applied' event with full metadata

**CLI Output:**

```bash
$ prax migrate dev --name add_user_roles

📝 Analyzing schema changes...
   ├─ Create table: user_roles
   ├─ Add column: users.role_id
   └─ Drop table: legacy_users

⚠️  Warnings:
   • Dropping table 'legacy_users' - data will be lost
   • Down migration can recreate structure but not data

✓  Generated: migrations/20260425120000_add_user_roles/
   ├─ up.sql (847 bytes)
   └─ down.sql (1.2 KB, 2 warnings)

🚀 Applying migration...
✓  Applied in 142ms (event_id: 1523)
```

### Event Processing

#### Event Store Repository

```rust
#[async_trait::async_trait]
pub trait MigrationEventStore: Send + Sync {
    /// Append a new event to the log
    async fn append_event(
        &self,
        migration_id: &str,
        event_type: EventType,
        event_data: EventData,
    ) -> MigrateResult<i64>;
    
    /// Get all events for a migration
    async fn get_events(&self, migration_id: &str) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Get all events (for replaying full history)
    async fn get_all_events(&self) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Query events by type
    async fn get_events_by_type(&self, event_type: EventType) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Get events in time range
    async fn get_events_since(&self, since: DateTime<Utc>) -> MigrateResult<Vec<MigrationEvent>>;
    
    /// Initialize the event log table
    async fn initialize(&self) -> MigrateResult<()>;
    
    /// Acquire exclusive lock for migrations
    async fn acquire_lock(&self) -> MigrateResult<MigrationLock>;
}
```

#### Event Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    Applied,
    RolledBack,
    Failed,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationEvent {
    pub event_id: i64,
    pub migration_id: String,
    pub event_type: EventType,
    pub event_data: EventData,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventData {
    Applied {
        checksum: String,
        duration_ms: i64,
        applied_by: Option<String>,
        up_sql_preview: Option<String>,
        auto_generated: bool,
    },
    RolledBack {
        checksum: String,
        duration_ms: i64,
        rolled_back_by: Option<String>,
        reason: Option<String>,
        parent_event_id: i64,
        down_sql_preview: Option<String>,
    },
    Failed {
        error: String,
        attempted_by: Option<String>,
        sql_preview: Option<String>,
    },
    Resolved {
        resolution_type: String,
        old_checksum: Option<String>,
        new_checksum: Option<String>,
        reason: String,
        resolved_by: Option<String>,
    },
}
```

#### State Projection

Current migration state is derived by replaying events:

```rust
pub struct MigrationState {
    applied_migrations: HashMap<String, MigrationStatus>,
}

#[derive(Debug, Clone)]
pub struct MigrationStatus {
    pub migration_id: String,
    pub checksum: String,
    pub is_applied: bool,
    pub last_applied_at: Option<DateTime<Utc>>,
    pub last_rolled_back_at: Option<DateTime<Utc>>,
    pub apply_count: u32,
    pub rollback_count: u32,
}

impl MigrationState {
    /// Build current state by replaying all events
    pub fn from_events(events: Vec<MigrationEvent>) -> Self {
        let mut state = HashMap::new();
        
        for event in events {
            let status = state.entry(event.migration_id.clone())
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
                    if let EventData::Resolved { new_checksum: Some(checksum), .. } = event.event_data {
                        status.checksum = checksum;
                    }
                }
            }
        }
        
        Self { applied_migrations: state }
    }
    
    /// Get currently applied migrations
    pub fn get_applied(&self) -> Vec<&MigrationStatus> {
        self.applied_migrations
            .values()
            .filter(|s| s.is_applied)
            .collect()
    }
    
    /// Check if a migration is currently applied
    pub fn is_applied(&self, migration_id: &str) -> bool {
        self.applied_migrations
            .get(migration_id)
            .map(|s| s.is_applied)
            .unwrap_or(false)
    }
}
```

**Why this approach:** State derivation ensures consistency. Even if code logic changes, replaying the same events always produces the same state. This makes the system easier to test and reason about.

### Rollback System

#### Enhanced Rollback API

```rust
impl<S: MigrationEventStore> MigrationEngine<S> {
    /// Rollback the last applied migration
    pub async fn rollback(
        &self,
        reason: Option<String>,
        rolled_back_by: Option<String>,
    ) -> MigrateResult<RollbackResult>;
    
    /// Rollback multiple migrations to a target
    pub async fn rollback_to(
        &self,
        target_migration_id: &str,
        reason: Option<String>,
        rolled_back_by: Option<String>,
    ) -> MigrateResult<Vec<RollbackResult>>;
    
    /// Get complete history for a migration
    pub async fn get_migration_history(
        &self,
        migration_id: &str,
    ) -> MigrateResult<MigrationHistory>;
}

#[derive(Debug)]
pub struct RollbackResult {
    pub migration_id: String,
    pub event_id: i64,
    pub duration_ms: i64,
}

#[derive(Debug)]
pub struct MigrationHistory {
    pub migration_id: String,
    pub applies: Vec<MigrationEvent>,
    pub rollbacks: Vec<MigrationEvent>,
    pub failures: Vec<MigrationEvent>,
}
```

#### Rollback Workflow

1. **Acquire lock** - Prevent concurrent migrations
2. **Get current state** - Replay events to find applied migrations
3. **Find last applied** - Identify most recent apply event
4. **Load migration file** - Read down.sql from disk
5. **Validate reversibility** - Check down.sql is not empty
6. **Execute down SQL** - Run in transaction
7. **Record rollback event** - With reason, user, parent_event_id, duration

**CLI Interface:**

```bash
# Rollback last migration
prax migrate rollback

# Rollback with reason
prax migrate rollback --reason "Reverting broken column type"

# Rollback to specific migration
prax migrate rollback --to 20260420120000_add_users

# View rollback history
prax migrate history --migration 20260425120000_add_roles
```

**CLI Output:**

```bash
$ prax migrate rollback --reason "Testing schema changes"

🔄 Rolling back last migration...
   Migration: 20260425120000_add_user_roles
   Applied: 2026-04-25 12:00:15 (142ms ago)

⚠️  Warnings:
   • Dropping table 'user_roles' - data will be lost
   • Dropping column 'users.role_id' - data cannot be recovered

✓  Rolled back in 89ms (event_id: 1524)
```

#### Migration History Query

```bash
$ prax migrate history --migration 20260425120000_add_user_roles

Migration: 20260425120000_add_user_roles
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  ✓ Applied      2026-04-25 12:00:15  (150ms)  by: user@system
  ✗ Rolled Back  2026-04-25 14:30:22  (89ms)   by: user@system
    Reason: Column type incompatible with existing data
  ✓ Applied      2026-04-25 15:45:10  (142ms)  by: user@system
  
Total: 2 applies, 1 rollback
Current state: APPLIED
```

### Error Handling

#### Failed Migration Events

When a migration fails during execution:

1. **Transaction rolled back** - Database left in clean state
2. **Failed event recorded** - Error message, attempted_by, SQL preview
3. **User notified** - Clear error message with recovery suggestions

```rust
async fn apply_migration_safe(
    &self,
    migration: &MigrationFile,
    applied_by: Option<String>,
) -> MigrateResult<i64> {
    let start = Instant::now();
    
    match self.execute_sql(&migration.up_sql).await {
        Ok(_) => {
            let duration_ms = start.elapsed().as_millis() as i64;
            self.event_store.append_event(
                &migration.id,
                EventType::Applied,
                EventData::Applied {
                    checksum: migration.checksum.clone(),
                    duration_ms,
                    applied_by,
                    up_sql_preview: Some(preview_sql(&migration.up_sql, 200)),
                    auto_generated: true,
                },
            ).await
        }
        Err(e) => {
            // Record failure event
            self.event_store.append_event(
                &migration.id,
                EventType::Failed,
                EventData::Failed {
                    error: e.to_string(),
                    attempted_by: applied_by,
                    sql_preview: Some(preview_sql(&migration.up_sql, 200)),
                },
            ).await?;
            
            Err(e)
        }
    }
}
```

#### Transaction Handling

```rust
async fn execute_sql(&self, sql: &str) -> MigrateResult<()> {
    if self.config.use_transaction {
        self.db.execute("BEGIN").await?;
        
        match self.db.execute(sql).await {
            Ok(_) => {
                self.db.execute("COMMIT").await?;
                Ok(())
            }
            Err(e) => {
                self.db.execute("ROLLBACK").await?;
                Err(e)
            }
        }
    } else {
        self.db.execute(sql).await?;
        Ok(())
    }
}
```

#### Recovery Options

When a migration fails, users can:

1. **Fix schema and regenerate** - `prax migrate dev --name fix_user_roles`
2. **Manually edit SQL and retry** - Edit migration file, then `prax migrate apply`
3. **Skip the migration** - `prax migrate resolve skip <id> --reason "..."`
4. **View failure details** - `prax migrate history --migration <id>`

#### Checksum Resolution Events

When a migration file is modified after being applied:

```rust
pub async fn resolve_checksum(
    &self,
    migration_id: &str,
    old_checksum: &str,
    new_checksum: &str,
    reason: String,
    resolved_by: Option<String>,
) -> MigrateResult<i64> {
    let event_id = self.event_store.append_event(
        migration_id,
        EventType::Resolved,
        EventData::Resolved {
            resolution_type: "checksum_accepted".to_string(),
            old_checksum: Some(old_checksum.to_string()),
            new_checksum: Some(new_checksum.to_string()),
            reason,
            resolved_by,
        },
    ).await?;
    
    self.resolutions.add(Resolution::accept_checksum(
        migration_id,
        old_checksum,
        new_checksum,
        &reason,
    ));
    self.save_resolutions().await?;
    
    Ok(event_id)
}
```

### Backwards Compatibility

#### Migration from V1 System

The migration system includes a self-migration bootstrap:

```rust
pub async fn bootstrap_event_sourcing(&self) -> MigrateResult<()> {
    if self.is_event_sourcing_enabled().await? {
        return Ok(());
    }
    
    // 1. Rename old table
    self.db.execute(
        r#"ALTER TABLE "_prax_migrations" 
           RENAME TO "_prax_migrations_v1_backup""#
    ).await?;
    
    // 2. Create new event log table
    self.db.execute(CREATE_EVENT_LOG_TABLE_SQL).await?;
    
    // 3. Migrate existing records as events
    self.db.execute(r#"
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
    "#).await?;
    
    // 4. Verify migration
    let v1_count: i64 = self.db.query_one(
        r#"SELECT COUNT(*) FROM "_prax_migrations_v1_backup""#
    ).await?;
    
    let v2_count: i64 = self.db.query_one(
        r#"SELECT COUNT(*) FROM "_prax_migrations""#
    ).await?;
    
    if v1_count != v2_count {
        return Err(MigrationError::BootstrapFailed(
            "Event count mismatch after migration"
        ));
    }
    
    Ok(())
}
```

**Bootstrap Trigger:** CLI auto-detects V1 format and prompts:

```bash
$ prax migrate dev --name add_feature

ℹ️  Detected legacy migration table format
🔄 Migrating to event sourcing system...
✓  Migrated 15 migration records
✓  Backup saved to _prax_migrations_v1_backup
✓  Event sourcing enabled

📝 Creating migration: add_feature
...
```

#### Legacy Migration Files

Existing manual migrations:
- ✅ Continue to work normally
- ✅ Can be applied and rolled back
- ✅ Events recorded with `auto_generated: false`
- ⚠️ Down migrations may be incomplete or missing

Detection:
```rust
impl MigrationFile {
    pub fn is_auto_generated(&self) -> bool {
        self.up_sql.contains("-- AUTO-GENERATED by Prax ORM")
    }
}
```

#### Configuration Options

```rust
#[derive(Debug, Clone)]
pub struct MigrationConfig {
    pub migrations_dir: PathBuf,
    pub auto_generate: bool,   // Default: true
    pub auto_apply: bool,       // Default: true (for dev workflow)
    pub use_transaction: bool,  // Default: true
    // ... other fields
}
```

Users can opt-out of auto-generation:

```toml
# prax.toml
[migrate]
auto_generate = false  # Generate files but don't auto-apply
auto_apply = false     # Require manual review before applying
```

### File Structure

```
migrations/
├── 20260425120000_add_user_roles/
│   ├── up.sql              # Auto-generated forward migration
│   ├── down.sql            # Auto-generated reverse migration
│   └── migration.json      # Optional metadata
├── 20260425140000_add_posts/
│   ├── up.sql
│   ├── down.sql
│   └── migration.json
└── resolutions.toml        # Conflict resolutions
```

**Optional migration.json:**
```json
{
  "id": "20260425120000",
  "name": "add_user_roles",
  "created_at": "2026-04-25T12:00:00Z",
  "auto_generated": true,
  "schema_snapshot": {
    "models": ["User", "Role", "UserRole"],
    "changes": ["created UserRole table", "added role_id to User"]
  },
  "warnings": [
    "down.sql recreates structure but cannot restore data"
  ]
}
```

## Implementation Plan

### Phase 1: Core Event System

1. Define event types and data structures
2. Implement `MigrationEventStore` trait and PostgreSQL implementation
3. Implement `MigrationState` projection from events
4. Add event log table creation SQL
5. Write unit tests for event replay logic

### Phase 2: Reverse SQL Generation

1. Implement `ReverseSqlGenerator` trait
2. Implement `PostgresReverseSqlGenerator` with operation reversal logic
3. Add data loss detection and warning generation
4. Write unit tests for each operation type
5. Integration tests for complex schema changes

### Phase 3: Migration Engine Refactor

1. Update `MigrationEngine` to use `MigrationEventStore`
2. Implement `dev()` method with auto-generation pipeline
3. Implement enhanced `rollback()` with event logging
4. Add `rollback_to()` for multiple migrations
5. Add `get_migration_history()` query method

### Phase 4: Bootstrap & Migration

1. Implement `bootstrap_event_sourcing()` self-migration
2. Add V1 format detection logic
3. Create migration SQL from V1 to V2 format
4. Add CLI prompts for bootstrap process
5. Integration tests for bootstrap process

### Phase 5: CLI Updates

1. Update `prax migrate dev` command with new output
2. Add `--reason` and `--user` flags to rollback command
3. Add `prax migrate history` command
4. Update status/list commands for event sourcing
5. Add configuration options to CLI

### Phase 6: Testing & Documentation

1. Property-based tests for event replay invariants
2. End-to-end integration tests
3. Performance tests for large event logs
4. Update user documentation
5. Write migration guide for existing users

## Testing Strategy

### Unit Tests

- Event serialization/deserialization
- State projection from various event sequences
- Reverse SQL generation for each operation type
- Checksum computation and validation
- Event store CRUD operations

### Integration Tests

- Full dev workflow (parse → diff → generate → apply → record)
- Rollback workflow with event logging
- Multiple apply/rollback cycles
- Bootstrap process from V1 format
- Checksum resolution events

### Property-Based Tests

Using `proptest`:
- Event replay is deterministic
- Apply then rollback returns to empty state
- State projection commutative (order-independent for same timestamp)
- Checksum changes detected correctly

### Performance Tests

- Event replay performance with 1000+ migrations
- Materialized view refresh time
- Concurrent migration lock behavior
- Large SQL generation (100+ tables)

## Security Considerations

1. **SQL Injection** - All generated SQL uses parameterized queries where possible, identifiers are quoted
2. **Advisory Locks** - PostgreSQL advisory locks prevent concurrent migrations
3. **Transaction Isolation** - Migrations run in serializable isolation level
4. **Audit Trail** - Complete event log provides security audit trail
5. **Access Control** - `applied_by`/`rolled_back_by` fields track user identity

## Performance Considerations

1. **Event Log Growth** - Table grows continuously; implement archival strategy for old events
2. **State Projection** - Cache `MigrationState` in memory; refresh only on new events
3. **Materialized View** - Optional for read-heavy workloads
4. **Index Strategy** - Compound index on (migration_id, created_at DESC) for fast queries
5. **JSONB Performance** - GIN index on event_data for complex queries (if needed)

## Open Questions

None - all design questions resolved during brainstorming phase.

## References

- Event Sourcing pattern: https://martinfowler.com/eaaDev/EventSourcing.html
- Prisma Migrate: https://www.prisma.io/docs/concepts/components/prisma-migrate
- Rails Active Record Migrations: https://guides.rubyonrails.org/active_record_migrations.html
- Flyway: https://flywaydb.org/documentation/concepts/migrations

## Appendices

### Appendix A: SQL Generation Examples

**Create Table:**
```sql
-- Forward
CREATE TABLE "users" (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) UNIQUE NOT NULL,
    name VARCHAR(255)
);

-- Reverse
DROP TABLE IF EXISTS "users";
```

**Add Column:**
```sql
-- Forward
ALTER TABLE "users" ADD COLUMN role_id INTEGER;

-- Reverse (with warning)
-- WARNING: Dropping column 'role_id' - data cannot be recovered
ALTER TABLE "users" DROP COLUMN IF EXISTS role_id;
```

**Alter Column Type:**
```sql
-- Forward
ALTER TABLE "users" ALTER COLUMN age TYPE BIGINT;

-- Reverse (with warning)
-- WARNING: Type conversion from BIGINT to INTEGER may fail if values exceed INTEGER range
ALTER TABLE "users" ALTER COLUMN age TYPE INTEGER;
```

### Appendix B: Event Store Implementation

PostgreSQL implementation using `tokio-postgres`:

```rust
pub struct PostgresEventStore {
    pool: Pool<PostgresConnectionManager<NoTls>>,
}

#[async_trait::async_trait]
impl MigrationEventStore for PostgresEventStore {
    async fn append_event(
        &self,
        migration_id: &str,
        event_type: EventType,
        event_data: EventData,
    ) -> MigrateResult<i64> {
        let client = self.pool.get().await?;
        
        let event_data_json = serde_json::to_value(&event_data)?;
        
        let row = client.query_one(
            r#"
            INSERT INTO "_prax_migrations" 
                (migration_id, event_type, event_data)
            VALUES ($1, $2, $3)
            RETURNING event_id
            "#,
            &[&migration_id, &event_type.as_str(), &event_data_json],
        ).await?;
        
        Ok(row.get(0))
    }
    
    async fn get_all_events(&self) -> MigrateResult<Vec<MigrationEvent>> {
        let client = self.pool.get().await?;
        
        let rows = client.query(
            r#"
            SELECT event_id, migration_id, event_type, event_data, created_at
            FROM "_prax_migrations"
            ORDER BY event_id ASC
            "#,
            &[],
        ).await?;
        
        rows.iter()
            .map(|row| {
                Ok(MigrationEvent {
                    event_id: row.get(0),
                    migration_id: row.get(1),
                    event_type: EventType::from_str(row.get(2))?,
                    event_data: serde_json::from_value(row.get(3))?,
                    created_at: row.get(4),
                })
            })
            .collect()
    }
    
    // ... other methods
}
```

### Appendix C: CLI Command Reference

```bash
# Development workflow (auto-generate and apply)
prax migrate dev --name <migration_name>

# Manual workflow (generate only, review, then apply)
prax migrate create --name <migration_name>
prax migrate apply

# Rollback operations
prax migrate rollback
prax migrate rollback --reason "Fixing broken schema"
prax migrate rollback --to <migration_id>

# Status and history
prax migrate status
prax migrate history
prax migrate history --migration <migration_id>

# Resolution
prax migrate resolve checksum <migration_id> <old> <new> --reason "..."
prax migrate resolve skip <migration_id> --reason "..."

# Bootstrap (automatic on first run)
prax migrate bootstrap
```
