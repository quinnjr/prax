//! `group_by!` proc-macro entry point.
//!
//! Lowers a `{ by: [...], where: ..., _count: ..., _sum: ..., _avg: ...,
//! _min: ..., _max: ..., having: ... }` brace block into a
//! `<accessor>.group_by_columns(by).with_group_by_args(<Model>GroupByArgs { ... })`
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
use crate::macros::lower::group_by_order_by::{AggPresence, lower_group_by_order_by};
use crate::macros::lower::having::lower_having;
use crate::macros::lower::where_input::lower_where_input_only;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const GROUP_BY_KEYS: &[&str] = &[
    "by", "where", "_count", "_sum", "_avg", "_min", "_max", "having", "order_by",
];

pub fn expand_group_by(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_group_by(&accessor, &block, &ctx)
    };
    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_group_by(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut by_value: Option<(&DslValue, proc_macro2::Span)> = None;
    let mut where_block: Option<&DslBlock> = None;
    let mut count_block: Option<&DslBlock> = None;
    let mut sum_block: Option<&DslBlock> = None;
    let mut avg_block: Option<&DslBlock> = None;
    let mut min_block: Option<&DslBlock> = None;
    let mut max_block: Option<&DslBlock> = None;
    let mut having_block: Option<&DslBlock> = None;
    let mut order_by_block: Option<&DslBlock> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`group_by!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "by" => {
                by_value = Some((value, key.span()));
            }
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
            "having" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`having:` expects `{ ... }`"));
                };
                having_block = Some(b);
            }
            "order_by" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`order_by:` expects `{ ... }`"));
                };
                order_by_block = Some(b);
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    GROUP_BY_KEYS,
                    "group_by",
                ));
            }
        }
    }

    let (by_val, by_span) = by_value.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "group_by! requires a `by: [...]` list of columns",
        )
    })?;

    let items = match by_val {
        DslValue::List(items) => items.as_slice(),
        _ => {
            return Err(syn::Error::new(
                by_span,
                "`by:` value must be a `[col1, col2]` list",
            ));
        }
    };

    if items.is_empty() {
        return Err(syn::Error::new(
            by_span,
            "group_by! requires at least one column in `by:`",
        ));
    }

    let model_name = ctx.model.name();
    let columns_enum = format_ident!("{}GroupByColumn", model_name);
    let module_ident = format_ident!("{}", model_name.to_case(Case::Snake));

    let mut by_variants: Vec<TokenStream> = Vec::new();
    let mut by_columns: Vec<String> = Vec::new();
    for item in items {
        let col_str = match item {
            DslValue::BareIdent(i) => i.to_string(),
            _ => {
                return Err(syn::Error::new(
                    by_span,
                    "`by:` items must be bare column identifiers",
                ));
            }
        };

        let field = ctx.model.get_field(&col_str).ok_or_else(|| {
            let candidates: Vec<String> = ctx.model.fields.keys().map(|k| k.to_string()).collect();
            let suggestion = crate::macros::validate::suggest(&col_str, &candidates);
            let msg = match suggestion {
                Some(s) => format!("unknown column `{}`; did you mean `{}`?", col_str, s),
                None => format!("unknown column `{}`", col_str),
            };
            syn::Error::new(by_span, msg)
        })?;

        if field.is_relation() {
            return Err(syn::Error::new(
                by_span,
                format!(
                    "by-column `{}` is a relation; group_by! requires scalar columns",
                    col_str
                ),
            ));
        }

        let variant = format_ident!("{}", to_pascal_case(&col_str));
        by_variants.push(quote! { #module_ident::#columns_enum::#variant });
        by_columns.push(col_str);
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

    let having_ts = match having_block {
        Some(hb) => {
            let conds = lower_having(hb, ctx)?;
            let having_ty = format_ident!("{}GroupByHaving", model_name);
            quote! {
                ::core::option::Option::Some(#module_ident::#having_ty {
                    conditions: #conds,
                })
            }
        }
        None => quote! { ::core::option::Option::None },
    };

    let mut present_aggs: Vec<AggPresence> = Vec::new();
    if let Some(b) = count_block {
        for k in block_keys(b) {
            if k == "_all" {
                present_aggs.push(AggPresence {
                    kind: AggKind::Count,
                    column: None,
                });
            } else {
                present_aggs.push(AggPresence {
                    kind: AggKind::Count,
                    column: Some(k),
                });
            }
        }
    }
    for (blk, kind) in [
        (sum_block, AggKind::Sum),
        (avg_block, AggKind::Avg),
        (min_block, AggKind::Min),
        (max_block, AggKind::Max),
    ] {
        if let Some(b) = blk {
            for k in block_keys(b) {
                present_aggs.push(AggPresence {
                    kind,
                    column: Some(k),
                });
            }
        }
    }

    let order_by_ts = match order_by_block {
        Some(ob) => {
            let items = lower_group_by_order_by(ob, ctx, &present_aggs, &by_columns)?;
            let order_by_ty = format_ident!("{}GroupByOrderBy", model_name);
            quote! { ::core::option::Option::Some(#module_ident::#order_by_ty { items: #items }) }
        }
        None => quote! { ::core::option::Option::None },
    };

    let args_ident = format_ident!("{}GroupByArgs", model_name);
    let accessor_expr = &accessor.accessor_expr;

    Ok(quote! {
        {
            let __by: ::std::vec::Vec<#module_ident::#columns_enum> =
                ::std::vec![#(#by_variants),*];
            let __args: #module_ident::#args_ident = #module_ident::#args_ident {
                by: __by.clone(),
                where_input: #where_ts,
                _count: #count_ts,
                _sum: #sum_ts,
                _avg: #avg_ts,
                _min: #min_ts,
                _max: #max_ts,
                having: #having_ts,
                order_by: #order_by_ts,
            };
            (#accessor_expr).group_by_columns(__by).with_group_by_args(__args)
        }
    })
}

fn block_keys(b: &DslBlock) -> Vec<String> {
    b.fields
        .iter()
        .filter_map(|f| match f {
            DslField::Pair { key, .. } => Some(key.to_string()),
            _ => None,
        })
        .collect()
}

fn to_pascal_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut upper = true;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}
