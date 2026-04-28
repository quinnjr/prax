//! Implementation of the `#[derive(Model)]` macro.

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
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

    // Generate field modules. Relation fields (`is_list`) don't map to
    // a column, so they skip the scalar-field emitters and go through
    // `relation_accessors::emit` instead.
    let field_modules: Vec<_> = field_infos
        .iter()
        .filter(|f| !f.is_list)
        .map(generate_field_module_from_derive)
        .collect();

    // Generate where param variants. `is_list` fields are relations,
    // not columns — emitting a `WhereOp` over `Vec<Related>` would try
    // to build filters over a non-scalar type and fail to compile.
    let where_variants: Vec<_> = field_infos
        .iter()
        .filter(|f| !f.is_list)
        .map(|f| {
            let variant_name = format_ident!("{}", f.name.to_string().to_case(Case::Pascal));
            let field_mod = &f.name;
            quote! { #variant_name(#field_mod::WhereOp) }
        })
        .collect();

    // Generate From<WhereParam> for Filter match arms. Mirrors the
    // same filter applied to `where_variants` above.
    let from_filter_arms: Vec<_> = field_infos
        .iter()
        .filter(|f| !f.is_list)
        .map(|f| {
            let variant_name = format_ident!("{}", f.name.to_string().to_case(Case::Pascal));
            quote! { WhereParam::#variant_name(op) => op.to_filter(), }
        })
        .collect();

    // Generate select param variants. Relation fields aren't columns,
    // so they don't show up in `SelectParam` either.
    let select_variants: Vec<_> = field_infos
        .iter()
        .filter(|f| !f.is_list)
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

    // Relation fields (`Vec<_>`) are handled separately by the relation
    // codegen; they're not columns and don't round-trip through FromRow.
    let all_columns: Vec<String> = field_infos
        .iter()
        .filter(|f| !f.is_list)
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
    // Relation fields get initialized to `Default::default()` on
    // `from_row` — the `.include()` path fills them afterwards.
    let from_row_relation_fields: Vec<Ident> = field_infos
        .iter()
        .filter(|f| f.is_list)
        .map(|f| f.name.clone())
        .collect();
    // Same shape as from_row_fields plus the is_id flag; the ModelWithPk
    // emitter needs is_id to route PK fields into pk_value() and every
    // scalar field into get_column_value().
    let model_with_pk_fields: Vec<(Ident, Type, String, bool)> = field_infos
        .iter()
        .filter(|f| !f.is_list)
        .map(|f| (f.name.clone(), f.ty.clone(), f.column_name.clone(), f.is_id))
        .collect();

    let model_trait_impl = super::derive_model_trait::emit(
        name,
        &name.to_string(),
        &table_name,
        &pk_columns_owned,
        &all_columns,
    );
    let from_row_impl =
        super::derive_from_row::emit(name, &from_row_fields, &from_row_relation_fields);
    let model_with_pk_impl = super::derive_model_with_pk::emit(name, &model_with_pk_fields);
    let client_impl = super::derive_client::emit(quote! { super::#name });

    // Per-relation `pub mod <field>` modules with `fetch()` and the
    // `Relation` marker. Only relation fields — scalar fields already
    // have their own `pub mod <field>` emitted by
    // `generate_field_module_from_derive`.
    let relation_mods: Vec<_> = field_infos
        .iter()
        .filter_map(|f| {
            f.relation.as_ref().map(|rel| {
                let kind = if f.is_list {
                    super::relation_accessors::RelationKindTokens::HasMany
                } else if f.is_optional {
                    super::relation_accessors::RelationKindTokens::HasOne
                } else {
                    super::relation_accessors::RelationKindTokens::BelongsTo
                };
                super::relation_accessors::emit(super::relation_accessors::RelationSpec {
                    field_name: &f.name,
                    owner: name,
                    target: &rel.target,
                    kind,
                    local_key: &rel.local_key,
                    foreign_key: &rel.foreign_key,
                })
            })
        })
        .collect();

    // Per-model `impl ModelRelationLoader<E>` dispatcher. Models with
    // no relations still get an impl — it errors on any unknown name,
    // preserving the uniform `ModelRelationLoader` bound on find
    // operations.
    let loader_relations: Vec<super::derive_relation_loader::LoaderRelation<'_>> = field_infos
        .iter()
        .filter_map(|f| {
            f.relation.as_ref().map(|rel| {
                let kind = if f.is_list {
                    super::derive_relation_loader::LoaderKind::HasMany
                } else {
                    super::derive_relation_loader::LoaderKind::HasOne
                };
                super::derive_relation_loader::LoaderRelation {
                    field_name: &f.name,
                    target: &rel.target,
                    kind,
                }
            })
        })
        .collect();
    let model_relation_loader_impl = super::derive_relation_loader::emit(name, &loader_relations);

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

            // Per-relation modules — emitted for every field marked
            // `#[prax(relation(...))]`. Each one defines `fetch()` +
            // `Relation` (a zero-sized `RelationMeta` marker).
            #(#relation_mods)*

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

            #client_impl
        }

        // Emit Model, FromRow, and ModelWithPk trait implementations at crate root
        #model_trait_impl
        #from_row_impl
        #model_with_pk_impl

        // Emit ModelRelationLoader<E> so every derived model — relations
        // or not — can be used on the `.include()` path uniformly.
        #model_relation_loader_impl
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
    relation: Option<RelationAttr>,
}

/// Parsed `#[prax(relation(target = "...", foreign_key = "...", local_key = "..."))]`.
///
/// Relation fields are not columns — the derive filters them out of
/// every column/WhereParam/SelectParam emission path and funnels them
/// instead into the per-relation `pub mod <field>` / `Relation` pair
/// emitted by [`super::relation_accessors`] plus the per-model
/// `ModelRelationLoader` impl emitted by
/// [`super::derive_relation_loader`].
#[derive(Debug)]
struct RelationAttr {
    /// Target model type identifier — the type the relation points at.
    target: syn::Ident,
    /// Column on the target model holding the FK back to this model's
    /// PK (for `HasMany` / `HasOne`). Required.
    foreign_key: String,
    /// Column on this model referencing the target's PK (for
    /// `BelongsTo`). Defaults to `"id"`.
    local_key: String,
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
    let mut relation: Option<RelationAttr> = None;

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
            } else if meta.path.is_ident("relation") {
                let mut target: Option<syn::Ident> = None;
                let mut fk: Option<String> = None;
                let mut lk: Option<String> = None;
                meta.parse_nested_meta(|inner| {
                    if inner.path.is_ident("target") {
                        let s: LitStr = inner.value()?.parse()?;
                        target = Some(format_ident!("{}", s.value()));
                    } else if inner.path.is_ident("foreign_key") {
                        let s: LitStr = inner.value()?.parse()?;
                        fk = Some(s.value());
                    } else if inner.path.is_ident("local_key") {
                        let s: LitStr = inner.value()?.parse()?;
                        lk = Some(s.value());
                    }
                    Ok(())
                })?;
                let target = target.ok_or_else(|| {
                    syn::Error::new(meta.path.span(), "relation requires target = \"ModelName\"")
                })?;
                let foreign_key = fk.ok_or_else(|| {
                    syn::Error::new(meta.path.span(), "relation requires foreign_key = \"...\"")
                })?;
                relation = Some(RelationAttr {
                    target,
                    foreign_key,
                    local_key: lk.unwrap_or_else(|| "id".to_string()),
                });
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
        relation,
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

/// Field type category for conditional filter emission.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeCategory {
    Numeric,
    String,
    Boolean,
    Other,
}

/// Classify a Rust type for filter operator emission.
///
/// Inspects the type to determine which filter operators make sense. Unwraps
/// Option<T> to classify the inner type. Returns the category that drives
/// conditional emission of comparison, string, and IN operators.
fn classify_field_type(ty: &Type) -> TypeCategory {
    // Unwrap Option<T> to get the inner type.
    let type_name = if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first() {
            if segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        // Recurse to classify the inner type.
                        return classify_field_type(inner);
                    }
                }
            }
        }
        // Not Option — extract the last segment as the type name.
        type_path.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    };

    match type_name.as_deref() {
        Some(
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "usize" | "f32" | "f64" | "Decimal" | "NaiveDate" | "NaiveDateTime" | "NaiveTime"
            | "DateTime",
        ) => TypeCategory::Numeric,
        Some("String" | "str") => TypeCategory::String,
        Some("bool") => TypeCategory::Boolean,
        _ => TypeCategory::Other,
    }
}

/// Generate a field module from derive macro field info.
fn generate_field_module_from_derive(field: &FieldInfo) -> TokenStream {
    let field_name = &field.name;
    let column_name = &field.column_name;
    let ty = &field.ty;
    let variant_name = format_ident!("{}", field.name.to_string().to_case(Case::Pascal));
    let is_optional = field.is_optional;
    let category = classify_field_type(ty);

    // Collect WhereOp enum variants conditionally.
    let mut variants = vec![quote! { Equals(#ty) }, quote! { Not(#ty) }];

    if is_optional {
        variants.push(quote! { IsNull });
        variants.push(quote! { IsNotNull });
    }

    match category {
        TypeCategory::Numeric => {
            variants.push(quote! { In(Vec<#ty>) });
            variants.push(quote! { NotIn(Vec<#ty>) });
            variants.push(quote! { Gt(#ty) });
            variants.push(quote! { Gte(#ty) });
            variants.push(quote! { Lt(#ty) });
            variants.push(quote! { Lte(#ty) });
        }
        TypeCategory::String => {
            variants.push(quote! { In(Vec<#ty>) });
            variants.push(quote! { NotIn(Vec<#ty>) });
            variants.push(quote! { Contains(String) });
            variants.push(quote! { StartsWith(String) });
            variants.push(quote! { EndsWith(String) });
        }
        TypeCategory::Boolean => {}
        TypeCategory::Other => {
            variants.push(quote! { In(Vec<#ty>) });
            variants.push(quote! { NotIn(Vec<#ty>) });
        }
    }

    // Collect to_filter match arms in the same conditional shape.
    let mut arms = vec![
        quote! { Self::Equals(v) => Filter::Equals(col, v.into()) },
        quote! { Self::Not(v) => Filter::NotEquals(col, v.into()) },
    ];

    if is_optional {
        arms.push(quote! { Self::IsNull => Filter::IsNull(col) });
        arms.push(quote! { Self::IsNotNull => Filter::IsNotNull(col) });
    }

    match category {
        TypeCategory::Numeric => {
            arms.push(
                quote! { Self::In(vs) => Filter::In(col, vs.into_iter().map(Into::into).collect()) },
            );
            arms.push(
                quote! { Self::NotIn(vs) => Filter::NotIn(col, vs.into_iter().map(Into::into).collect()) },
            );
            arms.push(quote! { Self::Gt(v) => Filter::Gt(col, v.into()) });
            arms.push(quote! { Self::Gte(v) => Filter::Gte(col, v.into()) });
            arms.push(quote! { Self::Lt(v) => Filter::Lt(col, v.into()) });
            arms.push(quote! { Self::Lte(v) => Filter::Lte(col, v.into()) });
        }
        TypeCategory::String => {
            arms.push(
                quote! { Self::In(vs) => Filter::In(col, vs.into_iter().map(Into::into).collect()) },
            );
            arms.push(
                quote! { Self::NotIn(vs) => Filter::NotIn(col, vs.into_iter().map(Into::into).collect()) },
            );
            arms.push(
                quote! { Self::Contains(v) => Filter::Contains(col, FilterValue::String(v)) },
            );
            arms.push(
                quote! { Self::StartsWith(v) => Filter::StartsWith(col, FilterValue::String(v)) },
            );
            arms.push(
                quote! { Self::EndsWith(v) => Filter::EndsWith(col, FilterValue::String(v)) },
            );
        }
        TypeCategory::Boolean => {}
        TypeCategory::Other => {
            arms.push(
                quote! { Self::In(vs) => Filter::In(col, vs.into_iter().map(Into::into).collect()) },
            );
            arms.push(
                quote! { Self::NotIn(vs) => Filter::NotIn(col, vs.into_iter().map(Into::into).collect()) },
            );
        }
    }

    // Collect constructor functions.
    let mut ctors = vec![quote! {
        pub fn equals(value: #ty) -> super::WhereParam {
            super::WhereParam::#variant_name(WhereOp::Equals(value))
        }

        pub fn not(value: #ty) -> super::WhereParam {
            super::WhereParam::#variant_name(WhereOp::Not(value))
        }
    }];

    if is_optional {
        ctors.push(quote! {
            pub fn is_null() -> super::WhereParam {
                super::WhereParam::#variant_name(WhereOp::IsNull)
            }

            pub fn is_not_null() -> super::WhereParam {
                super::WhereParam::#variant_name(WhereOp::IsNotNull)
            }
        });
    }

    match category {
        TypeCategory::Numeric => {
            ctors.push(quote! {
                pub fn in_(values: Vec<#ty>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::In(values))
                }

                pub fn not_in(values: Vec<#ty>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::NotIn(values))
                }

                pub fn gt(value: #ty) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::Gt(value))
                }

                pub fn gte(value: #ty) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::Gte(value))
                }

                pub fn lt(value: #ty) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::Lt(value))
                }

                pub fn lte(value: #ty) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::Lte(value))
                }
            });
        }
        TypeCategory::String => {
            ctors.push(quote! {
                pub fn in_(values: Vec<#ty>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::In(values))
                }

                pub fn not_in(values: Vec<#ty>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::NotIn(values))
                }

                pub fn contains(value: impl Into<String>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::Contains(value.into()))
                }

                pub fn starts_with(value: impl Into<String>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::StartsWith(value.into()))
                }

                pub fn ends_with(value: impl Into<String>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::EndsWith(value.into()))
                }
            });
        }
        TypeCategory::Boolean => {}
        TypeCategory::Other => {
            ctors.push(quote! {
                pub fn in_(values: Vec<#ty>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::In(values))
                }

                pub fn not_in(values: Vec<#ty>) -> super::WhereParam {
                    super::WhereParam::#variant_name(WhereOp::NotIn(values))
                }
            });
        }
    }

    quote! {
        pub mod #field_name {
            use super::*;

            pub const COLUMN: &str = #column_name;

            #[derive(Debug, Clone)]
            pub enum WhereOp {
                #(#variants,)*
            }

            impl WhereOp {
                /// Convert to prax_query::filter::Filter.
                pub fn to_filter(self) -> ::prax_query::filter::Filter {
                    use ::prax_query::filter::{Filter, FilterValue};
                    use ::std::borrow::Cow;
                    let col: Cow<'static, str> = Cow::Borrowed(COLUMN);
                    match self {
                        #(#arms,)*
                    }
                }
            }

            #(#ctors)*
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
