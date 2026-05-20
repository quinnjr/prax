//! Per-operation entry points for the read-operation macros.
//!
//! Each submodule exposes one `expand_*` function that the matching
//! top-level `#[proc_macro] fn <op>` wrapper in `lib.rs` calls.
//! Filled in by tasks 13-15.

pub(crate) mod count;
pub(crate) mod create;
pub(crate) mod delete;
pub(crate) mod delete_many;
pub(crate) mod find_first;
pub(crate) mod find_many;
pub(crate) mod find_unique;
pub(crate) mod shape;
pub(crate) mod update;
pub(crate) mod upsert;
