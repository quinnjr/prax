//! Parse + resolve a single model identifier for the standalone shape
//! macros (`where!`, `include!`, `select!`, `order_by!`, `cursor!`).
//!
//! Unlike the read-operation macros — which take an accessor expression
//! head (`client.user`, `Model on expr`, …) — the shape macros operate
//! directly on the typed input struct for a model, so the head is just
//! a model PascalCase ident followed by a `,`.

// Wired into `ops::shape` in the next task; until that lands the
// helper is only exercised by unit tests.
#![allow(dead_code)]

use convert_case::{Case, Casing};
use prax_schema::{Model, Schema};
use syn::parse::ParseStream;

/// Parse a single model identifier and resolve it against the schema.
///
/// Returns the parsed [`Ident`] (so callers can attach diagnostic
/// spans) plus a borrow of the [`Model`] entry. Performs the same
/// PascalCase fallback that
/// [`accessor::resolve_model_from_ident`](super::accessor) uses, and
/// produces a "did you mean" hint via
/// [`validate::suggest`](super::validate::suggest) on miss.
pub fn parse_model_ident<'a>(
    input: ParseStream<'_>,
    schema: &'a Schema,
) -> syn::Result<(syn::Ident, &'a Model)> {
    let ident: syn::Ident = input.parse()?;
    let name = ident.to_string();
    if let Some(m) = schema.get_model(&name) {
        return Ok((ident, m));
    }
    let pascal = name.to_case(Case::Pascal);
    if let Some(m) = schema.get_model(&pascal) {
        return Ok((ident, m));
    }
    let names: Vec<String> = schema.models.keys().map(|k| k.to_string()).collect();
    let suggestion = crate::macros::validate::suggest(&name, &names);
    let msg = match suggestion {
        Some(c) => format!("unknown model `{name}`. did you mean `{c}`?"),
        None => format!("unknown model `{name}`. Known models: {names:?}"),
    };
    Err(syn::Error::new(ident.span(), msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::parse::Parser;

    const SCHEMA: &str = include_str!("../../tests/fixtures/schema.prax");

    fn run(input: TokenStream, schema: &Schema) -> syn::Result<String> {
        let parser = move |s: ParseStream<'_>| -> syn::Result<String> {
            let (_, model) = parse_model_ident(s, schema)?;
            // Drain anything trailing so `Parser::parse2` doesn't trip on
            // leftover tokens.
            let _ = s.step(|cursor| {
                let mut rest = *cursor;
                while let Some((_, next)) = rest.token_tree() {
                    rest = next;
                }
                Ok(((), rest))
            });
            Ok(model.name().to_string())
        };
        Parser::parse2(parser, input)
    }

    #[test]
    fn shape_accessor_resolves_known_model() {
        let schema = parse_schema(SCHEMA).unwrap();
        let name = run(quote!(User), &schema).unwrap();
        assert_eq!(name, "User");
    }

    #[test]
    fn shape_accessor_resolves_post_model() {
        let schema = parse_schema(SCHEMA).unwrap();
        let name = run(quote!(Post), &schema).unwrap();
        assert_eq!(name, "Post");
    }

    #[test]
    fn shape_accessor_unknown_model_errors_with_suggestion() {
        let schema = parse_schema(SCHEMA).unwrap();
        let err = run(quote!(Useer), &schema).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown model"), "got: {msg}");
        assert!(msg.contains("did you mean"), "got: {msg}");
        assert!(msg.contains("User"), "got: {msg}");
    }

    #[test]
    fn shape_accessor_unknown_model_no_close_match_lists_known() {
        let schema = parse_schema(SCHEMA).unwrap();
        let err = run(quote!(Zzzzzzzz), &schema).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown model"), "got: {msg}");
        assert!(msg.contains("Known models"), "got: {msg}");
    }
}
