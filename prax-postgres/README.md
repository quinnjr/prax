# prax-postgres

PostgreSQL query engine for Prax ORM.

## Overview

`prax-postgres` provides an async PostgreSQL backend using `tokio-postgres` with `deadpool-postgres` connection pooling.

## Features

- Async query execution with Tokio
- Connection pooling via deadpool-postgres
- Transaction support with savepoints
- Prepared statement caching
- SSL/TLS support
- `LISTEN` / `NOTIFY` with `PgListener` (dedicated connection, automatic
  reconnect with backoff, `Lagged` / `ChannelsDropped` events) and
  `PgConnection::notify` / `PgTransaction::notify`
- Recycling policy via `PgRecycling` (`Fast` default, `Verified`, `Clean`)
  and `PoolConfig::recycle_timeout`; jittered `max_lifetime`
- Row locks (`FOR UPDATE` / `FOR SHARE`, `SKIP LOCKED`, `NOWAIT`) through
  `.lock(RowLock::for_update().skip_locked())` on `find_many` /
  `find_first` / `find_unique`
- Typed array and raw `bytea` parameter binding

## Usage

```rust
use prax_postgres::{PgConfig, PgEngine, PgPool, PoolConfig, PgRecycling};

let config = PgConfig::from_url("postgresql://user:pass@localhost/db")?;
let pool = PgPool::new(config).await?;
let engine = PgEngine::new(pool);

// Execute queries through the engine with a Prax client
let client = PraxClient::with_engine(engine);
let users = client.user().find_many().exec().await?;
```

## Configuration

`PoolConfig` is `#[non_exhaustive]`: start from `PoolConfig::default()` and
assign fields.

```rust
use std::time::Duration;

let mut pool_config = PoolConfig::default();
pool_config.max_connections = 20;
pool_config.recycling = PgRecycling::Clean; // reset session state on checkout
pool_config.recycle_timeout = Some(Duration::from_secs(5));
pool_config.max_lifetime = Some(Duration::from_secs(30 * 60));

let pool = PgPool::with_pool_config(config, pool_config).await?;
```

`PgRecycling::Fast` (the default) does not reset session state, so `SET`,
`SET ROLE`, `search_path` or tenant settings leak to the next checkout; use
`Clean` (or `SET LOCAL`) with row-level security. `max_lifetime` and
`idle_timeout` (default 10 minutes, measured from the connection's last
checkout) are enforced when a connection is checked out: an expired one is
replaced instead of handed out.

## LISTEN / NOTIFY

```rust
use prax_postgres::{PgListener, PgListenerEvent};

let mut listener = PgListener::connect(&config).await?;
listener.listen("events").await?;
loop {
    match listener.recv().await? {
        PgListenerEvent::Notification(n) => println!("{}: {}", n.channel, n.payload),
        PgListenerEvent::Reconnected | PgListenerEvent::Lagged { .. } => { /* resync */ }
        PgListenerEvent::ChannelsDropped { channels, .. } => eprintln!("dropped {channels:?}"),
        _ => {}
    }
}
```

Connect the listener directly to PostgreSQL, not through a
transaction-mode pooler. Any role can `NOTIFY` any channel: treat payloads as
untrusted.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

