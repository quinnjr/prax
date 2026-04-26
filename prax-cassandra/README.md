# prax-cassandra

Apache Cassandra driver for the Prax ORM, built on [cdrs-tokio](https://crates.io/crates/cdrs-tokio).

## Features

- Pure-Rust async driver (no FFI, no system library)
- CRUD, prepared statements, batches, lightweight transactions, paging
- Password + TLS + SASL authentication framework
- Cassandra 4.0+ virtual tables helpers
- User-defined function and aggregate management
- Migrations reuse `prax_migrate::CqlDialect` (same CQL as ScyllaDB)

## Quick Start

```rust,no_run
use prax_cassandra::{CassandraAuth, CassandraConfig, CassandraPool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = CassandraConfig::builder()
        .known_nodes(["127.0.0.1:9042".to_string()])
        .default_keyspace("myapp")
        .auth(CassandraAuth::Password {
            username: "cassandra".into(),
            password: "cassandra".into(),
        })
        .build();

    let pool = CassandraPool::connect(config).await?;

    // use pool ...

    pool.close().await?;
    Ok(())
}
```

## Migrations

Use the CQL dialect from `prax-migrate`:

```rust,no_run
use prax_cassandra::CassandraPool;
use prax_migrate::{CqlDialect, CqlSchemaDiff, MigrationDialect};

# async fn run(pool: CassandraPool, diff: CqlSchemaDiff) -> Result<(), Box<dyn std::error::Error>> {
let migration = CqlDialect::generate(&diff);
for stmt in migration.up.split("\n\n") {
    if !stmt.trim().is_empty() {
        pool.execute(stmt).await?;
    }
}
# Ok(()) }
```

## License

MIT OR Apache-2.0
