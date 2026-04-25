# prax-migrate

Database migration engine for Prax ORM.

## Overview

`prax-migrate` provides automatic schema diffing and migration generation for Prax schemas.

## Features

- Automatic schema diffing
- Migration file generation
- Up/down migration support
- Migration history tracking with event sourcing
- Shadow database for safe migrations
- Introspection from existing databases
- Complete audit trail of all migration operations
- Property-based tested state projection

## Usage

```rust
use prax_migrate::{MigrationEngine, MigrationConfig};

let engine = MigrationEngine::new(config).await?;

// Generate migrations from schema changes
engine.generate("add_user_table").await?;

// Apply pending migrations
engine.migrate().await?;

// Rollback last migration
engine.rollback(1).await?;
```

## Event Sourcing

`prax-migrate` uses event sourcing to track migration history. Every migration operation (apply, rollback, failure, resolution) is recorded as an immutable event. The current state is derived by replaying all events.

Benefits:
- Complete audit trail of all operations
- State can be reconstructed at any point in time
- Failed migrations are recorded but don't affect state
- Supports conflict resolution and migration editing

```rust
use prax_migrate::event::{EventData, EventType};
use prax_migrate::event_store::{InMemoryEventStore, MigrationEventStore};
use prax_migrate::state::MigrationState;

// Create event store
let store = InMemoryEventStore::new();

// Record apply event
store.append_event(
    "20260425120000_create_users",
    EventType::Applied,
    EventData::Applied {
        checksum: "abc123".to_string(),
        duration_ms: 150,
        applied_by: Some("system".to_string()),
        up_sql_preview: None,
        auto_generated: true,
    },
).await?;

// Build current state from events
let events = store.get_all_events().await?;
let state = MigrationState::from_events(events);

// Check if migration is applied
if state.is_applied("20260425120000_create_users") {
    println!("Migration is applied");
}
```

See `examples/event_sourcing.rs` for a complete walkthrough.

## CLI

```bash
# Generate a new migration
prax migrate generate add_posts_table

# Apply all pending migrations
prax migrate up

# Rollback the last migration
prax migrate down

# Show migration status
prax migrate status

# View migration history
prax migrate history

# Rollback with reason
prax migrate rollback --reason "Schema change needed"
```

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

