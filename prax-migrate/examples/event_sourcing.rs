//! Event sourcing example for Prax migrations.
//!
//! This example demonstrates the complete event sourcing workflow:
//! 1. Creating an event store
//! 2. Recording migration events (apply, rollback, failed, resolved)
//! 3. Querying migration state
//! 4. Viewing migration history
//!
//! Run this example with:
//!     cargo run --example event_sourcing

use chrono::Utc;
use prax_migrate::event::{EventData, EventType};
use prax_migrate::event_store::{InMemoryEventStore, MigrationEventStore};
use prax_migrate::state::MigrationState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Event Sourcing for Migrations Example ===\n");

    // Step 1: Create an in-memory event store
    println!("1. Creating event store...");
    let store = InMemoryEventStore::new();
    println!("   Event store created\n");

    // Step 2: Record an apply event
    println!("2. Applying migration '20260425120000_create_users'...");
    let event_id = store
        .append_event(
            "20260425120000_create_users",
            EventType::Applied,
            EventData::Applied {
                checksum: "abc123def456".to_string(),
                duration_ms: 150,
                applied_by: Some("system".to_string()),
                up_sql_preview: Some("CREATE TABLE users (".to_string()),
                auto_generated: true,
            },
        )
        .await?;
    println!("   Migration applied (event_id={})\n", event_id);

    // Step 3: Query current state
    println!("3. Querying current state...");
    let events = store.get_all_events().await?;
    let state = MigrationState::from_events(events);

    if state.is_applied("20260425120000_create_users") {
        println!("   Migration is APPLIED");
        if let Some(status) = state.get_status("20260425120000_create_users") {
            println!("   - Checksum: {}", status.checksum);
            println!("   - Apply count: {}", status.apply_count);
            println!("   - Rollback count: {}", status.rollback_count);
        }
    }
    println!();

    // Step 4: Record a rollback event
    println!("4. Rolling back migration '20260425120000_create_users'...");
    store
        .append_event(
            "20260425120000_create_users",
            EventType::RolledBack,
            EventData::RolledBack {
                checksum: "abc123def456".to_string(),
                duration_ms: 89,
                rolled_back_by: Some("admin".to_string()),
                reason: Some("Need to change schema".to_string()),
                parent_event_id: event_id,
                down_sql_preview: Some("DROP TABLE users".to_string()),
            },
        )
        .await?;
    println!("   Migration rolled back\n");

    // Step 5: Query state after rollback
    println!("5. Querying state after rollback...");
    let events = store.get_all_events().await?;
    let state = MigrationState::from_events(events);

    if !state.is_applied("20260425120000_create_users") {
        println!("   Migration is NOT APPLIED");
        if let Some(status) = state.get_status("20260425120000_create_users") {
            println!("   - Apply count: {}", status.apply_count);
            println!("   - Rollback count: {}", status.rollback_count);
        }
    }
    println!();

    // Step 6: Apply another migration
    println!("6. Applying migration '20260425120100_create_posts'...");
    store
        .append_event(
            "20260425120100_create_posts",
            EventType::Applied,
            EventData::Applied {
                checksum: "def456ghi789".to_string(),
                duration_ms: 200,
                applied_by: Some("system".to_string()),
                up_sql_preview: Some("CREATE TABLE posts (".to_string()),
                auto_generated: true,
            },
        )
        .await?;
    println!("   Migration applied\n");

    // Step 7: Simulate a failed migration
    println!("7. Attempting migration '20260425120200_create_comments' (will fail)...");
    store
        .append_event(
            "20260425120200_create_comments",
            EventType::Failed,
            EventData::Failed {
                error: "column 'post_id' referenced in foreign key not found".to_string(),
                attempted_by: Some("system".to_string()),
                sql_preview: Some("ALTER TABLE comments ADD FOREIGN KEY".to_string()),
            },
        )
        .await?;
    println!("   Migration failed (recorded in event log)\n");

    // Step 8: Show complete history
    println!("8. Complete migration history:");
    println!("   {}", "=".repeat(60));

    let events = store.get_all_events().await?;
    for event in &events {
        let event_type_str = format!("{:12}", event.event_type.as_str().to_uppercase());
        let timestamp = event.created_at.format("%Y-%m-%d %H:%M:%S");

        println!(
            "   [{}] {} - {}",
            event.event_id, event_type_str, event.migration_id
        );
        println!("       Time: {}", timestamp);

        match &event.event_data {
            EventData::Applied {
                checksum,
                duration_ms,
                applied_by,
                ..
            } => {
                println!("       Checksum: {}", checksum);
                println!("       Duration: {}ms", duration_ms);
                if let Some(user) = applied_by {
                    println!("       Applied by: {}", user);
                }
            }
            EventData::RolledBack {
                duration_ms,
                rolled_back_by,
                reason,
                parent_event_id,
                ..
            } => {
                println!("       Duration: {}ms", duration_ms);
                println!("       Parent event: {}", parent_event_id);
                if let Some(user) = rolled_back_by {
                    println!("       Rolled back by: {}", user);
                }
                if let Some(r) = reason {
                    println!("       Reason: {}", r);
                }
            }
            EventData::Failed {
                error,
                attempted_by,
                ..
            } => {
                println!("       Error: {}", error);
                if let Some(user) = attempted_by {
                    println!("       Attempted by: {}", user);
                }
            }
            EventData::Resolved {
                resolution_type,
                reason,
                resolved_by,
                ..
            } => {
                println!("       Type: {}", resolution_type);
                println!("       Reason: {}", reason);
                if let Some(user) = resolved_by {
                    println!("       Resolved by: {}", user);
                }
            }
        }
        println!();
    }

    // Step 9: Show current state summary
    println!("9. Current state summary:");
    println!("   {}", "=".repeat(60));

    let state = MigrationState::from_events(events);
    let applied = state.get_applied();

    println!("   Applied migrations: {}", applied.len());
    for status in applied {
        println!("       - {}", status.migration_id);
        println!("         Checksum: {}", status.checksum);
        if let Some(timestamp) = status.last_applied_at {
            println!("         Applied at: {}", timestamp.format("%Y-%m-%d %H:%M:%S"));
        }
    }

    // Step 10: Query by event type
    println!("\n10. Query events by type:");
    println!("   {}", "=".repeat(60));

    let applied_events = store.get_events_by_type(EventType::Applied).await?;
    println!("   Applied events: {}", applied_events.len());

    let rollback_events = store.get_events_by_type(EventType::RolledBack).await?;
    println!("   Rollback events: {}", rollback_events.len());

    let failed_events = store.get_events_by_type(EventType::Failed).await?;
    println!("   Failed events: {}", failed_events.len());

    // Step 11: Query events for a specific migration
    println!("\n11. Events for '20260425120000_create_users':");
    println!("   {}", "=".repeat(60));

    let migration_events = store.get_events("20260425120000_create_users").await?;
    for event in migration_events {
        println!(
            "   [{}] {} at {}",
            event.event_id,
            event.event_type.as_str().to_uppercase(),
            event.created_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    // Step 12: Query recent events
    println!("\n12. Recent events (last 5 seconds):");
    println!("   {}", "=".repeat(60));

    let cutoff = Utc::now() - chrono::Duration::seconds(5);
    let recent = store.get_events_since(cutoff).await?;
    println!(
        "   {} events occurred in the last 5 seconds",
        recent.len()
    );

    println!("\n=== Example Complete ===");
    println!("\nKey takeaways:");
    println!("  - Events are immutable and append-only");
    println!("  - State is derived by replaying all events");
    println!("  - Last event determines migration status (applied/not applied)");
    println!("  - Failed events are recorded but don't change state");
    println!("  - Complete audit trail of all migration operations");

    Ok(())
}
