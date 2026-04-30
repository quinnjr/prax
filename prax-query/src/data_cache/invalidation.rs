//! Cache invalidation strategies.

use std::fmt::{self, Display};
use std::time::Instant;

/// A tag for categorizing and invalidating cache entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityTag {
    /// The tag value.
    value: String,
}

impl EntityTag {
    /// Create a new tag.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// Create an entity-type tag.
    pub fn entity(entity: &str) -> Self {
        Self::new(format!("entity:{}", entity))
    }

    /// Create a record-specific tag.
    pub fn record<I: Display>(entity: &str, id: I) -> Self {
        Self::new(format!("record:{}:{}", entity, id))
    }

    /// Create a tenant tag.
    pub fn tenant(tenant: &str) -> Self {
        Self::new(format!("tenant:{}", tenant))
    }

    /// Create a query tag.
    pub fn query(name: &str) -> Self {
        Self::new(format!("query:{}", name))
    }

    /// Create a relation tag.
    pub fn relation(from: &str, to: &str) -> Self {
        Self::new(format!("rel:{}:{}", from, to))
    }

    /// Get the tag value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Check if this tag matches a pattern.
    pub fn matches(&self, pattern: &str) -> bool {
        if pattern.contains('*') {
            super::key::KeyPattern::new(pattern).matches_str(&self.value)
        } else {
            self.value == pattern
        }
    }
}

impl Display for EntityTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl From<&str> for EntityTag {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for EntityTag {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// An event that triggers cache invalidation.
#[derive(Debug, Clone)]
pub struct InvalidationEvent {
    /// Type of event.
    pub event_type: InvalidationEventType,
    /// Entity affected.
    pub entity: String,
    /// Record ID if applicable.
    pub record_id: Option<String>,
    /// When the event occurred.
    pub timestamp: Instant,
    /// Tags to invalidate.
    pub tags: Vec<EntityTag>,
    /// Additional metadata.
    pub metadata: Option<String>,
}

impl InvalidationEvent {
    /// Create a new invalidation event.
    pub fn new(event_type: InvalidationEventType, entity: impl Into<String>) -> Self {
        Self {
            event_type,
            entity: entity.into(),
            record_id: None,
            timestamp: Instant::now(),
            tags: Vec::new(),
            metadata: None,
        }
    }

    /// Create an insert event.
    pub fn insert(entity: impl Into<String>) -> Self {
        Self::new(InvalidationEventType::Insert, entity)
    }

    /// Create an update event.
    pub fn update(entity: impl Into<String>) -> Self {
        Self::new(InvalidationEventType::Update, entity)
    }

    /// Create a delete event.
    pub fn delete(entity: impl Into<String>) -> Self {
        Self::new(InvalidationEventType::Delete, entity)
    }

    /// Set the record ID.
    pub fn with_record<I: Display>(mut self, id: I) -> Self {
        self.record_id = Some(id.to_string());
        self
    }

    /// Add a tag.
    pub fn with_tag(mut self, tag: impl Into<EntityTag>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Add multiple tags.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = EntityTag>) -> Self {
        self.tags.extend(tags);
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: impl Into<String>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    /// Get all tags that should be invalidated.
    pub fn all_tags(&self) -> Vec<EntityTag> {
        let mut tags = self.tags.clone();

        // Add entity tag
        tags.push(EntityTag::entity(&self.entity));

        // Add record tag if present
        if let Some(ref id) = self.record_id {
            tags.push(EntityTag::record(&self.entity, id));
        }

        tags
    }
}

/// Type of invalidation event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationEventType {
    /// A new record was inserted.
    Insert,
    /// A record was updated.
    Update,
    /// A record was deleted.
    Delete,
    /// Multiple records were affected.
    Bulk,
    /// Schema changed (clear all for entity).
    SchemaChange,
    /// Manual invalidation.
    Manual,
}

impl Display for InvalidationEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Insert => write!(f, "insert"),
            Self::Update => write!(f, "update"),
            Self::Delete => write!(f, "delete"),
            Self::Bulk => write!(f, "bulk"),
            Self::SchemaChange => write!(f, "schema_change"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Strategy for cache invalidation.
#[derive(Debug, Clone, Default)]
pub enum InvalidationStrategy {
    /// Invalidate on every write.
    #[default]
    Immediate,

    /// Invalidate after a delay (batching).
    Delayed { delay_ms: u64 },

    /// Invalidate based on events.
    EventBased { events: Vec<InvalidationEventType> },

    /// Only invalidate specific tags.
    TagBased { tags: Vec<EntityTag> },

    /// Time-based expiration only (no explicit invalidation).
    TtlOnly,

    /// Custom invalidation logic.
    Custom { name: String },
}

impl InvalidationStrategy {
    /// Create an immediate invalidation strategy.
    pub fn immediate() -> Self {
        Self::Immediate
    }

    /// Create a delayed invalidation strategy.
    pub fn delayed(delay_ms: u64) -> Self {
        Self::Delayed { delay_ms }
    }

    /// Create an event-based strategy.
    pub fn on_events(events: Vec<InvalidationEventType>) -> Self {
        Self::EventBased { events }
    }

    /// Create a tag-based strategy.
    pub fn for_tags(tags: Vec<EntityTag>) -> Self {
        Self::TagBased { tags }
    }

    /// Create a TTL-only strategy.
    pub fn ttl_only() -> Self {
        Self::TtlOnly
    }

    /// Check if an event should trigger invalidation.
    pub fn should_invalidate(&self, event: &InvalidationEvent) -> bool {
        match self {
            Self::Immediate => true,
            Self::Delayed { .. } => true, // Will be batched
            Self::EventBased { events } => events.contains(&event.event_type),
            Self::TagBased { tags } => event.all_tags().iter().any(|t| tags.contains(t)),
            Self::TtlOnly => false,
            Self::Custom { .. } => true, // Let custom logic decide
        }
    }
}

/// An invalidation handler that can be registered with the cache.
pub trait InvalidationHandler: Send + Sync + 'static {
    /// Handle an invalidation event.
    fn handle(
        &self,
        event: &InvalidationEvent,
    ) -> impl std::future::Future<Output = super::CacheResult<()>> + Send;
}

/// A simple function-based handler.
pub struct FnHandler<F>(pub F);

impl<F, Fut> InvalidationHandler for FnHandler<F>
where
    F: Fn(&InvalidationEvent) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = super::CacheResult<()>> + Send,
{
    async fn handle(&self, event: &InvalidationEvent) -> super::CacheResult<()> {
        (self.0)(event).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_tag() {
        let tag = EntityTag::entity("User");
        assert_eq!(tag.value(), "entity:User");

        let record_tag = EntityTag::record("User", 123);
        assert_eq!(record_tag.value(), "record:User:123");
    }

    #[test]
    fn test_invalidation_event() {
        let event = InvalidationEvent::insert("User")
            .with_record(123)
            .with_tag("custom_tag");

        assert_eq!(event.entity, "User");
        assert_eq!(event.record_id, Some("123".to_string()));
        assert_eq!(event.event_type, InvalidationEventType::Insert);

        let tags = event.all_tags();
        assert!(tags.iter().any(|t| t.value() == "entity:User"));
        assert!(tags.iter().any(|t| t.value() == "record:User:123"));
    }

    #[test]
    fn test_invalidation_strategy() {
        let immediate = InvalidationStrategy::immediate();
        let event = InvalidationEvent::update("User");
        assert!(immediate.should_invalidate(&event));

        let events_only = InvalidationStrategy::on_events(vec![InvalidationEventType::Delete]);
        assert!(!events_only.should_invalidate(&event));

        let delete_event = InvalidationEvent::delete("User");
        assert!(events_only.should_invalidate(&delete_event));
    }

    #[test]
    fn test_tag_matching() {
        let tag = EntityTag::new("entity:User");
        assert!(tag.matches("entity:User"));
        assert!(tag.matches("entity:*"));
        assert!(!tag.matches("entity:Post"));
    }
}
