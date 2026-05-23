//! Code generation for Prax models.

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use prax_schema::ModelStyle;
use prax_schema::ast::{FieldType, Model, ScalarType, Schema, TypeModifier};

use super::fields::{
    generate_field_module, generate_order_by_param, generate_select_param, generate_set_param,
};
use super::inputs;
use super::inputs::create_input::CreateField;
use super::inputs::include_input::IncludeField;
use super::inputs::order_by_input::OrderByInputField;
use super::inputs::relation_meta::RelationMetaSpec;
use super::inputs::select_input::SelectField;
use super::inputs::update_input::UpdateField;
use super::inputs::where_input::WhereField;
use super::inputs::where_unique_input::UniqueColumn;
use super::{generate_doc_comment, pascal_ident, snake_ident};
use crate::types::field_type_to_rust;

/// Map a `ScalarType` to the string key used by `filter_category_for`.
fn scalar_type_name(s: &ScalarType) -> &'static str {
    match s {
        ScalarType::Int => "Int",
        ScalarType::BigInt => "BigInt",
        ScalarType::Float => "Float",
        ScalarType::Decimal => "Decimal",
        ScalarType::String => "String",
        ScalarType::Boolean => "Boolean",
        ScalarType::DateTime => "DateTime",
        ScalarType::Date => "Date",
        ScalarType::Time => "Time",
        ScalarType::Json => "Json",
        ScalarType::Bytes => "Bytes",
        ScalarType::Uuid => "Uuid",
        // String-backed IDs
        ScalarType::Cuid | ScalarType::Cuid2 | ScalarType::NanoId | ScalarType::Ulid => "String",
        // Vector / bit types — no filter wrapper; treated as unknown
        ScalarType::Vector(_)
        | ScalarType::HalfVector(_)
        | ScalarType::SparseVector(_)
        | ScalarType::Bit(_) => "",
    }
}

// ---------------------------------------------------------------------------
// Field-list adapter functions — build the input-spec lists from schema AST.
// ---------------------------------------------------------------------------

/// Build the per-model `WhereField` list for the schema path.
///
/// Enum-typed columns are skipped until enum-aware codegen wires through
/// the user enum's PascalCase ident — without that, the generator would
/// emit `EnumFilter` instead of `EnumFilter<E>` and fail to compile.
fn collect_where_fields(model: &Model) -> Vec<WhereField> {
    model
        .fields
        .values()
        .filter_map(|f| {
            match &f.field_type {
                FieldType::Model(_) => {
                    // Relation fields: skip until FilterMeta wiring is present
                    // (same decision as derive path).
                    None
                }
                FieldType::Scalar(s) => {
                    let type_name = scalar_type_name(s);
                    let category = inputs::filter_category_for(type_name);
                    Some(WhereField {
                        name: snake_ident(f.name()),
                        column: column_name_of(f),
                        category,
                        nullable: f.modifier.is_optional(),
                        relation_target_where_input: None,
                        is_to_many: false,
                    })
                }
                FieldType::Enum(_) => {
                    // Enum-typed columns are skipped until enum-aware codegen
                    // threads the user enum's PascalCase ident through. Without
                    // it, `where_input::generate` emits `EnumFilter` without
                    // the required `<E>` type parameter and fails to compile.
                    None
                }
                FieldType::Composite(_) | FieldType::Unsupported(_) => None,
            }
        })
        .collect()
}

fn collect_unique_columns(model: &Model) -> Vec<UniqueColumn> {
    // Enum-typed unique columns are skipped until enum-aware codegen wires
    // through the user enum's PascalCase ident (the `WhereUniqueInput`
    // generator panics on `FilterCategory::Enum` without a concrete ident).
    model
        .fields
        .values()
        .filter(|f| (f.is_id() || f.is_unique()) && !matches!(f.field_type, FieldType::Model(_)))
        .filter_map(|f| {
            let cat = match &f.field_type {
                FieldType::Scalar(s) => inputs::filter_category_for(scalar_type_name(s))?,
                FieldType::Enum(_) => return None,
                _ => return None,
            };
            Some(UniqueColumn {
                variant: format_ident!("{}", f.name().to_case(Case::Pascal)),
                column: column_name_of(f),
                category: cat,
                enum_ident: None,
            })
        })
        .collect()
}

fn collect_include_fields(model: &Model) -> Vec<IncludeField> {
    model
        .fields
        .values()
        .filter(|f| matches!(f.field_type, FieldType::Model(_)))
        .map(|f| IncludeField {
            name: snake_ident(f.name()),
            relation: f.name().to_string(),
        })
        .collect()
}

fn collect_select_fields(model: &Model) -> Vec<SelectField> {
    model
        .fields
        .values()
        .map(|f| SelectField {
            name: snake_ident(f.name()),
            column: column_name_of(f),
            is_relation: matches!(f.field_type, FieldType::Model(_)),
            is_no_column: false,
        })
        .collect()
}

fn collect_order_by_fields(model: &Model) -> Vec<OrderByInputField> {
    model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| OrderByInputField {
            variant: format_ident!("{}", f.name().to_case(Case::Pascal)),
            column: column_name_of(f),
            nullable: f.modifier.is_optional(),
        })
        .collect()
}

fn collect_create_fields(model: &Model) -> Vec<CreateField> {
    // Enum-typed columns are skipped until enum-aware codegen wires
    // through the user enum's PascalCase ident.
    model
        .fields
        .values()
        .filter(|f| is_mutable_scalar(f))
        .filter_map(|f| {
            let cat = match &f.field_type {
                FieldType::Scalar(s) => inputs::filter_category_for(scalar_type_name(s))?,
                _ => return None,
            };
            // Single attribute-list scan instead of one in `is_mutable_scalar`
            // plus another for `has_default`.
            let attrs = f.extract_attributes();
            Some(CreateField {
                name: snake_ident(f.name()),
                column: column_name_of(f),
                category: cat,
                nullable: f.modifier.is_optional(),
                has_default: attrs.default.is_some(),
                enum_ident: None,
            })
        })
        .collect()
}

fn collect_update_fields(model: &Model) -> Vec<UpdateField> {
    // Enum-typed columns are skipped until enum-aware codegen wires
    // through the user enum's PascalCase ident.
    model
        .fields
        .values()
        .filter(|f| is_mutable_scalar(f))
        .filter_map(|f| {
            let cat = match &f.field_type {
                FieldType::Scalar(s) => inputs::filter_category_for(scalar_type_name(s))?,
                _ => return None,
            };
            Some(UpdateField {
                name: snake_ident(f.name()),
                column: column_name_of(f),
                category: cat,
                nullable: f.modifier.is_optional(),
                enum_ident: None,
            })
        })
        .collect()
}

fn collect_relation_meta_specs(
    model: &Model,
    schema: &Schema,
) -> Result<Vec<RelationMetaSpec>, syn::Error> {
    // Resolve the parent PK column. Prefer the field-level `@id` form
    // because it respects `@map` rewrites via `column_name_of`. Composite
    // `@@id([...])` models are valid per the schema validator but produce
    // an empty `id_fields()`; fall back to the first field name listed in
    // the model-level `@@id` attribute. Fail loudly if a relation is
    // declared on a PK-less model.
    let parent_pk = model
        .id_fields()
        .into_iter()
        .next()
        .map(column_name_of)
        .or_else(|| get_primary_key_fields(model).into_iter().next())
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "model `{}` has a relation but no primary key — typed-input \
                     relation codegen requires at least one `@id` or `@@id` column",
                    model.name()
                ),
            )
        })?;

    let mut specs = Vec::new();
    for f in model.fields.values() {
        let target_model_name = match &f.field_type {
            FieldType::Model(name) => name.as_str(),
            _ => continue,
        };

        // Look up the target model for its actual table name. Unknown target
        // models are a schema error — emit a span'd diagnostic rather than
        // silently emitting a wrong CHILD_TABLE constant.
        let child_table = schema
            .models
            .get(target_model_name)
            .map(|m| m.table_name().to_string())
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "relation `{}` on model `{}` references unknown target model `{}`",
                        f.name(),
                        model.name(),
                        target_model_name
                    ),
                )
            })?;

        // The FK column comes from `@relation(fields: [...])`. Phase 2's
        // typed-input codegen requires it to be explicit — guessing a
        // default would silently emit a wrong CHILD_FK constant that
        // produces runtime "column does not exist" errors at query time.
        let child_fk = f
            .extract_attributes()
            .relation
            .and_then(|r| r.fields.into_iter().next())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!(
                        "relation `{}` on model `{}` must specify \
                         `@relation(fields: [...])` for typed-input codegen \
                         to resolve the FK column",
                        f.name(),
                        model.name()
                    ),
                )
            })?;

        specs.push(RelationMetaSpec {
            meta_ident: format_ident!(
                "{}{}FilterMeta",
                pascal_ident(model.name()),
                f.name().to_case(Case::Pascal)
            ),
            parent_table: model.table_name().to_string(),
            parent_pk: parent_pk.clone(),
            child_table,
            child_fk,
        });
    }
    Ok(specs)
}

/// Whether a field is eligible for include in create / update payloads.
///
/// Excludes relation fields (which are written via nested-write inputs in
/// a later phase) and DB-managed columns (`@auto`-generated PKs, fields
/// with `@updatedAt`). Shared by the new `collect_create_fields` /
/// `collect_update_fields` collectors and the legacy `create_fields` /
/// `update_fields` builders inside `generate_model_module_with_style` so
/// the exclusion rule stays consistent in both code paths.
fn is_mutable_scalar(f: &prax_schema::ast::Field) -> bool {
    if matches!(f.field_type, FieldType::Model(_)) {
        return false;
    }
    let attrs = f.extract_attributes();
    !attrs.is_auto && !attrs.is_updated_at
}

/// Pull the `@map("col")` override for a field, falling back to its
/// declared name. Mirrors the serde-rename logic further down — both
/// must agree on which string names the column in SQL vs. the struct
/// field in Rust.
fn column_name_of(field: &prax_schema::ast::Field) -> String {
    field
        .attributes
        .iter()
        .find(|a| a.name() == "map")
        .and_then(|a| a.first_arg())
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .unwrap_or_else(|| field.name().to_string())
}

/// Generate the complete module for a model.
///
/// When `model_style` is `GraphQL`, the generated structs will include
/// async-graphql derive macros (`SimpleObject`, `InputObject`).
#[allow(dead_code)]
pub fn generate_model_module(model: &Model, schema: &Schema) -> Result<TokenStream, syn::Error> {
    generate_model_module_with_style(model, schema, ModelStyle::Standard)
}

/// Generate the complete module for a model with a specific style.
pub fn generate_model_module_with_style(
    model: &Model,
    schema: &Schema,
    model_style: ModelStyle,
) -> Result<TokenStream, syn::Error> {
    let model_name = pascal_ident(model.name());
    let module_name = snake_ident(model.name());

    let doc = generate_doc_comment(model.documentation.as_ref().map(|d| d.text.as_str()));

    // Get database table name
    let table_name = model.table_name().to_string();
    let table_name_str = table_name.as_str();

    // Get primary key field(s)
    let pk_fields = get_primary_key_fields(model);
    let pk_field_names: Vec<_> = pk_fields.iter().map(|f| f.as_str()).collect();

    // Generate Data struct fields
    let data_fields: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let field_name = snake_ident(field.name());
            let field_type = field_type_to_rust(&field.field_type, &field.modifier);
            let field_doc =
                generate_doc_comment(field.documentation.as_ref().map(|d| d.text.as_str()));

            let serde_rename = field
                .attributes
                .iter()
                .find(|a| a.name() == "map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|name| quote! { #[serde(rename = #name)] })
                .unwrap_or_default();

            quote! {
                #field_doc
                #serde_rename
                pub #field_name: #field_type
            }
        })
        .collect();

    // Generate CreateInput fields (excluding auto-generated fields).
    // `is_mutable_scalar` and `attrs.default` together do two attribute-list
    // scans per field; hoist into one call.
    let create_fields: Vec<_> = model
        .fields
        .values()
        .filter(|f| is_mutable_scalar(f))
        .map(|field| {
            let field_name = snake_ident(field.name());
            let attrs = field.extract_attributes();
            let is_optional = field.modifier.is_optional() || attrs.default.is_some();
            let base_type = field_type_to_rust(&field.field_type, &TypeModifier::Required);
            let field_type = if is_optional {
                quote! { Option<#base_type> }
            } else {
                base_type
            };

            quote! {
                pub #field_name: #field_type
            }
        })
        .collect();

    // Generate UpdateInput fields (all optional)
    let update_fields: Vec<_> = model
        .fields
        .values()
        .filter(|f| is_mutable_scalar(f))
        .map(|field| {
            let field_name = snake_ident(field.name());
            let base_type = field_type_to_rust(&field.field_type, &TypeModifier::Required);

            quote! {
                pub #field_name: Option<#base_type>
            }
        })
        .collect();

    // Generate field modules
    let field_modules: Vec<_> = model
        .fields
        .values()
        .map(|field| generate_field_module(field, model))
        .collect();

    // Generate where param enum
    let where_param = generate_where_param(model);

    // Generate select, order by, and set params
    let select_param = generate_select_param(model);
    let order_by_param = generate_order_by_param(model);
    let set_param = generate_set_param(model);

    // Generate query builder
    let query_builder = generate_query_builder(model, &table_name);

    // Generate pre-compiled SQL constants
    let precompiled_sql = generate_precompiled_sql(model, &table_name);

    // Generate relation helpers
    let relation_helpers = generate_relation_helpers(model, schema);

    // Gather scalar columns + typed field tuples for the Model/FromRow impls.
    // Relation (`Vec<Model>`-typed) fields are excluded: they are not columns
    // and don't round-trip through FromRow.
    let all_columns: Vec<String> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(column_name_of)
        .collect();
    let pk_columns_owned: Vec<String> = pk_fields.clone();
    let from_row_fields: Vec<(syn::Ident, syn::Type, String)> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let rust_field = snake_ident(f.name());
            let rust_ty: syn::Type = syn::parse2(field_type_to_rust(&f.field_type, &f.modifier))
                .expect("generated Rust type should parse");
            (rust_field, rust_ty, column_name_of(f))
        })
        .collect();
    // Same tuple shape plus an is_id flag. ModelWithPk's `pk_value()`
    // reads only the id fields; `get_column_value()` routes every
    // scalar column, so we need the same rows from_row_fields has.
    let model_with_pk_fields: Vec<(syn::Ident, syn::Type, String, bool)> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| {
            let rust_field = snake_ident(f.name());
            let rust_ty: syn::Type = syn::parse2(field_type_to_rust(&f.field_type, &f.modifier))
                .expect("generated Rust type should parse");
            (rust_field, rust_ty, column_name_of(f), f.is_id())
        })
        .collect();

    // The prax_schema! path does not (yet) interpret @generated / aggregate
    // directives from the .prax schema AST — that wiring is Task 7/11.
    // Pass empty slices so the Model trait consts default to &[].
    let model_trait_impl = super::derive_model_trait::emit(
        &model_name,
        model.name(),
        &table_name,
        &pk_columns_owned,
        &all_columns,
        &[],
        &[],
    );
    // The prax_schema! path filters `FieldType::Model(_)` relation
    // fields out of `from_row_fields` above; pass an empty slice for
    // relation defaults to keep the `FromRow` shape unchanged.
    // The prax_schema! path does not yet interpret aggregate directives
    // from the .prax AST (Task 11 wires the derive path; schema path follows).
    // Pass an empty slice so aggregate fields default to the zero-state.
    let from_row_impl = super::derive_from_row::emit(&model_name, &from_row_fields, &[], &[]);
    let model_with_pk_impl = super::derive_model_with_pk::emit(&model_name, &model_with_pk_fields);
    let client_impl = super::derive_client::emit(quote! { #model_name });

    // Generate GraphQL derives if model_style is GraphQL
    let model_name_str = model.name();
    let (model_derives, create_input_derives, update_input_derives) = if model_style.is_graphql() {
        (
            quote! {
                #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, async_graphql::SimpleObject)]
                #[graphql(name = #model_name_str)]
            },
            quote! {
                #[derive(Debug, Clone, Default, Serialize, Deserialize, async_graphql::InputObject)]
                #[graphql(name = "CreateInput")]
            },
            quote! {
                #[derive(Debug, Clone, Default, Serialize, Deserialize, async_graphql::InputObject)]
                #[graphql(name = "UpdateInput")]
            },
        )
    } else {
        (
            quote! { #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)] },
            quote! { #[derive(Debug, Clone, Default, Serialize, Deserialize)] },
            quote! { #[derive(Debug, Clone, Default, Serialize, Deserialize)] },
        )
    };

    // -----------------------------------------------------------------------
    // Typed input generators (phase 2): collect field-spec lists and emit
    // the seven typed input types per model.  The same split used by the
    // derive path applies here: struct definitions go inside `pub mod`, and
    // trait impls go at crate-root scope to avoid E0446.
    // -----------------------------------------------------------------------

    let where_fields = collect_where_fields(model);
    let unique_columns = collect_unique_columns(model);
    super::inputs::where_unique_input::check_unique_column_collisions(
        &unique_columns,
        Some(model.name()),
    )?;
    let include_fields = collect_include_fields(model);
    let select_fields = collect_select_fields(model);
    let order_by_fields = collect_order_by_fields(model);
    let create_input_fields = collect_create_fields(model);
    let update_input_fields = collect_update_fields(model);
    let relation_meta_specs = collect_relation_meta_specs(model, schema)?;

    let super::inputs::where_input::WhereInputTokens {
        struct_tokens: where_input_struct,
        impl_tokens: where_input_impl,
    } = super::generate_where_input(&model_name, &module_name, &where_fields);

    let (where_unique_struct, where_unique_impl) =
        super::generate_where_unique_input(&model_name, &module_name, &unique_columns);

    let (include_struct, include_impl) =
        super::generate_include_input(&model_name, &module_name, &include_fields);

    let (select_struct, select_impl) =
        super::generate_select_input(&model_name, &module_name, &select_fields);

    let (order_by_struct, order_by_impl) =
        super::generate_order_by_input(&model_name, &module_name, &order_by_fields);

    let super::inputs::create_input::CreateInputTokens {
        struct_tokens: create_input_typed_struct,
        impl_tokens: create_input_impl,
    } = super::inputs::create_input::generate(&model_name, &module_name, &create_input_fields);

    let super::inputs::update_input::UpdateInputTokens {
        struct_tokens: update_input_typed_struct,
        impl_tokens: update_input_impl,
    } = super::inputs::update_input::generate(&model_name, &module_name, &update_input_fields);

    let (relation_meta_struct, relation_meta_impl) =
        super::inputs::relation_meta::generate(&module_name, &relation_meta_specs);

    // `<Model>Count` synthetic struct — emitted at crate-root scope,
    // outside the `pub mod <model>` block, so it is a sibling of the
    // generated model struct.  Only models with ≥1 outgoing relations
    // get this struct.
    let count_relation_names: Vec<String> = model
        .fields
        .values()
        .filter(|f| matches!(f.field_type, FieldType::Model(_)))
        .map(|f| f.name().to_string())
        .collect();
    let count_struct_outgoing: Vec<super::count_struct::OutgoingRelation<'_>> =
        count_relation_names
            .iter()
            .map(|n| super::count_struct::OutgoingRelation { field_name: n })
            .collect();
    let count_struct_tokens =
        super::count_struct::emit_count_struct(&model_name, &count_struct_outgoing);

    Ok(quote! {
        #doc
        pub mod #module_name {
            use serde::{Deserialize, Serialize};

            /// Database table name.
            pub const TABLE_NAME: &str = #table_name_str;

            /// Primary key column(s).
            pub const PRIMARY_KEY: &[&str] = &[#(#pk_field_names),*];

            #doc
            /// Represents a row from the `#table_name_str` table.
            #model_derives
            pub struct #model_name {
                #(#data_fields,)*
            }

            impl ::prax_orm::_prax_prelude::PraxModel for #model_name {
                const TABLE_NAME: &'static str = TABLE_NAME;
                const PRIMARY_KEY: &'static [&'static str] = PRIMARY_KEY;
            }

            /// Input type for creating a new record.
            #create_input_derives
            pub struct CreateInput {
                #(#create_fields,)*
            }

            /// Input type for updating a record.
            #update_input_derives
            pub struct UpdateInput {
                #(#update_fields,)*
            }

            // Field modules
            #(#field_modules)*

            // Where param enum
            #where_param

            // Select, OrderBy, and Set params
            #select_param
            #order_by_param
            #set_param

            // Model, FromRow, ModelWithPk, and Client<E> — mirrors what
            // #[derive(Model)] emits.
            #model_trait_impl
            #from_row_impl
            #model_with_pk_impl
            #client_impl

            // Query/Actions pre-compiled-SQL builders (legacy, being replaced by Client<E>).
            #query_builder

            // Pre-compiled SQL
            #precompiled_sql

            // Relation helpers
            #relation_helpers

            // Typed input struct definitions (phase 2).  Trait impls are
            // emitted below at crate-root scope to avoid E0446.
            #where_input_struct
            #where_unique_struct
            #include_struct
            #select_struct
            #order_by_struct
            #create_input_typed_struct
            #update_input_typed_struct

            // Per-relation FilterMeta marker structs.
            #relation_meta_struct
        }

        // Re-export the model type at the parent level
        pub use #module_name::#model_name;

        // Typed input trait impls at crate-root scope.  Emitted here so that
        // `type Model = #model_name` does not leak a potentially private type
        // through a public trait interface (E0446).  The schema path always
        // generates public models, so all impls are unconditionally emitted.
        #where_input_impl
        #where_unique_impl
        #include_impl
        #select_impl
        #order_by_impl
        #create_input_impl
        #update_input_impl

        // RelationFilterMeta impls — one per declared relation field.
        #relation_meta_impl

        // `<Model>Count` synthetic struct — only present when the model has
        // at least one outgoing relation.  See `count_struct` module for the
        // design rationale and deferral notes.
        #count_struct_tokens
    })
}

/// Get the primary key field names for a model.
fn get_primary_key_fields(model: &Model) -> Vec<String> {
    // Check for composite @@id
    if let Some(attr) = model.attributes.iter().find(|a| a.name() == "id")
        && let Some(prax_schema::ast::AttributeValue::FieldRefList(fields)) = attr.first_arg()
    {
        return fields.iter().map(|s| s.to_string()).collect();
    }

    // Otherwise, find @id field
    model
        .fields
        .values()
        .filter(|f| f.is_id())
        .map(|f| f.name().to_string())
        .collect()
}

/// Generate the WhereParam enum for a model.
fn generate_where_param(model: &Model) -> TokenStream {
    let variants: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let name = pascal_ident(field.name());
            let field_mod = snake_ident(field.name());
            quote! { #name(#field_mod::WhereOp) }
        })
        .collect();

    let to_sql_matches: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let name = pascal_ident(field.name());
            let field_mod = snake_ident(field.name());
            quote! { Self::#name(_op) => Some(#field_mod::COLUMN) }
        })
        .collect();

    let from_filter_arms: Vec<_> = model
        .fields
        .values()
        .map(|field| {
            let name = pascal_ident(field.name());
            quote! { WhereParam::#name(op) => op.to_filter(), }
        })
        .collect();

    quote! {
        /// Where clause parameters for filtering queries.
        #[derive(Debug, Clone)]
        pub enum WhereParam {
            #(#variants,)*
            /// Combine with AND.
            And(Vec<WhereParam>),
            /// Combine with OR.
            Or(Vec<WhereParam>),
            /// Negate the condition.
            Not(Box<WhereParam>),
        }

        impl WhereParam {
            /// Get the column name for simple conditions.
            pub fn column(&self) -> Option<&'static str> {
                match self {
                    #(#to_sql_matches,)*
                    Self::And(_) | Self::Or(_) | Self::Not(_) => None,
                }
            }

            /// Combine multiple conditions with AND.
            pub fn and(conditions: Vec<WhereParam>) -> Self {
                Self::And(conditions)
            }

            /// Combine multiple conditions with OR.
            pub fn or(conditions: Vec<WhereParam>) -> Self {
                Self::Or(conditions)
            }

            /// Negate a condition.
            pub fn not(condition: WhereParam) -> Self {
                Self::Not(Box::new(condition))
            }
        }

        impl From<WhereParam> for prax_query::filter::Filter {
            fn from(p: WhereParam) -> Self {
                match p {
                    #(#from_filter_arms)*
                    WhereParam::And(ps) => prax_query::filter::Filter::And(
                        ps.into_iter().map(Into::into).collect::<Vec<_>>().into_boxed_slice()
                    ),
                    WhereParam::Or(ps) => prax_query::filter::Filter::Or(
                        ps.into_iter().map(Into::into).collect::<Vec<_>>().into_boxed_slice()
                    ),
                    WhereParam::Not(p) => prax_query::filter::Filter::Not(Box::new((*p).into())),
                }
            }
        }
    }
}

/// Generate the query builder for a model.
fn generate_query_builder(_model: &Model, _table_name: &str) -> TokenStream {
    quote! {
        /// Query builder for the model.
        #[derive(Debug, Default)]
        pub struct Query {
            /// Select specific fields.
            pub select: Vec<SelectParam>,
            /// Where conditions.
            pub where_: Vec<WhereParam>,
            /// Order by clauses.
            pub order_by: Vec<OrderByParam>,
            /// Skip N records.
            pub skip: Option<usize>,
            /// Take N records.
            pub take: Option<usize>,
            /// Distinct fields.
            pub distinct: Vec<SelectParam>,
        }

        impl Query {
            /// Create a new query builder.
            pub fn new() -> Self {
                Self::default()
            }

            /// Add a where condition.
            pub fn r#where(mut self, param: WhereParam) -> Self {
                self.where_.push(param);
                self
            }

            /// Add multiple where conditions with AND.
            pub fn r#whereand(mut self, params: Vec<WhereParam>) -> Self {
                self.where_.push(WhereParam::And(params));
                self
            }

            /// Add multiple where conditions with OR.
            pub fn r#whereor(mut self, params: Vec<WhereParam>) -> Self {
                self.where_.push(WhereParam::Or(params));
                self
            }

            /// Order by a field.
            pub fn order_by(mut self, param: OrderByParam) -> Self {
                self.order_by.push(param);
                self
            }

            /// Skip N records.
            pub fn skip(mut self, n: usize) -> Self {
                self.skip = Some(n);
                self
            }

            /// Take N records.
            pub fn take(mut self, n: usize) -> Self {
                self.take = Some(n);
                self
            }

            /// Select specific fields.
            pub fn select(mut self, fields: Vec<SelectParam>) -> Self {
                self.select = fields;
                self
            }

            /// Get distinct values.
            pub fn distinct(mut self, fields: Vec<SelectParam>) -> Self {
                self.distinct = fields;
                self
            }

            /// Generate the SELECT SQL query.
            pub fn to_select_sql(&self) -> String {
                let columns = if self.select.is_empty() {
                    "*".to_string()
                } else {
                    self.select.iter().map(|s| s.column()).collect::<Vec<_>>().join(", ")
                };

                let distinct = if self.distinct.is_empty() {
                    String::new()
                } else {
                    format!(
                        "DISTINCT ON ({}) ",
                        self.distinct.iter().map(|d| d.column()).collect::<Vec<_>>().join(", ")
                    )
                };

                let mut sql = format!("SELECT {}{} FROM {}", distinct, columns, TABLE_NAME);

                // WHERE clause would be added here with parameter binding

                if !self.order_by.is_empty() {
                    sql.push_str(" ORDER BY ");
                    sql.push_str(
                        &self.order_by.iter().map(|o| o.to_sql()).collect::<Vec<_>>().join(", ")
                    );
                }

                if let Some(take) = self.take {
                    sql.push_str(&format!(" LIMIT {}", take));
                }

                if let Some(skip) = self.skip {
                    sql.push_str(&format!(" OFFSET {}", skip));
                }

                sql
            }
        }

        /// Actions available on the model.
        pub struct Actions;

        impl Actions {
            /// Find multiple records.
            pub fn find_many() -> Query {
                Query::new()
            }

            /// Find a unique record (by primary key or unique constraint).
            pub fn find_unique() -> Query {
                Query::new().take(1)
            }

            /// Find the first matching record.
            pub fn find_first() -> Query {
                Query::new().take(1)
            }

            /// Create input for a new record.
            pub fn create() -> CreateInput {
                CreateInput::default()
            }

            /// Update input for a record.
            pub fn update() -> UpdateInput {
                UpdateInput::default()
            }
        }
    }
}

/// Generate pre-compiled SQL constants for common queries.
///
/// This generates `const` SQL strings that can be used directly without
/// any runtime string construction, achieving ~5ns lookup time.
fn generate_precompiled_sql(model: &Model, table_name: &str) -> TokenStream {
    let pk_fields = get_primary_key_fields(model);

    // Generate column list for SELECT (all scalar fields)
    let columns: Vec<_> = model
        .fields
        .values()
        .filter(|f| !matches!(f.field_type, FieldType::Model(_)))
        .map(|f| f.name().to_string())
        .collect();
    let column_list = columns.join(", ");

    // Generate INSERT columns (exclude auto-generated)
    let insert_columns: Vec<_> = model
        .fields
        .values()
        .filter(|f| is_mutable_scalar(f))
        .map(|f| f.name().to_string())
        .collect();

    let insert_column_list = insert_columns.join(", ");
    let insert_placeholders: Vec<_> = (1..=insert_columns.len())
        .map(|i| format!("${}", i))
        .collect();
    let insert_placeholder_list = insert_placeholders.join(", ");

    // Generate UPDATE SET clause
    let update_columns: Vec<_> = model
        .fields
        .values()
        .filter(|f| is_mutable_scalar(f))
        .enumerate()
        .map(|(i, f)| format!("{} = ${}", f.name(), i + 1))
        .collect();
    let update_set_clause = update_columns.join(", ");
    let update_pk_placeholder = format!("${}", update_columns.len() + 1);

    // Primary key WHERE clause
    let pk_where = if pk_fields.len() == 1 {
        format!("{} = $1", pk_fields[0])
    } else {
        pk_fields
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{} = ${}", f, i + 1))
            .collect::<Vec<_>>()
            .join(" AND ")
    };

    // Generate SQL strings
    let find_all_sql = format!("SELECT {} FROM {}", column_list, table_name);
    let find_by_id_sql = format!(
        "SELECT {} FROM {} WHERE {}",
        column_list, table_name, pk_where
    );
    let count_sql = format!("SELECT COUNT(*) FROM {}", table_name);
    let insert_sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) RETURNING {}",
        table_name, insert_column_list, insert_placeholder_list, column_list
    );
    let insert_no_return_sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table_name, insert_column_list, insert_placeholder_list
    );
    let update_by_id_sql = format!(
        "UPDATE {} SET {} WHERE {} RETURNING {}",
        table_name,
        update_set_clause,
        pk_where.replace("$1", &update_pk_placeholder),
        column_list
    );
    let delete_by_id_sql = format!("DELETE FROM {} WHERE {}", table_name, pk_where);
    let exists_by_id_sql = format!(
        "SELECT EXISTS(SELECT 1 FROM {} WHERE {})",
        table_name, pk_where
    );

    // Generate cache key constants
    let cache_key_prefix = table_name.to_lowercase();
    let cache_key_find_all = format!("{}:find_all", cache_key_prefix);
    let cache_key_find_by_id = format!("{}:find_by_id", cache_key_prefix);
    let cache_key_count = format!("{}:count", cache_key_prefix);
    let cache_key_insert = format!("{}:insert", cache_key_prefix);
    let cache_key_update = format!("{}:update_by_id", cache_key_prefix);
    let cache_key_delete = format!("{}:delete_by_id", cache_key_prefix);

    // Parameter counts for const functions
    let insert_param_count = insert_columns.len();
    let update_param_count = update_columns.len() + 1; // +1 for the primary key

    quote! {
        /// Pre-compiled SQL constants for zero-allocation query building.
        ///
        /// These constants are generated at compile time and provide ~5ns access
        /// compared to runtime string construction.
        ///
        /// # Example
        ///
        /// ```rust,ignore
        /// // Use the const SQL directly
        /// let sql = user::sql::FIND_BY_ID;
        ///
        /// // Or use the typed query functions
        /// let (sql, param_count) = user::sql::find_by_id();
        /// ```
        pub mod sql {
            /// SELECT all columns from the table.
            pub const FIND_ALL: &str = #find_all_sql;

            /// SELECT by primary key.
            pub const FIND_BY_ID: &str = #find_by_id_sql;

            /// COUNT all records.
            pub const COUNT: &str = #count_sql;

            /// INSERT a new record (with RETURNING).
            pub const INSERT: &str = #insert_sql;

            /// INSERT a new record (without RETURNING).
            pub const INSERT_NO_RETURN: &str = #insert_no_return_sql;

            /// UPDATE by primary key (with RETURNING).
            pub const UPDATE_BY_ID: &str = #update_by_id_sql;

            /// DELETE by primary key.
            pub const DELETE_BY_ID: &str = #delete_by_id_sql;

            /// Check if record exists by primary key.
            pub const EXISTS_BY_ID: &str = #exists_by_id_sql;

            /// Cache keys for use with SqlTemplateCache.
            pub mod cache_keys {
                pub const FIND_ALL: &str = #cache_key_find_all;
                pub const FIND_BY_ID: &str = #cache_key_find_by_id;
                pub const COUNT: &str = #cache_key_count;
                pub const INSERT: &str = #cache_key_insert;
                pub const UPDATE_BY_ID: &str = #cache_key_update;
                pub const DELETE_BY_ID: &str = #cache_key_delete;
            }

            /// Get FIND_ALL SQL with parameter count.
            #[inline(always)]
            pub const fn find_all() -> (&'static str, usize) {
                (FIND_ALL, 0)
            }

            /// Get FIND_BY_ID SQL with parameter count.
            #[inline(always)]
            pub const fn find_by_id() -> (&'static str, usize) {
                (FIND_BY_ID, 1)
            }

            /// Get COUNT SQL with parameter count.
            #[inline(always)]
            pub const fn count() -> (&'static str, usize) {
                (COUNT, 0)
            }

            /// Get INSERT SQL with parameter count.
            #[inline(always)]
            pub const fn insert() -> (&'static str, usize) {
                (INSERT, #insert_param_count)
            }

            /// Get UPDATE_BY_ID SQL with parameter count.
            #[inline(always)]
            pub const fn update_by_id() -> (&'static str, usize) {
                (UPDATE_BY_ID, #update_param_count)
            }

            /// Get DELETE_BY_ID SQL with parameter count.
            #[inline(always)]
            pub const fn delete_by_id() -> (&'static str, usize) {
                (DELETE_BY_ID, 1)
            }

            /// Register all SQL templates in the global cache.
            ///
            /// Call this at application startup for fastest cache lookups.
            pub fn register_all_templates() {
                use prax_query::cache::register_global_template;
                register_global_template(cache_keys::FIND_ALL, FIND_ALL);
                register_global_template(cache_keys::FIND_BY_ID, FIND_BY_ID);
                register_global_template(cache_keys::COUNT, COUNT);
                register_global_template(cache_keys::INSERT, INSERT);
                register_global_template(cache_keys::UPDATE_BY_ID, UPDATE_BY_ID);
                register_global_template(cache_keys::DELETE_BY_ID, DELETE_BY_ID);
            }
        }
    }
}

/// Generate relation helper types.
fn generate_relation_helpers(model: &Model, _schema: &Schema) -> TokenStream {
    let relation_fields: Vec<_> = model
        .fields
        .values()
        .filter(|f| matches!(f.field_type, FieldType::Model(_)))
        .collect();

    if relation_fields.is_empty() {
        return TokenStream::new();
    }

    let include_variants: Vec<_> = relation_fields
        .iter()
        .map(|f| {
            let name = pascal_ident(f.name());
            let is_list = f.modifier.is_list();
            if is_list {
                quote! { #name(Option<Box<super::super::#name::Query>>) }
            } else {
                quote! { #name }
            }
        })
        .collect();

    quote! {
        /// Include related records in the query.
        #[derive(Debug, Clone, Default)]
        pub enum IncludeParam {
            #[default]
            None,
            #(#include_variants,)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prax_schema::ast::{Attribute, Field, Ident, ScalarType, Span};

    fn make_span() -> Span {
        Span::new(0, 0)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_simple_schema() -> Schema {
        let mut schema = Schema::new();
        let mut user = Model::new(make_ident("User"), make_span());
        user.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![
                Attribute::simple(make_ident("id"), make_span()),
                Attribute::simple(make_ident("auto"), make_span()),
            ],
            make_span(),
        ));
        user.add_field(Field::new(
            make_ident("email"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
            vec![Attribute::simple(make_ident("unique"), make_span())],
            make_span(),
        ));
        user.add_field(Field::new(
            make_ident("name"),
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
            vec![],
            make_span(),
        ));
        schema.add_model(user);
        schema
    }

    #[test]
    fn test_generate_model_module() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module(model, &schema);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();
        assert!(code.contains("pub mod user"));
        assert!(code.contains("pub struct User"));
        assert!(code.contains("pub struct CreateInput"));
        assert!(code.contains("pub struct UpdateInput"));
        assert!(code.contains("pub enum WhereParam"));
        assert!(code.contains("pub struct Query"));
        // Verify pre-compiled SQL module
        assert!(code.contains("pub mod sql"));
        assert!(code.contains("FIND_ALL"));
        assert!(code.contains("FIND_BY_ID"));
        assert!(code.contains("INSERT"));
    }

    #[test]
    fn test_get_primary_key_fields() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let pk = get_primary_key_fields(model);
        assert_eq!(pk, vec!["id"]);
    }

    #[test]
    fn test_generate_model_module_graphql_style() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module_with_style(model, &schema, ModelStyle::GraphQL);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();

        // Verify GraphQL derives are present
        assert!(
            code.contains("async_graphql :: SimpleObject"),
            "Should have SimpleObject derive"
        );
        assert!(
            code.contains("async_graphql :: InputObject"),
            "Should have InputObject derive"
        );

        // Verify graphql name attribute
        assert!(code.contains("graphql"), "Should have graphql attributes");
    }

    #[test]
    fn test_generate_model_module_standard_style() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module_with_style(model, &schema, ModelStyle::Standard);
        assert!(result.is_ok());

        let code = result.unwrap().to_string();

        // Verify GraphQL derives are NOT present
        assert!(
            !code.contains("async_graphql"),
            "Should NOT have async_graphql derives"
        );
        assert!(
            !code.contains("SimpleObject"),
            "Should NOT have SimpleObject derive"
        );
    }

    /// Verify that the schema path emits all seven typed input types per model.
    #[test]
    fn schema_path_emits_typed_inputs_for_user() {
        let schema = make_simple_schema();
        let model = schema.get_model("User").unwrap();

        let result = generate_model_module(model, &schema);
        assert!(result.is_ok(), "generate_model_module returned Err");

        let code = result.unwrap().to_string();

        assert!(
            code.contains("UserWhereInput"),
            "expected UserWhereInput in:\n{code}"
        );
        assert!(
            code.contains("UserWhereUniqueInput"),
            "expected UserWhereUniqueInput in:\n{code}"
        );
        assert!(
            code.contains("UserInclude"),
            "expected UserInclude in:\n{code}"
        );
        assert!(
            code.contains("UserSelect"),
            "expected UserSelect in:\n{code}"
        );
        assert!(
            code.contains("UserOrderBy"),
            "expected UserOrderBy in:\n{code}"
        );
        assert!(
            code.contains("UserCreateInput"),
            "expected UserCreateInput in:\n{code}"
        );
        assert!(
            code.contains("UserUpdateInput"),
            "expected UserUpdateInput in:\n{code}"
        );
    }

    /// Verify that relation target table names are resolved from the schema
    /// (not just snake-cased ident) in `RelationFilterMeta` specs.
    #[test]
    fn schema_path_relation_meta_uses_target_table_name() {
        use prax_schema::ast::{Attribute, AttributeArg, AttributeValue};

        let mut schema = prax_schema::ast::Schema::new();

        // Post model with @relation on `author` field.
        let mut post = Model::new(make_ident("Post"), make_span());
        post.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![
                Attribute::simple(make_ident("id"), make_span()),
                Attribute::simple(make_ident("auto"), make_span()),
            ],
            make_span(),
        ));
        // author_id FK scalar
        post.add_field(Field::new(
            make_ident("author_id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        // author relation field pointing at User
        let relation_attr = Attribute::new(
            make_ident("relation"),
            vec![
                AttributeArg::named(
                    make_ident("fields"),
                    AttributeValue::FieldRefList(vec!["author_id".into()]),
                    make_span(),
                ),
                AttributeArg::named(
                    make_ident("references"),
                    AttributeValue::FieldRefList(vec!["id".into()]),
                    make_span(),
                ),
            ],
            make_span(),
        );
        post.add_field(Field::new(
            make_ident("author"),
            FieldType::Model("User".into()),
            TypeModifier::Required,
            vec![relation_attr],
            make_span(),
        ));
        schema.add_model(post);

        // User model with @@map("app_users") so table_name != snake-cased ident.
        let mut user = Model::new(make_ident("User"), make_span());
        user.attributes.push(Attribute::new(
            make_ident("map"),
            vec![AttributeArg::positional(
                AttributeValue::String("app_users".into()),
                make_span(),
            )],
            make_span(),
        ));
        user.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![
                Attribute::simple(make_ident("id"), make_span()),
                Attribute::simple(make_ident("auto"), make_span()),
            ],
            make_span(),
        ));
        schema.add_model(user);

        let post_model = schema.get_model("Post").unwrap();
        let result = generate_model_module(post_model, &schema);
        assert!(
            result.is_ok(),
            "generate_model_module for Post returned Err"
        );

        let code = result.unwrap().to_string();

        // The RelationFilterMeta impl should reference "app_users" (the
        // resolved table_name), not "user" (the snake-cased ident fallback).
        assert!(
            code.contains("app_users"),
            "expected child_table 'app_users' (resolved from @@map), got:\n{code}"
        );
        assert!(
            !code.contains(r#""user""#),
            "should NOT fall back to snake-cased ident 'user', got:\n{code}"
        );
    }

    /// `collect_relation_meta_specs` must surface a span'd `syn::Error`
    /// (not a panic or silent default) when a relation target model is
    /// missing from the schema.
    #[test]
    fn schema_path_unknown_relation_target_returns_error() {
        use prax_schema::ast::{Attribute, AttributeArg, AttributeValue};

        let mut schema = prax_schema::ast::Schema::new();
        let mut post = Model::new(make_ident("Post"), make_span());
        post.add_field(Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![
                Attribute::simple(make_ident("id"), make_span()),
                Attribute::simple(make_ident("auto"), make_span()),
            ],
            make_span(),
        ));
        post.add_field(Field::new(
            make_ident("author_id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        ));
        // Relation pointing at a model that DOES NOT exist in the schema.
        let relation_attr = Attribute::new(
            make_ident("relation"),
            vec![AttributeArg::named(
                make_ident("fields"),
                AttributeValue::FieldRefList(vec!["author_id".into()]),
                make_span(),
            )],
            make_span(),
        );
        post.add_field(Field::new(
            make_ident("author"),
            FieldType::Model("NonExistentUser".into()),
            TypeModifier::Required,
            vec![relation_attr],
            make_span(),
        ));
        schema.add_model(post);

        let post_model = schema.get_model("Post").unwrap();
        let result = generate_model_module(post_model, &schema);
        assert!(
            result.is_err(),
            "expected syn::Error for unknown relation target, got Ok"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("NonExistentUser"),
            "error message should name the missing target, got: {msg}"
        );
    }
}
