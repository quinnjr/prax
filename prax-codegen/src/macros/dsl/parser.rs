//! Brace-block parser for the read-operation DSL.
//!
//! Implements `Parse` for [`DslBlock`] and the helper `parse_field`
//! routine for one `key: value` (or spread / conditional) entry.

use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Token, braced};

use super::ast::{CondKind, DslBlock, DslField};
use super::value::parse_value;

impl Parse for DslBlock {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        let brace = braced!(content in input);
        let span = brace.span.span();
        let mut fields = Vec::new();
        while !content.is_empty() {
            let field = parse_field(&content)?;
            fields.push(field);
            // Allow trailing comma; require comma between fields.
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        Ok(DslBlock { span, fields })
    }
}

/// Parse one `DslField` from the input stream.
pub fn parse_field(input: ParseStream) -> syn::Result<DslField> {
    // Spread: `..expr` or `..move expr`.
    if input.peek(Token![..]) {
        let dotdot: Token![..] = input.parse()?;
        let span = dotdot.spans[0];
        let by_move = if input.peek(Token![move]) {
            let _: Token![move] = input.parse()?;
            true
        } else {
            false
        };
        let expr: syn::Expr = input.parse()?;
        return Ok(DslField::Spread {
            expr,
            by_move,
            span,
        });
    }

    // Conditional: `#[if(...)] ident: value`, `#[else_if(...)] ident: value`,
    // `#[else] ident: value`.
    if input.peek(Token![#]) {
        return parse_conditional(input);
    }

    // Pair: `ident: value`. Use `parse_any` so reserved words like
    // `where`, `type`, `mod` can appear as DSL keys (mirrors how Prisma's
    // input shapes use `where`, `data`, etc.).
    let key: syn::Ident = syn::Ident::parse_any(input)?;
    let span = key.span();
    let _colon: Token![:] = input.parse()?;
    let value = parse_value(input)?;
    Ok(DslField::Pair { key, value, span })
}

fn parse_conditional(input: ParseStream) -> syn::Result<DslField> {
    let pound: Token![#] = input.parse()?;
    let bracketed;
    let _ = syn::bracketed!(bracketed in input);
    let cond_kw: syn::Ident = syn::Ident::parse_any(&bracketed)?;
    let kind = match cond_kw.to_string().as_str() {
        "if" => CondKind::If,
        "else_if" => CondKind::ElseIf,
        "else" => CondKind::Else,
        other => {
            return Err(syn::Error::new(
                cond_kw.span(),
                format!("expected `if`, `else_if`, or `else` in DSL conditional, got `{other}`"),
            ));
        }
    };
    let cond: syn::Expr = if kind == CondKind::Else {
        // `#[else]` has no parenthesized condition.
        if !bracketed.is_empty() {
            return Err(syn::Error::new(
                cond_kw.span(),
                "`#[else]` takes no condition argument",
            ));
        }
        syn::parse_quote!(true)
    } else {
        let paren;
        let _ = syn::parenthesized!(paren in bracketed);
        let cond: syn::Expr = paren.parse()?;
        if !bracketed.is_empty() {
            return Err(syn::Error::new(
                bracketed.span(),
                "extra tokens after conditional argument",
            ));
        }
        cond
    };
    let _ = pound; // silence "unused" — we kept it only for span purposes.

    // After the attribute, expect `ident: value` for the conditional pair.
    let key: syn::Ident = syn::Ident::parse_any(input)?;
    let _colon: Token![:] = input.parse()?;
    let value = parse_value(input)?;
    let span: Span = cond_kw.span();
    Ok(DslField::Conditional {
        cond,
        kind,
        key,
        value,
        span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::dsl::ast::DslValue;
    use quote::quote;

    fn parse_block(tokens: proc_macro2::TokenStream) -> syn::Result<DslBlock> {
        syn::parse2::<DslBlock>(tokens)
    }

    #[test]
    fn dsl_parser_empty_block() {
        let b = parse_block(quote!({})).unwrap();
        assert!(b.fields.is_empty());
    }

    #[test]
    fn dsl_parser_single_pair() {
        let b = parse_block(quote!({ take: 10 })).unwrap();
        assert_eq!(b.fields.len(), 1);
        let DslField::Pair { key, value, .. } = &b.fields[0] else {
            panic!("expected Pair");
        };
        assert_eq!(key.to_string(), "take");
        assert!(matches!(value, DslValue::Lit(_)));
    }

    #[test]
    fn dsl_parser_multiple_keys_with_trailing_comma() {
        let b = parse_block(quote!({ skip: 5, take: 10, })).unwrap();
        assert_eq!(b.fields.len(), 2);
    }

    #[test]
    fn dsl_parser_nested_block() {
        let b = parse_block(quote!({ where: { equals: 5 } })).unwrap();
        assert_eq!(b.fields.len(), 1);
        let DslField::Pair { key, value, .. } = &b.fields[0] else {
            panic!("expected Pair");
        };
        assert_eq!(key.to_string(), "where");
        let DslValue::Block(inner) = value else {
            panic!("expected Block, got {:?}", value);
        };
        assert_eq!(inner.fields.len(), 1);
    }

    #[test]
    fn dsl_parser_list_value() {
        let b = parse_block(quote!({ or: [{ a: 1 }, { b: 2 }] })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        let DslValue::List(items) = value else {
            panic!();
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn dsl_parser_bare_ident_value() {
        let b = parse_block(quote!({ role: Admin })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::BareIdent(_)));
    }

    #[test]
    fn dsl_parser_spread() {
        let b = parse_block(quote!({ ..base })).unwrap();
        let DslField::Spread { by_move, .. } = &b.fields[0] else {
            panic!("expected Spread");
        };
        assert!(!by_move);
    }

    #[test]
    fn dsl_parser_spread_move() {
        let b = parse_block(quote!({ ..move base })).unwrap();
        let DslField::Spread { by_move, .. } = &b.fields[0] else {
            panic!("expected Spread");
        };
        assert!(by_move);
    }

    #[test]
    fn dsl_parser_conditional_if() {
        let b = parse_block(quote!({ #[if(cond)] take: 5 })).unwrap();
        let DslField::Conditional { kind, key, .. } = &b.fields[0] else {
            panic!("expected Conditional");
        };
        assert_eq!(*kind, CondKind::If);
        assert_eq!(key.to_string(), "take");
    }

    #[test]
    fn dsl_parser_conditional_else_if_and_else() {
        let b = parse_block(quote!({
            #[if(a)] take: 5,
            #[else_if(b)] take: 10,
            #[else] take: 0,
        }))
        .unwrap();
        assert_eq!(b.fields.len(), 3);
        let kinds: Vec<_> = b
            .fields
            .iter()
            .map(|f| match f {
                DslField::Conditional { kind, .. } => *kind,
                _ => panic!("expected Conditional"),
            })
            .collect();
        assert_eq!(kinds, vec![CondKind::If, CondKind::ElseIf, CondKind::Else]);
    }

    #[test]
    fn dsl_parser_at_escape_expression() {
        let b = parse_block(quote!({ data: @(custom_expr()) })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Expr(_)));
    }

    #[test]
    fn dsl_parser_malformed_missing_colon_errors() {
        let err = parse_block(quote!({ take 5 })).unwrap_err();
        // Underlying syn diagnostic: "expected `:`".
        let msg = err.to_string();
        assert!(msg.contains(':') || msg.contains("expected"), "got: {msg}");
    }

    #[test]
    fn dsl_parser_path_value_with_separator() {
        let b = parse_block(quote!({ role: Role::Admin })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Path(_)));
    }

    #[test]
    fn dsl_parser_lit_int() {
        let b = parse_block(quote!({ take: 10 })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Lit(syn::Lit::Int(_))));
    }

    #[test]
    fn dsl_parser_lit_string() {
        let b = parse_block(quote!({ email: "alice@example.com" })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Lit(syn::Lit::Str(_))));
    }

    #[test]
    fn dsl_parser_bool_keyword_value() {
        let b = parse_block(quote!({ profile: true })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        match value {
            DslValue::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn dsl_parser_dotted_expression_value() {
        // `where: foo.bar` — `foo` is followed by `.` so the
        // disambiguation rule treats it as the start of an Expr.
        let b = parse_block(quote!({ where: foo.bar })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Expr(_)));
    }

    #[test]
    fn dsl_value_parser_call_expression() {
        // `where: count()` — bare ident `count` is followed by `(`, so
        // it parses as an Expr, not BareIdent.
        let b = parse_block(quote!({ where: count() })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Expr(_)));
    }

    #[test]
    fn dsl_value_parser_negative_literal_as_expr() {
        let b = parse_block(quote!({ age: -5 })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        // `-5` lowers to an Expr (Unary) — the parser cannot reach a
        // `Lit::Int(-5)` because syn's `Lit` excludes the leading minus.
        assert!(matches!(value, DslValue::Expr(_)));
    }

    #[test]
    fn dsl_value_parser_at_escape_with_complex_expr() {
        let b = parse_block(quote!({ data: @(map.get(&key).cloned().unwrap()) })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        assert!(matches!(value, DslValue::Expr(_)));
    }

    #[test]
    fn dsl_value_parser_path_two_segments() {
        let b = parse_block(quote!({ role: Role::Admin })).unwrap();
        let DslField::Pair { value, .. } = &b.fields[0] else {
            panic!();
        };
        // Two-segment path starting with a regular ident → DslValue::Path.
        // A `crate::...` form would start with the `crate` keyword and
        // fall through to the catch-all Expr parser instead, which is
        // also fine for downstream lowering — both shapes carry the
        // same path data.
        match value {
            DslValue::Path(p) => assert_eq!(p.segments.len(), 2),
            other => panic!("expected Path, got {:?}", other),
        }
    }
}
