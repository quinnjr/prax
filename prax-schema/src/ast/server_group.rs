//! Server group definitions for multi-server database configurations.
//!
//! Server groups allow organizing multiple database servers for:
//! - Read replicas (load balancing reads)
//! - Sharding (horizontal scaling)
//! - Multi-region deployment
//! - Failover and high availability

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Attribute, Documentation, Ident, Span};

/// A server group containing multiple database servers.
///
/// # Example
/// ```prax
/// serverGroup MainCluster {
///     @@strategy(ReadReplica)
///     @@loadBalance(RoundRobin)
///
///     server primary {
///         url = env("PRIMARY_DATABASE_URL")
///         role = "primary"
///         weight = 1
///     }
///
///     server replica1 {
///         url = env("REPLICA1_DATABASE_URL")
///         role = "replica"
///         weight = 2
///         region = "us-east-1"
///     }
///
///     server replica2 {
///         url = env("REPLICA2_DATABASE_URL")
///         role = "replica"
///         weight = 2
///         region = "us-west-2"
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerGroup {
    /// Group name.
    pub name: Ident,
    /// Servers in this group.
    pub servers: IndexMap<SmolStr, Server>,
    /// Group-level attributes (strategy, load balancing, etc.).
    pub attributes: Vec<Attribute>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
}

impl ServerGroup {
    /// Create a new server group.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            servers: IndexMap::new(),
            attributes: vec![],
            documentation: None,
            span,
        }
    }

    /// Add a server to the group.
    pub fn add_server(&mut self, server: Server) {
        self.servers.insert(server.name.name.clone(), server);
    }

    /// Add an attribute to the group.
    pub fn add_attribute(&mut self, attr: Attribute) {
        self.attributes.push(attr);
    }

    /// Set documentation.
    pub fn set_documentation(&mut self, doc: Documentation) {
        self.documentation = Some(doc);
    }

    /// Get the group strategy (e.g., ReadReplica, Sharding, MultiRegion).
    pub fn strategy(&self) -> Option<ServerGroupStrategy> {
        for attr in &self.attributes {
            if attr.name.name == "strategy"
                && let Some(arg) = attr.args.first()
            {
                let value_str = arg
                    .value
                    .as_string()
                    .map(|s| s.to_string())
                    .or_else(|| arg.value.as_ident().map(|s| s.to_string()))?;
                return ServerGroupStrategy::parse(&value_str);
            }
        }
        None
    }

    /// Get the load balancing strategy.
    pub fn load_balance(&self) -> Option<LoadBalanceStrategy> {
        for attr in &self.attributes {
            if attr.name.name == "loadBalance"
                && let Some(arg) = attr.args.first()
            {
                let value_str = arg
                    .value
                    .as_string()
                    .map(|s| s.to_string())
                    .or_else(|| arg.value.as_ident().map(|s| s.to_string()))?;
                return LoadBalanceStrategy::parse(&value_str);
            }
        }
        None
    }

    /// Get the primary server.
    pub fn primary(&self) -> Option<&Server> {
        self.servers
            .values()
            .find(|s| s.role() == Some(ServerRole::Primary))
    }

    /// Get all replica servers.
    pub fn replicas(&self) -> Vec<&Server> {
        self.servers
            .values()
            .filter(|s| s.role() == Some(ServerRole::Replica))
            .collect()
    }

    /// Get servers by region.
    pub fn servers_in_region(&self, region: &str) -> Vec<&Server> {
        self.servers
            .values()
            .filter(|s| s.region() == Some(region))
            .collect()
    }

    /// Get the failover order (sorted by priority).
    pub fn failover_order(&self) -> Vec<&Server> {
        let mut servers: Vec<_> = self.servers.values().collect();
        servers.sort_by_key(|s| s.priority().unwrap_or(u32::MAX));
        servers
    }
}

/// An individual server within a server group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    /// Server name.
    pub name: Ident,
    /// Server properties (url, role, weight, etc.).
    pub properties: IndexMap<SmolStr, ServerProperty>,
    /// Source location.
    pub span: Span,
}

impl Server {
    /// Create a new server.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            properties: IndexMap::new(),
            span,
        }
    }

    /// Add a property to the server.
    pub fn add_property(&mut self, prop: ServerProperty) {
        self.properties.insert(prop.name.clone(), prop);
    }

    /// Get a property value by name.
    pub fn get_property(&self, name: &str) -> Option<&ServerPropertyValue> {
        self.properties.get(name).map(|p| &p.value)
    }

    /// Get the server URL.
    pub fn url(&self) -> Option<&str> {
        match self.get_property("url")? {
            ServerPropertyValue::String(s) => Some(s),
            ServerPropertyValue::EnvVar(var) => Some(var),
            _ => None,
        }
    }

    /// Get the server role.
    pub fn role(&self) -> Option<ServerRole> {
        match self.get_property("role")? {
            ServerPropertyValue::String(s) | ServerPropertyValue::Identifier(s) => {
                ServerRole::parse(s)
            }
            _ => None,
        }
    }

    /// Get the server weight (for load balancing).
    pub fn weight(&self) -> Option<u32> {
        match self.get_property("weight")? {
            ServerPropertyValue::Number(n) => Some(*n as u32),
            _ => None,
        }
    }

    /// Get the server region.
    pub fn region(&self) -> Option<&str> {
        match self.get_property("region")? {
            ServerPropertyValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get the server priority (for failover).
    pub fn priority(&self) -> Option<u32> {
        match self.get_property("priority")? {
            ServerPropertyValue::Number(n) => Some(*n as u32),
            _ => None,
        }
    }

    /// Check if this is a read-only server.
    pub fn is_read_only(&self) -> bool {
        match self.get_property("readOnly") {
            Some(ServerPropertyValue::Boolean(b)) => *b,
            _ => self.role() == Some(ServerRole::Replica),
        }
    }

    /// Get the maximum connections for this server.
    pub fn max_connections(&self) -> Option<u32> {
        match self.get_property("maxConnections")? {
            ServerPropertyValue::Number(n) => Some(*n as u32),
            _ => None,
        }
    }

    /// Get the health check endpoint.
    pub fn health_check(&self) -> Option<&str> {
        match self.get_property("healthCheck")? {
            ServerPropertyValue::String(s) => Some(s),
            _ => None,
        }
    }
}

/// A server property key-value pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerProperty {
    /// Property name.
    pub name: SmolStr,
    /// Property value.
    pub value: ServerPropertyValue,
    /// Source location.
    pub span: Span,
}

impl ServerProperty {
    /// Create a new server property.
    pub fn new(name: impl Into<SmolStr>, value: ServerPropertyValue, span: Span) -> Self {
        Self {
            name: name.into(),
            value,
            span,
        }
    }
}

/// Server property value types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerPropertyValue {
    /// String value.
    String(String),
    /// Number value.
    Number(f64),
    /// Boolean value.
    Boolean(bool),
    /// Identifier (enum-like value).
    Identifier(String),
    /// Environment variable reference.
    EnvVar(String),
    /// Array of values.
    Array(Vec<ServerPropertyValue>),
}

impl std::fmt::Display for ServerPropertyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => write!(f, "\"{}\"", s),
            Self::Number(n) => write!(f, "{}", n),
            Self::Boolean(b) => write!(f, "{}", b),
            Self::Identifier(s) => write!(f, "{}", s),
            Self::EnvVar(var) => write!(f, "env(\"{}\")", var),
            Self::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
        }
    }
}

/// Server role within a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerRole {
    /// Primary/master server (handles writes).
    Primary,
    /// Replica/slave server (handles reads).
    Replica,
    /// Analytics server (for reporting queries).
    Analytics,
    /// Archive server (for historical data).
    Archive,
    /// Shard server (for horizontal partitioning).
    Shard,
}

impl ServerRole {
    /// Parse role from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "primary" | "master" | "writer" => Some(Self::Primary),
            "replica" | "slave" | "reader" | "read" => Some(Self::Replica),
            "analytics" | "reporting" | "olap" => Some(Self::Analytics),
            "archive" | "historical" => Some(Self::Archive),
            "shard" => Some(Self::Shard),
            _ => None,
        }
    }

    /// Get the role as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Replica => "replica",
            Self::Analytics => "analytics",
            Self::Archive => "archive",
            Self::Shard => "shard",
        }
    }
}

/// Server group strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerGroupStrategy {
    /// Read replica configuration (primary + replicas).
    ReadReplica,
    /// Sharding configuration (horizontal partitioning).
    Sharding,
    /// Multi-region deployment.
    MultiRegion,
    /// High availability with automatic failover.
    HighAvailability,
    /// Custom strategy.
    Custom,
}

impl ServerGroupStrategy {
    /// Parse strategy from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "readreplica" | "replication" => Some(Self::ReadReplica),
            "sharding" | "shard" | "partition" => Some(Self::Sharding),
            "multiregion" | "georeplica" | "geographic" => Some(Self::MultiRegion),
            "highavailability" | "ha" | "failover" => Some(Self::HighAvailability),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    /// Get the strategy as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadReplica => "ReadReplica",
            Self::Sharding => "Sharding",
            Self::MultiRegion => "MultiRegion",
            Self::HighAvailability => "HighAvailability",
            Self::Custom => "Custom",
        }
    }
}

/// Load balancing strategy for distributing queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadBalanceStrategy {
    /// Round-robin distribution.
    RoundRobin,
    /// Random selection.
    Random,
    /// Least connections.
    LeastConnections,
    /// Weighted distribution based on server weights.
    Weighted,
    /// Route to nearest (by latency or region).
    Nearest,
    /// Sticky sessions (same client to same server).
    Sticky,
}

impl LoadBalanceStrategy {
    /// Parse strategy from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "roundrobin" | "rr" => Some(Self::RoundRobin),
            "random" | "rand" => Some(Self::Random),
            "leastconnections" | "leastconn" | "least" => Some(Self::LeastConnections),
            "weighted" | "weight" => Some(Self::Weighted),
            "nearest" | "latency" | "geo" => Some(Self::Nearest),
            "sticky" | "affinity" | "session" => Some(Self::Sticky),
            _ => None,
        }
    }

    /// Get the strategy as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "RoundRobin",
            Self::Random => "Random",
            Self::LeastConnections => "LeastConnections",
            Self::Weighted => "Weighted",
            Self::Nearest => "Nearest",
            Self::Sticky => "Sticky",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_role_from_str() {
        assert_eq!(ServerRole::parse("primary"), Some(ServerRole::Primary));
        assert_eq!(ServerRole::parse("master"), Some(ServerRole::Primary));
        assert_eq!(ServerRole::parse("replica"), Some(ServerRole::Replica));
        assert_eq!(ServerRole::parse("slave"), Some(ServerRole::Replica));
        assert_eq!(ServerRole::parse("analytics"), Some(ServerRole::Analytics));
        assert_eq!(ServerRole::parse("shard"), Some(ServerRole::Shard));
        assert_eq!(ServerRole::parse("invalid"), None);
    }

    #[test]
    fn test_server_group_strategy_from_str() {
        assert_eq!(
            ServerGroupStrategy::parse("ReadReplica"),
            Some(ServerGroupStrategy::ReadReplica)
        );
        assert_eq!(
            ServerGroupStrategy::parse("sharding"),
            Some(ServerGroupStrategy::Sharding)
        );
        assert_eq!(
            ServerGroupStrategy::parse("multi-region"),
            Some(ServerGroupStrategy::MultiRegion)
        );
        assert_eq!(
            ServerGroupStrategy::parse("HA"),
            Some(ServerGroupStrategy::HighAvailability)
        );
    }

    #[test]
    fn test_load_balance_strategy_from_str() {
        assert_eq!(
            LoadBalanceStrategy::parse("RoundRobin"),
            Some(LoadBalanceStrategy::RoundRobin)
        );
        assert_eq!(
            LoadBalanceStrategy::parse("rr"),
            Some(LoadBalanceStrategy::RoundRobin)
        );
        assert_eq!(
            LoadBalanceStrategy::parse("weighted"),
            Some(LoadBalanceStrategy::Weighted)
        );
        assert_eq!(
            LoadBalanceStrategy::parse("nearest"),
            Some(LoadBalanceStrategy::Nearest)
        );
    }

    #[test]
    fn test_server_property_value_display() {
        assert_eq!(
            ServerPropertyValue::String("test".to_string()).to_string(),
            "\"test\""
        );
        assert_eq!(ServerPropertyValue::Number(42.0).to_string(), "42");
        assert_eq!(ServerPropertyValue::Boolean(true).to_string(), "true");
        assert_eq!(
            ServerPropertyValue::Identifier("primary".to_string()).to_string(),
            "primary"
        );
        assert_eq!(
            ServerPropertyValue::EnvVar("DATABASE_URL".to_string()).to_string(),
            "env(\"DATABASE_URL\")"
        );
    }

    fn test_span() -> Span {
        Span::new(0, 0)
    }

    #[test]
    fn test_server_group_primary_and_replicas() {
        let mut group = ServerGroup::new(Ident::new("TestCluster", test_span()), test_span());

        let mut primary = Server::new(Ident::new("primary", test_span()), test_span());
        primary.add_property(ServerProperty::new(
            "role",
            ServerPropertyValue::Identifier("primary".to_string()),
            test_span(),
        ));
        group.add_server(primary);

        let mut replica1 = Server::new(Ident::new("replica1", test_span()), test_span());
        replica1.add_property(ServerProperty::new(
            "role",
            ServerPropertyValue::Identifier("replica".to_string()),
            test_span(),
        ));
        group.add_server(replica1);

        let mut replica2 = Server::new(Ident::new("replica2", test_span()), test_span());
        replica2.add_property(ServerProperty::new(
            "role",
            ServerPropertyValue::Identifier("replica".to_string()),
            test_span(),
        ));
        group.add_server(replica2);

        assert!(group.primary().is_some());
        assert_eq!(group.primary().unwrap().name.name.as_str(), "primary");
        assert_eq!(group.replicas().len(), 2);
    }
}
