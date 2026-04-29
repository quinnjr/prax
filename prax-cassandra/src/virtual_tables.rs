//! Helpers for querying Cassandra 4.0+ virtual tables.
//!
//! Cassandra 4.0 introduced virtual tables in the `system_views` keyspace
//! that surface cluster metadata, metrics, and runtime state. This module
//! provides typed wrappers over the most useful ones.

use std::net::IpAddr;

use cdrs_tokio::types::rows::Row as CdrsRow;
use cdrs_tokio::types::{ByName, IntoRustByName};
use uuid::Uuid;

use crate::error::{CassandraError, CassandraResult};
use crate::pool::CassandraPool;

/// Read a column as `T`, collapsing NULL to `T::default()` and mapping
/// the cdrs error into our [`CassandraError::Query`]. Used by the
/// virtual-table readers where "column missing or NULL" is equivalent
/// to "unset".
fn col_or_default<T>(row: &CdrsRow, name: &str) -> CassandraResult<T>
where
    T: Default,
    CdrsRow: IntoRustByName<T>,
{
    Ok(ByName::by_name::<T>(row, name)
        .map_err(|e| CassandraError::Query(e.to_string()))?
        .unwrap_or_default())
}

/// Typed handle for querying virtual tables.
pub struct VirtualTables<'a> {
    pool: &'a CassandraPool,
}

impl<'a> VirtualTables<'a> {
    /// Create a new handle.
    pub fn new(pool: &'a CassandraPool) -> Self {
        Self { pool }
    }

    /// Query `system.local` for cluster information.
    pub async fn cluster_info(&self) -> CassandraResult<ClusterInfo> {
        let result = self
            .pool
            .query("SELECT cluster_name, partitioner, release_version FROM system.local")
            .await?;
        let row = result
            .rows
            .first()
            .map(|r| r.as_cdrs())
            .ok_or_else(|| CassandraError::Query("system.local returned no row".into()))?;
        Ok(ClusterInfo {
            cluster_name: col_or_default(row, "cluster_name")?,
            partitioner: col_or_default(row, "partitioner")?,
            release_version: col_or_default(row, "release_version")?,
        })
    }

    /// Query `system.peers_v2` for peer information. Falls back to the
    /// legacy `system.peers` table on clusters that don't yet expose
    /// the v2 virtual table.
    pub async fn peers(&self) -> CassandraResult<Vec<PeerInfo>> {
        let result = match self
            .pool
            .query("SELECT peer, data_center, host_id, rack, release_version FROM system.peers_v2")
            .await
        {
            Ok(r) => r,
            // Legacy clusters only have `system.peers`.
            Err(_) => self
                .pool
                .query("SELECT peer, data_center, host_id, rack, release_version FROM system.peers")
                .await?,
        };
        result
            .rows
            .iter()
            .map(|r| r.as_cdrs())
            .map(|row| {
                let peer = ByName::by_name::<IpAddr>(row, "peer")
                    .map_err(|e| CassandraError::Query(e.to_string()))?
                    .ok_or_else(|| CassandraError::Query("peer column was null".into()))?;
                Ok(PeerInfo {
                    peer,
                    data_center: col_or_default(row, "data_center")?,
                    host_id: col_or_default::<Uuid>(row, "host_id")?,
                    rack: col_or_default(row, "rack")?,
                    release_version: col_or_default(row, "release_version")?,
                })
            })
            .collect()
    }

    /// Query `system_views.settings` for runtime configuration.
    pub async fn settings(&self) -> CassandraResult<Vec<(String, String)>> {
        let result = self
            .pool
            .query("SELECT name, value FROM system_views.settings")
            .await?;
        result
            .rows
            .iter()
            .map(|r| r.as_cdrs())
            .map(|row| {
                Ok((
                    col_or_default::<String>(row, "name")?,
                    col_or_default::<String>(row, "value")?,
                ))
            })
            .collect()
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
