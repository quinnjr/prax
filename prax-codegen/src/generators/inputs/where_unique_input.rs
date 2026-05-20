//! Generate `<Model>WhereUniqueInput` — an enum over the model's
//! primary-key and `@unique` columns.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, scalar_payload_type};

/// Reject schemas where two unique columns PascalCase to the same enum
/// variant — without this check, codegen emits a duplicate-variant enum
/// and the user sees an opaque "identifier bound more than once" error
/// at their call site instead of one pointing at the schema.
///
/// `model_name` is the parent model; passing `None` produces a shorter
/// error message suitable for the derive path (where the model is
/// implied by the macro's span).
pub fn check_unique_column_collisions(
    columns: &[UniqueColumn],
    model_name: Option<&str>,
) -> Result<(), syn::Error> {
    let mut seen: Vec<String> = Vec::with_capacity(columns.len());
    for col in columns {
        let v = col.variant.to_string();
        if seen.iter().any(|prev| prev == &v) {
            let msg = match model_name {
                Some(model) => format!(
                    "model `{}`: two unique fields PascalCase to the same `{}` \
                     variant in the generated WhereUniqueInput enum",
                    model, v,
                ),
                None => format!(
                    "two unique fields PascalCase to the same `{}` variant \
                     in the generated WhereUniqueInput enum",
                    v,
                ),
            };
            return Err(syn::Error::new_spanned(&col.variant, msg));
        }
        seen.push(v);
    }
    Ok(())
}

/// One unique-key column.
pub struct UniqueColumn {
    /// The variant name (PascalCase of the column).
    pub variant: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Filter category for the scalar payload.
    pub category: FilterCategory,
    /// For enum columns: the enum's PascalCase ident. Currently always
    /// `None` — enum-typed unique columns are skipped from generation
    /// until enum-aware codegen wires the ident through.
    pub enum_ident: Option<Ident>,
}

/// Emit the `<Model>WhereUniqueInput` enum + `WhereUniqueInput` trait impl.
///
/// Returns `(struct_tokens, impl_tokens)`. The struct goes inside the
/// per-model `pub mod` block; the impl goes at the enclosing scope so
/// `type Model = #model_ident` resolves to the model struct alongside
/// the enum — avoiding E0446 "private type in public interface" when
/// the model struct is not `pub`. (See `where_input.rs` for the same
/// split rationale.)
pub fn generate(
    model_ident: &Ident,
    module_name: &Ident,
    columns: &[UniqueColumn],
) -> (TokenStream, TokenStream) {
    let where_unique_ident = format_ident!("{}WhereUniqueInput", model_ident);

    if columns.is_empty() {
        let struct_tokens = quote! {
            /// Uninhabited because the model has no unique key.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #where_unique_ident {}
        };
        let impl_tokens = quote! {
            impl ::prax_query::inputs::WhereUniqueInput for #module_name::#where_unique_ident {
                type Model = #model_ident;
                fn into_ir(self) -> ::prax_query::filter::Filter {
                    match self {}
                }
            }
        };
        return (struct_tokens, impl_tokens);
    }

    let variant_decls = columns.iter().filter_map(|c| {
        let v = &c.variant;
        let payload = match &c.enum_ident {
            Some(e) => quote! { #e },
            None => scalar_payload_type(c.category)?,
        };
        Some(quote! { #v(#payload) })
    });

    let lower_arms = columns.iter().map(|c| {
        let v = &c.variant;
        let col = &c.column;
        let body = match c.category {
            FilterCategory::Int => quote! { ::prax_query::filter::FilterValue::Int(value as i64) },
            FilterCategory::BigInt => quote! { ::prax_query::filter::FilterValue::Int(value) },
            FilterCategory::Float => quote! { ::prax_query::filter::FilterValue::Float(value) },
            FilterCategory::Bool => quote! { ::prax_query::filter::FilterValue::Bool(value) },
            FilterCategory::String => quote! { ::prax_query::filter::FilterValue::String(value) },
            FilterCategory::Decimal => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
            FilterCategory::Uuid => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
            FilterCategory::Bytes => quote! {
                {
                    use ::prax_query::base64::Engine as _;
                    ::prax_query::filter::FilterValue::String(
                        ::prax_query::base64::engine::general_purpose::STANDARD.encode(&value),
                    )
                }
            },
            FilterCategory::DateTime => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_rfc3339()) }
            }
            FilterCategory::Date => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
            FilterCategory::Time => quote! {
                ::prax_query::filter::FilterValue::String(value.format("%H:%M:%S").to_string())
            },
            FilterCategory::Json => quote! { ::prax_query::filter::FilterValue::Json(value) },
            FilterCategory::Enum => {
                quote! { ::prax_query::filter::FilterValue::String(value.to_string()) }
            }
        };
        // Fully-qualify the pattern with the module path to avoid introducing
        // a `use` statement at the impl scope (which triggers E0603 "private
        // struct import" when the macro is expanded in integration-test files).
        quote! {
            #module_name::#where_unique_ident::#v(value) => ::prax_query::filter::Filter::Equals(
                ::std::borrow::Cow::Borrowed(#col),
                #body,
            )
        }
    });

    let struct_tokens = quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum #where_unique_ident {
            #(#variant_decls,)*
        }
    };

    let impl_tokens = quote! {
        impl ::prax_query::inputs::WhereUniqueInput for #module_name::#where_unique_ident {
            type Model = #model_ident;
            fn into_ir(self) -> ::prax_query::filter::Filter {
                match self {
                    #(#lower_arms,)*
                }
            }
        }
    };

    (struct_tokens, impl_tokens)
}
