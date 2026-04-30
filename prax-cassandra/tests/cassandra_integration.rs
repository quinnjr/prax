//! Integration / E2E tests for prax-cassandra against a live Cassandra
//! cluster.
//!
//! Gated behind the `cassandra-live` feature. The docker-compose
//! `test-cassandra` runner sets `PRAX_E2E=1` and
//! `CASSANDRA_URL=cassandra://localhost:9043/prax_test` (Cassandra runs
//! on 9043 so it doesn't collide with ScyllaDB on 9042).
//!
//! ```sh
//! docker compose up -d cassandra
//! docker compose run --rm test-cassandra
//! ```
//!
//! ## Coverage
//!
//! - `e2e_pool_connect_succeeds` opens a real cdrs-tokio session
//!   against the docker-compose Cassandra container and pings it
//!   with `SELECT now() FROM system.local` via [`CassandraConnection::ping`].
//! - `e2e_cluster_is_reachable` raw TCP probe against the native-
//!   transport port, kept as a fast-failing sanity check.

#![cfg(feature = "cassandra-live")]

use std::net::ToSocketAddrs;
use std::time::Duration;

use prax_cassandra::{CassandraConfig, CassandraPool};

/// Return the contact point only when e2e is explicitly enabled AND a
/// URL is supplied. CI sets `PRAX_E2E=1` for the postgres/mysql/mssql
/// suites but does NOT start a Cassandra service; without this gate,
/// the tests would fall through to a default `localhost:9043` and
/// stall on cdrs-tokio's no-timeout connection retry loop. The
/// docker-compose `test-cassandra` runner supplies both env vars.
fn cassandra_contact_point() -> Option<(String, u16)> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    let url = std::env::var("CASSANDRA_URL").ok()?;
    let rest = url
        .strip_prefix("cassandra://")
        .expect("CASSANDRA_URL must start with cassandra://");
    let (host_port, _keyspace) = rest.split_once('/').unwrap_or((rest, "prax_test"));
    let (host, port) = host_port.split_once(':').unwrap_or((host_port, "9042"));
    Some((host.to_string(), port.parse().expect("valid port")))
}

#[tokio::test]
#[ignore = "requires running Cassandra via docker-compose"]
async fn e2e_pool_connect_succeeds() {
    let Some((host, port)) = cassandra_contact_point() else {
        return;
    };
    let config = CassandraConfig::builder()
        .known_nodes([format!("{host}:{port}")])
        .build();
    let pool = CassandraPool::connect(config)
        .await
        .expect("connect to docker-compose cassandra");
    pool.connection()
        .ping()
        .await
        .expect("ping `SELECT now() FROM system.local`");
}

/// Confirm the docker-compose Cassandra container is actually listening
/// on the native-transport port. We don't speak the binary CQL protocol
/// here — the goal is just to prove the service is reachable so that
/// the day `prax-cassandra`'s engine is wired up, the compose harness
/// is already known-good.
#[tokio::test]
#[ignore = "requires running Cassandra via docker-compose"]
async fn e2e_cluster_is_reachable() {
    let Some((host, port)) = cassandra_contact_point() else {
        return;
    };
    let addrs: Vec<_> = format!("{host}:{port}")
        .to_socket_addrs()
        .expect("resolve")
        .collect();
    assert!(!addrs.is_empty(), "no resolved addresses for {host}:{port}");

    let connect = tokio::net::TcpStream::connect(&addrs[0]);
    let stream = tokio::time::timeout(Duration::from_secs(10), connect)
        .await
        .expect("tcp connect did not time out")
        .unwrap_or_else(|e| panic!("failed to connect to {host}:{port}: {e}"));
    drop(stream);
}
