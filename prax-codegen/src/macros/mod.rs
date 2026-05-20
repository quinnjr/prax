//! Schema-aware proc-macro pipeline for the read-operation DSL
//! (`find_unique!`, `find_first!`, `find_many!`, `count!`, `delete!`,
//! `delete_many!`). Phase 3 of the typed-query-traits work.
//!
//! Pipeline:
//!   parse TokenStream
//!     -> resolve schema (env var / walk-up prax.toml)
//!     -> resolve accessor expression and model
//!     -> parse DSL brace block into a typed AST
//!     -> validate AST against schema (unknown field, wrong op, ...)
//!     -> lower AST to TokenStream constructing layer-2 input structs
//!     -> emit chained `with_*_input(...)` calls on the operation

pub(crate) mod accessor;
pub(crate) mod dsl;
pub(crate) mod lower;
pub(crate) mod ops;
pub(crate) mod schema_resolve;
pub(crate) mod shape_accessor;
pub(crate) mod validate;
