//! `find_unique!` proc-macro entry point.
//!
//! Like `find_many!` but the `where:` block lowers to
//! `<Model>WhereUniqueInput` (a single unique-key match), and only
//! `where`, `include`, and `select` are accepted at the top level.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::include_input::lower_include;
use crate::macros::lower::order_by_input::lower_cursor;
use crate::macros::lower::select_input::{SelectLowering, lower_select};
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const FIND_UNIQUE_KEYS: &[&str] = &["where", "include", "select"];

pub fn expand_find_unique(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_find_unique(&accessor, &block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_find_unique(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut where_tokens: Option<TokenStream> = None;
    let mut include_tokens: Option<TokenStream> = None;
    let mut select_lowering: Option<SelectLowering> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`find_unique!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "where" => {
                let DslValue::Block(b) = value else {
                    return Err(syn::Error::new(key.span(), "`where:` expects `{ ... }`"));
                };
                // `find_unique!`'s `where:` is the unique-input form.
                where_tokens = Some(lower_cursor(b, ctx)?);
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
                select_lowering = Some(lower_select(b, ctx)?);
            }
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    FIND_UNIQUE_KEYS,
                    "find_unique",
                ));
            }
        }
    }

    if select_lowering.is_some() && include_tokens.is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`select` and `include` are mutually exclusive — choose one",
        ));
    }
    let where_tokens = where_tokens.ok_or_else(|| {
        syn::Error::new(
            block.span,
            "`find_unique!` requires a `where:` block matching a unique column",
        )
    })?;

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = vec![quote! { .with_where_input(#where_tokens) }];
    if let Some(i) = include_tokens {
        chain.push(quote! { .with_include_input(#i) });
    }
    if let Some(sl) = select_lowering {
        let s = sl.select_struct;
        chain.push(quote! { .with_select_input(#s) });
        for proj in sl.scalar_projections {
            chain.push(proj);
        }
    }

    Ok(quote! {
        (#accessor_expr).find_unique() #(#chain)*
    })
}
