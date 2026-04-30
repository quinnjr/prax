//! Event types for migration event sourcing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::{MigrateResult, MigrationError};

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
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> MigrateResult<Self> {
        match s {
            "applied" => Ok(EventType::Applied),
            "rolled_back" => Ok(EventType::RolledBack),
            "failed" => Ok(EventType::Failed),
            "resolved" => Ok(EventType::Resolved),
            _ => Err(MigrationError::InvalidMigration(format!(
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
        assert_eq!(
            EventType::from_str("rolled_back").unwrap(),
            EventType::RolledBack
        );
        assert!(EventType::from_str("invalid").is_err());
    }

    #[test]
    fn test_event_type_display() {
        assert_eq!(format!("{}", EventType::Applied), "applied");
        assert_eq!(format!("{}", EventType::RolledBack), "rolled_back");
        assert_eq!(format!("{}", EventType::Failed), "failed");
        assert_eq!(format!("{}", EventType::Resolved), "resolved");
    }

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

    #[test]
    fn test_event_data_failed_serialization() {
        let data = EventData::Failed {
            error: "column already exists".to_string(),
            attempted_by: Some("user@system".to_string()),
            sql_preview: Some("ALTER TABLE".to_string()),
        };

        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["type"], "failed");
        assert_eq!(json["error"], "column already exists");
    }

    #[test]
    fn test_event_data_resolved_serialization() {
        let data = EventData::Resolved {
            resolution_type: "checksum_accepted".to_string(),
            old_checksum: Some("abc123".to_string()),
            new_checksum: Some("def456".to_string()),
            reason: "Fixed column type".to_string(),
            resolved_by: Some("user@system".to_string()),
        };

        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["type"], "resolved");
        assert_eq!(json["resolution_type"], "checksum_accepted");
        assert_eq!(json["old_checksum"], "abc123");
        assert_eq!(json["new_checksum"], "def456");
    }

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

    #[test]
    fn test_migration_event_serialization() {
        let event = MigrationEvent {
            event_id: 1,
            migration_id: "20260425120000".to_string(),
            event_type: EventType::Applied,
            event_data: EventData::Applied {
                checksum: "abc123".to_string(),
                duration_ms: 150,
                applied_by: Some("user".to_string()),
                up_sql_preview: Some("CREATE TABLE users".to_string()),
                auto_generated: true,
            },
            created_at: Utc::now(),
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event_id"], 1);
        assert_eq!(json["migration_id"], "20260425120000");

        let deserialized: MigrationEvent = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.event_id, 1);
        assert_eq!(deserialized.migration_id, "20260425120000");
    }
}
