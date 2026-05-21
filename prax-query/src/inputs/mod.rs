//! Typed input shapes for the Prisma-style DSL.
//!
//! This module holds the trait spine (`WhereInput`, `IncludeInput`, …),
//! the reusable scalar filter wrappers (`StringFilter`, `IntFilter`, …),
//! the relation filter wrappers (`ListRelationFilter`,
//! `SingleRelationFilter`), the scalar update wrappers
//! (`IntFieldUpdate`, `StringFieldUpdate`, …), and the per-operation
//! containers (`FindManyArgs`, `CreateArgs`, …).
//!
//! Codegen (phase 2) emits per-model concrete structs that implement
//! these traits and use these wrappers. The operation macros (phase 3+)
//! emit token streams that construct these inputs and feed them to
//! existing `*Operation` builders via `with_*_input` extension methods.
//!
//! Layer-1 callers can also build these inputs by hand — they form the
//! "third interface" alongside the macro DSL and the existing fluent
//! builder.

pub mod args;
pub mod relation;
pub mod scalar;
pub mod scalar_update;
pub mod traits;
pub mod write_payload;

pub use args::*;
pub use relation::*;
pub use scalar::*;
pub use scalar_update::*;
pub use traits::*;
pub use write_payload::*;
