//! `update!` proc-macro entry point.
//!
//! Top-level keys: `where:` (required, unique), `data:` (required),
//! `include` xor `select`. The `where:` block lowers to
//! `<Model>WhereUniqueInput` (the same form `find_unique!` /
//! `delete!` use) — call sites that need a non-unique filter must use
//! `update_many!` instead.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::data_input::lower_update_data;
use crate::macros::lower::include_input::lower_include;
use crate::macros::lower::order_by_input::lower_cursor;
use crate::macros::lower::select_input::lower_select;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const UPDATE_KEYS: &[&str] = &["where", "data", "include", "select"];

pub fn expand_update(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_update(&accessor, &block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_update(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut data_tokens: Option<TokenStream> = None;
    let mut include_tokens: Option<TokenStream> = None;
    let mut select_tokens: Option<TokenStream> = None;
    let mut select_span: Option<Span> = None;
    let mut include_span: Option<Span> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`update!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                // `update!` requires a unique-key match — reuse the
                // phase-3 cursor lowerer that targets WhereUniqueInput.
                where_tokens = Some(lower_cursor(b, ctx)?);
            }
            "data" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`data:` expects `{ ... }`"));
                };
                data_tokens = Some(lower_update_data(b, ctx)?);
            }
            "include" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`include:` expects `{ ... }`"));
                };
                include_tokens = Some(lower_include(b, ctx)?);
                include_span = Some(key.span());
            }
            "select" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`select:` expects `{ ... }`"));
                };
                select_tokens = Some(lower_select(b, ctx)?);
                select_span = Some(key.span());
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    UPDATE_KEYS,
                    "update",
                ));
            }
        }
    }

    if select_tokens.is_some() && include_tokens.is_some() {
        let span = select_span.or(include_span).unwrap_or_else(Span::call_site);
        return Err(syn::Error::new(
            span,
            "`select` and `include` are mutually exclusive — choose one",
        ));
    }

    let where_tokens = where_tokens.ok_or_else(|| {
        syn::Error::new(
            block.span,
            "`update!` requires a `where:` block matching a unique column",
        )
    })?;
    let data_tokens = data_tokens
        .ok_or_else(|| syn::Error::new(block.span, "`update!` requires a `data:` block"))?;

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = vec![
        quote! { .with_where_input(#where_tokens) },
        quote! { .with_update_input(#data_tokens) },
    ];
    if let Some(i) = include_tokens {
        chain.push(quote! { .with_include_input(#i) });
    }
    if let Some(s) = select_tokens {
        chain.push(quote! { .with_select_input(#s) });
    }

    Ok(quote! {
        (#accessor_expr).update() #(#chain)*
    })
}
