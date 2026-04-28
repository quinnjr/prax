# Prax ORM

<p align="center">
  <strong>A next-generation, type-safe ORM for Rust</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/prax-orm"><img src="https://img.shields.io/crates/v/prax-orm.svg" alt="crates.io"></a>
  <a href="https://docs.rs/prax-orm"><img src="https://docs.rs/prax-orm/badge.svg" alt="docs.rs"></a>
  <a href="https://github.com/pegasusheavy/prax-orm/actions"><img src="https://github.com/pegasusheavy/prax-orm/workflows/CI/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/rust-1.89%2B-blue.svg" alt="Rust 1.89+">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg" alt="License"></a>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#documentation">Documentation</a> •
  <a href="#license">License</a>
</p>

---

**Prax ORM** is a modern, Prisma-inspired ORM for Rust with first-class async support. Built on top of `tokio-postgres`, `sqlx`, and other async database clients, Prax provides a type-safe, ergonomic API for database operations with compile-time guarantees.

> ⚠️ **Work in Progress** - Prax is currently under active development. See [TODO.md](./TODO.md) for the implementation roadmap.

## Features

- 🔒 **Type-Safe Queries** - Compile-time checked queries with zero runtime overhead
- ⚡ **Async-First** - Built on Tokio for high-performance async I/O
- 🎯 **Fluent API** - Intuitive query builder with method chaining
- 🔗 **Relations** - Eager and lazy loading with `include` and `select`
- 📦 **Migrations** - Schema diffing, SQL generation, and migration tracking
- 🛠️ **Code Generation** - Proc-macros for compile-time model generation
- 🗄️ **Multi-Database** - PostgreSQL, MySQL, SQLite, MSSQL, MongoDB, DuckDB, ScyllaDB
- 🧠 **Vector Search** - pgvector integration for AI/ML embeddings and similarity search
- 🔌 **Framework Integration** - First-class support for [Armature](https://github.com/pegasusheavy/armature), Axum, and Actix-web
- 🏢 **Multi-Tenancy** - Row-level security, schema-based, and database-based isolation
- 📥 **Schema Import** - Migrate from Prisma, Diesel, or SeaORM

## Installation

Add Prax ORM to your `Cargo.toml`:

```toml
[dependencies]
prax-orm = "0.6"
tokio = { version = "1", features = ["full"] }
```

For specific database backends:

```toml
# PostgreSQL (default)
prax-orm = { version = "0.6", features = ["postgres"] }

# MySQL
prax-orm = { version = "0.6", features = ["mysql"] }

# SQLite
prax-orm = { version = "0.6", features = ["sqlite"] }

# MSSQL
prax-mssql = "0.6"

# MongoDB
prax-mongodb = "0.6"

# DuckDB (analytics)
prax-duckdb = "0.6"

# ScyllaDB (high-performance Cassandra-compatible)
prax-scylladb = "0.6"

# pgvector (AI/ML vector search)
prax-pgvector = "0.6"

# Armature framework integration
prax-armature = "0.6"
```

## Quick Start

Define models, wire them to a `PraxClient<E>`, and run typed CRUD against
a real connection pool. The snippet below mirrors
[`examples/client_crud_postgres.rs`](./examples/client_crud_postgres.rs)
and matches the runtime shape of the driver — field accessors are
`snake_case` (e.g. `user::created_at::desc()`), not `camelCase`.

```rust,ignore
use prax_orm::{client, Model, PraxClient};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

// Attaches `client.user()` via the generated `PraxClientExt` trait.
client!(User);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool: PgPool = PgPoolBuilder::new()
        .url("postgres://prax:prax_test_password@localhost:5432/prax_test")
        .build()
        .await?;
    let client = PraxClient::new(PgEngine::new(pool));

    // Create
    let alice = client
        .user()
        .create()
        .set("email", "alice@example.com")
        .exec()
        .await?;

    // Find with a where-filter and ordering. Field accessors use the
    // struct's lowercased field name: `user::created_at::desc()`.
    let recent = client
        .user()
        .find_many()
        .r#where(user::email::contains("@example.com"))
        .order_by(user::created_at::desc())
        .exec()
        .await?;

    // Transactions: closure receives a client bound to the tx. Commit on
    // Ok, rollback on Err.
    client
        .transaction(|tx| async move {
            tx.user()
                .update()
                .r#where(user::id::equals(alice.id))
                .set("email", "alice+updated@example.com")
                .exec()
                .await?;
            Ok::<_, prax_query::error::QueryError>(())
        })
        .await?;

    Ok(())
}
```

### Armature Framework Integration

Prax integrates seamlessly with [Armature](https://github.com/pegasusheavy/armature), providing dependency injection support:

```rust
use armature::prelude::*;
use prax_armature::PraxModule;

#[module_impl]
impl DatabaseModule {
    #[provider(singleton)]
    async fn prax_client() -> Arc<PraxClient> {
        Arc::new(
            PraxClient::new("postgresql://localhost/mydb")
                .await
                .expect("Database connection failed")
        )
    }
}

#[controller("/users")]
impl UserController {
    #[get("/")]
    async fn list(
        &self,
        #[inject] db: Arc<PraxClient>,
    ) -> Result<Json<Vec<User>>, HttpError> {
        let users = db.user().find_many().exec().await?;
        Ok(Json(users))
    }
}
```

## Query Operations

### Filtering

```rust
// Equals
user::email::equals("alice@example.com")

// Contains, starts with, ends with
user::name::contains("alice")
user::email::starts_with("admin")
user::email::ends_with("@company.com")

// Comparisons
user::age::gt(18)
user::age::gte(21)
user::age::lt(65)
user::created_at::lte(Utc::now())

// Logical operators
and![
    user::age::gte(18),
    user::status::equals("active")
]

or![
    user::role::equals("admin"),
    user::role::equals("moderator")
]

not!(user::banned::equals(true))

// Nested relation filters
user::posts::some(post::published::equals(true))
```

### Pagination

```rust
// Offset-based
client.user().find_many().skip(20).take(10).exec().await?;

// Cursor-based
client.user().find_many().cursor(user::id::equals(100)).take(10).exec().await?;
```

### Aggregations

```rust
let count = client.user().count().exec().await?;

let stats = client
    .post()
    .aggregate()
    .count()
    .avg(post::views)
    .sum(post::likes)
    .exec()
    .await?;

let grouped = client
    .user()
    .group_by(user::country)
    .count()
    .exec()
    .await?;
```

### Vector Similarity Search (pgvector)

```rust
use prax_pgvector::prelude::*;

// Create an embedding from your ML model output
let query = Embedding::new(vec![0.1, 0.2, 0.3, /* ... */]);

// Find the 10 most similar documents
let search = VectorSearchBuilder::new("documents", "embedding")
    .query(query)
    .metric(DistanceMetric::Cosine)
    .limit(10)
    .build();

// Hybrid search: combine vector similarity with full-text search
let hybrid = HybridSearchBuilder::new("documents")
    .vector_column("embedding")
    .text_column("body")
    .query_vector(Embedding::new(vec![0.1, 0.2, 0.3]))
    .query_text("machine learning")
    .vector_weight(0.7)
    .text_weight(0.3)
    .limit(10)
    .build();

// Manage HNSW indexes
let index = VectorIndex::hnsw("idx_doc_embedding", "documents", "embedding")
    .metric(DistanceMetric::Cosine)
    .config(HnswConfig::high_recall())
    .concurrent()
    .build()?;
```

## Architecture

Prax ORM is organized as a workspace of focused crates:

```
prax-orm/
├── prax-schema/         # Schema parser and AST
├── prax-codegen/        # Proc-macro crate for code generation
├── prax-query/          # Query builder + optimizations
├── prax-postgres/       # PostgreSQL (tokio-postgres) engine
├── prax-mysql/          # MySQL (mysql_async) engine
├── prax-sqlite/         # SQLite (rusqlite) engine
├── prax-mssql/          # MSSQL (tiberius) engine
├── prax-mongodb/        # MongoDB engine
├── prax-duckdb/         # DuckDB analytical engine
├── prax-scylladb/       # ScyllaDB (Cassandra-compatible) engine
├── prax-pgvector/       # pgvector integration (embeddings, vector search)
├── prax-sqlx/           # SQLx backend with compile-time checks
├── prax-migrate/        # Migration engine
├── prax-import/         # Import from Prisma/Diesel/SeaORM
├── prax-cli/            # CLI tool (prax-orm-cli)
├── prax-armature/       # Armature framework integration
├── prax-axum/           # Axum framework integration
├── prax-actix/          # Actix-web framework integration
└── src/                 # Main crate (prax-orm) re-exporting everything
```

## CLI

Prax ORM includes a CLI for schema management and migrations:

```bash
# Install the CLI
cargo install prax-orm-cli

# Initialize a new Prax project
prax init

# Generate client from schema
prax generate

# Create a migration
prax migrate dev --name add_users_table

# Apply migrations
prax migrate deploy

# Reset database
prax migrate reset

# Introspect existing database
prax db pull
```

## Comparison

| Feature | Prax ORM | Diesel | SeaORM | SQLx |
|---------|----------|--------|--------|------|
| Async Support | ✅ | ❌ | ✅ | ✅ |
| Type-Safe Queries | ✅ | ✅ | ✅ | ✅ |
| Schema DSL | ✅ | ❌ | ❌ | ❌ |
| Migrations | ✅ | ✅ | ✅ | ✅ |
| Relations | ✅ | ✅ | ✅ | ❌ |
| Code Generation | ✅ | ❌ | ✅ | ❌ |
| Fluent API | ✅ | ❌ | ✅ | ❌ |
| Multi-Tenancy | ✅ | ❌ | ❌ | ❌ |
| Built-in Caching | ✅ | ❌ | ❌ | ❌ |
| Vector Search (pgvector) | ✅ | ❌ | ❌ | ❌ |
| Schema Import | ✅ | ❌ | ❌ | ❌ |
| 7+ Database Backends | ✅ | ❌ | ❌ | ❌ |

## Contributing

Contributions are welcome! Please read the contributing guidelines before submitting a pull request.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

Copyright (c) 2025-2026 Pegasus Heavy Industries LLC

## Acknowledgments

Prax ORM is heavily inspired by:

- **[Prisma](https://www.prisma.io/)** - For pioneering the modern ORM developer experience
- **[Diesel](https://diesel.rs/)** - For proving type-safe database access in Rust is possible
- **[SeaORM](https://www.sea-ql.org/SeaORM/)** - For async ORM patterns in Rust
- **[Armature](https://github.com/pegasusheavy/armature)** - Our companion HTTP framework

