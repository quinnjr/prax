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
        use cdrs_tokio::types::ByName;
        let result = self
            .pool
            .query("SELECT cluster_name, partitioner, release_version FROM system.local")
            .await?;
        let cdrs_row = result
            .rows
            .iter()
            .filter_map(|r| r.as_cdrs())
            .next()
            .ok_or_else(|| {
                crate::error::CassandraError::Query("system.local returned no row".into())
            })?;
        Ok(ClusterInfo {
            cluster_name: cdrs_row
                .by_name::<String>("cluster_name")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default(),
            partitioner: cdrs_row
                .by_name::<String>("partitioner")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default(),
            release_version: cdrs_row
                .by_name::<String>("release_version")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default(),
        })
    }

    /// Query `system.peers_v2` for peer information. Falls back to the
    /// legacy `system.peers` table on clusters that don't yet expose
    /// the v2 virtual table.
    pub async fn peers(&self) -> CassandraResult<Vec<PeerInfo>> {
        use cdrs_tokio::types::ByName;
        let result = match self
            .pool
            .query("SELECT peer, data_center, host_id, rack, release_version FROM system.peers_v2")
            .await
        {
            Ok(r) => r,
            Err(_) => {
                // Legacy Cassandra clusters only have `system.peers`.
                self.pool
                    .query(
                        "SELECT peer, data_center, host_id, rack, release_version FROM system.peers",
                    )
                    .await?
            }
        };
        let mut peers = Vec::with_capacity(result.rows.len());
        for row in result.rows.iter().filter_map(|r| r.as_cdrs()) {
            let peer = row
                .by_name::<IpAddr>("peer")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .ok_or_else(|| {
                    crate::error::CassandraError::Query("peer column was null".into())
                })?;
            let data_center = row
                .by_name::<String>("data_center")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default();
            let host_id = row
                .by_name::<Uuid>("host_id")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_else(Uuid::nil);
            let rack = row
                .by_name::<String>("rack")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default();
            let release_version = row
                .by_name::<String>("release_version")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default();
            peers.push(PeerInfo {
                peer,
                data_center,
                host_id,
                rack,
                release_version,
            });
        }
        Ok(peers)
    }

    /// Query `system_views.settings` for runtime configuration.
    pub async fn settings(&self) -> CassandraResult<Vec<(String, String)>> {
        use cdrs_tokio::types::ByName;
        let result = self
            .pool
            .query("SELECT name, value FROM system_views.settings")
            .await?;
        let mut out = Vec::with_capacity(result.rows.len());
        for row in result.rows.iter().filter_map(|r| r.as_cdrs()) {
            let name = row
                .by_name::<String>("name")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default();
            let value = row
                .by_name::<String>("value")
                .map_err(|e| crate::error::CassandraError::Query(e.to_string()))?
                .unwrap_or_default();
            out.push((name, value));
        }
        Ok(out)
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
