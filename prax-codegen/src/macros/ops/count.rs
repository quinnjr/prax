//! `count!` proc-macro entry point.

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

const COUNT_KEYS: &[&str] = &["where", "select"];

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
    let mut where_block: Option<&DslBlock> = None;
    let mut select_block: Option<&DslBlock> = None;

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
                where_block = Some(b);
            }
            "select" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`select:` expects `{ ... }`"));
                };
                select_block = Some(b);
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

    if let Some(sel_block) = select_block {
        let count_select_ts = lower_agg_select(AggKind::Count, sel_block, ctx)?;

        let where_ts = match where_block {
            Some(wb) => {
                let w = lower_where_input_only(wb, ctx)?;
                quote! { ::core::option::Option::Some(#w) }
            }
            None => quote! { ::core::option::Option::None },
        };

        let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
        let args_ident = format_ident!("{}AggregateArgs", ctx.model.name());

        return Ok(quote! {
            {
                let __args: #module_ident::#args_ident = #module_ident::#args_ident {
                    where_input: #where_ts,
                    _count: ::core::option::Option::Some(#count_select_ts),
                    .. ::core::default::Default::default()
                };
                (#accessor_expr).aggregate().with_aggregate_args(__args)
            }
        });
    }

    let mut chain: Vec<TokenStream> = Vec::new();
    if let Some(wb) = where_block {
        let w = lower_where_input_only(wb, ctx)?;
        chain.push(quote! { .with_where_input(#w) });
    }
    Ok(quote! {
        (#accessor_expr).count() #(#chain)*
    })
}
