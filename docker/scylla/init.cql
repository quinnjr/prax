-- =============================================================================
-- Prax ORM - ScyllaDB Initialization Script
-- =============================================================================
-- Executed by the healthcheck-gated bootstrap sidecar after the node is up.
-- Keeps schema bootstrap out of the test code so each integration test can
-- assume the `prax_test` keyspace exists and is empty-per-run (tests DROP+
-- CREATE their own tables).

CREATE KEYSPACE IF NOT EXISTS prax_test
  WITH REPLICATION = { 'class': 'SimpleStrategy', 'replication_factor': 1 };
