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
}
