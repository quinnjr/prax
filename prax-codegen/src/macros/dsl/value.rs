//! Value-parser primitives for the read-operation DSL.
//!
//! See the spec §4 grammar:
//! ```text
//! value := literal | path | expr_in_parens
//!        | "{" field_list "}"          -- DslBlock
//!        | "[" expr_list "]"           -- list
//!        | "true" | "false"            -- bool keyword
//!        | "@(" expr ")"               -- Rust escape
//!        | bare_ident                  -- enum shorthand
//! ```

use proc_macro2::Span;
use syn::parse::ParseStream;
use syn::{Lit, Token, bracketed, parenthesized};

use super::ast::{DslBlock, DslValue};

/// Parse one DSL value from the stream.
pub fn parse_value(input: ParseStream) -> syn::Result<DslValue> {
    // `true` / `false` keyword shorthand. `syn` parses these as
    // `LitBool` (a `Lit::Bool` variant), not as keyword tokens. We
    // peek `LitBool` to disambiguate ahead of the generic `Lit` arm so
    // we can emit `DslValue::Bool` directly rather than `DslValue::Lit`.
    if input.peek(syn::LitBool) {
        let b: syn::LitBool = input.parse()?;
        return Ok(DslValue::Bool(b.value));
    }

    // Brace block: nested shape.
    if input.peek(syn::token::Brace) {
        let block: DslBlock = input.parse()?;
        return Ok(DslValue::Block(block));
    }

    // List literal.
    if input.peek(syn::token::Bracket) {
        let content;
        let _ = bracketed!(content in input);
        let mut items = Vec::new();
        while !content.is_empty() {
            items.push(parse_value(&content)?);
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        return Ok(DslValue::List(items));
    }

    // `@(...)` Rust expression escape.
    if input.peek(Token![@]) {
        let _: Token![@] = input.parse()?;
        let content;
        let _ = parenthesized!(content in input);
        let expr: syn::Expr = content.parse()?;
        if !content.is_empty() {
            return Err(syn::Error::new(
                content.span(),
                "extra tokens after `@(expr)` escape",
            ));
        }
        return Ok(DslValue::Expr(expr));
    }

    // Negative literal: `-5`. `syn::Lit` doesn't include leading sign,
    // so peek a `-` followed by a literal and assemble a syn::ExprUnary.
    if input.peek(Token![-]) && input.peek2(Lit) {
        let expr: syn::Expr = input.parse()?;
        return Ok(DslValue::Expr(expr));
    }

    // Literal: string, int, float, byte, char.
    if input.peek(Lit) {
        let lit: Lit = input.parse()?;
        return Ok(DslValue::Lit(lit));
    }

    // Path or bare ident.
    if input.peek(syn::Ident) {
        // If the input starts with an ident followed by `::`, treat as Path.
        // Otherwise the disambiguation rule (§4): a single-segment ident
        // is `BareIdent` iff the next token is `,`, `}`, `]`, or EOF.
        // Anything else (`.`, `(`, `<`, ...) means it's the start of a
        // larger expression.
        let fork = input.fork();
        let _ident: syn::Ident = fork.parse()?;
        if fork.peek(Token![::]) {
            // Path with at least two segments.
            let path: syn::Path = input.parse()?;
            return Ok(DslValue::Path(path));
        }
        // Single ident; check the disambiguation set.
        if fork.is_empty()
            || fork.peek(Token![,])
            || fork.peek(syn::token::Brace)
            || fork.peek(syn::token::Bracket)
        {
            let id: syn::Ident = input.parse()?;
            // BareIdent treatment also covers `}` (end of containing
            // block) and `]` (end of containing list). `syn::token::Brace`/
            // `Bracket` peeks here are the closing delimiters because we
            // are inside the parent's `braced!`/`bracketed!` content.
            return Ok(DslValue::BareIdent(id));
        }
        // Otherwise it's the start of an expression like `foo.bar`,
        // `foo()`, `foo[..]`.
        let expr: syn::Expr = input.parse()?;
        return Ok(DslValue::Expr(expr));
    }

    // Catch-all: parse a full Rust expression. This handles `crate::X`,
    // `Some(5)`, parenthesized expressions, etc.
    if input.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "expected a DSL value but the input is empty",
        ));
    }
    let expr: syn::Expr = input.parse()?;
    Ok(DslValue::Expr(expr))
}
