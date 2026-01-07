//! # prax-import
//!
//! Import schemas from Prisma, Diesel, and SeaORM to Prax ORM.
//!
//! This crate provides utilities to migrate existing Prisma, Diesel, and SeaORM schemas
//! to Prax's schema format, making it easy to switch ORMs or start using Prax
//! in existing projects.
//!
//! ## Features
//!
//! - **Prisma Import**: Parse Prisma schema files (`.prisma`) and convert to Prax
//! - **Diesel Import**: Parse Diesel schema files (Rust code with `table!` macros) and convert to Prax
//! - **SeaORM Import**: Parse SeaORM entity files (Rust code with `DeriveEntityModel`) and convert to Prax
//! - **Type Mapping**: Automatic conversion of types between ORMs
//! - **Relation Mapping**: Preserve relations and foreign keys
//! - **Attribute Mapping**: Convert attributes and constraints
//!
//! ## Quick Start
//!
//! ### Import from Prisma
//!
//! ```rust,no_run
//! use prax_import::prisma::import_prisma_schema;
//!
//! let prisma_schema = r#"
//! model User {
//!   id        Int      @id @default(autoincrement())
//!   email     String   @unique
//!   name      String?
//!   posts     Post[]
//!   createdAt DateTime @default(now())
//! }
//!
//! model Post {
//!   id        Int      @id @default(autoincrement())
//!   title     String
//!   content   String?
//!   published Boolean  @default(false)
//!   authorId  Int
//!   author    User     @relation(fields: [authorId], references: [id])
//! }
//! "#;
//!
//! let prax_schema = import_prisma_schema(prisma_schema).unwrap();
//! println!("Converted {} models", prax_schema.models.len());
//! ```
//!
//! ### Import from Diesel
//!
//! ```rust,no_run
//! use prax_import::diesel::import_diesel_schema;
//!
//! let diesel_schema = r#"
//! table! {
//!     users (id) {
//!         id -> Int4,
//!         email -> Varchar,
//!         name -> Nullable<Varchar>,
//!         created_at -> Timestamp,
//!     }
//! }
//!
//! table! {
//!     posts (id) {
//!         id -> Int4,
//!         title -> Varchar,
//!         content -> Nullable<Text>,
//!         published -> Bool,
//!         author_id -> Int4,
//!     }
//! }
//!
//! joinable!(posts -> users (author_id));
//! "#;
//!
//! let prax_schema = import_diesel_schema(diesel_schema).unwrap();
//! println!("Converted {} models", prax_schema.models.len());
//! ```
//!
//! ### Import from SeaORM
//!
//! ```rust,no_run
//! use prax_import::seaorm::import_seaorm_entity;
//!
//! let seaorm_entity = r#"
//! use sea_orm::entity::prelude::*;
//!
//! #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
//! #[sea_orm(table_name = "users")]
//! pub struct Model {
//!     #[sea_orm(primary_key, auto_increment)]
//!     pub id: i32,
//!     pub email: String,
//!     pub name: Option<String>,
//! }
//!
//! #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
//! pub enum Relation {}
//! "#;
//!
//! let prax_schema = import_seaorm_entity(seaorm_entity).unwrap();
//! println!("Converted {} models", prax_schema.models.len());
//! ```
//!
//! ## Type Mappings
//!
//! ### Prisma to Prax
//!
//! | Prisma Type | Prax Type |
//! |-------------|-----------|
//! | `Int` | `Int` |
//! | `BigInt` | `BigInt` |
//! | `Float` | `Float` |
//! | `Decimal` | `Decimal` |
//! | `String` | `String` |
//! | `Boolean` | `Boolean` |
//! | `DateTime` | `DateTime` |
//! | `Json` | `Json` |
//! | `Bytes` | `Bytes` |
//!
//! ### Diesel to Prax
//!
//! | Diesel Type | Prax Type |
//! |-------------|-----------|
//! | `Int4` | `Int` |
//! | `Int8` | `BigInt` |
//! | `Float4` / `Float8` | `Float` |
//! | `Numeric` | `Decimal` |
//! | `Varchar` / `Text` | `String` |
//! | `Bool` | `Boolean` |
//! | `Timestamp` | `DateTime` |
//! | `Json` / `Jsonb` | `Json` |
//! | `Bytea` | `Bytes` |

pub mod error;

#[cfg(feature = "prisma")]
pub mod prisma;

#[cfg(feature = "diesel")]
pub mod diesel;

#[cfg(feature = "seaorm")]
pub mod seaorm;

mod converter;

pub use error::{ImportError, ImportResult};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use crate::error::{ImportError, ImportResult};

    #[cfg(feature = "prisma")]
    pub use crate::prisma::{import_prisma_schema, import_prisma_schema_file};

    #[cfg(feature = "diesel")]
    pub use crate::diesel::{import_diesel_schema, import_diesel_schema_file};

    #[cfg(feature = "seaorm")]
    pub use crate::seaorm::{import_seaorm_entity, import_seaorm_entity_file};
}
