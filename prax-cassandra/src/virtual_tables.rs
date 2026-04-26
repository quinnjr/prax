//! Helpers for querying Cassandra 4.0+ virtual tables.
//!
//! Cassandra 4.0 introduced virtual tables in the `system_views` keyspace
//! that surface cluster metadata, metrics, and runtime state. This module
//! provides typed wrappers over the most useful ones.

use std::net::IpAddr;

use uuid::Uuid;

use crate::error::CassandraResult;
use crate::pool::CassandraPool;

/// Typed handle for querying virtual tables.
pub struct VirtualTables<'a> {
    #[allow(dead_code)]
    pool: &'a CassandraPool,
}

impl<'a> VirtualTables<'a> {
    /// Create a new handle.
    pub fn new(pool: &'a CassandraPool) -> Self {
        Self { pool }
    }

    /// Query `system.local` for cluster information.
    pub async fn cluster_info(&self) -> CassandraResult<ClusterInfo> {
        Err(crate::error::CassandraError::Query(
            "virtual_tables::cluster_info not yet wired".into(),
        ))
    }

    /// Query `system.peers_v2` for peer information.
    pub async fn peers(&self) -> CassandraResult<Vec<PeerInfo>> {
        Err(crate::error::CassandraError::Query(
            "virtual_tables::peers not yet wired".into(),
        ))
    }

    /// Query `system_views.settings` for runtime configuration.
    pub async fn settings(&self) -> CassandraResult<Vec<(String, String)>> {
        Err(crate::error::CassandraError::Query(
            "virtual_tables::settings not yet wired".into(),
        ))
    }
}

/// Basic cluster information (from `system.local`).
#[derive(Debug, Clone)]
pub struct ClusterInfo {
    /// Cluster name configured in cassandra.yaml.
    pub cluster_name: String,
    /// Partitioner class (e.g., "Murmur3Partitioner").
    pub partitioner: String,
    /// Cassandra release version.
    pub release_version: String,
}

/// Peer node information (from `system.peers_v2`).
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Peer IP address.
    pub peer: IpAddr,
    /// Data center name.
    pub data_center: String,
    /// Host identifier.
    pub host_id: Uuid,
    /// Rack name.
    pub rack: String,
    /// Release version reported by the peer.
    pub release_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_cluster_info_debug() {
        let ci = ClusterInfo {
            cluster_name: "Test Cluster".into(),
            partitioner: "Murmur3Partitioner".into(),
            release_version: "4.1.0".into(),
        };
        let dbg = format!("{:?}", ci);
        assert!(dbg.contains("Test Cluster"));
    }

    #[test]
    fn test_peer_info_construction() {
        let pi = PeerInfo {
            peer: IpAddr::from_str("192.168.1.1").unwrap(),
            data_center: "dc1".into(),
            host_id: Uuid::nil(),
            rack: "rack1".into(),
            release_version: "4.1.0".into(),
        };
        assert_eq!(pi.data_center, "dc1");
    }
}
