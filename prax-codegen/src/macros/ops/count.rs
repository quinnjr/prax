//! `count!` proc-macro entry point.
//!
//! The runtime `CountOperation` only supports `where:` (plus an
//! internal `distinct` column for future use). The DSL accepts
//! `where:` only. `select:` (for the Prisma-style `_count`
//! aggregate-spec) is rejected with a phase-6 marker, per the plan.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::where_input::lower_where;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const COUNT_KEYS: &[&str] = &["where"];

pub fn expand_count(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_count(&accessor, &block, &ctx)
    };
    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_count(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`count!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                where_tokens = Some(lower_where(b, ctx)?);
            }
            "select" => {
                return Err(syn::Error::new(
                    key.span(),
                    "`select:` on `count!` (Prisma-style `_count` aggregate) is a phase-6 feature \
                     not yet implemented. Track progress in the typed-query-traits design.",
                ));
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    COUNT_KEYS,
                    "count",
                ));
            }
        }
    }

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = Vec::new();
    if let Some(w) = where_tokens {
        chain.push(quote! { .with_where_input(#w) });
    }
    Ok(quote! {
        (#accessor_expr).count() #(#chain)*
    })
}
