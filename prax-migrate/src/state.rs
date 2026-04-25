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

    // Property-based tests using proptest
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Property: Applying then rolling back a migration returns to not-applied state
            #[test]
            fn prop_apply_then_rollback_returns_to_not_applied(
                migration_id in "[a-z0-9]{20}",
            ) {
                let events = vec![
                    MigrationEvent {
                        event_id: 1,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Applied,
                        event_data: EventData::Applied {
                            checksum: "checksum1".to_string(),
                            duration_ms: 150,
                            applied_by: None,
                            up_sql_preview: None,
                            auto_generated: true,
                        },
                        created_at: Utc::now(),
                    },
                    MigrationEvent {
                        event_id: 2,
                        migration_id: migration_id.clone(),
                        event_type: EventType::RolledBack,
                        event_data: EventData::RolledBack {
                            checksum: "checksum1".to_string(),
                            duration_ms: 89,
                            rolled_back_by: None,
                            reason: None,
                            parent_event_id: 1,
                            down_sql_preview: None,
                        },
                        created_at: Utc::now(),
                    },
                ];

                let state = MigrationState::from_events(events);

                // Should not be applied after rollback
                prop_assert!(!state.is_applied(&migration_id));

                // Should have one apply and one rollback
                let status = state.get_status(&migration_id).unwrap();
                prop_assert_eq!(status.apply_count, 1);
                prop_assert_eq!(status.rollback_count, 1);
            }

            /// Property: Multiple apply events increment counter correctly
            #[test]
            fn prop_multiple_applies_increment_counter(
                migration_id in "[a-z0-9]{20}",
                apply_count in 1usize..10,
            ) {
                let mut events = Vec::new();

                for i in 0..apply_count {
                    events.push(MigrationEvent {
                        event_id: (i + 1) as i64,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Applied,
                        event_data: EventData::Applied {
                            checksum: format!("checksum{}", i),
                            duration_ms: 150,
                            applied_by: None,
                            up_sql_preview: None,
                            auto_generated: true,
                        },
                        created_at: Utc::now(),
                    });
                }

                let state = MigrationState::from_events(events);

                // Should be applied
                prop_assert!(state.is_applied(&migration_id));

                // Apply count should match number of events
                let status = state.get_status(&migration_id).unwrap();
                prop_assert_eq!(status.apply_count as usize, apply_count);
                prop_assert_eq!(status.rollback_count, 0);
            }

            /// Property: Event order matters - last event wins for is_applied
            #[test]
            fn prop_last_event_wins(
                migration_id in "[a-z0-9]{20}",
                cycle_count in 1usize..5,
                end_with_apply in prop::bool::ANY,
            ) {
                let mut events = Vec::new();
                let mut event_id = 1i64;

                // Create multiple apply/rollback cycles
                for _ in 0..cycle_count {
                    events.push(MigrationEvent {
                        event_id,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Applied,
                        event_data: EventData::Applied {
                            checksum: format!("checksum{}", event_id),
                            duration_ms: 150,
                            applied_by: None,
                            up_sql_preview: None,
                            auto_generated: true,
                        },
                        created_at: Utc::now(),
                    });
                    event_id += 1;

                    events.push(MigrationEvent {
                        event_id,
                        migration_id: migration_id.clone(),
                        event_type: EventType::RolledBack,
                        event_data: EventData::RolledBack {
                            checksum: format!("checksum{}", event_id - 1),
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

                // Optionally add one more apply at the end
                if end_with_apply {
                    events.push(MigrationEvent {
                        event_id,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Applied,
                        event_data: EventData::Applied {
                            checksum: format!("checksum{}", event_id),
                            duration_ms: 150,
                            applied_by: None,
                            up_sql_preview: None,
                            auto_generated: true,
                        },
                        created_at: Utc::now(),
                    });
                }

                let state = MigrationState::from_events(events);

                // Final state should match whether we ended with apply or rollback
                prop_assert_eq!(state.is_applied(&migration_id), end_with_apply);

                let status = state.get_status(&migration_id).unwrap();
                let expected_applies = if end_with_apply {
                    cycle_count as u32 + 1
                } else {
                    cycle_count as u32
                };
                prop_assert_eq!(status.apply_count, expected_applies);
                prop_assert_eq!(status.rollback_count, cycle_count as u32);
            }

            /// Property: Failed events don't change applied state
            #[test]
            fn prop_failed_events_dont_change_state(
                migration_id in "[a-z0-9]{20}",
                start_applied in prop::bool::ANY,
                failure_count in 1usize..5,
            ) {
                let mut events = Vec::new();
                let mut event_id = 1i64;

                // Optionally start with an applied state
                if start_applied {
                    events.push(MigrationEvent {
                        event_id,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Applied,
                        event_data: EventData::Applied {
                            checksum: "initial".to_string(),
                            duration_ms: 150,
                            applied_by: None,
                            up_sql_preview: None,
                            auto_generated: true,
                        },
                        created_at: Utc::now(),
                    });
                    event_id += 1;
                }

                // Add multiple failed events
                for i in 0..failure_count {
                    events.push(MigrationEvent {
                        event_id: event_id + i as i64,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Failed,
                        event_data: EventData::Failed {
                            error: format!("error {}", i),
                            attempted_by: None,
                            sql_preview: None,
                        },
                        created_at: Utc::now(),
                    });
                }

                let state = MigrationState::from_events(events);

                // Applied state should match initial state, unaffected by failures
                prop_assert_eq!(state.is_applied(&migration_id), start_applied);
            }

            /// Property: Checksum is always from the last applied or resolved event
            #[test]
            fn prop_checksum_from_last_apply_or_resolve(
                migration_id in "[a-z0-9]{20}",
            ) {
                let events = vec![
                    MigrationEvent {
                        event_id: 1,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Applied,
                        event_data: EventData::Applied {
                            checksum: "first_checksum".to_string(),
                            duration_ms: 150,
                            applied_by: None,
                            up_sql_preview: None,
                            auto_generated: true,
                        },
                        created_at: Utc::now(),
                    },
                    MigrationEvent {
                        event_id: 2,
                        migration_id: migration_id.clone(),
                        event_type: EventType::Resolved,
                        event_data: EventData::Resolved {
                            resolution_type: "checksum_accepted".to_string(),
                            old_checksum: Some("first_checksum".to_string()),
                            new_checksum: Some("resolved_checksum".to_string()),
                            reason: "Fixed".to_string(),
                            resolved_by: None,
                        },
                        created_at: Utc::now(),
                    },
                ];

                let state = MigrationState::from_events(events);
                let status = state.get_status(&migration_id).unwrap();

                // Checksum should be from the resolved event
                prop_assert_eq!(&status.checksum, "resolved_checksum");
            }
        }
    }
}
