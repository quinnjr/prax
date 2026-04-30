//! Replication and high availability support.
//!
//! This module provides types for managing database replication, read replicas,
//! connection routing, and failover handling.
//!
//! # Database Support
//!
//! | Feature            | PostgreSQL | MySQL | SQLite | MSSQL     | MongoDB     |
//! |--------------------|------------|-------|--------|-----------|-------------|
//! | Read replicas      | ✅         | ✅    | ❌     | ✅ Always | ✅ Replica  |
//! | Logical replication| ✅         | ✅    | ❌     | ✅        | ✅          |
//! | Connection routing | ✅         | ✅    | ❌     | ✅        | ✅          |
//! | Auto-failover      | ✅         | ✅    | ❌     | ✅        | ✅          |
//! | Read preference    | ❌         | ❌    | ❌     | ❌        | ✅          |

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{QueryError, QueryResult};

// ============================================================================
// Replica Configuration
// ============================================================================

/// Configuration for a database replica.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaConfig {
    /// Unique identifier for this replica.
    pub id: String,
    /// Connection URL.
    pub url: String,
    /// Role of this replica.
    pub role: ReplicaRole,
    /// Priority for failover (higher = preferred).
    pub priority: u32,
    /// Weight for load balancing (higher = more traffic).
    pub weight: u32,
    /// Region/datacenter for locality-aware routing.
    pub region: Option<String>,
    /// Maximum acceptable replication lag.
    pub max_lag: Option<Duration>,
}

/// Role of a replica in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicaRole {
    /// Primary/master - handles writes.
    Primary,
    /// Secondary/replica - handles reads.
    Secondary,
    /// Arbiter - for elections only (MongoDB).
    Arbiter,
    /// Hidden - for backups, not queryable.
    Hidden,
}

impl ReplicaConfig {
    /// Create a primary replica config.
    pub fn primary(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            role: ReplicaRole::Primary,
            priority: 100,
            weight: 100,
            region: None,
            max_lag: None,
        }
    }

    /// Create a secondary replica config.
    pub fn secondary(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            role: ReplicaRole::Secondary,
            priority: 50,
            weight: 100,
            region: None,
            max_lag: Some(Duration::from_secs(10)),
        }
    }

    /// Set region.
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set weight.
    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Set max lag.
    pub fn with_max_lag(mut self, lag: Duration) -> Self {
        self.max_lag = Some(lag);
        self
    }
}

// ============================================================================
// Replica Set Configuration
// ============================================================================

/// Configuration for a replica set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSetConfig {
    /// Name of the replica set.
    pub name: String,
    /// List of replicas.
    pub replicas: Vec<ReplicaConfig>,
    /// Default read preference.
    pub default_read_preference: ReadPreference,
    /// Health check interval.
    pub health_check_interval: Duration,
    /// Failover timeout.
    pub failover_timeout: Duration,
}

impl ReplicaSetConfig {
    /// Create a new replica set config.
    pub fn new(name: impl Into<String>) -> ReplicaSetBuilder {
        ReplicaSetBuilder::new(name)
    }

    /// Get the primary replica.
    pub fn primary(&self) -> Option<&ReplicaConfig> {
        self.replicas
            .iter()
            .find(|r| r.role == ReplicaRole::Primary)
    }

    /// Get all secondary replicas.
    pub fn secondaries(&self) -> impl Iterator<Item = &ReplicaConfig> {
        self.replicas
            .iter()
            .filter(|r| r.role == ReplicaRole::Secondary)
    }

    /// Get replicas in a specific region.
    pub fn in_region(&self, region: &str) -> impl Iterator<Item = &ReplicaConfig> {
        self.replicas
            .iter()
            .filter(move |r| r.region.as_deref() == Some(region))
    }
}

/// Builder for replica set configuration.
#[derive(Debug, Clone)]
pub struct ReplicaSetBuilder {
    name: String,
    replicas: Vec<ReplicaConfig>,
    default_read_preference: ReadPreference,
    health_check_interval: Duration,
    failover_timeout: Duration,
}

impl ReplicaSetBuilder {
    /// Create a new builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            replicas: Vec::new(),
            default_read_preference: ReadPreference::Primary,
            health_check_interval: Duration::from_secs(10),
            failover_timeout: Duration::from_secs(30),
        }
    }

    /// Add a replica.
    pub fn replica(mut self, config: ReplicaConfig) -> Self {
        self.replicas.push(config);
        self
    }

    /// Add primary.
    pub fn primary(self, id: impl Into<String>, url: impl Into<String>) -> Self {
        self.replica(ReplicaConfig::primary(id, url))
    }

    /// Add secondary.
    pub fn secondary(self, id: impl Into<String>, url: impl Into<String>) -> Self {
        self.replica(ReplicaConfig::secondary(id, url))
    }

    /// Set default read preference.
    pub fn read_preference(mut self, pref: ReadPreference) -> Self {
        self.default_read_preference = pref;
        self
    }

    /// Set health check interval.
    pub fn health_check_interval(mut self, interval: Duration) -> Self {
        self.health_check_interval = interval;
        self
    }

    /// Set failover timeout.
    pub fn failover_timeout(mut self, timeout: Duration) -> Self {
        self.failover_timeout = timeout;
        self
    }

    /// Build the config.
    pub fn build(self) -> ReplicaSetConfig {
        ReplicaSetConfig {
            name: self.name,
            replicas: self.replicas,
            default_read_preference: self.default_read_preference,
            health_check_interval: self.health_check_interval,
            failover_timeout: self.failover_timeout,
        }
    }
}

// ============================================================================
// Read Preference
// ============================================================================

/// Read preference for query routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReadPreference {
    /// Always read from primary.
    #[default]
    Primary,
    /// Prefer primary, fallback to secondary.
    PrimaryPreferred,
    /// Always read from secondary.
    Secondary,
    /// Prefer secondary, fallback to primary.
    SecondaryPreferred,
    /// Read from nearest replica by latency.
    Nearest,
    /// Read from specific region.
    Region(String),
    /// Custom tag set (MongoDB).
    TagSet(Vec<HashMap<String, String>>),
}

impl ReadPreference {
    /// Create a region preference.
    pub fn region(region: impl Into<String>) -> Self {
        Self::Region(region.into())
    }

    /// Create a tag set preference.
    pub fn tag_set(tags: Vec<HashMap<String, String>>) -> Self {
        Self::TagSet(tags)
    }

    /// Convert to MongoDB read preference string.
    pub fn to_mongodb(&self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::PrimaryPreferred => "primaryPreferred",
            Self::Secondary => "secondary",
            Self::SecondaryPreferred => "secondaryPreferred",
            Self::Nearest => "nearest",
            Self::Region(_) | Self::TagSet(_) => "nearest",
        }
    }

    /// Check if this preference allows reading from primary.
    pub fn allows_primary(&self) -> bool {
        matches!(
            self,
            Self::Primary
                | Self::PrimaryPreferred
                | Self::Nearest
                | Self::Region(_)
                | Self::TagSet(_)
        )
    }

    /// Check if this preference allows reading from secondary.
    pub fn allows_secondary(&self) -> bool {
        !matches!(self, Self::Primary)
    }
}

// ============================================================================
// Replica Health
// ============================================================================

/// Health status of a replica.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Replica is healthy and accepting connections.
    Healthy,
    /// Replica is degraded (high lag, slow responses).
    Degraded,
    /// Replica is unhealthy (not responding).
    Unhealthy,
    /// Health status unknown (not yet checked).
    Unknown,
}

/// Health information for a replica.
#[derive(Debug, Clone)]
pub struct ReplicaHealth {
    /// Replica ID.
    pub id: String,
    /// Current health status.
    pub status: HealthStatus,
    /// Current replication lag (if known).
    pub lag: Option<Duration>,
    /// Last successful health check.
    pub last_check: Option<Instant>,
    /// Response latency.
    pub latency: Option<Duration>,
    /// Consecutive failures.
    pub consecutive_failures: u32,
}

impl ReplicaHealth {
    /// Create a new health record.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: HealthStatus::Unknown,
            lag: None,
            last_check: None,
            latency: None,
            consecutive_failures: 0,
        }
    }

    /// Mark as healthy.
    pub fn mark_healthy(&mut self, latency: Duration, lag: Option<Duration>) {
        self.status = HealthStatus::Healthy;
        self.latency = Some(latency);
        self.lag = lag;
        self.last_check = Some(Instant::now());
        self.consecutive_failures = 0;
    }

    /// Mark as degraded.
    pub fn mark_degraded(&mut self, reason: &str) {
        self.status = HealthStatus::Degraded;
        self.last_check = Some(Instant::now());
        let _ = reason; // Could log this
    }

    /// Mark as unhealthy.
    pub fn mark_unhealthy(&mut self) {
        self.status = HealthStatus::Unhealthy;
        self.last_check = Some(Instant::now());
        self.consecutive_failures += 1;
    }

    /// Check if replica is usable for queries.
    pub fn is_usable(&self) -> bool {
        matches!(self.status, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

// ============================================================================
// Connection Router
// ============================================================================

/// Query type for routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    /// Read-only query.
    Read,
    /// Write query.
    Write,
    /// Transaction (requires primary).
    Transaction,
}

/// Connection router for read/write splitting.
#[derive(Debug)]
pub struct ConnectionRouter {
    /// Replica set configuration.
    config: ReplicaSetConfig,
    /// Health status of each replica.
    health: HashMap<String, ReplicaHealth>,
    /// Current primary ID.
    current_primary: Option<String>,
    /// Round-robin counter for load balancing.
    round_robin: AtomicUsize,
    /// Whether router is in failover mode.
    in_failover: AtomicBool,
}

impl ConnectionRouter {
    /// Create a new router.
    pub fn new(config: ReplicaSetConfig) -> Self {
        let mut health = HashMap::new();
        let mut primary_id = None;

        for replica in &config.replicas {
            health.insert(replica.id.clone(), ReplicaHealth::new(&replica.id));
            if replica.role == ReplicaRole::Primary {
                primary_id = Some(replica.id.clone());
            }
        }

        Self {
            config,
            health,
            current_primary: primary_id,
            round_robin: AtomicUsize::new(0),
            in_failover: AtomicBool::new(false),
        }
    }

    /// Get replica for a query based on query type and read preference.
    pub fn route(
        &self,
        query_type: QueryType,
        preference: Option<&ReadPreference>,
    ) -> QueryResult<&ReplicaConfig> {
        let pref = preference.unwrap_or(&self.config.default_read_preference);

        match query_type {
            QueryType::Write | QueryType::Transaction => self.get_primary(),
            QueryType::Read => self.route_read(pref),
        }
    }

    /// Get the primary replica.
    pub fn get_primary(&self) -> QueryResult<&ReplicaConfig> {
        let primary_id = self
            .current_primary
            .as_ref()
            .ok_or_else(|| QueryError::connection("No primary replica available"))?;

        self.config
            .replicas
            .iter()
            .find(|r| &r.id == primary_id)
            .ok_or_else(|| QueryError::connection("Primary replica not found"))
    }

    /// Route a read query based on preference.
    fn route_read(&self, preference: &ReadPreference) -> QueryResult<&ReplicaConfig> {
        match preference {
            ReadPreference::Primary => self.get_primary(),
            ReadPreference::PrimaryPreferred => {
                self.get_primary().or_else(|_| self.get_any_secondary())
            }
            ReadPreference::Secondary => self.get_any_secondary(),
            ReadPreference::SecondaryPreferred => {
                self.get_any_secondary().or_else(|_| self.get_primary())
            }
            ReadPreference::Nearest => self.get_nearest(),
            ReadPreference::Region(region) => self.get_in_region(region),
            ReadPreference::TagSet(_tags) => {
                // Simplified: just get nearest for now
                self.get_nearest()
            }
        }
    }

    /// Get any healthy secondary.
    fn get_any_secondary(&self) -> QueryResult<&ReplicaConfig> {
        let secondaries: Vec<_> = self
            .config
            .secondaries()
            .filter(|r| self.is_replica_healthy(&r.id))
            .collect();

        if secondaries.is_empty() {
            return Err(QueryError::connection(
                "No healthy secondary replicas available",
            ));
        }

        // Round-robin selection
        let idx = self.round_robin.fetch_add(1, Ordering::Relaxed) % secondaries.len();
        Ok(secondaries[idx])
    }

    /// Get the nearest replica by latency.
    fn get_nearest(&self) -> QueryResult<&ReplicaConfig> {
        let mut best: Option<(&ReplicaConfig, Duration)> = None;

        for replica in &self.config.replicas {
            if !self.is_replica_healthy(&replica.id) {
                continue;
            }

            if let Some(health) = self.health.get(&replica.id)
                && let Some(latency) = health.latency
            {
                match &best {
                    None => best = Some((replica, latency)),
                    Some((_, best_latency)) if latency < *best_latency => {
                        best = Some((replica, latency));
                    }
                    _ => {}
                }
            }
        }

        best.map(|(r, _)| r)
            .ok_or_else(|| QueryError::connection("No healthy replicas available"))
    }

    /// Get replica in specific region.
    fn get_in_region(&self, region: &str) -> QueryResult<&ReplicaConfig> {
        let replicas: Vec<_> = self
            .config
            .in_region(region)
            .filter(|r| self.is_replica_healthy(&r.id))
            .collect();

        if replicas.is_empty() {
            // Fallback to nearest
            return self.get_nearest();
        }

        let idx = self.round_robin.fetch_add(1, Ordering::Relaxed) % replicas.len();
        Ok(replicas[idx])
    }

    /// Check if a replica is healthy.
    fn is_replica_healthy(&self, id: &str) -> bool {
        self.health.get(id).map(|h| h.is_usable()).unwrap_or(false)
    }

    /// Update health status of a replica.
    pub fn update_health(
        &mut self,
        id: &str,
        status: HealthStatus,
        latency: Option<Duration>,
        lag: Option<Duration>,
    ) {
        if let Some(health) = self.health.get_mut(id) {
            match status {
                HealthStatus::Healthy => {
                    health.mark_healthy(latency.unwrap_or(Duration::ZERO), lag);
                }
                HealthStatus::Degraded => {
                    health.mark_degraded("degraded");
                }
                HealthStatus::Unhealthy => {
                    health.mark_unhealthy();
                }
                HealthStatus::Unknown => {}
            }
        }
    }

    /// Check if replication lag is acceptable.
    pub fn check_lag(&self, replica_id: &str, max_lag: Duration) -> bool {
        self.health
            .get(replica_id)
            .and_then(|h| h.lag)
            .map(|lag| lag <= max_lag)
            .unwrap_or(false)
    }

    /// Initiate failover to a new primary.
    pub fn initiate_failover(&mut self) -> QueryResult<String> {
        self.in_failover.store(true, Ordering::SeqCst);

        // Find best candidate (highest priority secondary that is healthy)
        let candidate = self
            .config
            .replicas
            .iter()
            .filter(|r| r.role == ReplicaRole::Secondary)
            .filter(|r| self.is_replica_healthy(&r.id))
            .max_by_key(|r| r.priority);

        match candidate {
            Some(new_primary) => {
                let new_primary_id = new_primary.id.clone();
                self.current_primary = Some(new_primary_id.clone());
                self.in_failover.store(false, Ordering::SeqCst);
                Ok(new_primary_id)
            }
            None => {
                self.in_failover.store(false, Ordering::SeqCst);
                Err(QueryError::connection(
                    "No suitable failover candidate found",
                ))
            }
        }
    }

    /// Check if router is currently in failover mode.
    pub fn is_in_failover(&self) -> bool {
        self.in_failover.load(Ordering::SeqCst)
    }
}

// ============================================================================
// Replication Lag Monitor
// ============================================================================

/// Monitor for tracking replication lag.
#[derive(Debug)]
pub struct LagMonitor {
    /// Lag measurements per replica.
    measurements: HashMap<String, LagMeasurement>,
    /// Maximum acceptable lag.
    max_acceptable_lag: Duration,
}

/// Lag measurement for a replica.
#[derive(Debug, Clone)]
pub struct LagMeasurement {
    /// Current lag.
    pub current: Duration,
    /// Average lag.
    pub average: Duration,
    /// Maximum observed lag.
    pub max: Duration,
    /// Timestamp of measurement.
    pub timestamp: Instant,
    /// Number of samples.
    pub samples: u64,
}

impl LagMonitor {
    /// Create a new lag monitor.
    pub fn new(max_acceptable_lag: Duration) -> Self {
        Self {
            measurements: HashMap::new(),
            max_acceptable_lag,
        }
    }

    /// Record a lag measurement.
    pub fn record(&mut self, replica_id: &str, lag: Duration) {
        let entry = self
            .measurements
            .entry(replica_id.to_string())
            .or_insert_with(|| LagMeasurement {
                current: Duration::ZERO,
                average: Duration::ZERO,
                max: Duration::ZERO,
                timestamp: Instant::now(),
                samples: 0,
            });

        entry.current = lag;
        entry.max = entry.max.max(lag);
        entry.samples += 1;

        // Exponential moving average
        let alpha = 0.3;
        let new_avg = Duration::from_secs_f64(
            entry.average.as_secs_f64() * (1.0 - alpha) + lag.as_secs_f64() * alpha,
        );
        entry.average = new_avg;
        entry.timestamp = Instant::now();
    }

    /// Check if a replica is within acceptable lag.
    pub fn is_acceptable(&self, replica_id: &str) -> bool {
        self.measurements
            .get(replica_id)
            .map(|m| m.current <= self.max_acceptable_lag)
            .unwrap_or(true) // Unknown = assume OK
    }

    /// Get current lag for a replica.
    pub fn get_lag(&self, replica_id: &str) -> Option<Duration> {
        self.measurements.get(replica_id).map(|m| m.current)
    }

    /// Get all replicas that are lagging too much.
    pub fn get_lagging_replicas(&self) -> Vec<&str> {
        self.measurements
            .iter()
            .filter(|(_, m)| m.current > self.max_acceptable_lag)
            .map(|(id, _)| id.as_str())
            .collect()
    }
}

// ============================================================================
// SQL Helpers for Replication Lag
// ============================================================================

/// SQL queries for checking replication lag.
pub mod lag_queries {
    use crate::sql::DatabaseType;

    /// Generate SQL to check replication lag.
    pub fn check_lag_sql(db_type: DatabaseType) -> &'static str {
        match db_type {
            DatabaseType::PostgreSQL => {
                // Returns lag in seconds
                "SELECT EXTRACT(EPOCH FROM (now() - pg_last_xact_replay_timestamp()))::INT AS lag_seconds"
            }
            DatabaseType::MySQL => {
                // Returns Seconds_Behind_Master
                "SHOW SLAVE STATUS"
            }
            DatabaseType::MSSQL => {
                // Check AG synchronization state
                "SELECT datediff(s, last_commit_time, getdate()) AS lag_seconds \
                 FROM sys.dm_hadr_database_replica_states \
                 WHERE is_local = 1"
            }
            DatabaseType::SQLite => {
                // SQLite doesn't have replication
                "SELECT 0 AS lag_seconds"
            }
        }
    }

    /// Generate SQL to check if replica is primary.
    pub fn is_primary_sql(db_type: DatabaseType) -> &'static str {
        match db_type {
            DatabaseType::PostgreSQL => "SELECT NOT pg_is_in_recovery() AS is_primary",
            DatabaseType::MySQL => "SELECT @@read_only = 0 AS is_primary",
            DatabaseType::MSSQL => {
                "SELECT CASE WHEN role = 1 THEN 1 ELSE 0 END AS is_primary \
                 FROM sys.dm_hadr_availability_replica_states \
                 WHERE is_local = 1"
            }
            DatabaseType::SQLite => "SELECT 1 AS is_primary",
        }
    }

    /// Generate SQL to get replica status.
    pub fn replica_status_sql(db_type: DatabaseType) -> &'static str {
        match db_type {
            DatabaseType::PostgreSQL => {
                "SELECT \
                     pg_is_in_recovery() AS is_replica, \
                     pg_last_wal_receive_lsn() AS receive_lsn, \
                     pg_last_wal_replay_lsn() AS replay_lsn"
            }
            DatabaseType::MySQL => "SHOW REPLICA STATUS",
            DatabaseType::MSSQL => {
                "SELECT synchronization_state_desc, synchronization_health_desc \
                 FROM sys.dm_hadr_database_replica_states \
                 WHERE is_local = 1"
            }
            DatabaseType::SQLite => "SELECT 'primary' AS status",
        }
    }
}

// ============================================================================
// MongoDB Specific
// ============================================================================

/// MongoDB-specific replication types.
pub mod mongodb {
    use serde::{Deserialize, Serialize};
    use serde_json::Value as JsonValue;

    use super::ReadPreference;

    /// MongoDB read concern level.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ReadConcern {
        /// Read from local data (fastest, may be stale).
        Local,
        /// Read data committed to majority of nodes.
        Majority,
        /// Linearizable reads (strongest, slowest).
        Linearizable,
        /// Read data available at query start.
        Snapshot,
        /// Read available data (may return orphaned docs).
        Available,
    }

    impl ReadConcern {
        /// Convert to MongoDB string.
        pub fn as_str(&self) -> &'static str {
            match self {
                Self::Local => "local",
                Self::Majority => "majority",
                Self::Linearizable => "linearizable",
                Self::Snapshot => "snapshot",
                Self::Available => "available",
            }
        }
    }

    /// MongoDB write concern level.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub enum WriteConcern {
        /// Acknowledge after writing to primary.
        W1,
        /// Acknowledge after writing to majority.
        Majority,
        /// Acknowledge after writing to specific number of nodes.
        W(u32),
        /// Acknowledge after writing to specific tag set.
        Tag(String),
    }

    impl WriteConcern {
        /// Convert to MongoDB options.
        pub fn to_options(&self) -> JsonValue {
            match self {
                Self::W1 => serde_json::json!({ "w": 1 }),
                Self::Majority => serde_json::json!({ "w": "majority" }),
                Self::W(n) => serde_json::json!({ "w": n }),
                Self::Tag(tag) => serde_json::json!({ "w": tag }),
            }
        }
    }

    /// MongoDB read preference with options.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MongoReadPreference {
        /// Read preference mode.
        pub mode: ReadPreference,
        /// Maximum staleness in seconds.
        pub max_staleness_seconds: Option<u32>,
        /// Tag sets for filtering replicas.
        pub tag_sets: Vec<serde_json::Map<String, JsonValue>>,
        /// Hedge reads for sharded clusters.
        pub hedge: Option<bool>,
    }

    impl MongoReadPreference {
        /// Create a new read preference.
        pub fn new(mode: ReadPreference) -> Self {
            Self {
                mode,
                max_staleness_seconds: None,
                tag_sets: Vec::new(),
                hedge: None,
            }
        }

        /// Set max staleness.
        pub fn max_staleness(mut self, seconds: u32) -> Self {
            self.max_staleness_seconds = Some(seconds);
            self
        }

        /// Add a tag set.
        pub fn tag_set(mut self, tags: serde_json::Map<String, JsonValue>) -> Self {
            self.tag_sets.push(tags);
            self
        }

        /// Enable hedged reads.
        pub fn hedged(mut self) -> Self {
            self.hedge = Some(true);
            self
        }

        /// Convert to MongoDB connection string options.
        pub fn to_connection_options(&self) -> String {
            let mut opts = vec![format!("readPreference={}", self.mode.to_mongodb())];

            if let Some(staleness) = self.max_staleness_seconds {
                opts.push(format!("maxStalenessSeconds={}", staleness));
            }

            opts.join("&")
        }

        /// Convert to MongoDB command options.
        pub fn to_command_options(&self) -> JsonValue {
            let mut opts = serde_json::Map::new();
            opts.insert(
                "mode".to_string(),
                serde_json::json!(self.mode.to_mongodb()),
            );

            if let Some(staleness) = self.max_staleness_seconds {
                opts.insert(
                    "maxStalenessSeconds".to_string(),
                    serde_json::json!(staleness),
                );
            }

            if !self.tag_sets.is_empty() {
                opts.insert("tagSets".to_string(), serde_json::json!(self.tag_sets));
            }

            if let Some(hedge) = self.hedge {
                opts.insert("hedge".to_string(), serde_json::json!({ "enabled": hedge }));
            }

            serde_json::json!(opts)
        }
    }

    /// MongoDB replica set status (from replSetGetStatus).
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ReplicaSetStatus {
        /// Replica set name.
        pub set: String,
        /// Members.
        pub members: Vec<MemberStatus>,
    }

    /// Status of a replica set member.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MemberStatus {
        /// Member ID.
        pub id: u32,
        /// Member name (host:port).
        pub name: String,
        /// State (PRIMARY, SECONDARY, etc.).
        pub state_str: String,
        /// Health (1 = healthy).
        pub health: f64,
        /// Replication lag in seconds.
        #[serde(default)]
        pub lag_seconds: Option<i64>,
    }

    impl MemberStatus {
        /// Check if this member is primary.
        pub fn is_primary(&self) -> bool {
            self.state_str == "PRIMARY"
        }

        /// Check if this member is secondary.
        pub fn is_secondary(&self) -> bool {
            self.state_str == "SECONDARY"
        }

        /// Check if this member is healthy.
        pub fn is_healthy(&self) -> bool {
            self.health >= 1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replica_config() {
        let primary =
            ReplicaConfig::primary("pg1", "postgres://primary:5432/db").with_region("us-east-1");

        assert_eq!(primary.role, ReplicaRole::Primary);
        assert_eq!(primary.region.as_deref(), Some("us-east-1"));
    }

    #[test]
    fn test_replica_set_builder() {
        let config = ReplicaSetConfig::new("myapp")
            .primary("pg1", "postgres://primary:5432/db")
            .secondary("pg2", "postgres://secondary1:5432/db")
            .secondary("pg3", "postgres://secondary2:5432/db")
            .read_preference(ReadPreference::SecondaryPreferred)
            .build();

        assert_eq!(config.name, "myapp");
        assert_eq!(config.replicas.len(), 3);
        assert!(config.primary().is_some());
        assert_eq!(config.secondaries().count(), 2);
    }

    #[test]
    fn test_read_preference_mongodb() {
        assert_eq!(ReadPreference::Primary.to_mongodb(), "primary");
        assert_eq!(
            ReadPreference::SecondaryPreferred.to_mongodb(),
            "secondaryPreferred"
        );
        assert_eq!(ReadPreference::Nearest.to_mongodb(), "nearest");
    }

    #[test]
    fn test_connection_router_write() {
        let config = ReplicaSetConfig::new("test")
            .primary("pg1", "postgres://primary:5432/db")
            .secondary("pg2", "postgres://secondary:5432/db")
            .build();

        let mut router = ConnectionRouter::new(config);

        // Mark replicas as healthy
        router.update_health(
            "pg1",
            HealthStatus::Healthy,
            Some(Duration::from_millis(5)),
            None,
        );
        router.update_health(
            "pg2",
            HealthStatus::Healthy,
            Some(Duration::from_millis(10)),
            Some(Duration::from_secs(1)),
        );

        // Write should go to primary
        let target = router.route(QueryType::Write, None).unwrap();
        assert_eq!(target.id, "pg1");
    }

    #[test]
    fn test_connection_router_read_secondary() {
        let config = ReplicaSetConfig::new("test")
            .primary("pg1", "postgres://primary:5432/db")
            .secondary("pg2", "postgres://secondary:5432/db")
            .read_preference(ReadPreference::Secondary)
            .build();

        let mut router = ConnectionRouter::new(config);
        router.update_health(
            "pg1",
            HealthStatus::Healthy,
            Some(Duration::from_millis(5)),
            None,
        );
        router.update_health(
            "pg2",
            HealthStatus::Healthy,
            Some(Duration::from_millis(10)),
            Some(Duration::from_secs(1)),
        );

        // Read with Secondary preference should go to secondary
        let target = router.route(QueryType::Read, None).unwrap();
        assert_eq!(target.id, "pg2");
    }

    #[test]
    fn test_lag_monitor() {
        let mut monitor = LagMonitor::new(Duration::from_secs(10));

        monitor.record("pg2", Duration::from_secs(5));
        assert!(monitor.is_acceptable("pg2"));

        monitor.record("pg3", Duration::from_secs(15));
        assert!(!monitor.is_acceptable("pg3"));

        let lagging = monitor.get_lagging_replicas();
        assert_eq!(lagging, vec!["pg3"]);
    }

    #[test]
    fn test_failover() {
        let config = ReplicaSetConfig::new("test")
            .primary("pg1", "postgres://primary:5432/db")
            .replica(
                ReplicaConfig::secondary("pg2", "postgres://secondary1:5432/db").with_priority(80),
            )
            .replica(
                ReplicaConfig::secondary("pg3", "postgres://secondary2:5432/db").with_priority(60),
            )
            .build();

        let mut router = ConnectionRouter::new(config);
        router.update_health("pg1", HealthStatus::Unhealthy, None, None);
        router.update_health(
            "pg2",
            HealthStatus::Healthy,
            Some(Duration::from_millis(10)),
            None,
        );
        router.update_health(
            "pg3",
            HealthStatus::Healthy,
            Some(Duration::from_millis(15)),
            None,
        );

        let new_primary = router.initiate_failover().unwrap();
        assert_eq!(new_primary, "pg2"); // Higher priority
    }

    mod mongodb_tests {
        use super::super::mongodb::*;
        use super::*;

        #[test]
        fn test_read_concern() {
            assert_eq!(ReadConcern::Majority.as_str(), "majority");
            assert_eq!(ReadConcern::Local.as_str(), "local");
        }

        #[test]
        fn test_write_concern() {
            let w = WriteConcern::Majority;
            let opts = w.to_options();
            assert_eq!(opts["w"], "majority");

            let w2 = WriteConcern::W(3);
            let opts2 = w2.to_options();
            assert_eq!(opts2["w"], 3);
        }

        #[test]
        fn test_mongo_read_preference() {
            let pref = MongoReadPreference::new(ReadPreference::SecondaryPreferred)
                .max_staleness(90)
                .hedged();

            let conn_opts = pref.to_connection_options();
            assert!(conn_opts.contains("readPreference=secondaryPreferred"));
            assert!(conn_opts.contains("maxStalenessSeconds=90"));

            let cmd_opts = pref.to_command_options();
            assert_eq!(cmd_opts["mode"], "secondaryPreferred");
            assert_eq!(cmd_opts["maxStalenessSeconds"], 90);
        }
    }
}
