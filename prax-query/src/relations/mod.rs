#![allow(dead_code)]

//! Relation loading support for eager and lazy loading.
//!
//! This module provides types and operations for loading related data:
//! - `Include` for eager loading relations
//! - `Select` for specifying which fields to return
//! - Nested relation specifications
//!
//! ## Example
//!
//! ```rust,ignore
//! // Eager load posts with their author
//! let posts = client
//!     .post()
//!     .find_many()
//!     .include(post::author::fetch())
//!     .include(post::comments::fetch().take(5))
//!     .exec()
//!     .await?;
//!
//! // Select specific fields
//! let users = client
//!     .user()
//!     .find_many()
//!     .select(user::select! {
//!         id,
//!         email,
//!         posts: { id, title }
//!     })
//!     .exec()
//!     .await?;
//! ```

pub mod executor;
mod include;
mod loader;
mod meta;
mod select;
mod spec;

pub use include::{Include, IncludeSpec};
pub use loader::{RelationLoadStrategy, RelationLoader};
pub use meta::{RelationKind, RelationMeta};
pub use select::{FieldSelection, SelectSpec};
pub use spec::{RelationSpec, RelationType};
