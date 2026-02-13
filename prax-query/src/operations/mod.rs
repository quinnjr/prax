//! Query operations for the fluent API.
//!
//! This module provides the various query operation types:
//! - `FindManyOperation` - Find multiple records
//! - `FindUniqueOperation` - Find one record by unique constraint
//! - `FindFirstOperation` - Find the first matching record
//! - `CreateOperation` - Create a new record
//! - `UpdateOperation` - Update existing records
//! - `DeleteOperation` - Delete records
//! - `UpsertOperation` - Create or update a record
//! - `CountOperation` - Count matching records
//! - `AggregateOperation` - Aggregate operations (sum, avg, min, max)
//! - `GroupByOperation` - Group by with aggregation
//!
//! ## View Operations (Read-Only)
//! - `ViewFindManyOperation` - Find multiple records from a view
//! - `ViewFindFirstOperation` - Find the first matching record from a view
//! - `ViewCountOperation` - Count records in a view
//! - `RefreshMaterializedViewOperation` - Refresh a materialized view
//! - `ViewQueryBuilder` - Query builder for views

mod aggregate;
mod count;
mod create;
mod delete;
mod find_first;
mod find_many;
mod find_unique;
mod update;
mod upsert;
mod view;

pub use aggregate::{
    AggregateField, AggregateOperation, AggregateResult, GroupByOperation, GroupByResult,
    HavingCondition, HavingOp, having,
};
pub use count::CountOperation;
pub use create::{CreateManyOperation, CreateOperation};
pub use delete::{DeleteManyOperation, DeleteOperation};
pub use find_first::FindFirstOperation;
pub use find_many::FindManyOperation;
pub use find_unique::FindUniqueOperation;
pub use update::{UpdateManyOperation, UpdateOperation};
pub use upsert::UpsertOperation;
pub use view::{
    MaterializedViewAccessor, RefreshMaterializedViewOperation, ViewAccessor, ViewCountOperation,
    ViewFindFirstOperation, ViewFindManyOperation, ViewQueryBuilder,
};
