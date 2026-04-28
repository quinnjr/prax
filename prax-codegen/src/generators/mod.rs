//! Code generators for Prax models, enums, types, and views.

mod derive;
mod derive_client;
mod derive_from_row;
mod derive_model_trait;
mod derive_model_with_pk;
mod enum_gen;
mod fields;
mod filters;
mod model;
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
pub fn snake_ident(name: &str) -> proc_macro2::Ident {
    format_ident!("{}", to_snake_case(name))
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
    fn test_pascal_ident() {
        let ident = pascal_ident("user_profile");
        assert_eq!(ident.to_string(), "UserProfile");
    }
}
