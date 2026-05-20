//! Accessor expression parser for the read-operation macros.
//!
//! The head of every operation-macro invocation is one of:
//!
//! 1. `EXPR, { ... }`           — implicit model. The model is inferred
//!    from the last path-segment of EXPR (`client.user` → "User").
//! 2. `MODEL on EXPR, { ... }`  — explicit type-position model ident.
//! 3. `EXPR, for MODEL, { ... }` — explicit `for` annotation.
//!
//! The parser returns an [`AccessorSpec`] holding the accessor
//! expression to dot into and the resolved [`Model`] from the schema.

#![allow(dead_code)]

use convert_case::{Case, Casing};
use prax_schema::{Model, Schema};
use proc_macro2::Span;
use syn::parse::ParseStream;
use syn::{Expr, Ident, Token};

/// A parsed accessor + resolved model.
#[derive(Debug, Clone)]
pub struct AccessorSpec {
    /// Expression that evaluates to a `Client<E>` (or compatible
    /// accessor) — e.g. `client.user`.
    pub accessor_expr: Expr,
    /// Model name in PascalCase (e.g. "User").
    pub model_name: String,
    /// Span of the model — points at whatever piece of the input
    /// established the model identity.
    pub model_span: Span,
}

/// Parse the accessor head from the macro input.
///
/// On entry, `input` is positioned at the start of the macro args.
/// The function consumes everything up to (and including) the first
/// top-level comma that separates the accessor head from the DSL
/// brace block.
pub fn parse_accessor<'a>(
    input: ParseStream<'_>,
    schema: &'a Schema,
) -> syn::Result<(AccessorSpec, &'a Model)> {
    // Form 2: `MODEL on EXPR`. Detect by peeking an Ident followed by
    // the `on` keyword.
    if input.peek(Ident) && input.peek2(syn::Token![=>]).then_some(()).is_none() {
        let fork = input.fork();
        if let Ok(_id) = fork.parse::<Ident>() {
            // Check if next token is the bareword `on`.
            if fork.peek(syn::Ident) {
                let next: Ident = fork
                    .parse()
                    .unwrap_or_else(|_| Ident::new("_", Span::call_site()));
                if next == "on" {
                    // Confirmed: MODEL on EXPR.
                    let model_id: Ident = input.parse()?;
                    let _on: Ident = input.parse()?;
                    let expr: Expr = input.parse()?;
                    return finalize(model_id, expr, schema);
                }
            }
        }
    }

    // Otherwise, parse an expression up to the first top-level `,`.
    // Then check whether what follows is `for MODEL,` (form 3) or
    // immediately the DSL block (form 1).
    let expr: Expr = input.parse()?;
    if !input.peek(Token![,]) {
        return Err(syn::Error::new(
            Span::call_site(),
            "expected `,` after accessor expression",
        ));
    }
    let _comma: Token![,] = input.parse()?;

    // Form 3: `EXPR, for MODEL, { ... }`. `for` is a Rust keyword, so
    // use `parse_any` to admit it as an ident.
    {
        use syn::ext::IdentExt;
        let fork = input.fork();
        if let Ok(id) = Ident::parse_any(&fork)
            && id == "for"
        {
            let _for_kw: Token![for] = input.parse()?;
            let model_id: Ident = input.parse()?;
            let _comma: Token![,] = input.parse()?;
            return finalize(model_id, expr, schema);
        }
    }

    // Form 1: infer model from the expression's tail.
    let inferred = infer_model_ident(&expr).ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "couldn't infer model from accessor expression. \
             Use the `EXPR, for Model, { ... }` form instead.",
        )
    })?;
    finalize(inferred, expr, schema)
}

fn finalize(
    model_id: Ident,
    accessor_expr: Expr,
    schema: &Schema,
) -> syn::Result<(AccessorSpec, &Model)> {
    let (model, name) = resolve_model_from_ident(&model_id, schema)?;
    let model_span = model_id.span();
    let spec = AccessorSpec {
        accessor_expr,
        model_name: name,
        model_span,
    };
    Ok((spec, model))
}

/// Infer the model PascalCase ident from a path-expression like
/// `client.user` or `state.db.user`. Takes the last path-segment after
/// converting from snake_case.
fn infer_model_ident(expr: &Expr) -> Option<Ident> {
    let last = last_path_or_field_segment(expr)?;
    let pascal = last.to_string().to_case(Case::Pascal);
    Some(Ident::new(&pascal, last.span()))
}

/// Walk an `Expr` looking for the trailing ident — either a field
/// access (`a.b.c` → `c`) or a method call (`get_client().user()` →
/// `user`).
fn last_path_or_field_segment(expr: &Expr) -> Option<Ident> {
    match expr {
        Expr::Field(f) => match &f.member {
            syn::Member::Named(id) => Some(id.clone()),
            _ => None,
        },
        Expr::MethodCall(mc) => Some(mc.method.clone()),
        Expr::Path(p) => p.path.segments.last().map(|s| s.ident.clone()),
        Expr::Call(c) => match &*c.func {
            Expr::Path(p) => p.path.segments.last().map(|s| s.ident.clone()),
            Expr::Field(f) => match &f.member {
                syn::Member::Named(id) => Some(id.clone()),
                _ => None,
            },
            _ => None,
        },
        Expr::Paren(p) => last_path_or_field_segment(&p.expr),
        _ => None,
    }
}

fn resolve_model_from_ident<'a>(
    id: &Ident,
    schema: &'a Schema,
) -> syn::Result<(&'a Model, String)> {
    let name = id.to_string();
    if let Some(m) = schema.get_model(&name) {
        return Ok((m, name));
    }
    let pascal = name.to_case(Case::Pascal);
    if let Some(m) = schema.get_model(&pascal) {
        return Ok((m, pascal));
    }
    let names: Vec<String> = schema.models.keys().map(|k| k.to_string()).collect();
    let suggestion = crate::macros::validate::suggest(&name, &names);
    let msg = match suggestion {
        Some(c) => format!("unknown model `{name}`. did you mean `{c}`?"),
        None => format!("unknown model `{name}`. Known models: {names:?}"),
    };
    Err(syn::Error::new(id.span(), msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::parse_schema;
    use syn::parse::Parser;

    const SCHEMA: &str = include_str!("../../tests/fixtures/schema.prax");

    fn parse_accessor_str(input: &str, schema: &Schema) -> syn::Result<(AccessorSpec, String)> {
        let tokens: proc_macro2::TokenStream = input
            .parse()
            .map_err(|e| syn::Error::new(Span::call_site(), format!("lex error: {e}")))?;
        let parser = move |s: ParseStream<'_>| -> syn::Result<(AccessorSpec, String)> {
            let (spec, model) = parse_accessor(s, schema)?;
            // Drain any trailing tokens (the DSL block) so
            // `Parser::parse2` doesn't trip on leftover input.
            let _ = s.step(|cursor| {
                let mut rest = *cursor;
                while let Some((_, next)) = rest.token_tree() {
                    rest = next;
                }
                Ok(((), rest))
            });
            Ok((spec, model.name().to_string()))
        };
        Parser::parse2(parser, tokens)
    }

    #[test]
    fn accessor_form_1_client_dot_user_resolves_user_model() {
        let schema = parse_schema(SCHEMA).unwrap();
        let (spec, model) =
            parse_accessor_str("client.user, { where: { id: 1 } }", &schema).unwrap();
        assert_eq!(spec.model_name, "User");
        assert_eq!(model, "User");
    }

    #[test]
    fn accessor_form_1_method_call_resolves_model() {
        let schema = parse_schema(SCHEMA).unwrap();
        let (spec, _model) = parse_accessor_str("get_client().user(), { }", &schema).unwrap();
        assert_eq!(spec.model_name, "User");
    }

    #[test]
    fn accessor_form_2_model_on_expr() {
        let schema = parse_schema(SCHEMA).unwrap();
        let (spec, _model) =
            parse_accessor_str("User on &engine, { where: { id: 1 } }", &schema).unwrap();
        assert_eq!(spec.model_name, "User");
    }

    #[test]
    fn accessor_form_3_for_annotation() {
        let schema = parse_schema(SCHEMA).unwrap();
        let (spec, _model) =
            parse_accessor_str("foo().bar(), for User, { where: { id: 1 } }", &schema).unwrap();
        assert_eq!(spec.model_name, "User");
    }

    #[test]
    fn accessor_unknown_model_errors_with_suggestion() {
        let schema = parse_schema(SCHEMA).unwrap();
        // PascalCase typo: `Useer` should suggest `User`.
        let err = parse_accessor_str("Useer on &e, {}", &schema).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown model"), "got: {msg}");
        assert!(msg.contains("did you mean"), "got: {msg}");
        assert!(msg.contains("User"), "got: {msg}");
    }

    #[test]
    fn accessor_form_1_unknown_model_errors() {
        let schema = parse_schema(SCHEMA).unwrap();
        let err = parse_accessor_str("client.nope, {}", &schema).unwrap_err();
        assert!(err.to_string().contains("unknown model"));
    }
}
