//! `update_many!` proc-macro entry point.
//!
//! Top-level keys: `where:` (optional, non-unique form), `data:`
//! (required). Atomic operators inside `data:` work the same way as
//! on the single-row `update!`.
//!
//! **Warning:** an empty / missing `where:` block produces
//! `Filter::None`, which lowers to `WHERE TRUE` and matches every
//! row. See the trait-level note on `WhereInput`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::data_input::lower_update_data;
use crate::macros::lower::where_input::lower_where_input_only;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const UPDATE_MANY_KEYS: &[&str] = &["where", "data"];

pub fn expand_update_many(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_update_many(&accessor, &block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_update_many(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut data_tokens: Option<TokenStream> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`update_many!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                where_tokens = Some(lower_where_input_only(b, ctx)?);
            }
            "data" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`data:` expects `{ ... }`"));
                };
                data_tokens = Some(lower_update_data(b, ctx)?);
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    UPDATE_MANY_KEYS,
                    "update_many",
                ));
            }
        }
    }

    let data_tokens = data_tokens
        .ok_or_else(|| syn::Error::new(block.span, "`update_many!` requires a `data:` block"))?;

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = Vec::new();
    if let Some(w) = where_tokens {
        chain.push(quote! { .with_where_input(#w) });
    }
    chain.push(quote! { .with_update_input(#data_tokens) });

    Ok(quote! {
        (#accessor_expr).update_many() #(#chain)*
    })
}
