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

// The four sibling modules are stubs in phase 1; their pub-use lines will
// resolve to concrete items once tasks 6-11 populate them.
#[allow(unused_imports)]
pub use args::*;
#[allow(unused_imports)]
pub use relation::*;
#[allow(unused_imports)]
pub use scalar::*;
#[allow(unused_imports)]
pub use scalar_update::*;
pub use traits::*;
