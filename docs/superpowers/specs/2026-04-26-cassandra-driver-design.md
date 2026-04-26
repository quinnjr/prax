# prax-cassandra Driver Design

**Date:** 2026-04-26  
**Status:** Approved  
**Type:** New Sub-Crate

## Overview

Add `prax-cassandra`, a new workspace member providing a pure-Rust async Apache Cassandra driver for the Prax ORM. Structurally parallel to `prax-scylladb` but uses `cdrs-tokio` as its underlying driver, remaining independent of the Scylla-specific `scylla` crate.

The existing `CqlDialect` in `prax-migrate` already supports Cassandra unchanged (same CQL dialect), so migrations work without additional code.

## Goals

1. **Pure-Rust Cassandra driver** via `cdrs-tokio` — no C++ FFI, no system libraries
2. **API parity** with `prax-scylladb` for CRUD, batches, LWT, paging
3. **Production-ready auth** — password, SSL/TLS, SASL framework for LDAP/Kerberos extension
4. **Cassandra-specific features** — virtual tables (4.0+), UDFs/UDAs, experimental materialized views
5. **No regression** in existing workspace crates
6. **Reuse CqlDialect** for migrations — no new migration code needed

## Non-Goals

- Sharing code with `prax-scylladb` (different underlying drivers)
- Live Cassandra cluster in CI (integration tests opt-in via feature flag)
- Full SASL plugin implementations (LDAP, Kerberos) — framework only, concrete plugins future work
- Cassandra-to-Scylla migration tooling

## Background

Apache Cassandra and ScyllaDB share the CQL protocol, so migrations written for one work on the other. However, the Rust driver ecosystem splits them:

- **`scylla`** (DataStax Rust driver) — Scylla-optimized, also works against Cassandra but not idiomatic for Cassandra-specific features
- **`cdrs-tokio`** — Pure-Rust, Tokio-based, maintained fork of CDRS, Cassandra-idiomatic
- **`cassandra-cpp`** — Rust bindings over DataStax C++ driver; requires system library, FFI overhead
- **`cassandra-protocol`** — Low-level CQL protocol only, requires building session/pool manually

`cdrs-tokio` is the pragmatic choice: pure-Rust, tokio-native, active maintenance, Cassandra-idiomatic.

## Architecture

### Crate Layout

```
prax-cassandra/
├── Cargo.toml
├── README.md
└── src/
    ├── lib.rs           # module entry + re-exports
    ├── config.rs        # CassandraConfig builder
    ├── connection.rs    # session wrapper + health checks
    ├── pool.rs          # CassandraPool
    ├── engine.rs        # query/execute/batch/LWT/paging
    ├── row.rs           # FromRow trait + row extraction
    ├── types.rs         # CQL type conversions
    ├── error.rs         # CassandraError
    ├── auth.rs          # SASL framework + PlainSasl
    ├── virtual_tables.rs# Cassandra 4.0+ system.* helpers
    └── udf.rs           # UDF/UDA management
```

### Workspace Integration

**In root `Cargo.toml`:**

1. Add to `[workspace] members`:
   ```toml
   "prax-cassandra",
   ```

2. Add to `[workspace.dependencies]`:
   ```toml
   prax-cassandra = { path = "prax-cassandra", version = "0.7.2" }
   cdrs-tokio = "9.0"  # or latest
   ```

### Driver Choice (Decision Q2 → A)

`cdrs-tokio` selected for:
- Pure Rust (no C++ FFI)
- Active maintenance (maintained fork of original CDRS)
- Tokio-native
- Cassandra-idiomatic API

Rejected alternatives:
- `cassandra-cpp` — FFI overhead, system library dependency
- `cassandra-protocol` + custom layer — too much low-level work
- Reuse `scylla` crate — misses Cassandra-specific idioms

## Components

### CassandraConfig (`config.rs`)

Builder-pattern config mirroring `ScyllaConfig`:

```rust
#[derive(Debug, Clone)]
pub struct CassandraConfig {
    pub known_nodes: Vec<String>,
    pub default_keyspace: Option<String>,
    pub auth: Option<CassandraAuth>,
    pub tls: Option<TlsConfig>,
    pub pool_size: usize,
    pub connection_timeout: Duration,
    pub request_timeout: Duration,
    pub consistency: Consistency,
    pub retry_policy: RetryPolicyKind,
}

impl CassandraConfig {
    pub fn builder() -> CassandraConfigBuilder { ... }
}

#[derive(Debug, Clone)]
pub enum CassandraAuth {
    Password {
        username: String,
        password: String,
    },
    Sasl(Arc<dyn SaslMechanism>),
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub ca_cert: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub verify_hostname: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Consistency {
    Any,
    One,
    Two,
    Three,
    Quorum,
    All,
    LocalQuorum,
    EachQuorum,
    LocalOne,
    Serial,
    LocalSerial,
}

#[derive(Debug, Clone)]
pub enum RetryPolicyKind {
    Default,
    Downgrading,
    Never,
}
```

### Auth (`auth.rs`) (Decision Q3 → C)

```rust
#[async_trait::async_trait]
pub trait SaslMechanism: Send + Sync + std::fmt::Debug {
    /// Mechanism name, e.g., "PLAIN", "GSSAPI".
    fn name(&self) -> &str;

    /// Generate the initial authentication response.
    async fn initial_response(&self) -> CassandraResult<Vec<u8>>;

    /// Respond to a SASL challenge from the server.
    async fn evaluate(&self, challenge: &[u8]) -> CassandraResult<Vec<u8>>;
}

/// PLAIN SASL mechanism (username + password).
#[derive(Debug, Clone)]
pub struct PlainSasl {
    pub username: String,
    pub password: String,
}

#[async_trait::async_trait]
impl SaslMechanism for PlainSasl {
    fn name(&self) -> &str { "PLAIN" }

    async fn initial_response(&self) -> CassandraResult<Vec<u8>> {
        let mut buf = Vec::new();
        buf.push(0);
        buf.extend_from_slice(self.username.as_bytes());
        buf.push(0);
        buf.extend_from_slice(self.password.as_bytes());
        Ok(buf)
    }

    async fn evaluate(&self, _challenge: &[u8]) -> CassandraResult<Vec<u8>> {
        // PLAIN completes in one round
        Ok(Vec::new())
    }
}
```

LDAP/Kerberos implementations are follow-up crates; this trait is the extension point.

### Connection (`connection.rs`)

Thin wrapper over the `cdrs_tokio::cluster::session::Session`:

```rust
pub struct CassandraConnection {
    session: Arc<Session<...>>,  // cdrs-tokio session type
    config: CassandraConfig,
}

impl CassandraConnection {
    pub async fn connect(config: CassandraConfig) -> CassandraResult<Self> { ... }
    pub async fn ping(&self) -> CassandraResult<()> {
        // SELECT now() FROM system.local
    }
    pub fn session(&self) -> &Session<...> { &self.session }
}
```

### CassandraPool (`pool.rs`)

```rust
pub struct CassandraPool {
    connection: Arc<CassandraConnection>,
}

impl CassandraPool {
    pub async fn connect(config: CassandraConfig) -> CassandraResult<Self> { ... }
    pub async fn close(self) -> CassandraResult<()> { ... }
    pub fn connection(&self) -> &CassandraConnection { &self.connection }
}
```

cdrs-tokio manages its internal connection pool; `CassandraPool` is primarily the public handle + session lifecycle manager.

### Query Engine (`engine.rs`)

```rust
impl CassandraPool {
    /// Execute a query expecting rows.
    pub async fn query(
        &self,
        cql: &str,
        values: impl QueryValues + 'static,
    ) -> CassandraResult<QueryResult>;

    /// Execute a query not expecting rows (INSERT, UPDATE, DELETE, DDL).
    pub async fn execute(
        &self,
        cql: &str,
        values: impl QueryValues + 'static,
    ) -> CassandraResult<()>;

    /// Query a single row, deserialized.
    pub async fn query_one<T: FromRow>(
        &self,
        cql: &str,
        values: impl QueryValues + 'static,
    ) -> CassandraResult<T>;

    /// Query many rows, deserialized.
    pub async fn query_many<T: FromRow>(
        &self,
        cql: &str,
        values: impl QueryValues + 'static,
    ) -> CassandraResult<Vec<T>>;

    /// Execute a lightweight transaction; returns whether the CAS succeeded.
    pub async fn execute_lwt(
        &self,
        cql: &str,
        values: impl QueryValues + 'static,
    ) -> CassandraResult<bool>;

    /// Build a batch of statements.
    pub fn batch(&self) -> BatchBuilder;

    /// Stream a paged query.
    pub fn page(&self, cql: &str, page_size: i32) -> PagedStream;
}

pub struct BatchBuilder { ... }

impl BatchBuilder {
    pub fn add(mut self, cql: &str, values: impl QueryValues + 'static) -> Self;
    pub async fn execute(self) -> CassandraResult<()>;
    pub async fn execute_logged(self) -> CassandraResult<()>;
    pub async fn execute_unlogged(self) -> CassandraResult<()>;
    pub async fn execute_counter(self) -> CassandraResult<()>;
}

pub struct PagedStream { ... }

impl futures::Stream for PagedStream { ... }
```

### Row & Types (`row.rs`, `types.rs`)

`FromRow` trait for deserializing rows:

```rust
pub trait FromRow: Sized {
    fn from_row(row: &Row) -> CassandraResult<Self>;
}
```

Implementations for common types: String, i64, i32, Uuid, DateTime<Utc>, bool, f64, Vec<u8>, Option<T>, Vec<T>.

### Error (`error.rs`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum CassandraError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Query execution failed: {0}")]
    Query(String),

    #[error("Row deserialization failed: {0}")]
    Deserialization(String),

    #[error("Timeout after {duration:?}: {operation}")]
    Timeout { operation: String, duration: Duration },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Lightweight transaction not applied")]
    LwtNotApplied,

    #[error("Driver error: {0}")]
    Driver(#[from] cdrs_tokio::error::Error),
}

pub type CassandraResult<T> = Result<T, CassandraError>;
```

### Virtual Tables (`virtual_tables.rs`) (Decision Q4 → A)

Cassandra 4.0+ exposes diagnostic data via virtual tables:

```rust
pub struct VirtualTables<'a> {
    pool: &'a CassandraPool,
}

impl<'a> VirtualTables<'a> {
    pub async fn cluster_info(&self) -> CassandraResult<ClusterInfo>;
    pub async fn peers(&self) -> CassandraResult<Vec<PeerInfo>>;
    pub async fn settings(&self) -> CassandraResult<HashMap<String, String>>;
    pub async fn caches(&self) -> CassandraResult<Vec<CacheInfo>>;
    pub async fn clients(&self) -> CassandraResult<Vec<ClientInfo>>;
}

pub struct ClusterInfo {
    pub cluster_name: String,
    pub partitioner: String,
    pub release_version: String,
}

pub struct PeerInfo {
    pub peer: IpAddr,
    pub data_center: String,
    pub host_id: Uuid,
    pub rack: String,
    pub release_version: String,
}
```

These wrap queries like `SELECT * FROM system_views.clients` and return typed structs.

### UDF/UDA Management (`udf.rs`)

```rust
impl CassandraPool {
    pub async fn create_function(&self, def: &UdfDefinition) -> CassandraResult<()>;
    pub async fn drop_function(&self, keyspace: &str, name: &str, arg_types: &[&str]) -> CassandraResult<()>;
    pub async fn create_aggregate(&self, def: &UdaDefinition) -> CassandraResult<()>;
    pub async fn drop_aggregate(&self, keyspace: &str, name: &str, arg_types: &[&str]) -> CassandraResult<()>;
}

pub struct UdfDefinition {
    pub keyspace: String,
    pub name: String,
    pub arguments: Vec<(String, String)>,  // (arg_name, cql_type)
    pub return_type: String,
    pub language: UdfLanguage,              // Java, JavaScript (deprecated)
    pub body: String,
    pub called_on_null: bool,
}

pub struct UdaDefinition {
    pub keyspace: String,
    pub name: String,
    pub arg_types: Vec<String>,
    pub state_function: String,
    pub state_type: String,
    pub final_function: Option<String>,
    pub initial_condition: Option<String>,
}

pub enum UdfLanguage {
    Java,
    JavaScript,
}
```

### Library Entry (`lib.rs`)

```rust
//! # prax-cassandra
//!
//! Apache Cassandra driver for Prax ORM using cdrs-tokio.
//!
//! Provides async CRUD, prepared statements, batches, lightweight transactions,
//! paging, virtual tables (Cassandra 4.0+), and UDF/UDA management.
//!
//! ## Example
//!
//! ```rust,no_run
//! use prax_cassandra::{CassandraConfig, CassandraPool};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = CassandraConfig::builder()
//!         .known_nodes(["127.0.0.1:9042"])
//!         .default_keyspace("myapp")
//!         .build();
//!
//!     let pool = CassandraPool::connect(config).await?;
//!     // ...
//!     Ok(())
//! }
//! ```

pub mod auth;
pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod row;
pub mod types;
pub mod udf;
pub mod virtual_tables;

pub use auth::{PlainSasl, SaslMechanism};
pub use config::{
    CassandraAuth, CassandraConfig, CassandraConfigBuilder, Consistency, RetryPolicyKind,
    TlsConfig,
};
pub use connection::CassandraConnection;
pub use engine::{BatchBuilder, PagedStream};
pub use error::{CassandraError, CassandraResult};
pub use pool::CassandraPool;
pub use row::FromRow;
pub use udf::{UdaDefinition, UdfDefinition, UdfLanguage};
pub use virtual_tables::{CacheInfo, ClientInfo, ClusterInfo, PeerInfo, VirtualTables};
```

## Migration Integration (Decision Q5 → A)

No new code required. `prax-migrate::CqlDialect` already supports Cassandra:

```rust
use prax_migrate::{CqlDialect, CqlMigrationGenerator, CqlSchemaDiff, MigrationDialect};
use prax_cassandra::CassandraPool;

let migration = CqlDialect::generate(&diff);
// Apply up statements to a Cassandra pool
for stmt in migration.up.split("\n\n") {
    if !stmt.trim().is_empty() {
        pool.execute(stmt, ()).await?;
    }
}
```

The README for `prax-cassandra` will document this pattern.

## Dependencies

### New workspace dependency
- `cdrs-tokio = "9.0"` (or most recent stable)

### Re-used workspace dependencies
- `tokio` (with full features)
- `futures`
- `async-trait`
- `serde` (with derive)
- `serde_json`
- `thiserror`
- `chrono` (with serde)
- `uuid` (with v4, serde)
- `tracing`
- `smol_str`

### Dev dependencies
- `tokio-test = "0.4"` (inherit pattern from prax-scylladb)
- `pretty_assertions` (workspace)
- `criterion` (workspace) for benchmarks

## Testing Strategy

### Unit Tests (inline in each module)

- `config.rs`: builder chains, defaults, auth/TLS variants
- `auth.rs`: PlainSasl PLAIN-mechanism challenge construction
- `error.rs`: error variant construction, Display output

### Integration Tests

Location: `prax-cassandra/tests/cassandra_integration.rs`

Gated behind feature flag:

```toml
[features]
default = []
cassandra-live = []
```

Tests annotated `#[cfg(feature = "cassandra-live")]`. Require an operator to run a local Cassandra instance (e.g., `docker run cassandra:4.1`). CI runs unit tests by default; live tests run on-demand via `cargo test --features cassandra-live`.

**Coverage:**
- Connect + authenticate (password)
- CRUD operations
- Prepared statements
- Batch (logged + unlogged)
- Lightweight transactions (success + failure paths)
- Paging over large result sets
- Virtual table queries (Cassandra 4.0+)
- UDF creation and invocation
- TLS connection (optional)

### Benchmarks

`prax-cassandra/benches/cassandra_operations.rs` mirroring prax-scylladb's benchmark file. Feature-gated behind `cassandra-live`.

## Migration Path for Existing Users

None needed — `prax-cassandra` is a new crate. Existing code is unaffected.

Users with Cassandra deployments previously using `prax-scylladb` (since Scylla's driver works against Cassandra) can migrate to `prax-cassandra` by:
1. Replacing `use prax_scylladb::*` with `use prax_cassandra::*`
2. Renaming `ScyllaConfig` → `CassandraConfig`, `ScyllaPool` → `CassandraPool`
3. No schema changes required (same CQL)

## Success Criteria

**Functional:**
- Compiles as workspace member at v0.7.2 using `cdrs-tokio` driver
- Supports password + TLS + SASL authentication
- Provides CRUD, prepared statements, batch, LWT, paging APIs
- Virtual tables API surfaces Cassandra 4.0+ diagnostic data
- UDF/UDA management helpers work against a live Cassandra instance

**Testing:**
- Unit tests pass (config, auth, error)
- Integration tests (gated) pass against a local Cassandra
- No regressions in other workspace crates
- `cargo build --workspace` clean
- `cargo fmt --check` clean
- `cargo clippy --workspace` clean (no new warnings)

**Documentation:**
- Crate-level doc comment with example
- Every public type has a doc comment
- README.md with quick-start example
- Migration integration pattern documented

## Future Work

- LDAP SASL mechanism (`prax-cassandra-ldap` crate)
- Kerberos/GSSAPI SASL (`prax-cassandra-gssapi` crate)
- Token-aware routing policies
- Speculative execution for latency reduction
- Integration with prax-schema for type-safe query generation
- Live integration tests in CI (requires Cassandra container in CI config)
- Migration from prax-scylladb docs + tooling
