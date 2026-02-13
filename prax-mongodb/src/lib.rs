//! # prax-mongodb
//!
//! MongoDB driver for the Prax ORM with document mapping and aggregation support.
//!
//! This crate provides:
//! - Connection management with the official MongoDB driver
//! - Built-in connection pooling
//! - Document serialization/deserialization via BSON
//! - Type-safe query building
//! - Aggregation pipeline support
//! - Change streams for real-time updates
//!
//! ## Example
//!
//! ```rust,ignore
//! use prax_mongodb::MongoClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a client (connection pooling is built-in)
//!     let client = MongoClient::builder()
//!         .uri("mongodb://localhost:27017")
//!         .database("mydb")
//!         .build()
//!         .await?;
//!
//!     // Get a collection
//!     let users = client.collection::<User>("users");
//!
//!     // Insert a document
//!     users.insert_one(User { name: "Alice".into(), age: 30 }).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Document Mapping
//!
//! Models can be mapped to MongoDB documents using serde:
//!
//! ```rust,ignore
//! use serde::{Deserialize, Serialize};
//! use prax_mongodb::ObjectId;
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct User {
//!     #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
//!     id: Option<ObjectId>,
//!     name: String,
//!     email: String,
//! }
//! ```

pub mod client;
pub mod config;
pub mod document;
pub mod engine;
pub mod error;
pub mod filter;
pub mod types;
pub mod view;

pub use bson::oid::ObjectId;
pub use bson::{Bson, Document, doc};
pub use client::{MongoClient, MongoClientBuilder};
pub use config::{MongoConfig, MongoConfigBuilder};
pub use engine::MongoEngine;
pub use error::{MongoError, MongoResult};
pub use filter::FilterBuilder;
pub use view::{
    AggregationView, AggregationViewBuilder, MaterializedAggregationView, MergeAction,
    MergeNotMatchedAction, MergeOptions,
};

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::client::{MongoClient, MongoClientBuilder};
    pub use crate::config::{MongoConfig, MongoConfigBuilder};
    pub use crate::document::DocumentExt;
    pub use crate::engine::MongoEngine;
    pub use crate::error::{MongoError, MongoResult};
    pub use crate::filter::FilterBuilder;
    pub use crate::view::{
        AggregationView, AggregationViewBuilder, MaterializedAggregationView, MergeAction,
        MergeNotMatchedAction, MergeOptions, accumulators, stages,
    };
    pub use bson::oid::ObjectId;
    pub use bson::{Bson, Document, doc};
}
