//! Lower DSL `select:` blocks to the per-model `<Model>Select`
//! constructor emitted by phase 2 codegen.
//!
//! Phase 2's `<Model>Select` is a struct of `Option<bool>` — one per
//! scalar field plus one per relation. Relation fields under `select`
//! gate the relation off in the lowered IR (`Select::Fields`) the same
//! way scalar fields do. Nested per-relation args (`select: { posts:
//! { where: ... } }`) are accepted but flattened to `Some(true)`
//! until phase 5 introduces `<Relation>SelectArgs`.

#![allow(dead_code)]

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

pub fn lower_select(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let select_ident = format_ident!("{}Select", ctx.model.name());

    let mut setters: Vec<TokenStream> = Vec::new();
    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`select` does not support spread or conditional fields yet",
            ));
        };
        let key_str = key.to_string();
        let target = ctx.model.get_field(&key_str).ok_or_else(|| {
            syn::Error::new(
                key.span(),
                format!(
                    "unknown field `{}` on model `{}` in select block",
                    key_str,
                    ctx.model.name()
                ),
            )
        })?;
        let assign_ident = format_ident!("{}", target.name().to_case(Case::Snake));
        let bool_expr = match value {
            DslValue::Bool(b) => quote! { #b },
            DslValue::Block(_) => quote! { true },
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "select value for `{}` must be `true`, `false`, or a `{{ ... }}` block",
                        key_str
                    ),
                ));
            }
        };
        setters.push(quote! {
            __s.#assign_ident = ::core::option::Option::Some(#bool_expr);
        });
    }

    Ok(quote! {
        {
            let mut __s: #module_ident::#select_ident =
                <#module_ident::#select_ident as ::core::default::Default>::default();
            #(#setters)*
            __s
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use quote::quote;

    const SCHEMA: &str = include_str!("../../../tests/fixtures/schema.prax");

    fn lower(model_name: &str, tokens: TokenStream) -> TokenStream {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model(model_name).unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(tokens).unwrap();
        lower_select(&block, &ctx).unwrap()
    }

    #[test]
    fn lower_select_mixed_scalar_and_relation() {
        let out = lower("User", quote!({ id: true, email: true, profile: true }));
        let s = out.to_string();
        assert!(s.contains("UserSelect"));
        assert!(s.contains("id"));
        assert!(s.contains("email"));
        assert!(s.contains("profile"));
    }

    #[test]
    fn lower_select_unknown_field_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ nope: true })).unwrap();
        let err = lower_select(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }
}
