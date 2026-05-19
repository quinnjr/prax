//! Generate `<Model>WhereInput` for a parsed model.
//!
//! The struct has one `Option<ScalarFilter>` field per scalar column +
//! `Option<ListRelationFilter<...>>` for each to-many relation +
//! `Option<SingleRelationFilter<...>>` for each to-one relation. Plus the
//! `and` / `or` / `not` logical combinators.
//!
//! ## Visibility note
//!
//! `impl WhereInput for <Model>WhereInput { type Model = <Model>; }` must be
//! emitted **outside** the `pub mod <model>` block, at the same scope as the
//! model struct itself.  If it were emitted inside the module,
//! `type Model = super::<Model>` would expose the (potentially private) model
//! struct through a public trait impl and trigger E0446.  The caller
//! (`derive_model_impl`) must splice `struct_tokens` inside the per-model
//! `pub mod` and `impl_tokens` at the crate-root call site.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use super::{FilterCategory, filter_wrapper_ident};

/// One field's metadata as seen by the where-input generator.
pub struct WhereField {
    /// Field name in the source code (snake_case ident).
    pub name: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Filter category — `Some` for scalar fields, `None` for relation fields.
    pub category: Option<FilterCategory>,
    /// Whether the field is `Option<T>` (nullable).
    pub nullable: bool,
    /// For relation fields: the target model's `WhereInput` type ident.
    /// `None` for scalar fields.
    pub relation_target_where_input: Option<Ident>,
    /// For relation fields: `true` = to-many, `false` = to-one.
    pub is_to_many: bool,
}

/// Output of the where-input generator, split to avoid E0446.
///
/// `struct_tokens` goes inside `pub mod <model> { ... }`.
/// `impl_tokens` goes at the enclosing (crate-root) scope alongside the
/// other trait impls so `<Model>` is not leaked through a public interface.
pub struct WhereInputTokens {
    /// The `pub struct <Model>WhereInput { ... }` definition.
    pub struct_tokens: TokenStream,
    /// The `impl WhereInput for <Model>WhereInput { ... }` impl.
    pub impl_tokens: TokenStream,
}

/// Emit `<Model>WhereInput` + its `WhereInput` trait impl.
///
/// The returned [`WhereInputTokens`] must be split by the caller — see the
/// struct-level doc for the placement rules.
pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    fields: &[WhereField],
) -> WhereInputTokens {
    let where_input_ident = format_ident!("{}WhereInput", model_ident);

    let scalar_field_decls: Vec<TokenStream> = fields
        .iter()
        .filter(|f| f.category.is_some())
        .map(|f| {
            let name = &f.name;
            let cat = f.category.expect("scalar field has category");
            let wrapper = filter_wrapper_ident(cat, f.nullable);
            quote! {
                pub #name: ::core::option::Option<::prax_query::inputs::#wrapper>
            }
        })
        .collect();

    let relation_field_decls: Vec<TokenStream> = fields
        .iter()
        .filter(|f| f.relation_target_where_input.is_some())
        .map(|f| {
            let name = &f.name;
            let target = f.relation_target_where_input.as_ref().expect("relation");
            if f.is_to_many {
                quote! {
                    pub #name: ::core::option::Option<
                        ::prax_query::inputs::ListRelationFilter<#target>
                    >
                }
            } else {
                quote! {
                    pub #name: ::core::option::Option<
                        ::prax_query::inputs::SingleRelationFilter<#target>
                    >
                }
            }
        })
        .collect();

    let scalar_lowerings: Vec<TokenStream> = fields
        .iter()
        .filter(|f| f.category.is_some())
        .map(|f| {
            let name = &f.name;
            let col = &f.column;
            quote! {
                if let ::core::option::Option::Some(__inner) = self.#name {
                    use ::prax_query::inputs::ScalarFilter as _;
                    let __f = __inner.into_filter(#col);
                    if !matches!(__f, ::prax_query::filter::Filter::None) {
                        parts.push(__f);
                    }
                }
            }
        })
        .collect();

    let relation_lowerings: Vec<TokenStream> = fields
        .iter()
        .filter(|f| f.relation_target_where_input.is_some())
        .map(|f| {
            let name = &f.name;
            let meta_ident = {
                let pascal_rel = super::super::pascal_ident(&f.name.to_string());
                format_ident!("{}{}FilterMeta", model_ident, pascal_rel)
            };
            quote! {
                if let ::core::option::Option::Some(__inner) = self.#name {
                    use ::prax_query::inputs::LowerRelationFilter as _;
                    let __f = __inner.lower::<#meta_ident>();
                    if !matches!(__f, ::prax_query::filter::Filter::None) {
                        parts.push(__f);
                    }
                }
            }
        })
        .collect();

    // The struct is emitted inside `pub mod <module>`.
    let struct_tokens = quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #where_input_ident {
            #(#scalar_field_decls,)*
            #(#relation_field_decls,)*
            pub and: ::core::option::Option<::std::vec::Vec<#where_input_ident>>,
            pub or: ::core::option::Option<::std::vec::Vec<#where_input_ident>>,
            pub not: ::core::option::Option<::std::boxed::Box<#where_input_ident>>,
        }
    };

    // The impl is emitted at the enclosing scope (alongside the model struct)
    // so `#model_ident` is the struct at that scope, not `super::<Model>`.
    // This avoids E0446 "private type in public interface" when the model
    // struct is not `pub`.
    let impl_tokens = quote! {
        impl ::prax_query::inputs::WhereInput for #module_name::#where_input_ident {
            type Model = #model_ident;
            fn into_ir(self) -> ::prax_query::filter::Filter {
                let mut parts: ::std::vec::Vec<::prax_query::filter::Filter> =
                    ::std::vec::Vec::new();

                #(#scalar_lowerings)*
                #(#relation_lowerings)*

                if let ::core::option::Option::Some(ands) = self.and {
                    let inner: ::std::vec::Vec<::prax_query::filter::Filter> = ands
                        .into_iter()
                        .map(|w| <#module_name::#where_input_ident as
                            ::prax_query::inputs::WhereInput>::into_ir(w))
                        .collect();
                    parts.push(::prax_query::filter::Filter::and(inner));
                }
                if let ::core::option::Option::Some(ors) = self.or {
                    let inner: ::std::vec::Vec<::prax_query::filter::Filter> = ors
                        .into_iter()
                        .map(|w| <#module_name::#where_input_ident as
                            ::prax_query::inputs::WhereInput>::into_ir(w))
                        .collect();
                    parts.push(::prax_query::filter::Filter::or(inner));
                }
                if let ::core::option::Option::Some(n) = self.not {
                    parts.push(::prax_query::filter::Filter::Not(::std::boxed::Box::new(
                        <#module_name::#where_input_ident as
                            ::prax_query::inputs::WhereInput>::into_ir(*n),
                    )));
                }

                match parts.len() {
                    0 => ::prax_query::filter::Filter::None,
                    1 => parts.into_iter().next().unwrap(),
                    _ => ::prax_query::filter::Filter::and(parts),
                }
            }
        }
    };

    WhereInputTokens {
        struct_tokens,
        impl_tokens,
    }
}
