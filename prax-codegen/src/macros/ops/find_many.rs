//! `find_many!` proc-macro entry point.
//!
//! Pipeline:
//! ```text
//! parse TokenStream
//!   -> resolve schema
//!   -> parse accessor head + model
//!   -> parse DSL brace block
//!   -> dispatch top-level keys (where / include / select / order_by /
//!      cursor / skip / take / distinct)
//!   -> emit chained `with_*_input(...)` + builder calls on the
//!      accessor's `.find_many()` op.
//! ```

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::include_input::lower_include;
use crate::macros::lower::order_by_input::{lower_cursor, lower_order_by};
use crate::macros::lower::select_input::lower_select;
use crate::macros::lower::where_input::lower_where;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

/// Top-level keys allowed inside `find_many!(client.user, { ... })`.
pub(crate) const FIND_MANY_KEYS: &[&str] = &[
    "where", "order_by", "cursor", "skip", "take", "distinct", "include", "select",
];

/// Expand `find_many!`. Returns a token-stream that builds a
/// `FindManyOperation` from the accessor + DSL.
pub fn expand_find_many(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = resolve_schema()?;
    let schema_path = resolve_schema_path()?;
    let dep = track_schema_dep(&schema_path);

    let parser = move |s: syn::parse::ParseStream<'_>| -> syn::Result<TokenStream> {
        let (accessor, model) = parse_accessor(s, &schema)?;
        // `parse_accessor` consumes the trailing `,` for Form 1 and
        // Form 3 (after `for MODEL,`). Form 2 (`MODEL on EXPR`) leaves
        // the `,` behind. Accept either case here.
        if s.peek(Token![,]) {
            let _comma: Token![,] = s.parse()?;
        }
        let block: DslBlock = s.parse()?;
        let ctx = LowerCtx::new(&schema, model);
        lower_find_many(&accessor, &block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;

    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_find_many(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut include_tokens: Option<TokenStream> = None;
    let mut select_tokens: Option<TokenStream> = None;
    let mut order_by_tokens: Option<TokenStream> = None;
    let mut cursor_tokens: Option<TokenStream> = None;
    let mut skip_expr: Option<TokenStream> = None;
    let mut take_expr: Option<TokenStream> = None;
    let mut distinct_expr: Option<TokenStream> = None;

    let mut select_span: Option<Span> = None;
    let mut include_span: Option<Span> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`find_many!` does not accept spread or conditional fields at the top level",
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
                include_span = Some(key.span());
            }
            "select" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`select:` expects `{ ... }`"));
                };
                select_tokens = Some(lower_select(b, ctx)?);
                select_span = Some(key.span());
            }
            "order_by" => {
                order_by_tokens = Some(lower_order_by(value, ctx)?);
            }
            "cursor" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`cursor:` expects `{ ... }`"));
                };
                cursor_tokens = Some(lower_cursor(b, ctx)?);
            }
            "skip" => skip_expr = Some(lower_scalar_n(value, key.span())?),
            "take" => take_expr = Some(lower_scalar_n(value, key.span())?),
            "distinct" => distinct_expr = Some(lower_distinct(value, key.span())?),
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    FIND_MANY_KEYS,
                    "find_many",
                ));
            }
        }
    }

    // select xor include — emit on the *second* occurrence.
    if select_tokens.is_some() && include_tokens.is_some() {
        let span = select_span.or(include_span).unwrap_or_else(Span::call_site);
        return Err(syn::Error::new(
            span,
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
    if let Some(c) = cursor_tokens {
        chain.push(quote! { .cursor({
            let __c = #c;
            ::core::convert::Into::into(__c)
        }) });
    }
    if let Some(s) = skip_expr {
        chain.push(quote! { .skip(#s) });
    }
    if let Some(t) = take_expr {
        chain.push(quote! { .take(#t) });
    }
    if let Some(d) = distinct_expr {
        chain.push(quote! { .distinct(#d) });
    }

    Ok(quote! {
        (#accessor_expr).find_many() #(#chain)*
    })
}

fn lower_scalar_n(value: &DslValue, span: Span) -> syn::Result<TokenStream> {
    match value {
        DslValue::Lit(l) => Ok(quote! { #l as u64 }),
        DslValue::Expr(e) => Ok(quote! { (#e) as u64 }),
        _ => Err(syn::Error::new(
            span,
            "`skip` / `take` expect an integer literal or `@(expr)`",
        )),
    }
}

fn lower_distinct(value: &DslValue, span: Span) -> syn::Result<TokenStream> {
    let DslValue::List(items) = value else {
        return Err(syn::Error::new(
            span,
            "`distinct:` expects a list of field names: `[\"col_a\", \"col_b\"]`",
        ));
    };
    let strs: Vec<TokenStream> = items
        .iter()
        .map(|v| match v {
            DslValue::Lit(syn::Lit::Str(s)) => Ok(quote! { #s.to_string() }),
            DslValue::BareIdent(id) => {
                let s = id.to_string();
                Ok(quote! { #s.to_string() })
            }
            _ => Err(syn::Error::new(
                span,
                "distinct entries must be string literals or bare idents",
            )),
        })
        .collect::<syn::Result<_>>()?;
    Ok(quote! { ::std::vec![ #(#strs),* ] })
}
