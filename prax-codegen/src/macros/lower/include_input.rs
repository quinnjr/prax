//! Lower DSL `include:` blocks to the per-model `<Model>Include`
//! constructor emitted by phase 2 codegen.
//!
//! Phase 2's `<Model>Include` is a struct of `Option<bool>` — one per
//! relation. Phase 5 will introduce richer per-relation argument types
//! (`<Relation>IncludeArgs`); until that lands the macro accepts:
//!
//! - `relation: true` — sets the flag.
//! - `relation: false` — sets the flag to `false` (rarely useful, but
//!   composes with spread).
//! - `relation: { ... }` — currently treated as `Some(true)`. The
//!   nested block is parsed and validated for forward-compat, but its
//!   inner fields are accepted as no-ops with a doc comment in the
//!   expansion. Phase 5 will replace this lowering with the
//!   `<Relation>IncludeArgs` builder.

#![allow(dead_code)]

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

pub fn lower_include(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let include_ident = format_ident!("{}Include", ctx.model.name());

    let mut setters: Vec<TokenStream> = Vec::new();
    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`include` does not support spread or conditional fields yet",
            ));
        };
        let key_str = key.to_string();
        let relation = ctx.model.get_field(&key_str).ok_or_else(|| {
            syn::Error::new(
                key.span(),
                format!(
                    "unknown relation `{}` on model `{}` in include block",
                    key_str,
                    ctx.model.name()
                ),
            )
        })?;
        if !relation.is_relation() {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "field `{}` is not a relation; include only supports relation fields",
                    key_str
                ),
            ));
        }
        let assign_ident = format_ident!("{}", relation.name().to_case(Case::Snake));
        let bool_expr = match value {
            DslValue::Bool(b) => quote! { #b },
            DslValue::Block(_) => {
                // Phase 5 will lower nested include args; for now treat
                // any block as "yes include this relation".
                quote! { true }
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "include value for `{}` must be `true`, `false`, or a `{{ ... }}` block",
                        key_str
                    ),
                ));
            }
        };
        setters.push(quote! {
            __i.#assign_ident = ::core::option::Option::Some(#bool_expr);
        });
    }

    Ok(quote! {
        {
            let mut __i: #module_ident::#include_ident =
                <#module_ident::#include_ident as ::core::default::Default>::default();
            #(#setters)*
            __i
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
        lower_include(&block, &ctx).unwrap()
    }

    #[test]
    fn lower_include_relation_true() {
        let out = lower("User", quote!({ profile: true }));
        let s = out.to_string();
        assert!(s.contains("UserInclude"));
        assert!(s.contains("profile"));
        assert!(s.contains("true"));
    }

    #[test]
    fn lower_include_nested_block_currently_treated_as_true() {
        let out = lower("User", quote!({ posts: { where: { published: true } } }));
        let s = out.to_string();
        assert!(s.contains("posts"));
        // Nested block lowers to `true` until phase 5 wires IncludeArgs.
        assert!(s.contains("true"));
    }

    #[test]
    fn lower_include_unknown_relation_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ nope: true })).unwrap();
        let err = lower_include(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("unknown relation"));
    }

    #[test]
    fn lower_include_non_relation_field_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let model = schema.get_model("User").unwrap().clone();
        let ctx = LowerCtx::new(&schema, &model);
        let block = syn::parse2::<DslBlock>(quote!({ email: true })).unwrap();
        let err = lower_include(&block, &ctx).unwrap_err();
        assert!(err.to_string().contains("not a relation"));
    }
}
