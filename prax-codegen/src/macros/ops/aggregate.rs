//! `aggregate!` proc-macro entry point.
//!
//! Lowers a `{ where: ..., _count: ..., _sum: ..., _avg: ..., _min: ...,
//! _max: ... }` brace block into an
//! `<accessor>.aggregate().with_aggregate_args(<Model>AggregateArgs { ... })`
//! token stream.

use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::aggregate_select::{AggKind, lower_agg_select};
use crate::macros::lower::where_input::lower_where_input_only;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const AGGREGATE_KEYS: &[&str] = &["where", "_count", "_sum", "_avg", "_min", "_max"];

pub fn expand_aggregate(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_aggregate(&accessor, &block, &ctx)
    };
    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_aggregate(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_block: Option<&DslBlock> = None;
    let mut count_block: Option<&DslBlock> = None;
    let mut sum_block: Option<&DslBlock> = None;
    let mut avg_block: Option<&DslBlock> = None;
    let mut min_block: Option<&DslBlock> = None;
    let mut max_block: Option<&DslBlock> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`aggregate!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                where_block = Some(b);
            }
            "_count" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`_count:` expects `{ ... }`"));
                };
                count_block = Some(b);
            }
            "_sum" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`_sum:` expects `{ ... }`"));
                };
                sum_block = Some(b);
            }
            "_avg" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`_avg:` expects `{ ... }`"));
                };
                avg_block = Some(b);
            }
            "_min" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`_min:` expects `{ ... }`"));
                };
                min_block = Some(b);
            }
            "_max" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`_max:` expects `{ ... }`"));
                };
                max_block = Some(b);
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    AGGREGATE_KEYS,
                    "aggregate",
                ));
            }
        }
    }

    if count_block.is_none()
        && sum_block.is_none()
        && avg_block.is_none()
        && min_block.is_none()
        && max_block.is_none()
    {
        return Err(syn::Error::new(
            Span::call_site(),
            "aggregate! requires at least one of _count, _sum, _avg, _min, _max",
        ));
    }

    let lower_opt = |k: AggKind, b: Option<&DslBlock>| -> syn::Result<TokenStream> {
        match b {
            Some(blk) => {
                let ts = lower_agg_select(k, blk, ctx)?;
                Ok(quote! { ::core::option::Option::Some(#ts) })
            }
            None => Ok(quote! { ::core::option::Option::None }),
        }
    };

    let count_ts = lower_opt(AggKind::Count, count_block)?;
    let sum_ts = lower_opt(AggKind::Sum, sum_block)?;
    let avg_ts = lower_opt(AggKind::Avg, avg_block)?;
    let min_ts = lower_opt(AggKind::Min, min_block)?;
    let max_ts = lower_opt(AggKind::Max, max_block)?;

    let where_ts = match where_block {
        Some(wb) => {
            let w = lower_where_input_only(wb, ctx)?;
            quote! { ::core::option::Option::Some(#w) }
        }
        None => quote! { ::core::option::Option::None },
    };

    let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
    let args_ident = format_ident!("{}AggregateArgs", ctx.model.name());
    let accessor_expr = &accessor.accessor_expr;

    Ok(quote! {
        {
            let __args: #module_ident::#args_ident = #module_ident::#args_ident {
                where_input: #where_ts,
                _count: #count_ts,
                _sum: #sum_ts,
                _avg: #avg_ts,
                _min: #min_ts,
                _max: #max_ts,
            };
            (#accessor_expr).aggregate().with_aggregate_args(__args)
        }
    })
}
