//! `create_many!` proc-macro entry point.
//!
//! Top-level keys: `data:` (required, list of blocks),
//! `skip_duplicates:` (optional bool).
//!
//! Lowers each `data:` block via `lower_create_data` and emits a chain
//! that builds a `Vec<<Model>CreateInput>` literal then calls
//! `with_create_inputs(...)` on the `CreateManyOperation`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::accessor::parse_accessor;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::LowerCtx;
use crate::macros::lower::data_input::lower_create_data;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::validate::unknown_top_key_error;

const CREATE_MANY_KEYS: &[&str] = &["data", "skip_duplicates"];

pub fn expand_create_many(input: TokenStream) -> syn::Result<TokenStream> {
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
        lower_create_many(&accessor, &block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

fn lower_create_many(
    accessor: &crate::macros::accessor::AccessorSpec,
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
) -> syn::Result<TokenStream> {
    let mut data_rows: Option<Vec<TokenStream>> = None;
    let mut skip_dup_expr: Option<TokenStream> = None;

    for field in &block.fields {
        let DslField::Pair { key, value, .. } = field else {
            return Err(syn::Error::new(
                Span::call_site(),
                "`create_many!` does not accept spread or conditional fields at the top level",
            ));
        };
        let key_str = key.to_string();
        match key_str.as_str() {
            "data" => {
                let DslValue::List(items) = value else {
                    return Err(syn::Error::new(
                        key.span(),
                        "`data:` on `create_many!` expects a list of `{ ... }` blocks: \
                         `data: [{ ... }, { ... }]`",
                    ));
                };
                let mut rows: Vec<TokenStream> = Vec::with_capacity(items.len());
                for entry in items {
                    let DslValue::Block(b) = entry else {
                        return Err(syn::Error::new(
                            key.span(),
                            "every entry in `create_many!`'s `data:` list must be a `{ ... }` block",
                        ));
                    };
                    rows.push(lower_create_data(b, ctx)?);
                }
                data_rows = Some(rows);
            }
            "skip_duplicates" => match value {
                DslValue::Bool(b) => {
                    skip_dup_expr = Some(quote! { #b });
                }
                DslValue::Expr(e) => {
                    skip_dup_expr = Some(quote! { (#e) });
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "`skip_duplicates:` expects `true`, `false`, or `@(bool_expr)`",
                    ));
                }
            },
            _ => {
                return Err(unknown_top_key_error(
                    &key_str,
                    key.span(),
                    CREATE_MANY_KEYS,
                    "create_many",
                ));
            }
        }
    }

    let data_rows = data_rows
        .ok_or_else(|| syn::Error::new(block.span, "`create_many!` requires a `data:` list"))?;

    let accessor_expr = &accessor.accessor_expr;
    let mut chain: Vec<TokenStream> = vec![quote! {
        .with_create_inputs(::std::vec![ #(#data_rows),* ])
    }];
    if let Some(s) = skip_dup_expr {
        chain.push(quote! { .with_skip_duplicates(#s) });
    }

    Ok(quote! {
        (#accessor_expr).create_many() #(#chain)*
    })
}
