//! `find_first!` proc-macro entry point.
//!
//! `find_first` always limits to one row in the runtime IR. The DSL
//! accepts `where`, `order_by`, `include`, and `select` — `skip`,
//! `take`, and `cursor` are explicitly rejected because the IR has no
//! place to thread them. (Use `find_many!` with `take: 1` to get an
//! "Nth-match" workflow.)

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::include_input::lower_include;
use crate::macros::lower::order_by_input::lower_order_by;
use crate::macros::lower::select_input::lower_select;
use crate::macros::lower::where_input::lower_where;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const FIND_FIRST_KEYS: &[&str] = &["where", "order_by", "include", "select"];

pub fn expand_find_first(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_find_first(&accessor, &block, &ctx)
    };
    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_find_first(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut include_tokens: Option<TokenStream> = None;
    let mut select_tokens: Option<TokenStream> = None;
    let mut order_by_tokens: Option<TokenStream> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`find_first!` does not accept spread or conditional fields at the top level",
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
            "include" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`include:` expects `{ ... }`"));
                };
                include_tokens = Some(lower_include(b, ctx)?);
            }
            "select" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`select:` expects `{ ... }`"));
                };
                select_tokens = Some(lower_select(b, ctx)?);
            }
            "order_by" => order_by_tokens = Some(lower_order_by(value, ctx)?),
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    FIND_FIRST_KEYS,
                    "find_first",
                ));
            }
        }
    }

    if select_tokens.is_some() && include_tokens.is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`select` and `include` are mutually exclusive — choose one",
        ));
    }

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = Vec::new();
    if let Some(w) = where_tokens {
        chain.push(quote! { .with_where_input(#w) });
    }
    if let Some(i) = include_tokens {
        chain.push(quote! { .with_include_input(#i) });
    }
    if let Some(s) = select_tokens {
        chain.push(quote! { .with_select_input(#s) });
    }
    if let Some(ob) = order_by_tokens {
        chain.push(quote! { .order_by(#ob) });
    }

    Ok(quote! {
        (#accessor_expr).find_first() #(#chain)*
    })
}
