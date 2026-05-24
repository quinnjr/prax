//! Lower a `having: { _count: { _all: { gt: 5 } }, _sum: { views: { gte: 100 } } }`
//! block to a `Vec<HavingCondition>` token expression.
//!
//! Supported operators: equals, not_equals, lt, lte, gt, gte (matches
//! the phase 5.5 aggregate-filter operator set, minus `in`/`not_in`
//! which don't apply to scalar aggregates).
//!
//! Used by `group_by!` (Task 10). `count!` / `aggregate!` do not have
//! HAVING clauses.

#![allow(dead_code)]

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Lit;

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::aggregate_select::AggKind;

/// Lower the `having:` block to a `Vec<HavingCondition>` expression.
pub fn lower_having(block: &DslBlock, _ctx: &LowerCtx<'_>) -> syn::Result<TokenStream> {
    let mut conditions: Vec<TokenStream> = Vec::new();

    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(
                Span::call_site(),
                "having block does not support spread or conditional fields",
            ));
        };
        let agg_key = key.to_string();
        let kind = match agg_key.as_str() {
            "_count" => AggKind::Count,
            "_sum" => AggKind::Sum,
            "_avg" => AggKind::Avg,
            "_min" => AggKind::Min,
            "_max" => AggKind::Max,
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown having key `{}`; use one of _count, _sum, _avg, _min, _max",
                        other
                    ),
                ));
            }
        };
        let DslValue::Block(inner) = value else {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "having `{}` value must be a `{{ col: {{ op: value }} }}` block",
                    agg_key
                ),
            ));
        };

        for col_entry in &inner.fields {
            let DslField::Pair {
                key: col_key,
                value: col_val,
                ..
            } = col_entry
            else {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "having column block does not support spread",
                ));
            };
            let col = col_key.to_string();
            let DslValue::Block(op_block) = col_val else {
                return Err(syn::Error::new(
                    col_key.span(),
                    format!(
                        "having `{}.{}` must be a `{{ op: value }}` block",
                        agg_key, col
                    ),
                ));
            };

            for op_entry in &op_block.fields {
                let DslField::Pair {
                    key: op_key,
                    value: op_val,
                    ..
                } = op_entry
                else {
                    continue;
                };
                let op = op_key.to_string();
                let expr = dsl_value_to_f64_expr(op_val, op_key.span())?;
                let ctor = build_having_ctor(kind, &col, &op, &expr, op_key.span())?;
                conditions.push(ctor);
            }
        }
    }

    Ok(quote! {
        {
            let mut __conds: ::std::vec::Vec<::prax_query::operations::HavingCondition>
                = ::std::vec::Vec::new();
            #( __conds.push(#conditions); )*
            __conds
        }
    })
}

fn build_having_ctor(
    kind: AggKind,
    col: &str,
    op: &str,
    expr: &TokenStream,
    span: Span,
) -> syn::Result<TokenStream> {
    if matches!(kind, AggKind::Count) {
        if col != "_all" {
            return Err(syn::Error::new(
                span,
                format!(
                    "count on a specific column (`{}`) in having is not supported; use `_count: {{ _all: {{ {}: value }} }}`",
                    col, op
                ),
            ));
        }
        let path = match op {
            "gt" => quote! { ::prax_query::operations::having::count_gt(#expr) },
            "gte" => quote! { ::prax_query::operations::having::count_gte(#expr) },
            "lt" => quote! { ::prax_query::operations::having::count_lt(#expr) },
            "lte" => quote! { ::prax_query::operations::having::count_lte(#expr) },
            "equals" => quote! { ::prax_query::operations::having::count_eq(#expr) },
            "not_equals" => quote! { ::prax_query::operations::having::count_ne(#expr) },
            _ => {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "unsupported having operator `{}` on `_count`; use one of equals/not_equals/lt/lte/gt/gte",
                        op
                    ),
                ));
            }
        };
        return Ok(path);
    }

    let path = match (kind, op) {
        (AggKind::Sum, "gt") => {
            quote! { ::prax_query::operations::having::sum_gt(#col, #expr) }
        }
        (AggKind::Sum, "gte") => {
            quote! { ::prax_query::operations::having::sum_gte(#col, #expr) }
        }
        (AggKind::Sum, "lt") => {
            quote! { ::prax_query::operations::having::sum_lt(#col, #expr) }
        }
        (AggKind::Sum, "lte") => {
            quote! { ::prax_query::operations::having::sum_lte(#col, #expr) }
        }
        (AggKind::Sum, "equals") => {
            quote! { ::prax_query::operations::having::sum_eq(#col, #expr) }
        }
        (AggKind::Sum, "not_equals") => {
            quote! { ::prax_query::operations::having::sum_ne(#col, #expr) }
        }

        (AggKind::Avg, "gt") => {
            quote! { ::prax_query::operations::having::avg_gt(#col, #expr) }
        }
        (AggKind::Avg, "gte") => {
            quote! { ::prax_query::operations::having::avg_gte(#col, #expr) }
        }
        (AggKind::Avg, "lt") => {
            quote! { ::prax_query::operations::having::avg_lt(#col, #expr) }
        }
        (AggKind::Avg, "lte") => {
            quote! { ::prax_query::operations::having::avg_lte(#col, #expr) }
        }
        (AggKind::Avg, "equals") => {
            quote! { ::prax_query::operations::having::avg_eq(#col, #expr) }
        }
        (AggKind::Avg, "not_equals") => {
            quote! { ::prax_query::operations::having::avg_ne(#col, #expr) }
        }

        (AggKind::Min, "gt") => {
            quote! { ::prax_query::operations::having::min_gt(#col, #expr) }
        }
        (AggKind::Min, "gte") => {
            quote! { ::prax_query::operations::having::min_gte(#col, #expr) }
        }
        (AggKind::Min, "lt") => {
            quote! { ::prax_query::operations::having::min_lt(#col, #expr) }
        }
        (AggKind::Min, "lte") => {
            quote! { ::prax_query::operations::having::min_lte(#col, #expr) }
        }
        (AggKind::Min, "equals") => {
            quote! { ::prax_query::operations::having::min_eq(#col, #expr) }
        }
        (AggKind::Min, "not_equals") => {
            quote! { ::prax_query::operations::having::min_ne(#col, #expr) }
        }

        (AggKind::Max, "gt") => {
            quote! { ::prax_query::operations::having::max_gt(#col, #expr) }
        }
        (AggKind::Max, "gte") => {
            quote! { ::prax_query::operations::having::max_gte(#col, #expr) }
        }
        (AggKind::Max, "lt") => {
            quote! { ::prax_query::operations::having::max_lt(#col, #expr) }
        }
        (AggKind::Max, "lte") => {
            quote! { ::prax_query::operations::having::max_lte(#col, #expr) }
        }
        (AggKind::Max, "equals") => {
            quote! { ::prax_query::operations::having::max_eq(#col, #expr) }
        }
        (AggKind::Max, "not_equals") => {
            quote! { ::prax_query::operations::having::max_ne(#col, #expr) }
        }

        (_, _) => {
            return Err(syn::Error::new(
                span,
                format!(
                    "unsupported having operator `{}` on `{}`; use one of equals/not_equals/lt/lte/gt/gte",
                    op,
                    kind.key()
                ),
            ));
        }
    };

    Ok(path)
}

fn dsl_value_to_f64_expr(v: &DslValue, span: Span) -> syn::Result<TokenStream> {
    match v {
        DslValue::Lit(Lit::Int(i)) => Ok(quote! { (#i as f64) }),
        DslValue::Lit(Lit::Float(f)) => Ok(quote! { (#f as f64) }),
        DslValue::Expr(e) => Ok(quote! { ((#e) as f64) }),
        _ => Err(syn::Error::new(
            span,
            "having operator value must be a numeric literal",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsl_value_int_lit_emits_f64_cast() {
        let lit_int: syn::LitInt = syn::parse_str("5").unwrap();
        let v = DslValue::Lit(Lit::Int(lit_int));
        let ts = dsl_value_to_f64_expr(&v, Span::call_site()).unwrap();
        assert_eq!(ts.to_string(), "(5 as f64)");
    }

    #[test]
    fn dsl_value_float_lit_emits_f64_cast() {
        let lit_float: syn::LitFloat = syn::parse_str("3.14").unwrap();
        let v = DslValue::Lit(Lit::Float(lit_float));
        let ts = dsl_value_to_f64_expr(&v, Span::call_site()).unwrap();
        assert_eq!(ts.to_string(), "(3.14 as f64)");
    }

    #[test]
    fn dsl_value_string_errors() {
        let lit_str: syn::LitStr = syn::parse_str("\"hello\"").unwrap();
        let v = DslValue::Lit(Lit::Str(lit_str));
        let err = dsl_value_to_f64_expr(&v, Span::call_site()).unwrap_err();
        assert!(err.to_string().contains("numeric literal"));
    }
}
