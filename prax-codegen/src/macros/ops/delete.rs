//! `delete!` proc-macro entry point.
//!
//! The `where:` block lowers to `<Model>WhereUniqueInput` (the same
//! shape as `find_unique!`) — the deletion is intentionally precise.
//! `include:` / `select:` describe the return-shape.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::order_by_input::lower_cursor;
use crate::macros::lower::select_input::lower_select_struct_only;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const DELETE_KEYS: &[&str] = &["where", "select"];

pub fn expand_delete(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = resolve_schema()?;
    let schema_path = resolve_schema_path()?;
    let dep = track_schema_dep(&schema_path);

    let parser = move |s: syn::parse::ParseStream<'_>| -> syn::Result<TokenStream> {
        let (accessor, model) = parse_accessor(s, &schema)?;
        if s.peek(Token![,]) {
            let _: Token![,] = s.parse()?;
        }
        let block: DslBlock = s.parse()?;
        let ctx = LowerCtx::new(&schema, model);
        lower_delete(&accessor, &block, &ctx)
    };
    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_delete(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut select_tokens: Option<TokenStream> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`delete!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                where_tokens = Some(lower_cursor(b, ctx)?);
            }
            "select" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`select:` expects `{ ... }`"));
                };
                select_tokens = Some(lower_select_struct_only(b, ctx)?);
            }
            "include" => {
                return Err(syn::Error::new(
                    key.span(),
                    "`include:` on `delete!` (returning relation rows on a deleted parent) \
                     is a phase-5 feature. Use `select:` for the return-shape if needed.",
                ));
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    DELETE_KEYS,
                    "delete",
                ));
            }
        }
    }

    let where_tokens = where_tokens.ok_or_else(|| {
        syn::Error::new(
            block.span,
            "`delete!` requires a `where:` block matching a unique column",
        )
    })?;

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = vec![quote! { .with_where_input(#where_tokens) }];
    if let Some(s) = select_tokens {
        chain.push(quote! { .with_select_input(#s) });
    }
    Ok(quote! {
        (#accessor_expr).delete() #(#chain)*
    })
}
