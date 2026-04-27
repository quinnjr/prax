-- =============================================================================
-- Prax ORM - Cassandra Initialization Script
-- =============================================================================
-- Executed by the healthcheck-gated bootstrap sidecar after the node is up.
-- Cassandra runs on native transport port 9043 in this compose (9042 is
-- reserved for ScyllaDB since both clusters share the host network).

CREATE KEYSPACE IF NOT EXISTS prax_test
  WITH REPLICATION = { 'class': 'SimpleStrategy', 'replication_factor': 1 };
