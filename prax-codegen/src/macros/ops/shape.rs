//! Standalone shape-macro entry points.
//!
//! Each `expand_*_shape` function below is the body of the matching
//! top-level `#[proc_macro]` in `prax-codegen/src/lib.rs`. They are
//! intentionally thin: resolve the schema, parse `Model, ...`, then
//! delegate to the existing phase-3 lowering helpers in
//! [`macros::lower`](super::super::lower).
//!
//! Shape macros return values (not operations) — they emit the typed
//! input struct directly, so the result composes with the read macros
//! via spread:
//!
//! ```rust,ignore
//! let active = prax::where!(User, { active: true });
//! let _ = prax::find_many!(client.user, {
//!     ..active,
//!     email: { contains: "@x.com" },
//! });
//! ```

use proc_macro2::TokenStream;
use quote::quote;
use syn::Token;
use syn::parse::Parser;

use crate::macros::dsl::ast::DslBlock;
use crate::macros::lower::LowerCtx;
use crate::macros::lower::include_input::lower_include;
use crate::macros::lower::select_input::lower_select;
use crate::macros::lower::where_input::lower_where;
use crate::macros::schema_resolve::{resolve_schema, resolve_schema_path, track_schema_dep};
use crate::macros::shape_accessor::parse_model_ident;

/// Expand `prax::where!(Model, { ... })` to a `<Model>WhereInput` value.
pub fn expand_where_shape(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = resolve_schema()?;
    let schema_path = resolve_schema_path()?;
    let dep = track_schema_dep(&schema_path);

    let parser = move |s: syn::parse::ParseStream<'_>| -> syn::Result<TokenStream> {
        let (_ident, model) = parse_model_ident(s, &schema)?;
        let _: Token![,] = s.parse()?;
        let block: DslBlock = s.parse()?;
        let ctx = LowerCtx::new(&schema, model);
        lower_where(&block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

/// Expand `prax::include!(Model, { ... })` to a `<Model>Include` value.
pub fn expand_include_shape(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = resolve_schema()?;
    let schema_path = resolve_schema_path()?;
    let dep = track_schema_dep(&schema_path);

    let parser = move |s: syn::parse::ParseStream<'_>| -> syn::Result<TokenStream> {
        let (_ident, model) = parse_model_ident(s, &schema)?;
        let _: Token![,] = s.parse()?;
        let block: DslBlock = s.parse()?;
        let ctx = LowerCtx::new(&schema, model);
        lower_include(&block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}

/// Expand `prax::select!(Model, { ... })` to a `<Model>Select` value.
pub fn expand_select_shape(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = resolve_schema()?;
    let schema_path = resolve_schema_path()?;
    let dep = track_schema_dep(&schema_path);

    let parser = move |s: syn::parse::ParseStream<'_>| -> syn::Result<TokenStream> {
        let (_ident, model) = parse_model_ident(s, &schema)?;
        let _: Token![,] = s.parse()?;
        let block: DslBlock = s.parse()?;
        let ctx = LowerCtx::new(&schema, model);
        lower_select(&block, &ctx)
    };

    let body = Parser::parse2(parser, input)?;
    Ok(quote! {
        {
            #dep
            #body
        }
    })
}
