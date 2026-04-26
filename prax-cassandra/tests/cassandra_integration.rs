//! Integration tests against a live Cassandra cluster.
//!
//! These tests are gated behind the `cassandra-live` feature and require
//! a running Cassandra instance at `127.0.0.1:9042`. Run with:
//!
//! ```bash
//! cargo test -p prax-cassandra --features cassandra-live
//! ```

#![cfg(feature = "cassandra-live")]

use prax_cassandra::{CassandraConfig, CassandraPool};

#[tokio::test]
async fn test_connect_to_local_cluster() {
    let config = CassandraConfig::builder()
        .known_nodes(["127.0.0.1:9042".to_string()])
        .build();
    let pool = CassandraPool::connect(config).await;
    assert!(
        pool.is_ok(),
        "expected to connect to local Cassandra: {:?}",
        pool.err()
    );
}
