//! DSL → TokenStream lowering for the read-operation macros.
//!
//! Each submodule lowers one piece of the per-model input shape.
//! Filled in by tasks 7-10.

pub(crate) mod aggregate_select;
pub(crate) mod data_input;
pub(crate) mod data_relation;
pub(crate) mod group_by_order_by;
pub(crate) mod having;
pub(crate) mod include_input;
pub(crate) mod order_by_input;
pub(crate) mod scalar_filter;
pub(crate) mod select_input;
pub(crate) mod where_input;

use prax_schema::{Model, Schema};
use proc_macro2::{Span, TokenStream};

/// Context threaded through every lowering pass.
///
/// Holds references to the schema and the model the current shape
/// applies to, plus the `crate_root` token stream the lowered output
/// uses as a path prefix (usually `::prax` for downstream users, or
/// `::prax_codegen` for internal callers).
#[allow(dead_code)]
pub struct LowerCtx<'a> {
    /// Whole schema — needed when lowering relation filters that
    /// switch context to the target model.
    pub schema: &'a Schema,
    /// The model this shape applies to.
    pub model: &'a Model,
    /// Crate-root prefix for generated paths (always `::prax`).
    pub crate_root: TokenStream,
}

#[allow(dead_code)]
impl<'a> LowerCtx<'a> {
    /// Construct a context for the given model.
    pub fn new(schema: &'a Schema, model: &'a Model) -> Self {
        Self {
            schema,
            model,
            crate_root: quote::quote!(::prax),
        }
    }

    /// Switch the context to a different model — used when lowering
    /// nested relation filters.
    pub fn for_model<'b>(&self, model: &'b Model) -> LowerCtx<'b>
    where
        'a: 'b,
    {
        LowerCtx {
            schema: self.schema,
            model,
            crate_root: self.crate_root.clone(),
        }
    }
}

/// `select` xor `include`: at most one may be set on any read input.
///
/// Returns an error pointing at `dup_span` (the second of the two keys
/// in source order) so the diagnostic lands where the user added the
/// conflict.
#[allow(dead_code)]
pub fn check_select_include_xor(
    has_select: bool,
    has_include: bool,
    dup_span: Span,
) -> syn::Result<()> {
    if has_select && has_include {
        return Err(syn::Error::new(
            dup_span,
            "`select` and `include` are mutually exclusive — choose one",
        ));
    }
    Ok(())
}
