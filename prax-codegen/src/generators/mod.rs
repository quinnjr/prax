//! Code generators for Prax models, enums, types, and views.

mod derive;
mod derive_client;
mod derive_from_row;
mod derive_model_trait;
mod derive_model_with_pk;
mod derive_relation_loader;
mod enum_gen;
mod fields;
mod filters;
mod model;
mod relation_accessors;
mod type_gen;
mod view;

pub use derive::derive_model_impl;
pub use enum_gen::generate_enum_module;
#[allow(unused_imports)]
pub use model::generate_model_module;
pub use model::generate_model_module_with_style;
pub use type_gen::generate_type_module;
pub use view::generate_view_module;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::types::{to_pascal_case, to_snake_case};

/// Generate documentation comment tokens from an optional doc string.
pub fn generate_doc_comment(doc: Option<&str>) -> TokenStream {
    match doc {
        Some(doc) => {
            let lines: Vec<_> = doc.lines().map(|line| line.trim()).collect();
            let doc_lines = lines.iter().map(|line| {
                quote! { #[doc = #line] }
            });
            quote! { #(#doc_lines)* }
        }
        None => TokenStream::new(),
    }
}

/// Generate a snake_case identifier from a name.
///
/// If the snake-cased form collides with a Rust reserved keyword
/// (`type`, `match`, `use`, `loop`, `move`, …) the emitted ident is
/// prefixed with the raw-identifier marker `r#` so the generated code
/// parses as a plain field access. Without this guard, schemas with
/// columns like `type` (common in Prisma) produce codegen output that
/// fails to parse with `expected identifier, found keyword \`type\``.
pub fn snake_ident(name: &str) -> proc_macro2::Ident {
    let snake = to_snake_case(name);
    if is_rust_keyword(&snake) {
        format_ident!("r#{}", snake)
    } else {
        format_ident!("{}", snake)
    }
}

/// Generate a PascalCase identifier from a name.
pub fn pascal_ident(name: &str) -> proc_macro2::Ident {
    format_ident!("{}", to_pascal_case(name))
}

/// Generate raw identifier (for reserved keywords).
#[allow(dead_code)]
pub fn raw_ident(name: &str) -> proc_macro2::Ident {
    format_ident!("r#{}", name)
}

/// Rust reserved keywords that must be escaped with the `r#` prefix
/// when used as field or variable identifiers in generated code.
///
/// Only keywords Rust permits as raw identifiers are listed. The four
/// keywords that are NOT legal as raw identifiers — `crate`, `self`,
/// `Self`, `super` — are intentionally omitted so `snake_ident` leaves
/// them un-escaped; a column literally named `self` would still fail to
/// compile, which is the correct behavior (the schema should be fixed).
fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "abstract"
            | "as"
            | "async"
            | "await"
            | "become"
            | "box"
            | "break"
            | "const"
            | "continue"
            | "do"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "final"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "macro"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "override"
            | "priv"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "trait"
            | "true"
            | "try"
            | "type"
            | "typeof"
            | "unsafe"
            | "unsized"
            | "use"
            | "virtual"
            | "where"
            | "while"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_doc_comment() {
        let doc = generate_doc_comment(Some("This is a test.\nSecond line."));
        let code = doc.to_string();
        assert!(code.contains("This is a test"));
        assert!(code.contains("Second line"));
    }

    #[test]
    fn test_generate_doc_comment_none() {
        let doc = generate_doc_comment(None);
        assert!(doc.is_empty());
    }

    #[test]
    fn test_snake_ident() {
        let ident = snake_ident("UserProfile");
        assert_eq!(ident.to_string(), "user_profile");
    }

    #[test]
    fn snake_ident_escapes_reserved_keywords_as_raw_idents() {
        // Columns named `type`, `match`, `use`, etc. (common in Prisma
        // schemas) would otherwise emit as `pub type: …`, which fails to
        // parse. `snake_ident` must prefix them with `r#`.
        assert_eq!(snake_ident("type").to_string(), "r#type");
        assert_eq!(snake_ident("match").to_string(), "r#match");
        assert_eq!(snake_ident("use").to_string(), "r#use");
        assert_eq!(snake_ident("loop").to_string(), "r#loop");
        assert_eq!(snake_ident("move").to_string(), "r#move");
    }

    #[test]
    fn snake_ident_does_not_escape_non_keywords_that_merely_start_with_one() {
        // Guard against an over-eager `starts_with` check — these names
        // are plain identifiers, not keywords.
        assert_eq!(snake_ident("type_id").to_string(), "type_id");
        assert_eq!(snake_ident("matches").to_string(), "matches");
    }

    #[test]
    fn test_pascal_ident() {
        let ident = pascal_ident("user_profile");
        assert_eq!(ident.to_string(), "UserProfile");
    }
}
