//! Integration tests for event sourcing workflow.
//!
//! Tests the complete event sourcing flow from appending events to building
//! migration state, ensuring all components work together correctly.

use prax_migrate::event::{EventData, EventType};
use prax_migrate::event_store::{InMemoryEventStore, MigrationEventStore};
use prax_migrate::state::MigrationState;

#[tokio::test]
async fn test_full_apply_rollback_workflow() {
    // Create event store
    let store = InMemoryEventStore::new();

    // Append apply event
    let event_id = store
        .append_event(
            "20260425120000_create_users",
            EventType::Applied,
            EventData::Applied {
                checksum: "abc123def456".to_string(),
                duration_ms: 150,
                applied_by: Some("user@system".to_string()),
                up_sql_preview: Some("CREATE TABLE users".to_string()),
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    assert_eq!(event_id, 1);

    // Build state from events
    let events = store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);

    // Verify migration is applied
    assert!(state.is_applied("20260425120000_create_users"));
    let status = state.get_status("20260425120000_create_users").unwrap();
    assert_eq!(status.apply_count, 1);
    assert_eq!(status.rollback_count, 0);
    assert_eq!(status.checksum, "abc123def456");

    // Append rollback event
    store
        .append_event(
            "20260425120000_create_users",
            EventType::RolledBack,
            EventData::RolledBack {
                checksum: "abc123def456".to_string(),
                duration_ms: 89,
                rolled_back_by: Some("user@system".to_string()),
                reason: Some("Testing rollback".to_string()),
                parent_event_id: event_id,
                down_sql_preview: Some("DROP TABLE users".to_string()),
            },
        )
        .await
        .unwrap();

    // Rebuild state
    let events = store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);

    // Verify migration is not applied
    assert!(!state.is_applied("20260425120000_create_users"));
    let status = state.get_status("20260425120000_create_users").unwrap();
    assert_eq!(status.apply_count, 1);
    assert_eq!(status.rollback_count, 1);
}

#[tokio::test]
async fn test_multiple_migrations_workflow() {
    let store = InMemoryEventStore::new();

    // Apply three migrations
    let migration_ids = vec![
        "20260425120000_create_users",
        "20260425120100_create_posts",
        "20260425120200_create_comments",
    ];

    for migration_id in &migration_ids {
        store
            .append_event(
                migration_id,
                EventType::Applied,
                EventData::Applied {
                    checksum: format!("checksum_{}", migration_id),
                    duration_ms: 150,
                    applied_by: Some("system".to_string()),
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();
    }

    // Verify all three are applied
    let events = store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);

    for migration_id in &migration_ids {
        assert!(state.is_applied(migration_id));
    }

    let applied = state.get_applied();
    assert_eq!(applied.len(), 3);

    // Rollback the second migration
    store
        .append_event(
            "20260425120100_create_posts",
            EventType::RolledBack,
            EventData::RolledBack {
                checksum: "checksum_20260425120100_create_posts".to_string(),
                duration_ms: 75,
                rolled_back_by: Some("admin".to_string()),
                reason: Some("Schema change needed".to_string()),
                parent_event_id: 2,
                down_sql_preview: None,
            },
        )
        .await
        .unwrap();

    // Rebuild state
    let events = store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);

    // Verify first and third are applied, second is not
    assert!(state.is_applied("20260425120000_create_users"));
    assert!(!state.is_applied("20260425120100_create_posts"));
    assert!(state.is_applied("20260425120200_create_comments"));

    let applied = state.get_applied();
    assert_eq!(applied.len(), 2);

    // Verify rollback count
    let status = state.get_status("20260425120100_create_posts").unwrap();
    assert_eq!(status.apply_count, 1);
    assert_eq!(status.rollback_count, 1);
}

#[tokio::test]
async fn test_failed_migration_does_not_affect_state() {
    let store = InMemoryEventStore::new();

    // Apply a migration
    store
        .append_event(
            "20260425120000_create_users",
            EventType::Applied,
            EventData::Applied {
                checksum: "abc123".to_string(),
                duration_ms: 150,
                applied_by: Some("system".to_string()),
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    // Attempt another migration that fails
    store
        .append_event(
            "20260425120100_create_posts",
            EventType::Failed,
            EventData::Failed {
                error: "column 'user_id' does not exist".to_string(),
                attempted_by: Some("system".to_string()),
                sql_preview: Some("ALTER TABLE posts ADD FOREIGN KEY".to_string()),
            },
        )
        .await
        .unwrap();

    // Build state
    let events = store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);

    // First migration should be applied
    assert!(state.is_applied("20260425120000_create_users"));

    // Failed migration should not be applied
    assert!(!state.is_applied("20260425120100_create_posts"));

    // Only one migration should be in applied state
    let applied = state.get_applied();
    assert_eq!(applied.len(), 1);
}

#[tokio::test]
async fn test_resolved_migration_updates_checksum() {
    let store = InMemoryEventStore::new();

    // Apply a migration
    store
        .append_event(
            "20260425120000_create_users",
            EventType::Applied,
            EventData::Applied {
                checksum: "old_checksum".to_string(),
                duration_ms: 150,
                applied_by: Some("system".to_string()),
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    // Resolve the migration with a new checksum
    store
        .append_event(
            "20260425120000_create_users",
            EventType::Resolved,
            EventData::Resolved {
                resolution_type: "checksum_accepted".to_string(),
                old_checksum: Some("old_checksum".to_string()),
                new_checksum: Some("new_checksum".to_string()),
                reason: "Migration file was edited to fix syntax".to_string(),
                resolved_by: Some("admin".to_string()),
            },
        )
        .await
        .unwrap();

    // Build state
    let events = store.get_all_events().await.unwrap();
    let state = MigrationState::from_events(events);

    // Migration should still be applied
    assert!(state.is_applied("20260425120000_create_users"));

    // But checksum should be updated
    let status = state.get_status("20260425120000_create_users").unwrap();
    assert_eq!(status.checksum, "new_checksum");
}

#[tokio::test]
async fn test_query_events_by_type() {
    let store = InMemoryEventStore::new();

    // Apply two migrations
    store
        .append_event(
            "migration1",
            EventType::Applied,
            EventData::Applied {
                checksum: "abc".to_string(),
                duration_ms: 100,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    store
        .append_event(
            "migration2",
            EventType::Applied,
            EventData::Applied {
                checksum: "def".to_string(),
                duration_ms: 200,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    // Rollback one
    store
        .append_event(
            "migration1",
            EventType::RolledBack,
            EventData::RolledBack {
                checksum: "abc".to_string(),
                duration_ms: 50,
                rolled_back_by: None,
                reason: None,
                parent_event_id: 1,
                down_sql_preview: None,
            },
        )
        .await
        .unwrap();

    // Fail one
    store
        .append_event(
            "migration3",
            EventType::Failed,
            EventData::Failed {
                error: "syntax error".to_string(),
                attempted_by: None,
                sql_preview: None,
            },
        )
        .await
        .unwrap();

    // Query by type
    let applied_events = store.get_events_by_type(EventType::Applied).await.unwrap();
    assert_eq!(applied_events.len(), 2);

    let rollback_events = store
        .get_events_by_type(EventType::RolledBack)
        .await
        .unwrap();
    assert_eq!(rollback_events.len(), 1);

    let failed_events = store.get_events_by_type(EventType::Failed).await.unwrap();
    assert_eq!(failed_events.len(), 1);
}

#[tokio::test]
async fn test_query_events_for_migration() {
    let store = InMemoryEventStore::new();

    // Create a migration with multiple events
    store
        .append_event(
            "20260425120000_users",
            EventType::Applied,
            EventData::Applied {
                checksum: "abc".to_string(),
                duration_ms: 100,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    store
        .append_event(
            "20260425120000_users",
            EventType::RolledBack,
            EventData::RolledBack {
                checksum: "abc".to_string(),
                duration_ms: 50,
                rolled_back_by: None,
                reason: None,
                parent_event_id: 1,
                down_sql_preview: None,
            },
        )
        .await
        .unwrap();

    store
        .append_event(
            "20260425120000_users",
            EventType::Applied,
            EventData::Applied {
                checksum: "abc".to_string(),
                duration_ms: 100,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    // Add another migration
    store
        .append_event(
            "20260425120100_posts",
            EventType::Applied,
            EventData::Applied {
                checksum: "def".to_string(),
                duration_ms: 150,
                applied_by: None,
                up_sql_preview: None,
                auto_generated: true,
            },
        )
        .await
        .unwrap();

    // Query events for first migration
    let events = store.get_events("20260425120000_users").await.unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type, EventType::Applied);
    assert_eq!(events[1].event_type, EventType::RolledBack);
    assert_eq!(events[2].event_type, EventType::Applied);

    // Query events for second migration
    let events = store.get_events("20260425120100_posts").await.unwrap();
    assert_eq!(events.len(), 1);
}
