//! SeaORM entity parser and converter.

use crate::converter::{FieldBuilder, ModelBuilder, SchemaBuilder};
use crate::error::ImportResult;
use crate::seaorm::types::*;
use prax_schema::ast::*;
use smol_str::SmolStr;
use std::fs;
use std::path::Path;
use syn::{Attribute, Fields, Item, Meta, Type};

/// Parse a SeaORM entity file from a string.
pub fn parse_seaorm_entity(input: &str) -> ImportResult<SeaOrmEntity> {
    let syntax = syn::parse_file(input).map_err(|e| {
        crate::error::ImportError::DieselParseError(format!("Failed to parse Rust file: {}", e))
    })?;

    let mut entity = None;
    let mut relations = vec![];

    for item in syntax.items {
        match item {
            Item::Struct(item_struct) => {
                // Check if this is an entity model (has DeriveEntityModel)
                if has_derive(&item_struct.attrs, "DeriveEntityModel") {
                    entity = Some(parse_entity_struct(item_struct)?);
                }
            }
            Item::Enum(item_enum) => {
                // Check if this is a Relation enum
                if has_derive(&item_enum.attrs, "DeriveRelation") {
                    relations = parse_relation_enum(item_enum)?;
                }
            }
            _ => {}
        }
    }

    let mut entity = entity.ok_or_else(|| {
        crate::error::ImportError::DieselParseError(
            "No entity struct found with #[derive(DeriveEntityModel)]".to_string(),
        )
    })?;

    entity.relations = relations;

    Ok(entity)
}

/// Parse a SeaORM entity from a file.
pub fn parse_seaorm_entity_file<P: AsRef<Path>>(path: P) -> ImportResult<SeaOrmEntity> {
    let content = fs::read_to_string(path)?;
    parse_seaorm_entity(&content)
}

/// Convert SeaORM entity to Prax schema.
pub fn import_seaorm_entity(input: &str) -> ImportResult<Schema> {
    let entity = parse_seaorm_entity(input)?;
    convert_seaorm_to_prax(vec![entity])
}

/// Convert a SeaORM entity file to Prax schema.
pub fn import_seaorm_entity_file<P: AsRef<Path>>(path: P) -> ImportResult<Schema> {
    let entity = parse_seaorm_entity_file(path)?;
    convert_seaorm_to_prax(vec![entity])
}

/// Check if attributes contain a specific derive.
fn has_derive(attrs: &[Attribute], derive_name: &str) -> bool {
    attrs.iter().any(|attr| {
        if attr.path().is_ident("derive") {
            // Parse as Meta::List to access nested items
            if let Meta::List(list) = &attr.meta {
                let tokens = list.tokens.to_string();
                return tokens.contains(derive_name);
            }
        }
        false
    })
}

/// Parse entity struct into SeaOrmEntity.
fn parse_entity_struct(item_struct: syn::ItemStruct) -> ImportResult<SeaOrmEntity> {
    let name = item_struct.ident.to_string();

    // Extract table name from sea_orm attribute
    let table_name = extract_table_name(&item_struct.attrs).unwrap_or_else(|| {
        // Convert struct name to snake_case and pluralize
        let snake = name.to_lowercase();
        if snake.ends_with('y') {
            format!("{}ies", &snake[..snake.len() - 1])
        } else {
            format!("{}s", snake)
        }
    });

    let mut fields = vec![];

    if let Fields::Named(named_fields) = item_struct.fields {
        for field in named_fields.named {
            let field_name = field.ident.unwrap().to_string();
            let (field_type, is_optional) = parse_field_type(&field.ty)?;
            let attributes = parse_field_attributes(&field.attrs)?;

            fields.push(SeaOrmField {
                name: field_name,
                field_type,
                is_optional,
                attributes,
                documentation: None,
            });
        }
    }

    Ok(SeaOrmEntity {
        name,
        table_name,
        fields,
        relations: vec![],
        documentation: None,
    })
}

/// Extract table name from sea_orm attribute.
fn extract_table_name(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("sea_orm") {
            if let Ok(meta) = attr.parse_args::<syn::Meta>() {
                if let syn::Meta::NameValue(nv) = meta {
                    if nv.path.is_ident("table_name") {
                        if let syn::Expr::Lit(lit) = &nv.value {
                            if let syn::Lit::Str(s) = &lit.lit {
                                return Some(s.value());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parse field type and check if optional.
fn parse_field_type(ty: &Type) -> ImportResult<(SeaOrmFieldType, bool)> {
    // Check if it's Option<T>
    if let Type::Path(type_path) = ty {
        let segments = &type_path.path.segments;

        if let Some(last_segment) = segments.last() {
            let type_name = last_segment.ident.to_string();

            // Handle Option<T>
            if type_name == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                        let (inner_type, _) = parse_field_type(inner_ty)?;
                        return Ok((inner_type, true));
                    }
                }
            }

            // Map Rust types to SeaORM types
            let field_type = match type_name.as_str() {
                "i32" => SeaOrmFieldType::I32,
                "i64" => SeaOrmFieldType::I64,
                "f32" => SeaOrmFieldType::F32,
                "f64" => SeaOrmFieldType::F64,
                "String" => SeaOrmFieldType::String,
                "bool" => SeaOrmFieldType::Bool,
                "DateTime" => SeaOrmFieldType::DateTime,
                "Date" => SeaOrmFieldType::Date,
                "Time" => SeaOrmFieldType::Time,
                "Decimal" => SeaOrmFieldType::Decimal,
                "Value" => SeaOrmFieldType::Json, // serde_json::Value
                "Uuid" => SeaOrmFieldType::Uuid,
                "Vec" => {
                    // Check if Vec<u8>
                    if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
                        if let Some(syn::GenericArgument::Type(Type::Path(inner_path))) =
                            args.args.first()
                        {
                            if let Some(seg) = inner_path.path.segments.last() {
                                if seg.ident == "u8" {
                                    return Ok((SeaOrmFieldType::Bytes, false));
                                }
                            }
                        }
                    }
                    SeaOrmFieldType::Custom("Vec".to_string())
                }
                other => SeaOrmFieldType::Custom(other.to_string()),
            };

            return Ok((field_type, false));
        }
    }

    Ok((SeaOrmFieldType::Custom("Unknown".to_string()), false))
}

/// Parse field attributes from sea_orm.
fn parse_field_attributes(attrs: &[Attribute]) -> ImportResult<Vec<SeaOrmFieldAttribute>> {
    let mut attributes = vec![];

    for attr in attrs {
        if attr.path().is_ident("sea_orm") {
            // Parse sea_orm attributes
            if let Ok(meta) = attr.parse_args::<syn::Meta>() {
                match meta {
                    syn::Meta::Path(path) => {
                        // Single identifiers like primary_key, auto_increment
                        if path.is_ident("primary_key") {
                            attributes.push(SeaOrmFieldAttribute::PrimaryKey);
                        } else if path.is_ident("auto_increment") {
                            attributes.push(SeaOrmFieldAttribute::AutoIncrement);
                        } else if path.is_ident("unique") {
                            attributes.push(SeaOrmFieldAttribute::Unique);
                        } else if path.is_ident("indexed") {
                            attributes.push(SeaOrmFieldAttribute::Indexed);
                        } else if path.is_ident("nullable") {
                            attributes.push(SeaOrmFieldAttribute::Nullable);
                        }
                    }
                    syn::Meta::NameValue(nv) => {
                        // Key-value pairs like column_name = "..."
                        if nv.path.is_ident("column_name") {
                            if let syn::Expr::Lit(lit) = &nv.value {
                                if let syn::Lit::Str(s) = &lit.lit {
                                    attributes.push(SeaOrmFieldAttribute::ColumnName(s.value()));
                                }
                            }
                        } else if nv.path.is_ident("column_type") {
                            if let syn::Expr::Lit(lit) = &nv.value {
                                if let syn::Lit::Str(s) = &lit.lit {
                                    attributes.push(SeaOrmFieldAttribute::ColumnType(s.value()));
                                }
                            }
                        } else if nv.path.is_ident("default_value") {
                            if let syn::Expr::Lit(lit) = &nv.value {
                                if let syn::Lit::Str(s) = &lit.lit {
                                    attributes.push(SeaOrmFieldAttribute::DefaultValue(s.value()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(attributes)
}

/// Parse Relation enum.
fn parse_relation_enum(item_enum: syn::ItemEnum) -> ImportResult<Vec<SeaOrmRelation>> {
    let mut relations = vec![];

    for variant in item_enum.variants {
        let name = variant.ident.to_string();

        // Parse sea_orm relation attributes
        for attr in &variant.attrs {
            if attr.path().is_ident("sea_orm") {
                if let Ok(meta) = attr.parse_args::<syn::Meta>() {
                    if let Some(relation) = parse_relation_attribute(name.clone(), meta)? {
                        relations.push(relation);
                    }
                }
            }
        }
    }

    Ok(relations)
}

/// Parse a single relation attribute.
fn parse_relation_attribute(name: String, meta: syn::Meta) -> ImportResult<Option<SeaOrmRelation>> {
    match meta {
        syn::Meta::NameValue(nv) => {
            let relation_type = if nv.path.is_ident("has_one") {
                SeaOrmRelationType::HasOne
            } else if nv.path.is_ident("has_many") {
                SeaOrmRelationType::HasMany
            } else if nv.path.is_ident("belongs_to") {
                SeaOrmRelationType::BelongsTo
            } else {
                return Ok(None);
            };

            // Extract entity path
            let entity = if let syn::Expr::Lit(lit) = &nv.value {
                if let syn::Lit::Str(s) = &lit.lit {
                    s.value()
                } else {
                    return Ok(None);
                }
            } else if let syn::Expr::Path(path) = &nv.value {
                // Handle super::entity::Entity format
                path.path
                    .segments
                    .iter()
                    .filter(|seg| seg.ident != "super" && seg.ident != "Entity")
                    .map(|seg| seg.ident.to_string())
                    .last()
                    .unwrap_or_default()
            } else {
                return Ok(None);
            };

            Ok(Some(SeaOrmRelation {
                name,
                relation_type,
                entity,
                from: None,
                to: None,
                on_delete: None,
                on_update: None,
            }))
        }
        _ => Ok(None),
    }
}

/// Convert SeaORM entities to Prax schema.
fn convert_seaorm_to_prax(entities: Vec<SeaOrmEntity>) -> ImportResult<Schema> {
    let mut builder = SchemaBuilder::new();

    for entity in entities {
        let model = convert_entity(entity)?;
        builder.add_model(model);
    }

    Ok(builder.build())
}

/// Convert a SeaORM entity to a Prax model.
fn convert_entity(entity: SeaOrmEntity) -> ImportResult<Model> {
    let mut model_builder = ModelBuilder::new(&entity.name).with_db_name(&entity.table_name);

    // Convert fields
    for field in entity.fields {
        let prax_field = convert_field(field)?;
        model_builder.add_field(prax_field);
    }

    // TODO: Handle relations when we add relation support
    // For now, relations are comments or separate fields

    Ok(model_builder.build())
}

/// Convert a SeaORM field to a Prax field.
fn convert_field(field: SeaOrmField) -> ImportResult<Field> {
    let (prax_type, modifier) = convert_field_type(&field.field_type, field.is_optional)?;
    let field_name = field.name.clone();
    let mut field_builder = FieldBuilder::new(&field_name, prax_type, modifier);

    // Convert attributes
    for attr in field.attributes {
        match attr {
            SeaOrmFieldAttribute::PrimaryKey => {
                field_builder = field_builder.with_id();
            }
            SeaOrmFieldAttribute::AutoIncrement => {
                field_builder = field_builder.with_auto();
            }
            SeaOrmFieldAttribute::Unique => {
                field_builder = field_builder.with_unique();
            }
            SeaOrmFieldAttribute::ColumnName(col_name) => {
                field_builder = field_builder.with_map(col_name);
            }
            SeaOrmFieldAttribute::DefaultValue(val) => {
                // Parse default value
                let default_val = if val == "true" {
                    AttributeValue::Boolean(true)
                } else if val == "false" {
                    AttributeValue::Boolean(false)
                } else if let Ok(n) = val.parse::<i64>() {
                    AttributeValue::Int(n)
                } else if let Ok(f) = val.parse::<f64>() {
                    AttributeValue::Float(f)
                } else {
                    AttributeValue::String(val)
                };
                field_builder = field_builder.with_default(default_val);
            }
            _ => {}
        }
    }

    Ok(field_builder.build())
}

/// Convert SeaORM field type to Prax field type.
fn convert_field_type(
    field_type: &SeaOrmFieldType,
    is_optional: bool,
) -> ImportResult<(FieldType, TypeModifier)> {
    let base_type = match field_type {
        SeaOrmFieldType::I32 | SeaOrmFieldType::I64 => FieldType::Scalar(ScalarType::Int),
        SeaOrmFieldType::F32 | SeaOrmFieldType::F64 => FieldType::Scalar(ScalarType::Float),
        SeaOrmFieldType::String => FieldType::Scalar(ScalarType::String),
        SeaOrmFieldType::Bool => FieldType::Scalar(ScalarType::Boolean),
        SeaOrmFieldType::DateTime => FieldType::Scalar(ScalarType::DateTime),
        SeaOrmFieldType::Date => FieldType::Scalar(ScalarType::Date),
        SeaOrmFieldType::Time => FieldType::Scalar(ScalarType::Time),
        SeaOrmFieldType::Decimal => FieldType::Scalar(ScalarType::Decimal),
        SeaOrmFieldType::Json => FieldType::Scalar(ScalarType::Json),
        SeaOrmFieldType::Bytes => FieldType::Scalar(ScalarType::Bytes),
        SeaOrmFieldType::Uuid => FieldType::Scalar(ScalarType::Uuid),
        SeaOrmFieldType::Custom(name) => FieldType::Enum(SmolStr::from(name.as_str())),
    };

    let modifier = if is_optional {
        TypeModifier::Optional
    } else {
        TypeModifier::Required
    };

    Ok((base_type, modifier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_entity() {
        let entity_code = r#"
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "users")]
        pub struct Model {
            #[sea_orm(primary_key)]
            pub id: i32,
            pub email: String,
            pub name: Option<String>,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}
        "#;

        let result = parse_seaorm_entity(entity_code);
        assert!(result.is_ok());

        let entity = result.unwrap();
        assert_eq!(entity.name, "Model");
        assert_eq!(entity.table_name, "users");
        assert_eq!(entity.fields.len(), 3);
    }

    #[test]
    fn test_import_entity() {
        let entity_code = r#"
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "posts")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment)]
            pub id: i32,
            pub title: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}
        "#;

        let result = import_seaorm_entity(entity_code);
        assert!(result.is_ok());

        let schema = result.unwrap();
        assert_eq!(schema.models.len(), 1);
    }
}
