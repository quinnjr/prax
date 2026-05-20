//! `upsert!` proc-macro entry point.
//!
//! Top-level keys: `where:` (required, unique), `create:` (required),
//! `update:` (required), `include` xor `select`. The `where:` block
//! identifies which unique row to look up; on miss the `create:`
//! payload is inserted, on hit the `update:` payload is applied.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::data_input::{lower_create_data, lower_update_data};
use crate::macros::lower::include_input::lower_include;
use crate::macros::lower::order_by_input::lower_cursor;
use crate::macros::lower::select_input::lower_select;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

/// Extract the conflict column from a `where: { col: val }` block.
///
/// The `where:` block on `upsert!` doubles as the conflict target —
/// the `UpsertOperation` SQL emitter needs an `ON CONFLICT (col)` term
/// in addition to the typed `WhereUniqueInput` value. The unique-key
/// constraint that `lower_cursor` enforces guarantees this column is
/// `@id` or `@unique` (so the conflict target is well-defined).
fn extract_conflict_column(block: &DslBlock, ctx: &LowerCtx<'_>) -> syn::Result<String> {
    if block.fields.len() != 1 {
        return Err(syn::Error::new(
            block.span,
            "`upsert!` `where:` block must have exactly one unique-key field",
        ));
    }
    let DslField::Pair { key, .. } = &block.fields[0] else {
        return Err(syn::Error::new(
            block.span,
            "`upsert!` `where:` block must be a `{ field: value }` pair",
        ));
    };
    let key_str = key.to_string();
    let field = ctx.model.get_field(&key_str).ok_or_else(|| {
        syn::Error::new(
            key.span(),
            format!("unknown field `{key_str}` on `{}`", ctx.model.name()),
        )
    })?;
    let attrs = field.extract_attributes();
    Ok(attrs.map.unwrap_or_else(|| field.name().to_string()))
}

const UPSERT_KEYS: &[&str] = &["where", "create", "update", "include", "select"];

pub fn expand_upsert(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_upsert(&accessor, &block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_upsert(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut conflict_column: Option<String> = None;
    let mut create_tokens: Option<TokenStream> = None;
    let mut update_tokens: Option<TokenStream> = None;
    let mut include_tokens: Option<TokenStream> = None;
    let mut select_tokens: Option<TokenStream> = None;
    let mut select_span: Option<Span> = None;
    let mut include_span: Option<Span> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`upsert!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                conflict_column = Some(extract_conflict_column(b, ctx)?);
                where_tokens = Some(lower_cursor(b, ctx)?);
            }
            "create" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`create:` expects `{ ... }`"));
                };
                create_tokens = Some(lower_create_data(b, ctx)?);
            }
            "update" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`update:` expects `{ ... }`"));
                };
                update_tokens = Some(lower_update_data(b, ctx)?);
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
                    UPSERT_KEYS,
                    "upsert",
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
            "`upsert!` requires a `where:` block matching a unique column",
        )
    })?;
    let create_tokens = create_tokens
        .ok_or_else(|| syn::Error::new(block.span, "`upsert!` requires a `create:` block"))?;
    let update_tokens = update_tokens
        .ok_or_else(|| syn::Error::new(block.span, "`upsert!` requires an `update:` block"))?;

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = vec![quote! { .with_where_input(#where_tokens) }];
    if let Some(col) = conflict_column {
        chain.push(quote! { .on_conflict([#col]) });
    }
    chain.push(quote! { .with_create_input(#create_tokens) });
    chain.push(quote! { .with_update_input(#update_tokens) });
    if let Some(i) = include_tokens {
        chain.push(quote! { .with_include_input(#i) });
    }
    if let Some(s) = select_tokens {
        chain.push(quote! { .with_select_input(#s) });
    }

    Ok(quote! {
        (#accessor_expr).upsert() #(#chain)*
    })
}
