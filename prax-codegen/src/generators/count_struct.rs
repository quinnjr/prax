//! Emits the `<Model>Count` synthetic struct used by the
//! `select: { _count: { rel: true } }` ad-hoc accessor and by
//! schema-level relation aggregates.
//!
//! # Design
//!
//! For every model that has at least one outgoing relation, the emitter
//! produces a public struct named `<Model>Count` with one `pub <rel>:
//! Option<i64>` field per outgoing relation.  The struct derives
//! `Debug`, `Clone`, and `Default` so callers can zero-initialize it
//! and fill in only the relations they selected.
//!
//! Models with **zero** outgoing relations: no struct is emitted.
//! Attempting to use `_count` on such a model becomes a compile-time
//! error (enforced in Task 14 macro lowering / Task 15 trybuild tests).
//!
//! # Deferred work
//!
//! Adding a `pub _count: Option<<Model>Count>` field to the user's own
//! model struct is intentionally deferred to Task 14.  The `#[derive(Model)]`
//! macro has no way to inject new fields into the struct it is applied
//! to — the struct definition is already fixed by the time proc-macro
//! expansion runs.  Task 14 will return `<Model>Count` values through a
//! richer query result type (e.g., `FindManyResult<T>` carrying a
//! parallel `Vec<Option<<Model>Count>>`).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Information about an outgoing relation needed to emit a single
/// `Option<i64>` field on `<Model>Count`.
pub struct OutgoingRelation<'a> {
    /// Field name on the parent model (e.g., `"posts"`, `"comments"`).
    pub field_name: &'a str,
}

/// Emit the `<Model>Count` synthetic struct, or `None` if the model
/// has no outgoing relations.
///
/// The returned `TokenStream` (when `Some`) is a top-level item — the
/// caller is responsible for placing it in the correct scope.  For the
/// `#[derive(Model)]` path it should be emitted at the same level as
/// the derive output (crate-root scope, outside the `pub mod <model>`
/// module).  For the schema path it should be emitted at the same
/// level as the generated model struct.
pub fn emit_count_struct(
    model_ident: &syn::Ident,
    outgoing: &[OutgoingRelation<'_>],
) -> Option<TokenStream> {
    if outgoing.is_empty() {
        return None;
    }

    let struct_name = format_ident!("{}Count", model_ident);
    let fields = outgoing.iter().map(|rel| {
        let f = format_ident!("{}", rel.field_name);
        quote! { pub #f: Option<i64> }
    });

    Some(quote! {
        /// Synthetic aggregate-count struct for the
        #[doc = concat!("`", stringify!(#model_ident), "`")]
        /// model.
        ///
        /// Contains one `Option<i64>` per outgoing relation.  A field
        /// is `Some(n)` when the corresponding relation count was
        /// requested via `select: { _count: { <rel>: true } }`;
        /// otherwise it is `None`.
        ///
        /// Use `<Model>Count::default()` to obtain a zero-initialised
        /// value with all counts set to `None`.
        #[derive(Debug, Clone, Default)]
        pub struct #struct_name {
            #(#fields,)*
        }
    })
}
