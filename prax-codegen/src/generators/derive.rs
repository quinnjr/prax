//! Implementation of the `#[derive(Model)]` macro.

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, LitStr, Type};

/// Parse and generate code for the `#[derive(Model)]` macro.
pub fn derive_model_impl(input: &DeriveInput) -> Result<TokenStream, syn::Error> {
    let name = &input.ident;
    let module_name = format_ident!("{}", name.to_string().to_case(Case::Snake));

    // Extract struct fields
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "Model derive only supports structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input,
                "Model derive only supports structs",
            ));
        }
    };

    // Parse struct-level attributes
    let struct_attrs = parse_struct_attrs(input)?;
    let table_name = struct_attrs
        .table_name
        .unwrap_or_else(|| name.to_string().to_case(Case::Snake));

    // Parse field attributes
    let field_infos: Vec<FieldInfo> = fields.iter().map(parse_field).collect::<Result<_, _>>()?;

    // Find primary key fields
    let pk_fields: Vec<_> = field_infos
        .iter()
        .filter(|f| f.is_id)
        .map(|f| f.column_name.as_str())
        .collect();

    if pk_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "Model must have at least one field marked with #[prax(id)]",
        ));
    }

    // Generate field modules
    let field_modules: Vec<_> = field_infos
        .iter()
        .map(generate_field_module_from_derive)
        .collect();

    // Generate where param variants
    let where_variants: Vec<_> = field_infos
        .iter()
        .map(|f| {
            let variant_name = format_ident!("{}", f.name.to_string().to_case(Case::Pascal));
            let field_mod = &f.name;
            quote! { #variant_name(#field_mod::WhereOp) }
        })
        .collect();

    // Generate From<WhereParam> for Filter match arms
    let from_filter_arms: Vec<_> = field_infos
        .iter()
        .map(|f| {
            let variant_name = format_ident!("{}", f.name.to_string().to_case(Case::Pascal));
            quote! { WhereParam::#variant_name(op) => op.to_filter(), }
        })
        .collect();

    // Generate select param variants
    let select_variants: Vec<_> = field_infos
        .iter()
        .map(|f| {
            let variant_name = format_ident!("{}", f.name.to_string().to_case(Case::Pascal));
            quote! { #variant_name }
        })
        .collect();

    // Generate order by param variants
    let order_variants: Vec<_> = field_infos
        .iter()
        .filter(|f| !f.is_list)
        .map(|f| {
            let variant_name = format_ident!("{}", f.name.to_string().to_case(Case::Pascal));
            quote! { #variant_name(::prax_orm::_prax_prelude::SortOrder) }
        })
        .collect();

    // Prepare data for Model and FromRow trait implementations
    let all_columns: Vec<String> = field_infos
        .iter()
        .filter(|f| !f.is_list) // relations are list-typed and handled elsewhere
        .map(|f| f.column_name.clone())
        .collect();
    let pk_columns_owned: Vec<String> = field_infos
        .iter()
        .filter(|f| f.is_id)
        .map(|f| f.column_name.clone())
        .collect();
    let from_row_fields: Vec<(Ident, Type, String)> = field_infos
        .iter()
        .filter(|f| !f.is_list)
        .map(|f| (f.name.clone(), f.ty.clone(), f.column_name.clone()))
        .collect();

    let model_trait_impl = super::derive_model_trait::emit(
        name,
        &name.to_string(),
        &table_name,
        &pk_columns_owned,
        &all_columns,
    );
    let from_row_impl = super::derive_from_row::emit(name, &from_row_fields);

    Ok(quote! {
        /// Generated module for the #name model.
        pub mod #module_name {
            use super::*;

            /// Database table name.
            pub const TABLE_NAME: &str = #table_name;

            /// Primary key column(s).
            pub const PRIMARY_KEY: &[&str] = &[#(#pk_fields),*];

            impl ::prax_orm::_prax_prelude::PraxModel for #name {
                const TABLE_NAME: &'static str = TABLE_NAME;
                const PRIMARY_KEY: &'static [&'static str] = PRIMARY_KEY;
            }

            // Field modules
            #(#field_modules)*

            /// Where clause parameters.
            #[derive(Debug, Clone)]
            pub enum WhereParam {
                #(#where_variants,)*
                And(Vec<WhereParam>),
                Or(Vec<WhereParam>),
                Not(Box<WhereParam>),
            }

            impl From<WhereParam> for ::prax_query::filter::Filter {
                fn from(p: WhereParam) -> Self {
                    match p {
                        #(#from_filter_arms)*
                        WhereParam::And(ps) => ::prax_query::filter::Filter::And(
                            ps.into_iter().map(Into::into).collect::<Vec<_>>().into_boxed_slice()
                        ),
                        WhereParam::Or(ps) => ::prax_query::filter::Filter::Or(
                            ps.into_iter().map(Into::into).collect::<Vec<_>>().into_boxed_slice()
                        ),
                        WhereParam::Not(p) => ::prax_query::filter::Filter::Not(Box::new((*p).into())),
                    }
                }
            }

            /// Select parameters.
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub enum SelectParam {
                #(#select_variants,)*
            }

            /// Order by parameters.
            #[derive(Debug, Clone, Copy)]
            pub enum OrderByParam {
                #(#order_variants,)*
            }

            /// Query builder.
            #[derive(Debug, Default)]
            pub struct Query {
                pub select: Vec<SelectParam>,
                pub where_: Vec<WhereParam>,
                pub order_by: Vec<OrderByParam>,
                pub skip: Option<usize>,
                pub take: Option<usize>,
            }

            impl Query {
                pub fn new() -> Self {
                    Self::default()
                }

                pub fn r#where(mut self, param: WhereParam) -> Self {
                    self.where_.push(param);
                    self
                }

                pub fn order_by(mut self, param: OrderByParam) -> Self {
                    self.order_by.push(param);
                    self
                }

                pub fn skip(mut self, n: usize) -> Self {
                    self.skip = Some(n);
                    self
                }

                pub fn take(mut self, n: usize) -> Self {
                    self.take = Some(n);
                    self
                }
            }

            /// Model actions.
            pub struct Actions;

            impl Actions {
                pub fn find_many() -> Query {
                    Query::new()
                }

                pub fn find_unique() -> Query {
                    Query::new().take(1)
                }

                pub fn find_first() -> Query {
                    Query::new().take(1)
                }
            }
        }

        // Emit Model and FromRow trait implementations at crate root
        #model_trait_impl
        #from_row_impl
    })
}

/// Struct-level attributes parsed from `#[prax(...)]`.
#[derive(Debug, Default)]
struct StructAttrs {
    table_name: Option<String>,
    schema_name: Option<String>,
}

/// Parse struct-level `#[prax(...)]` attributes.
fn parse_struct_attrs(input: &DeriveInput) -> Result<StructAttrs, syn::Error> {
    let mut attrs = StructAttrs::default();

    for attr in &input.attrs {
        if !attr.path().is_ident("prax") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                let value: LitStr = meta.value()?.parse()?;
                attrs.table_name = Some(value.value());
            } else if meta.path.is_ident("schema") {
                let value: LitStr = meta.value()?.parse()?;
                attrs.schema_name = Some(value.value());
            }
            Ok(())
        })?;
    }

    Ok(attrs)
}

/// Information about a field.
#[derive(Debug)]
#[allow(dead_code)]
struct FieldInfo {
    name: Ident,
    ty: Type,
    column_name: String,
    is_id: bool,
    is_auto: bool,
    is_unique: bool,
    is_optional: bool,
    is_list: bool,
}

/// Parse a field and its `#[prax(...)]` attributes.
fn parse_field(field: &syn::Field) -> Result<FieldInfo, syn::Error> {
    let name = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new_spanned(field, "Fields must be named"))?;

    let ty = field.ty.clone();
    let mut column_name = name.to_string().to_case(Case::Snake);
    let mut is_id = false;
    let mut is_auto = false;
    let mut is_unique = false;

    // Determine if the type is Optional or Vec
    let is_optional = is_option_type(&ty);
    let is_list = is_vec_type(&ty);

    for attr in &field.attrs {
        if !attr.path().is_ident("prax") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                is_id = true;
            } else if meta.path.is_ident("auto") {
                is_auto = true;
            } else if meta.path.is_ident("unique") {
                is_unique = true;
            } else if meta.path.is_ident("column") {
                let value: LitStr = meta.value()?.parse()?;
                column_name = value.value();
            }
            Ok(())
        })?;
    }

    Ok(FieldInfo {
        name,
        ty,
        column_name,
        is_id,
        is_auto,
        is_unique,
        is_optional,
        is_list,
    })
}

/// Check if a type is `Option<T>`.
fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first() {
            return segment.ident == "Option";
        }
    }
    false
}

/// Check if a type is `Vec<T>`.
fn is_vec_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first() {
            return segment.ident == "Vec";
        }
    }
    false
}

/// Generate a field module from derive macro field info.
fn generate_field_module_from_derive(field: &FieldInfo) -> TokenStream {
    let field_name = &field.name;
    let column_name = &field.column_name;
    let ty = &field.ty;

    // Generate PascalCase variant name for WhereParam
    let variant_name = format_ident!("{}", field.name.to_string().to_case(Case::Pascal));

    // Generate basic where operations
    let where_ops = quote! {
        #[derive(Debug, Clone)]
        pub enum WhereOp {
            Equals(#ty),
            Not(#ty),
            IsNull,
            IsNotNull,
        }

        impl WhereOp {
            /// Convert to prax_query::filter::Filter.
            pub fn to_filter(self) -> ::prax_query::filter::Filter {
                use ::prax_query::filter::{Filter, FilterValue};
                use ::std::borrow::Cow;
                let col: Cow<'static, str> = Cow::Borrowed(COLUMN);
                match self {
                    Self::Equals(v) => Filter::Equals(col, v.into()),
                    Self::Not(v) => Filter::NotEquals(col, v.into()),
                    Self::IsNull => Filter::IsNull(col),
                    Self::IsNotNull => Filter::IsNotNull(col),
                }
            }
        }

        pub fn equals(value: #ty) -> super::WhereParam {
            super::WhereParam::#variant_name(WhereOp::Equals(value))
        }

        pub fn not(value: #ty) -> super::WhereParam {
            super::WhereParam::#variant_name(WhereOp::Not(value))
        }
    };

    quote! {
        pub mod #field_name {
            use super::*;

            pub const COLUMN: &str = #column_name;

            #where_ops
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_parse_simple_model() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            struct User {
                #[prax(id, auto)]
                id: i32,
                #[prax(unique)]
                email: String,
                name: Option<String>,
            }
        };

        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "Failed: {:?}", result.err());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub mod user"));
        assert!(code.contains("TABLE_NAME"));
        assert!(code.contains("users"));
    }

    #[test]
    fn test_parse_model_without_id() {
        let input: DeriveInput = parse_quote! {
            struct NoId {
                name: String,
            }
        };

        let result = derive_model_impl(&input);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_option_type() {
        let ty: Type = parse_quote!(Option<String>);
        assert!(is_option_type(&ty));

        let ty: Type = parse_quote!(String);
        assert!(!is_option_type(&ty));
    }

    #[test]
    fn test_is_vec_type() {
        let ty: Type = parse_quote!(Vec<i32>);
        assert!(is_vec_type(&ty));

        let ty: Type = parse_quote!(i32);
        assert!(!is_vec_type(&ty));
    }
}
