//! Event store for migration events.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};

use crate::error::MigrateResult;
use crate::event::{EventData, EventType, MigrationEvent};
use crate::history::MigrationLock;

/// Event store for migration events.
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

    /// Get all events in the log.
    async fn get_all_events(&self) -> MigrateResult<Vec<MigrationEvent>>;

    /// Get all events of a specific type.
    async fn get_events_by_type(&self, event_type: EventType)
    -> MigrateResult<Vec<MigrationEvent>>;

    /// Get all events since a specific timestamp.
    async fn get_events_since(&self, since: DateTime<Utc>) -> MigrateResult<Vec<MigrationEvent>>;

    /// Initialize the event store (create tables, etc.).
    async fn initialize(&self) -> MigrateResult<()>;

    /// Acquire an exclusive lock for migrations.
    async fn acquire_lock(&self) -> MigrateResult<MigrationLock>;
}

/// In-memory implementation of event store for testing.
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

impl Default for InMemoryEventStore {
    fn default() -> Self {
        Self::new()
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
        let event_id = {
            let mut next_id = self.next_id.lock().unwrap();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let event = MigrationEvent {
            event_id,
            migration_id: migration_id.to_string(),
            event_type,
            event_data,
            created_at: Utc::now(),
        };

        self.events.lock().unwrap().push(event);

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
        Ok(self.events.lock().unwrap().clone())
    }

    async fn get_events_by_type(
        &self,
        event_type: EventType,
    ) -> MigrateResult<Vec<MigrationEvent>> {
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
        // In-memory store doesn't need initialization
        Ok(())
    }

    async fn acquire_lock(&self) -> MigrateResult<MigrationLock> {
        // Return a dummy lock that does nothing on drop
        Ok(MigrationLock::new(42, || {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventData;

    #[tokio::test]
    async fn test_append_and_get_event() {
        let store = InMemoryEventStore::new();

        let event_id = store
            .append_event(
                "20260425120000",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        assert_eq!(event_id, 1);

        let events = store.get_events("20260425120000").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].migration_id, "20260425120000");
        assert_eq!(events[0].event_type, EventType::Applied);
    }

    #[tokio::test]
    async fn test_event_id_increment() {
        let store = InMemoryEventStore::new();

        let id1 = store
            .append_event(
                "migration1",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        let id2 = store
            .append_event(
                "migration2",
                EventType::Applied,
                EventData::Applied {
                    checksum: "def456".to_string(),
                    duration_ms: 200,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test]
    async fn test_get_all_events() {
        let store = InMemoryEventStore::new();

        store
            .append_event(
                "migration1",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
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
                    checksum: "def456".to_string(),
                    duration_ms: 200,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        let events = store.get_all_events().await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_get_events_by_type() {
        let store = InMemoryEventStore::new();

        store
            .append_event(
                "migration1",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
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
                EventType::Failed,
                EventData::Failed {
                    error: "Connection error".to_string(),
                    attempted_by: None,
                    sql_preview: None,
                },
            )
            .await
            .unwrap();

        store
            .append_event(
                "migration3",
                EventType::Applied,
                EventData::Applied {
                    checksum: "ghi789".to_string(),
                    duration_ms: 300,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        let applied_events = store.get_events_by_type(EventType::Applied).await.unwrap();
        assert_eq!(applied_events.len(), 2);

        let failed_events = store.get_events_by_type(EventType::Failed).await.unwrap();
        assert_eq!(failed_events.len(), 1);
    }

    #[tokio::test]
    async fn test_get_events_since() {
        let store = InMemoryEventStore::new();

        let before = Utc::now();

        store
            .append_event(
                "migration1",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        // Sleep briefly to ensure timestamp difference
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let cutoff = Utc::now();

        store
            .append_event(
                "migration2",
                EventType::Applied,
                EventData::Applied {
                    checksum: "def456".to_string(),
                    duration_ms: 200,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        let recent_events = store.get_events_since(cutoff).await.unwrap();
        assert_eq!(recent_events.len(), 1);
        assert_eq!(recent_events[0].migration_id, "migration2");

        let all_since_before = store.get_events_since(before).await.unwrap();
        assert_eq!(all_since_before.len(), 2);
    }

    #[tokio::test]
    async fn test_initialize() {
        let store = InMemoryEventStore::new();
        let result = store.initialize().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_acquire_lock() {
        let store = InMemoryEventStore::new();
        let lock = store.acquire_lock().await.unwrap();
        assert_eq!(lock.id(), 42);
    }

    #[tokio::test]
    async fn test_get_events_for_specific_migration() {
        let store = InMemoryEventStore::new();

        store
            .append_event(
                "migration1",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 150,
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
                    checksum: "def456".to_string(),
                    duration_ms: 200,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: true,
                },
            )
            .await
            .unwrap();

        store
            .append_event(
                "migration1",
                EventType::RolledBack,
                EventData::RolledBack {
                    checksum: "abc123".to_string(),
                    duration_ms: 89,
                    rolled_back_by: None,
                    reason: None,
                    parent_event_id: 1,
                    down_sql_preview: None,
                },
            )
            .await
            .unwrap();

        let migration1_events = store.get_events("migration1").await.unwrap();
        assert_eq!(migration1_events.len(), 2);
        assert_eq!(migration1_events[0].event_type, EventType::Applied);
        assert_eq!(migration1_events[1].event_type, EventType::RolledBack);

        let migration2_events = store.get_events("migration2").await.unwrap();
        assert_eq!(migration2_events.len(), 1);
    }
}
