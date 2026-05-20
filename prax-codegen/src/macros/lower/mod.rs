//! DSL → TokenStream lowering for the read-operation macros.
//!
//! Each submodule lowers one piece of the per-model input shape.
//! Filled in by tasks 7-10.

pub(crate) mod include_input;
pub(crate) mod order_by_input;
pub(crate) mod scalar_filter;
pub(crate) mod select_input;
pub(crate) mod where_input;
