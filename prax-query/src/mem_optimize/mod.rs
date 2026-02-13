//! Advanced memory optimizations for prax-query.
//!
//! This module provides high-performance memory management utilities:
//!
//! - **Enhanced string interning**: Global and scoped interning with auto-intern for identifiers
//! - **Typed arena allocators**: Efficient arena allocation for query builder chains
//! - **Lazy schema parsing**: On-demand parsing of introspection results
//!
//! # Performance Gains
//!
//! | Optimization | Feature | Memory Reduction |
//! |--------------|---------|------------------|
//! | String interning | All query builders | 20-30% |
//! | Arena allocation | High-throughput queries | 15-25% |
//! | Lazy parsing | Introspection | 40-50% |
//!
//! # Example
//!
//! ```rust,ignore
//! use prax_query::mem_optimize::{
//!     interning::{GlobalInterner, ScopedInterner},
//!     arena::{QueryArena, ArenaAllocated},
//!     lazy::{LazySchema, LazyColumn},
//! };
//!
//! // Global string interning for identifiers
//! let field = GlobalInterner::get().intern("user_id");
//!
//! // Scoped arena for query building
//! let arena = QueryArena::new();
//! arena.scope(|scope| {
//!     let filter = scope.alloc_filter(/* ... */);
//!     let query = scope.build_query(filter);
//!     query.to_sql() // Returns owned SQL, arena freed on scope exit
//! });
//!
//! // Lazy schema parsing
//! let schema = LazySchema::from_raw(raw_data);
//! // Columns only parsed when accessed
//! let name = schema.get_table("users")?.get_column("name")?.db_type();
//! ```

pub mod arena;
pub mod interning;
pub mod lazy;

pub use arena::{ArenaScope, QueryArena, ScopedFilter, ScopedQuery, ScopedValue};
pub use interning::{GlobalInterner, IdentifierCache, InternedStr, ScopedInterner};
pub use lazy::{LazyColumn, LazyForeignKey, LazyIndex, LazySchema, LazyTable, ParseOnDemand};
