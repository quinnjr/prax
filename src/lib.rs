//! # Prax - A Next-Generation ORM for Rust
//!
//! Prax is a type-safe, async-first Object-Relational Mapper (ORM) for Rust,
//! inspired by Prisma. It provides a delightful developer experience with
//! compile-time guarantees and a powerful schema definition language.
//!
//! ## Features
//!
//! - **Schema Definition Language**: Define your data models in a clear, readable `.prax` file
//! - **Type-Safe Queries**: Compile-time checked queries with a fluent builder API
//! - **Async-First**: Built on Tokio for high-performance async database operations
//! - **Multi-Database Support**: PostgreSQL, MySQL, SQLite, and MongoDB
//! - **Automatic Migrations**: Generate and apply database migrations from your schema
//! - **Relations**: Define and query relationships between models with ease
//! - **Transactions**: Full transaction support with savepoints and isolation levels
//! - **Middleware System**: Extensible query interception for logging, metrics, and more
//! - **Multi-Tenant Support**: Built-in support for multi-tenant applications
//!
//! ## Quick Start
//!
//! ### 1. Define Your Schema
//!
//! Create a `prax/schema.prax` file in your project:
//!
//! ```text
//! // Models define your database tables
//! model User {
//!     id        Int      @id @auto
//!     email     String   @unique
//!     name      String?
//!     posts     Post[]
//!     createdAt DateTime @default(now())
//! }
//!
//! model Post {
//!     id        Int      @id @auto
//!     title     String
//!     content   String?  @db.Text
//!     published Boolean  @default(false)
//!     authorId  Int
//!     author    User     @relation(fields: [authorId], references: [id])
//! }
//! ```
//!
//! ### 2. Configure Your Database
//!
//! Create a `prax.toml` configuration file in your project root:
//!
//! ```toml
//! [database]
//! provider = "postgresql"
//! url = "${DATABASE_URL}"
//!
//! [database.pool]
//! max_connections = 10
//!
//! [generator.client]
//! output = "./src/generated"
//! ```
//!
//! ### 3. Use in Your Application
//!
//! ```rust,ignore
//! use prax::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), prax::SchemaError> {
//!     // Initialize the client (in real usage, generated from schema)
//!     let client = PraxClient::new("postgresql://localhost/mydb").await?;
//!
//!     // Create a new user
//!     let user = client
//!         .user()
//!         .create(CreateData::new()
//!             .set("email", "alice@example.com")
//!             .set("name", "Alice"))
//!         .exec()
//!         .await?;
//!
//!     // Find users with filtering
//!     let users = client
//!         .user()
//!         .find_many()
//!         .r#where(user::email::contains("@example.com"))
//!         .order_by(user::createdAt::desc())
//!         .take(10)
//!         .exec()
//!         .await?;
//!
//!     // Update a user
//!     let updated = client
//!         .user()
//!         .update()
//!         .r#where(user::id::equals(1))
//!         .data(UpdateData::new().set("name", "Alice Smith"))
//!         .exec()
//!         .await?;
//!
//!     // Delete a user
//!     client
//!         .user()
//!         .delete()
//!         .r#where(user::id::equals(1))
//!         .exec()
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Schema Language
//!
//! The Prax schema language provides a powerful way to define your data models:
//!
//! ### Models
//!
//! Models represent database tables:
//!
//! ```text
//! model User {
//!     id    Int    @id @auto     // Primary key with auto-increment
//!     email String @unique       // Unique constraint
//!     name  String?              // Optional (nullable) field
//! }
//! ```
//!
//! ### Field Types
//!
//! | Type       | Description                    | Database Mapping  |
//! |------------|--------------------------------|-------------------|
//! | `Int`      | Integer                        | INTEGER/INT       |
//! | `BigInt`   | 64-bit integer                 | BIGINT            |
//! | `Float`    | Floating point                 | FLOAT/DOUBLE      |
//! | `Decimal`  | Exact decimal                  | DECIMAL/NUMERIC   |
//! | `String`   | Text                           | VARCHAR/TEXT      |
//! | `Boolean`  | True/false                     | BOOLEAN           |
//! | `DateTime` | Date and time                  | TIMESTAMP         |
//! | `Date`     | Date only                      | DATE              |
//! | `Time`     | Time only                      | TIME              |
//! | `Json`     | JSON data                      | JSON/JSONB        |
//! | `Bytes`    | Binary data                    | BYTEA/BLOB        |
//! | `Uuid`     | UUID                           | UUID              |
//!
//! ### Field Attributes
//!
//! | Attribute              | Description                              |
//! |------------------------|------------------------------------------|
//! | `@id`                  | Primary key                              |
//! | `@auto`                | Auto-increment/generate                  |
//! | `@unique`              | Unique constraint                        |
//! | `@default(value)`      | Default value                            |
//! | `@map("name")`         | Custom column name                       |
//! | `@db.Type`             | Database-specific type                   |
//! | `@index`               | Create index on field                    |
//! | `@updated_at`          | Auto-update timestamp                    |
//! | `@relation(...)`       | Define relation                          |
//!
//! ### Relations
//!
//! Define relationships between models:
//!
//! ```text
//! model User {
//!     id    Int    @id @auto
//!     posts Post[]                    // One-to-many
//! }
//!
//! model Post {
//!     id       Int  @id @auto
//!     authorId Int
//!     author   User @relation(fields: [authorId], references: [id])
//! }
//! ```
//!
//! ### Enums
//!
//! Define enumerated types:
//!
//! ```text
//! enum Role {
//!     User
//!     Admin
//!     Moderator
//! }
//!
//! model User {
//!     id   Int  @id @auto
//!     role Role @default(User)
//! }
//! ```
//!
//! ## Query API
//!
//! ### Finding Records
//!
//! ```rust,ignore
//! // Find many with filters
//! let users = client
//!     .user()
//!     .find_many()
//!     .r#where(user::email::contains("@example.com"))
//!     .r#where(user::createdAt::gt(some_date))
//!     .order_by(user::name::asc())
//!     .skip(10)
//!     .take(20)
//!     .exec()
//!     .await?;
//!
//! // Find unique record
//! let user = client
//!     .user()
//!     .find_unique()
//!     .r#where(user::id::equals(1))
//!     .exec()
//!     .await?;
//!
//! // Find first matching
//! let user = client
//!     .user()
//!     .find_first()
//!     .r#where(user::email::ends_with("@company.com"))
//!     .exec()
//!     .await?;
//! ```
//!
//! ### Filter Operations
//!
//! ```rust,ignore
//! // Equality
//! user::email::equals("test@example.com")
//!
//! // Comparison
//! user::age::gt(18)
//! user::age::gte(18)
//! user::age::lt(65)
//! user::age::lte(65)
//!
//! // String operations
//! user::name::contains("john")
//! user::name::starts_with("Dr.")
//! user::name::ends_with("Smith")
//!
//! // List operations
//! user::status::in_list(vec!["active", "pending"])
//! user::status::not_in(vec!["banned"])
//!
//! // Null checks
//! user::deleted_at::is_null()
//! user::verified_at::is_not_null()
//!
//! // Combining filters
//! Filter::and(vec![
//!     user::active::equals(true),
//!     user::email::contains("@company.com"),
//! ])
//!
//! Filter::or(vec![
//!     user::role::equals(Role::Admin),
//!     user::role::equals(Role::Moderator),
//! ])
//! ```
//!
//! ### Creating Records
//!
//! ```rust,ignore
//! // Single create
//! let user = client
//!     .user()
//!     .create(CreateData::new()
//!         .set("email", "new@example.com")
//!         .set("name", "New User"))
//!     .exec()
//!     .await?;
//!
//! // Create many
//! let count = client
//!     .user()
//!     .create_many(vec![
//!         CreateData::new().set("email", "user1@example.com"),
//!         CreateData::new().set("email", "user2@example.com"),
//!     ])
//!     .exec()
//!     .await?;
//!
//! // Create with nested relation
//! let user = client
//!     .user()
//!     .create(CreateData::new()
//!         .set("email", "author@example.com")
//!         .connect("posts", post::id::equals(1)))
//!     .exec()
//!     .await?;
//! ```
//!
//! ### Updating Records
//!
//! ```rust,ignore
//! // Update single
//! let user = client
//!     .user()
//!     .update()
//!     .r#where(user::id::equals(1))
//!     .data(UpdateData::new()
//!         .set("name", "Updated Name")
//!         .increment("login_count", 1))
//!     .exec()
//!     .await?;
//!
//! // Update many
//! let count = client
//!     .user()
//!     .update_many()
//!     .r#where(user::active::equals(false))
//!     .data(UpdateData::new().set("active", true))
//!     .exec()
//!     .await?;
//!
//! // Upsert (create or update)
//! let user = client
//!     .user()
//!     .upsert()
//!     .r#where(user::email::equals("test@example.com"))
//!     .create(CreateData::new().set("email", "test@example.com"))
//!     .update(UpdateData::new().set("login_count", 1))
//!     .exec()
//!     .await?;
//! ```
//!
//! ### Deleting Records
//!
//! ```rust,ignore
//! // Delete single
//! client
//!     .user()
//!     .delete()
//!     .r#where(user::id::equals(1))
//!     .exec()
//!     .await?;
//!
//! // Delete many
//! let count = client
//!     .user()
//!     .delete_many()
//!     .r#where(user::active::equals(false))
//!     .exec()
//!     .await?;
//! ```
//!
//! ### Including Relations
//!
//! ```rust,ignore
//! // Include related records
//! let user = client
//!     .user()
//!     .find_unique()
//!     .r#where(user::id::equals(1))
//!     .include(user::posts::include())
//!     .exec()
//!     .await?;
//!
//! // Nested includes
//! let post = client
//!     .post()
//!     .find_unique()
//!     .r#where(post::id::equals(1))
//!     .include(post::author::include(
//!         user::posts::include()
//!     ))
//!     .exec()
//!     .await?;
//!
//! // Select specific fields
//! let user = client
//!     .user()
//!     .find_unique()
//!     .r#where(user::id::equals(1))
//!     .select(user::select!(id, email, name))
//!     .exec()
//!     .await?;
//! ```
//!
//! ## Transactions
//!
//! ```rust,ignore
//! // Basic transaction
//! let result = client.transaction(|tx| async move {
//!     let user = tx.user()
//!         .create(CreateData::new().set("email", "tx@example.com"))
//!         .exec()
//!         .await?;
//!
//!     tx.post()
//!         .create(CreateData::new()
//!             .set("title", "My First Post")
//!             .set("authorId", user.id))
//!         .exec()
//!         .await?;
//!
//!     Ok(user)
//! }).await?;
//!
//! // Transaction with options
//! let result = client
//!     .transaction_with_config(TransactionConfig::new()
//!         .isolation(IsolationLevel::Serializable)
//!         .timeout(Duration::from_secs(30)))
//!     .run(|tx| async move {
//!         // ... operations
//!         Ok(())
//!     })
//!     .await?;
//! ```
//!
//! ## Raw SQL
//!
//! When you need to execute raw SQL:
//!
//! ```rust,ignore
//! use prax::raw_query;
//!
//! // Type-safe raw query
//! let users: Vec<User> = client
//!     .query_raw(raw_query!(
//!         "SELECT * FROM users WHERE email = {}",
//!         "test@example.com"
//!     ))
//!     .await?;
//!
//! // Execute raw SQL
//! let affected = client
//!     .execute_raw(raw_query!(
//!         "UPDATE users SET verified = true WHERE id = {}",
//!         user_id
//!     ))
//!     .await?;
//! ```
//!
//! ## Aggregations
//!
//! ```rust,ignore
//! // Count
//! let count = client
//!     .user()
//!     .count()
//!     .r#where(user::active::equals(true))
//!     .exec()
//!     .await?;
//!
//! // Aggregate functions
//! let stats = client
//!     .post()
//!     .aggregate()
//!     .avg(post::views)
//!     .sum(post::views)
//!     .min(post::created_at)
//!     .max(post::created_at)
//!     .exec()
//!     .await?;
//!
//! // Group by
//! let by_status = client
//!     .post()
//!     .group_by(post::status)
//!     .count()
//!     .having(count::gt(10))
//!     .exec()
//!     .await?;
//! ```
//!
//! ## Multi-Tenant Applications
//!
//! Prax provides built-in support for multi-tenant applications:
//!
//! ```rust,ignore
//! use prax::tenant::{TenantConfig, TenantMiddleware, IsolationStrategy};
//!
//! // Configure tenant isolation
//! let config = TenantConfig::builder()
//!     .strategy(IsolationStrategy::RowLevel {
//!         tenant_column: "tenant_id".into(),
//!     })
//!     .build();
//!
//! // Add tenant middleware
//! let client = client.with_middleware(TenantMiddleware::new(config));
//!
//! // Set tenant context for requests
//! let client = client.with_tenant("tenant-123");
//!
//! // All queries are automatically scoped to this tenant
//! let users = client.user().find_many().exec().await?;
//! ```
//!
//! ## Middleware
//!
//! Extend Prax with custom middleware:
//!
//! ```rust,ignore
//! use prax::middleware::{LoggingMiddleware, MetricsMiddleware, MiddlewareBuilder};
//!
//! let client = client.with_middleware(
//!     MiddlewareBuilder::new()
//!         .add(LoggingMiddleware::new())
//!         .add(MetricsMiddleware::new())
//!         .build()
//! );
//! ```
//!
//! ## CLI Commands
//!
//! Prax provides a CLI for common operations:
//!
//! ```bash
//! # Initialize a new project
//! prax init
//!
//! # Generate client code from schema
//! prax generate
//!
//! # Create a new migration
//! prax migrate create "add_users_table"
//!
//! # Apply pending migrations
//! prax migrate deploy
//!
//! # Reset database
//! prax migrate reset
//!
//! # Validate schema
//! prax validate
//!
//! # Format schema files
//! prax format
//! ```
//!
//! ## Crate Features
//!
//! Enable features in your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! prax = { version = "0.1", features = ["postgres", "mysql", "sqlite"] }
//! ```
//!
//! | Feature    | Description                                    |
//! |------------|------------------------------------------------|
//! | `postgres` | PostgreSQL support via `tokio-postgres`        |
//! | `mysql`    | MySQL support via `mysql_async`                |
//! | `sqlite`   | SQLite support via `tokio-rusqlite`            |
//! | `mongodb`  | MongoDB support                                |
//! | `sqlx`     | Alternative backend with compile-time checks   |
//! | `tracing`  | Integration with the `tracing` crate           |
//! | `serde`    | Serialization support for schema types         |
//!
//! ## Error Handling
//!
//! Prax provides detailed error types with actionable messages:
//!
//! ```rust,ignore
//! use prax::error::{QueryError, ErrorCode};
//!
//! match client.user().find_unique().r#where(user::id::equals(1)).exec().await {
//!     Ok(user) => println!("Found: {:?}", user),
//!     Err(e) => {
//!         eprintln!("Error: {}", e);
//!         eprintln!("Code: {:?}", e.code());
//!         if let Some(suggestion) = e.suggestion() {
//!             eprintln!("Suggestion: {}", suggestion);
//!         }
//!     }
//! }
//! ```
//!
//! ## Performance
//!
//! Prax is designed for high performance:
//!
//! - **Connection Pooling**: Built-in connection pool with configurable limits
//! - **Prepared Statement Caching**: Automatic caching of prepared statements
//! - **Batch Operations**: Efficient bulk create, update, and delete
//! - **Lazy Loading**: Relations are loaded only when requested
//!
//! ## Further Reading
//!
//! - [Schema Documentation](schema/index.html)
//! - [Query API](query/index.html)
//! - [Configuration](config/index.html)
//! - [Migration Guide](migrate/index.html)
//! - [Examples](https://github.com/pegasusheavy/prax-orm/tree/main/examples)

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

/// Schema parsing and AST types.
///
/// This module provides everything needed to work with Prax schema files:
///
/// - [`schema::parse_schema`] - Parse a schema string
/// - [`schema::parse_schema_file`] - Parse a schema from a file
/// - [`schema::validate_schema`] - Parse and validate a schema
/// - [`schema::PraxConfig`] - Configuration from `prax.toml`
/// - [`schema::Schema`] - The parsed schema representation
///
/// ## Example
///
/// ```rust,ignore
/// use prax::schema::{parse_schema, validate_schema, PraxConfig};
///
/// // Parse a schema
/// let schema = parse_schema(r#"
///     model User {
///         id    Int    @id @auto
///         email String @unique
///     }
/// "#)?;
///
/// // Access schema information
/// println!("Models: {:?}", schema.model_names().collect::<Vec<_>>());
///
/// // Load configuration
/// let config = PraxConfig::from_file("prax.toml")?;
/// println!("Database: {}", config.database.provider);
/// ```
pub mod schema {
    pub use prax_schema::*;
}

// Re-export proc macros
pub use prax_codegen::Model;
pub use prax_codegen::prax_schema;

/// Top-level `PraxClient<E>` and the `prax::client!` macro. See
/// [`client`] module docs for usage.
pub mod client;
pub use client::PraxClient;
// The `client!` macro is re-exported automatically by `#[macro_export]`.

// Macro plumbing: the expansion of `client!` references `$crate::__paste`
// and `$crate::__prelude`. Re-export them at the crate root so callers do
// not have to think about where the symbols live.
#[doc(hidden)]
pub use client::__paste;
#[doc(hidden)]
pub use client::__prelude;

/// Prelude module for convenient imports.
///
/// Import everything you need with a single line:
///
/// ```rust,ignore
/// use prax::prelude::*;
/// ```
///
/// This includes:
/// - Schema parsing functions
/// - Configuration types
/// - Common traits and types
pub mod prelude {
    pub use crate::client::PraxClient;
    pub use crate::schema::{PraxConfig, Schema, parse_schema, parse_schema_file};
    pub use crate::{Model, prax_schema};
}

// Re-export key types at the crate root
pub use schema::{Schema, SchemaError};

/// Error types for the Prax ORM.
///
/// The main error type is [`SchemaError`] which covers all schema-related
/// errors including parsing, validation, and file I/O.
pub mod error {
    pub use prax_schema::SchemaError;
}

/// Common types used by generated Prax models.
///
/// This module is referenced by the `#[derive(Model)]` proc-macro.
#[doc(hidden)]
pub mod _prax_prelude {
    /// Marker trait for Prax models.
    pub trait PraxModel {
        /// The table name in the database.
        const TABLE_NAME: &'static str;

        /// The primary key column(s).
        const PRIMARY_KEY: &'static [&'static str];
    }

    /// Sort direction for order by clauses.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SortOrder {
        /// Ascending order (A-Z, 0-9).
        Asc,
        /// Descending order (Z-A, 9-0).
        Desc,
    }
}
