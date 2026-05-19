//! # prax-schema
//!
//! Schema parser and AST for the Prax ORM.
//!
//! This crate provides:
//! - Schema Definition Language (SDL) parser for `.prax` files
//! - Configuration parser for `prax.toml` files
//! - Abstract Syntax Tree (AST) types for schema representation
//! - Schema validation and semantic analysis
//!
//! ## Quick Start
//!
//! Parse a schema string:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
//! model User {
//!     id    Int    @id @auto
//!     email String @unique
//!     name  String?
//! }
//! "#).unwrap();
//!
//! assert!(schema.get_model("User").is_some());
//! ```
//!
//! ## Parsing Models
//!
//! Models define database tables with typed fields:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
//! model Post {
//!     id        Int      @id @auto
//!     title     String
//!     content   String?
//!     published Boolean  @default(false)
//!     viewCount Int      @default(0)
//! }
//! "#).unwrap();
//!
//! let post = schema.get_model("Post").unwrap();
//! assert_eq!(post.fields.len(), 5);
//!
//! // Check field properties
//! let title = post.fields.get("title").unwrap();
//! assert!(!title.is_optional());
//!
//! let content = post.fields.get("content").unwrap();
//! assert!(content.is_optional());
//! ```
//!
//! ## Parsing Enums
//!
//! Enums define custom value types:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
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
//! "#).unwrap();
//!
//! let role_enum = schema.get_enum("Role").unwrap();
//! assert_eq!(role_enum.variants.len(), 3);
//! ```
//!
//! ## Parsing Relations
//!
//! Relations define relationships between models:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
//! model User {
//!     id    Int    @id @auto
//!     posts Post[]
//! }
//!
//! model Post {
//!     id       Int  @id @auto
//!     authorId Int
//!     author   User @relation(fields: [authorId], references: [id])
//! }
//! "#).unwrap();
//!
//! let post = schema.get_model("Post").unwrap();
//! let author_field = post.fields.get("author").unwrap();
//! assert!(author_field.is_relation());
//! ```
//!
//! ## Parsing Views
//!
//! Views represent SQL views:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
//! model User {
//!     id     Int    @id @auto
//!     active Boolean
//! }
//!
//! view ActiveUsers {
//!     id Int @unique
//!
//!     @@map("active_users_view")
//! }
//! "#).unwrap();
//!
//! assert!(schema.get_view("ActiveUsers").is_some());
//! ```
//!
//! ## Parsing Server Groups
//!
//! Server groups define multi-server configurations:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
//! serverGroup MainCluster {
//!     server primary {
//!         url  = "postgres://primary/db"
//!         role = "primary"
//!     }
//!
//!     server replica1 {
//!         url    = "postgres://replica1/db"
//!         role   = "replica"
//!         weight = 100
//!     }
//!
//!     @@strategy("ReadReplica")
//!     @@loadBalance("RoundRobin")
//! }
//! "#).unwrap();
//!
//! let cluster = schema.get_server_group("MainCluster").unwrap();
//! assert_eq!(cluster.servers.len(), 2);
//! ```
//!
//! ## Schema Validation
//!
//! Validate schemas for correctness:
//!
//! ```rust
//! use prax_schema::validate_schema;
//!
//! // Valid schema passes validation
//! let result = validate_schema(r#"
//! model User {
//!     id    Int    @id @auto
//!     email String @unique
//! }
//! "#);
//! assert!(result.is_ok());
//!
//! // Schema with relations validates correctly
//! let result = validate_schema(r#"
//! model User {
//!     id    Int    @id @auto
//!     posts Post[]
//! }
//! model Post {
//!     id       Int  @id @auto
//!     authorId Int
//!     author   User @relation(fields: [authorId], references: [id])
//! }
//! "#);
//! assert!(result.is_ok());
//! ```
//!
//! ## Schema Statistics
//!
//! Get statistics about a schema:
//!
//! ```rust
//! use prax_schema::parse_schema;
//!
//! let schema = parse_schema(r#"
//! model User { id Int @id @auto }
//! model Post { id Int @id @auto }
//! enum Role { User Admin }
//! type Address { street String }
//! "#).unwrap();
//!
//! let stats = schema.stats();
//! assert_eq!(stats.model_count, 2);
//! assert_eq!(stats.enum_count, 1);
//! assert_eq!(stats.type_count, 1);
//! ```
//!
//! ## Configuration Parsing
//!
//! Parse `prax.toml` configuration:
//!
//! ```rust
//! use prax_schema::config::PraxConfig;
//!
//! let config: PraxConfig = toml::from_str(r#"
//! [database]
//! provider = "postgresql"
//! url = "postgres://localhost/mydb"
//!
//! [database.pool]
//! max_connections = 10
//!
//! [generator.client]
//! output = "./src/generated"
//! "#).unwrap();
//!
//! assert_eq!(config.database.pool.max_connections, 10);
//! ```
//!
//! ## Prelude
//!
//! Import commonly used types:
//!
//! ```rust
//! use prax_schema::prelude::*;
//!
//! // Now you can use parse_schema, Schema, etc.
//! let schema = parse_schema("model User { id Int @id }").unwrap();
//! ```

pub mod ast;
pub mod cache;
pub mod config;
pub mod error;
pub mod loader;
pub mod parser;
pub mod validator;

pub use ast::*;
pub use cache::{
    CacheStats, DocString, FieldAttrsCache, LazyFieldAttrs, SchemaCache, ValidationTypePool,
};
pub use config::{ModelStyle, PraxConfig};
pub use error::{SchemaError, SchemaResult};
pub use loader::{SourceFile, SourceId, SourceMap};
pub use parser::{parse_schema, parse_schema_file};
pub use validator::{Validator, validate_schema};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use crate::ast::*;
    pub use crate::cache::{DocString, SchemaCache};
    pub use crate::config::PraxConfig;
    pub use crate::error::{SchemaError, SchemaResult};
    pub use crate::parser::{parse_schema, parse_schema_file};
    pub use crate::validator::{Validator, validate_schema};
}
