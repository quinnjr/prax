//! Generate per-relation `RelationFilterMeta` impls.
//!
//! For each declared relation, emit a zero-sized marker struct
//! `<Model><Relation>FilterMeta` and an `impl RelationFilterMeta` with
//! the parent table/PK and child table/FK as const &'static str.
//!
//! Returns `(struct_tokens, impl_tokens)` — markers in the per-model
//! `pub mod`, impls at crate-root scope.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

pub struct RelationMetaSpec {
    /// Marker struct ident, e.g. `UserPostsFilterMeta`.
    pub meta_ident: Ident,
    /// Parent table name.
    pub parent_table: String,
    /// Parent PK column name.
    pub parent_pk: String,
    /// Child table name.
    pub child_table: String,
    /// Child FK column name.
    pub child_fk: String,
}

pub fn generate(module_name: &Ident, specs: &[RelationMetaSpec]) -> (TokenStream, TokenStream) {
    if specs.is_empty() {
        return (TokenStream::new(), TokenStream::new());
    }

    let marker_decls = specs.iter().map(|s| {
        let m = &s.meta_ident;
        quote! {
            #[doc(hidden)]
            pub struct #m;
        }
    });

    let impl_blocks = specs.iter().map(|s| {
        let m = &s.meta_ident;
        let pt = &s.parent_table;
        let pp = &s.parent_pk;
        let ct = &s.child_table;
        let cf = &s.child_fk;
        quote! {
            impl ::prax_query::inputs::RelationFilterMeta for #module_name::#m {
                const PARENT_TABLE: &'static str = #pt;
                const PARENT_PK: &'static str = #pp;
                const CHILD_TABLE: &'static str = #ct;
                const CHILD_FK: &'static str = #cf;
            }
        }
    });

    let struct_tokens = quote! { #(#marker_decls)* };
    let impl_tokens = quote! { #(#impl_blocks)* };

    (struct_tokens, impl_tokens)
}
