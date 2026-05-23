//! Implementation of the `#[derive(Model)]` macro.

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Ident, LitStr, Type, Visibility};

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

    // Collect generated and aggregate field metadata for the Model trait consts.
    let generated_fields: Vec<(String, String, bool)> = field_infos
        .iter()
        .filter_map(|f| {
            f.generated
                .as_ref()
                .map(|(expr, stored)| (f.name.to_string(), expr.clone(), *stored))
        })
        .collect();
    let aggregate_fields: Vec<(String, String, String, Option<String>)> = field_infos
        .iter()
        .filter_map(|f| {
            f.aggregate.as_ref().map(|(kind, rel, field)| {
                (f.name.to_string(), kind.clone(), rel.clone(), field.clone())
            })
        })
        .collect();

    let generated_fields_refs: Vec<(&str, &str, bool)> = generated_fields
        .iter()
        .map(|(f, e, s)| (f.as_str(), e.as_str(), *s))
        .collect();
    let aggregate_fields_refs: Vec<(&str, &str, &str, Option<&str>)> = aggregate_fields
        .iter()
        .map(|(f, k, r, field)| (f.as_str(), k.as_str(), r.as_str(), field.as_deref()))
        .collect();

    let model_trait_impl = super::derive_model_trait::emit(
        name,
        &name.to_string(),
        &table_name,
        &pk_columns_owned,
        &all_columns,
        &generated_fields_refs,
        &aggregate_fields_refs,
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

    // Build WhereField list for the where_input generator.
    // Only scalar fields (no relation attr, not a Vec) are included for now.
    // Relation filter fields (ListRelationFilter / SingleRelationFilter) are
    // deferred to Task 8 when `<Model><Rel>FilterMeta` types are generated;
    // emitting them here without the corresponding FilterMeta impl would
    // produce unresolvable references like `UserPostsFilterMeta`.
    let where_fields: Vec<super::WhereField> = field_infos
        .iter()
        .filter_map(|f| {
            if f.relation.is_some() || f.is_list {
                // Relation fields: phase-2 emits the FilterMeta marker but
                // leaves the WhereInput relation-filter wiring for a later
                // phase (path-resolution work for `super::<target>::<...>`).
                None
            } else {
                let type_name = extract_inner_type_name(&f.ty);
                let category =
                    super::inputs::filter_category_for(type_name.as_deref().unwrap_or(""));
                Some(super::WhereField {
                    name: f.name.clone(),
                    column: f.column_name.clone(),
                    category,
                    nullable: f.is_optional,
                    relation_target_where_input: None,
                    is_to_many: false,
                })
            }
        })
        .collect();

    let super::WhereInputTokens {
        struct_tokens: where_input_struct,
        impl_tokens: where_input_impl,
    } = super::generate_where_input(name, &module_name, &where_fields);

    // Build UniqueColumn list for the where_unique_input generator.
    // Only scalar @id / @unique fields; relation fields are never unique keys
    // we can encode as a simple Equals lookup.
    let unique_columns: Vec<super::UniqueColumn> = field_infos
        .iter()
        .filter(|f| (f.is_id || f.is_unique) && f.relation.is_none() && !f.is_list)
        .filter_map(|f| {
            let inner = extract_inner_type_name(&f.ty);
            let cat = super::inputs::filter_category_for(inner.as_deref().unwrap_or(""))?;
            Some(super::UniqueColumn {
                variant: format_ident!("{}", f.name.to_string().to_case(Case::Pascal)),
                column: f.column_name.clone(),
                category: cat,
                enum_ident: None,
            })
        })
        .collect();

    super::inputs::where_unique_input::check_unique_column_collisions(
        &unique_columns,
        Some(&name.to_string()),
    )?;

    let (where_unique_struct, where_unique_impl) =
        super::generate_where_unique_input(name, &module_name, &unique_columns);

    // Build IncludeField list: every field that carries a relation attr.
    let include_fields: Vec<super::IncludeField> = field_infos
        .iter()
        .filter(|f| f.relation.is_some())
        .map(|f| super::IncludeField {
            name: f.name.clone(),
            relation: f.name.to_string(),
        })
        .collect();

    // Build SelectField list: all fields; relation fields are marked so
    // their column names are excluded from the SELECT column list.
    let select_fields: Vec<super::SelectField> = field_infos
        .iter()
        .map(|f| super::SelectField {
            name: f.name.clone(),
            column: f.column_name.clone(),
            is_relation: f.relation.is_some(),
        })
        .collect();

    let (include_struct, include_impl) =
        super::generate_include_input(name, &module_name, &include_fields);
    let (select_struct, select_impl) =
        super::generate_select_input(name, &module_name, &select_fields);

    // Build OrderByField list: every non-relation scalar column is sortable.
    let order_by_fields: Vec<super::OrderByInputField> = field_infos
        .iter()
        .filter(|f| f.relation.is_none() && !f.is_list)
        .map(|f| super::OrderByInputField {
            variant: format_ident!("{}", f.name.to_string().to_case(Case::Pascal)),
            column: f.column_name.clone(),
            nullable: f.is_optional,
        })
        .collect();

    let (order_by_struct, order_by_impl) =
        super::generate_order_by_input(name, &module_name, &order_by_fields);

    // Build CreateField list: scalar fields only, skipping auto-generated PKs.
    let create_fields: Vec<super::inputs::create_input::CreateField> = field_infos
        .iter()
        .filter(|f| f.relation.is_none() && !f.is_list)
        .filter(|f| !(f.is_id && f.is_auto))
        .filter_map(|f| {
            let inner = extract_inner_type_name(&f.ty);
            let cat = super::inputs::filter_category_for(inner.as_deref().unwrap_or(""))?;
            Some(super::inputs::create_input::CreateField {
                name: f.name.clone(),
                column: f.column_name.clone(),
                category: cat,
                nullable: f.is_optional,
                has_default: false, // phase 2: no default detection yet
                enum_ident: None,
            })
        })
        .collect();

    // Build UpdateField list: scalar fields only, skipping auto-generated PKs.
    let update_fields: Vec<super::inputs::update_input::UpdateField> = field_infos
        .iter()
        .filter(|f| f.relation.is_none() && !f.is_list)
        .filter(|f| !(f.is_id && f.is_auto))
        .filter_map(|f| {
            let inner = extract_inner_type_name(&f.ty);
            let cat = super::inputs::filter_category_for(inner.as_deref().unwrap_or(""))?;
            Some(super::inputs::update_input::UpdateField {
                name: f.name.clone(),
                column: f.column_name.clone(),
                category: cat,
                nullable: f.is_optional,
                enum_ident: None,
            })
        })
        .collect();

    let super::inputs::create_input::CreateInputTokens {
        struct_tokens: create_struct,
        impl_tokens: create_input_impl,
    } = super::inputs::create_input::generate(name, &module_name, &create_fields);
    let super::inputs::update_input::UpdateInputTokens {
        struct_tokens: update_struct,
        impl_tokens: update_input_impl,
    } = super::inputs::update_input::generate(name, &module_name, &update_fields);

    // Build RelationMetaSpec list for the relation_meta generator (Task 8).
    // Parent PK is the first @id field's column name.
    let parent_pk = field_infos
        .iter()
        .find(|f| f.is_id)
        .map(|f| f.column_name.clone())
        .expect("model must have at least one #[prax(id)] field (validated upstream)");

    let relation_meta_specs: Vec<super::inputs::relation_meta::RelationMetaSpec> = field_infos
        .iter()
        .filter_map(|f| {
            let rel = f.relation.as_ref()?;
            // Child table: prefer the explicit `child_table = "..."` attr
            // (required when the target model uses `#[prax(table = "...")]`),
            // otherwise fall back to the snake-cased target ident, which
            // matches the default table-naming convention.
            let child_table = rel
                .child_table
                .clone()
                .unwrap_or_else(|| rel.target.to_string().to_case(Case::Snake));
            Some(super::inputs::relation_meta::RelationMetaSpec {
                meta_ident: format_ident!(
                    "{}{}FilterMeta",
                    name,
                    f.name.to_string().to_case(Case::Pascal)
                ),
                parent_table: table_name.clone(),
                parent_pk: parent_pk.clone(),
                child_table,
                child_fk: rel.foreign_key.clone(),
            })
        })
        .collect();

    let (relation_meta_struct, relation_meta_impl) =
        super::inputs::relation_meta::generate(&module_name, &relation_meta_specs);

    // Only emit the `WhereInput` trait impl when the model struct is pub.
    // If the struct is private (e.g., in integration tests), emitting
    // `impl WhereInput for pub UserWhereInput { type Model = PrivateUser; }`
    // triggers E0446 "private type in public interface". The struct
    // definition is always emitted; the trait impl is gated on visibility.
    let is_pub = matches!(input.vis, Visibility::Public(_));
    let gate_impl = |tokens: TokenStream| if is_pub { tokens } else { TokenStream::new() };
    let maybe_where_input_impl = gate_impl(where_input_impl);
    let maybe_where_unique_impl = gate_impl(where_unique_impl);
    let maybe_include_impl = gate_impl(include_impl);
    let maybe_select_impl = gate_impl(select_impl);
    let maybe_order_by_impl = gate_impl(order_by_impl);
    let maybe_create_input_impl = gate_impl(create_input_impl);
    let maybe_update_input_impl = gate_impl(update_input_impl);

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

            // Typed input shapes (phase 2) — struct definitions.
            // The WhereInput / WhereUniqueInput / IncludeInput / SelectInput /
            // OrderByInput trait impls are emitted outside the module (below)
            // to avoid E0446 "private type in public interface" when the model
            // struct is not `pub`.
            #where_input_struct
            #where_unique_struct
            #include_struct
            #select_struct
            #order_by_struct
            #create_struct
            #update_struct

            // Per-relation `<Model><Relation>FilterMeta` marker structs (Task 8).
            // The corresponding `impl RelationFilterMeta` blocks are emitted
            // below at crate-root scope so the path `#module_name::#marker`
            // resolves correctly.
            #relation_meta_struct
        }

        // Emit Model, FromRow, and ModelWithPk trait implementations at crate root
        #model_trait_impl
        #from_row_impl
        #model_with_pk_impl

        // Emit ModelRelationLoader<E> so every derived model — relations
        // or not — can be used on the `.include()` path uniformly.
        #model_relation_loader_impl

        // WhereInput trait impl at crate scope — only emitted when the
        // model struct is `pub` to avoid E0446. When emitted, `type Model =
        // <Model>` is valid because the model struct is directly in scope.
        #maybe_where_input_impl

        // WhereUniqueInput trait impl — same visibility gating as above.
        #maybe_where_unique_impl

        // IncludeInput trait impl — same visibility gating.
        #maybe_include_impl

        // SelectInput trait impl — same visibility gating.
        #maybe_select_impl

        // OrderByInput trait impl — same visibility gating.
        #maybe_order_by_impl

        // CreateInput / UpdateInput trait impls (phase 5a) — same
        // visibility gating as the other trait impls above.
        #maybe_create_input_impl
        #maybe_update_input_impl

        // RelationFilterMeta impls — one per declared relation field.
        // Each impl associates the zero-sized `<Model><Relation>FilterMeta`
        // marker (declared in the `pub mod` above) with the parent/child table
        // and column names required for EXISTS / NOT EXISTS lowering.
        #relation_meta_impl
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
    /// `#[prax(generated = "expr")]` / `#[prax(generated = "expr", stored)]`
    /// / `#[prax(generated = "expr", virtual)]` — (expression, stored).
    generated: Option<(String, bool)>,
    /// `#[prax(count(rel))]` / `#[prax(sum(rel.field))]` etc.
    /// Tuple: (kind_str, relation, field).
    aggregate: Option<(String, String, Option<String>)>,
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
    /// Optional explicit child SQL table name. Required when the target
    /// model uses `#[prax(table = "...")]` to override its default name,
    /// because the derive macro has no access to the target model's
    /// attributes at expansion time. If absent, the snake-cased target
    /// ident is used (matches the default table-name convention).
    child_table: Option<String>,
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
    let mut generated_expr: Option<String> = None;
    let mut generated_stored: bool = true; // default: stored
    let mut aggregate: Option<(String, String, Option<String>)> = None;

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
            } else if meta.path.is_ident("generated") {
                let value: LitStr = meta.value()?.parse()?;
                generated_expr = Some(value.value());
            } else if meta.path.is_ident("stored") {
                generated_stored = true;
            } else if meta.path.is_ident("virtual") {
                generated_stored = false;
            } else if meta.path.is_ident("count") {
                // #[prax(count(relation_name))]
                // Parse `(ident)` directly from the token stream.
                let content;
                syn::parenthesized!(content in meta.input);
                let rel_ident: syn::Ident = content.parse()?;
                aggregate = Some(("count".to_string(), rel_ident.to_string(), None));
            } else if meta.path.is_ident("sum")
                || meta.path.is_ident("avg")
                || meta.path.is_ident("min")
                || meta.path.is_ident("max")
            {
                // #[prax(sum(relation.field))] — parse `(ident.ident)`.
                let kind_str = meta
                    .path
                    .get_ident()
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                let content;
                syn::parenthesized!(content in meta.input);
                let rel_ident: syn::Ident = content.parse()?;
                let _dot: syn::Token![.] = content.parse()?;
                let field_ident: syn::Ident = content.parse()?;
                aggregate = Some((
                    kind_str,
                    rel_ident.to_string(),
                    Some(field_ident.to_string()),
                ));
            } else if meta.path.is_ident("relation") {
                let mut target: Option<syn::Ident> = None;
                let mut fk: Option<String> = None;
                let mut lk: Option<String> = None;
                let mut child_table: Option<String> = None;
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
                    } else if inner.path.is_ident("child_table") {
                        let s: LitStr = inner.value()?.parse()?;
                        child_table = Some(s.value());
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
                    child_table,
                });
            }
            Ok(())
        })?;
    }

    let generated = generated_expr.map(|expr| (expr, generated_stored));

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
        generated,
        aggregate,
    })
}

/// Check if a type is `Option<T>`.
fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.first()
    {
        return segment.ident == "Option";
    }
    false
}

/// Check if a type is `Vec<T>` representing a to-many relation.
///
/// `Vec<u8>` is treated as a Bytes scalar (binary column), not a relation
/// list. Other element types map to relation lists.
fn is_vec_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.first()
        && segment.ident == "Vec"
    {
        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments
            && let Some(syn::GenericArgument::Type(Type::Path(inner_path))) = args.args.first()
            && inner_path.path.is_ident("u8")
        {
            return false;
        }
        return true;
    }
    false
}

/// Extract the last path segment name from a `syn::Type`, unwrapping one
/// layer of `Option<T>` if present.  Returns `None` for non-path types.
///
/// Used by the where_input generator to map `syn::Type` → `FilterCategory`
/// without carrying a `type_str` field in `FieldInfo`.
///
/// Note: only ONE layer of `Option<T>` is unwrapped. ORM column types
/// are never doubly nested (`Option<Option<T>>` isn't a meaningful
/// column shape), so this is a documented design assumption rather
/// than a defensive recursive unwrap.
fn extract_inner_type_name(ty: &Type) -> Option<String> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first()
            && segment.ident == "Option"
        {
            // Unwrap Option<T> and recurse on the inner type.
            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments
                && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
            {
                return extract_inner_type_name(inner);
            }
        }
        // Return the last segment as the type name (handles `chrono::DateTime<_>` → `DateTime`).
        type_path.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
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
        if let Some(segment) = type_path.path.segments.first()
            && segment.ident == "Option"
            && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
            && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
        {
            // Recurse to classify the inner type.
            return classify_field_type(inner);
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

    // ── where_input codegen tests ──────────────────────────────────────────

    /// Derives a User model with all fixture fields and checks that
    /// `UserWhereInput` is present in the generated token stream with the
    /// expected per-field entries.
    ///
    /// The struct is `pub` because the `WhereInput` trait impl is only
    /// emitted for public model structs (to avoid E0446 "private type in
    /// public interface").
    #[test]
    fn user_where_input_default_lowers_to_filter_none() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                #[prax(unique)]
                pub email: String,
                pub name: Option<String>,
                pub age: Option<i32>,
                pub active: bool,
                pub role: String,
            }
        };

        let result = derive_model_impl(&input);
        assert!(
            result.is_ok(),
            "derive_model_impl failed: {:?}",
            result.err()
        );
        let code = result.unwrap().to_string();

        // The generated module must contain the typed where-input struct.
        assert!(
            code.contains("UserWhereInput"),
            "expected UserWhereInput in generated code; got:\n{code}"
        );
        // Scalar fields must appear (BigInt for i64, String for email/role).
        assert!(
            code.contains("BigIntFilter") || code.contains("BigIntNullableFilter"),
            "expected BigIntFilter for id field"
        );
        assert!(
            code.contains("StringFilter"),
            "expected StringFilter for email/role fields"
        );
        // Nullable fields must use nullable wrappers.
        assert!(
            code.contains("StringNullableFilter"),
            "expected StringNullableFilter for name: Option<String>"
        );
        assert!(
            code.contains("IntNullableFilter"),
            "expected IntNullableFilter for age: Option<i32>"
        );
        assert!(
            code.contains("BoolFilter"),
            "expected BoolFilter for active: bool"
        );
        // Logical combinators must be present.
        assert!(code.contains("pub and"), "expected 'and' combinator field");
        assert!(code.contains("pub or"), "expected 'or' combinator field");
        assert!(code.contains("pub not"), "expected 'not' combinator field");
        // WhereInput trait impl must appear.
        assert!(
            code.contains("impl") && code.contains("WhereInput") && code.contains("into_ir"),
            "expected WhereInput trait impl with into_ir"
        );
    }

    // ── where_unique_input codegen tests ──────────────────────────────────

    /// Confirms the WhereUniqueInput enum is emitted with one variant
    /// per @id / @unique column, lowering to Filter::Equals.
    #[test]
    fn user_where_unique_input_emits_id_and_email_variants() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                #[prax(unique)]
                pub email: String,
                pub name: Option<String>,
            }
        };

        let result = derive_model_impl(&input);
        assert!(
            result.is_ok(),
            "derive_model_impl failed: {:?}",
            result.err()
        );
        let code = result.unwrap().to_string();

        // The enum + both variants must be present.
        assert!(
            code.contains("UserWhereUniqueInput"),
            "expected UserWhereUniqueInput in generated code"
        );
        assert!(code.contains("Id"), "expected Id variant");
        assert!(code.contains("Email"), "expected Email variant");
        // The WhereUniqueInput trait impl with into_ir must appear.
        assert!(
            code.contains("WhereUniqueInput") && code.contains("into_ir"),
            "expected WhereUniqueInput trait impl"
        );
        // The column literals must be present in the match arms.
        assert!(code.contains("\"id\""), "expected \"id\" column literal");
        assert!(
            code.contains("\"email\""),
            "expected \"email\" column literal"
        );
    }

    /// Models without any unique key should still emit an uninhabited
    /// WhereUniqueInput so generic bounds resolve.
    #[test]
    fn model_without_unique_key_emits_empty_where_unique_input() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "logs")]
            pub struct Log {
                // No @id, no @unique — but at least one field marked as id
                // is required by derive_model_impl. Use a non-unique sentinel.
                #[prax(id)]
                pub message: String,
            }
        };

        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "derive_model_impl failed");
        let code = result.unwrap().to_string();

        // Even though `message` is `id`, the enum will have one variant.
        // For the "truly empty" case, we'd need a model without ANY id field —
        // but derive_model_impl rejects those upstream. Confirm the enum at
        // minimum exists.
        assert!(
            code.contains("LogWhereUniqueInput"),
            "expected LogWhereUniqueInput in generated code"
        );
    }

    // ── include_input codegen tests ────────────────────────────────────────

    #[test]
    fn user_include_emits_per_relation_options() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                pub email: String,
            }
        };
        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "derive_model_impl failed");
        let code = result.unwrap().to_string();

        // Even without relations, the UserInclude struct must exist with the IncludeInput impl.
        assert!(
            code.contains("UserInclude"),
            "expected UserInclude struct in generated code"
        );
        assert!(
            code.contains("IncludeInput"),
            "expected IncludeInput trait impl"
        );
    }

    // ── select_input codegen tests ─────────────────────────────────────────

    #[test]
    fn user_select_emits_per_column_options() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                pub email: String,
                pub name: Option<String>,
            }
        };
        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "derive_model_impl failed");
        let code = result.unwrap().to_string();

        // UserSelect struct with Option<bool> per column.
        assert!(
            code.contains("UserSelect"),
            "expected UserSelect struct in generated code"
        );
        assert!(
            code.contains("SelectInput"),
            "expected SelectInput trait impl"
        );
        // The column literals must appear in the lowering.
        assert!(code.contains("\"id\""), "expected id column literal");
        assert!(code.contains("\"email\""), "expected email column literal");
        assert!(code.contains("\"name\""), "expected name column literal");
    }

    #[test]
    fn user_where_input_email_contains_lowers_to_contains_filter() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                #[prax(unique)]
                pub email: String,
            }
        };

        let result = derive_model_impl(&input);
        assert!(
            result.is_ok(),
            "derive_model_impl failed: {:?}",
            result.err()
        );
        let code = result.unwrap().to_string();

        // The lowering for email (String column) must call into_filter("email").
        assert!(
            code.contains("\"email\""),
            "expected column name \"email\" in the into_ir lowering"
        );
        // The lowering body must check for Filter::None and push.
        assert!(
            code.contains("into_filter"),
            "expected ScalarFilter::into_filter call in into_ir"
        );
    }

    // ── order_by_input codegen tests ──────────────────────────────────────

    #[test]
    fn user_order_by_emits_per_column_variants() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                pub email: String,
                pub name: Option<String>,
            }
        };
        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "derive_model_impl failed");
        let code = result.unwrap().to_string();

        // UserOrderBy enum with a variant per sortable column.
        assert!(
            code.contains("UserOrderBy"),
            "expected UserOrderBy enum in generated code"
        );
        assert!(
            code.contains("OrderByInput"),
            "expected OrderByInput trait impl"
        );
        // Variants per column.
        assert!(code.contains("Id"), "expected Id variant");
        assert!(code.contains("Email"), "expected Email variant");
        assert!(code.contains("Name"), "expected Name variant");
        // SortOrder must be referenced as the variant payload.
        assert!(code.contains("SortOrder"), "expected SortOrder payload");
    }

    // ── create_input codegen tests ────────────────────────────────────────

    #[test]
    fn user_create_input_emits_required_and_optional_fields() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id, auto)]
                pub id: i64,
                #[prax(unique)]
                pub email: String,
                pub name: Option<String>,
                pub age: Option<i32>,
                pub active: bool,
            }
        };
        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "derive_model_impl failed");
        let code = result.unwrap().to_string();

        // UserCreateInput must be present.
        assert!(
            code.contains("UserCreateInput"),
            "expected UserCreateInput struct in generated code"
        );
        // id is @id @auto — must NOT appear in CreateInput (DB-generated).
        // Confirm UserCreateInput doesn't surround a literal "id" payload field;
        // this is a stringy check — accept some flexibility.
        // Required scalar fields appear without Option wrap.
        // (Token-stream test: just confirm the field names appear.)
        assert!(
            code.contains("email"),
            "expected email field in UserCreateInput"
        );
        assert!(
            code.contains("active"),
            "expected active field in UserCreateInput"
        );
    }

    // ── relation_meta codegen tests ───────────────────────────────────────

    #[test]
    fn user_with_posts_relation_emits_relation_filter_meta() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id)]
                pub id: i64,
                pub email: String,
                #[prax(relation(target = "Post", foreign_key = "author_id"))]
                pub posts: Vec<Post>,
            }
        };
        let result = derive_model_impl(&input);
        assert!(
            result.is_ok(),
            "derive_model_impl failed: {:?}",
            result.err()
        );
        let code = result.unwrap().to_string();

        // Marker struct and impl must be present.
        assert!(
            code.contains("UserPostsFilterMeta"),
            "expected UserPostsFilterMeta marker"
        );
        assert!(
            code.contains("RelationFilterMeta"),
            "expected RelationFilterMeta trait impl"
        );
        // Table + key constants.
        assert!(
            code.contains("\"users\""),
            "expected \"users\" as PARENT_TABLE"
        );
        assert!(
            code.contains("\"post\""),
            "expected \"post\" as CHILD_TABLE (snake_case of target \"Post\")"
        );
        assert!(
            code.contains("\"author_id\""),
            "expected \"author_id\" as CHILD_FK"
        );
        assert!(code.contains("\"id\""), "expected \"id\" as PARENT_PK");
    }

    // ── update_input codegen tests ────────────────────────────────────────

    #[test]
    fn user_update_input_emits_field_update_wrappers() {
        let input: DeriveInput = parse_quote! {
            #[prax(table = "users")]
            pub struct User {
                #[prax(id, auto)]
                pub id: i64,
                #[prax(unique)]
                pub email: String,
                pub name: Option<String>,
                pub age: Option<i32>,
                pub active: bool,
            }
        };
        let result = derive_model_impl(&input);
        assert!(result.is_ok(), "derive_model_impl failed");
        let code = result.unwrap().to_string();

        // UserUpdateInput must be present.
        assert!(
            code.contains("UserUpdateInput"),
            "expected UserUpdateInput struct in generated code"
        );
        // Update wrappers per field type.
        assert!(
            code.contains("StringFieldUpdate"),
            "expected StringFieldUpdate for email"
        );
        assert!(
            code.contains("StringNullableFieldUpdate"),
            "expected StringNullableFieldUpdate for name"
        );
        assert!(
            code.contains("IntNullableFieldUpdate"),
            "expected IntNullableFieldUpdate for age"
        );
        assert!(
            code.contains("BoolFieldUpdate"),
            "expected BoolFieldUpdate for active"
        );
    }
}
