//! Diesel schema parser and converter.

use crate::converter::{
    FieldBuilder, ModelBuilder, SchemaBuilder, column_name_to_field_name, table_name_to_model_name,
};
use crate::diesel::types::*;
use crate::error::ImportResult;
use once_cell::sync::Lazy;
use prax_schema::ast::*;
use regex_lite::Regex;
use smol_str::SmolStr;
use std::fs;
use std::path::Path;

// Pre-compiled regex patterns for better performance
static TABLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)table!\s*\{\s*(\w+)\s*\(([^)]+)\)\s*\{([^}]+)\}\s*\}").unwrap());

static COLUMN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\w+)\s*->\s*([\w<>]+),?").unwrap());

static JOINABLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"joinable!\s*\(\s*(\w+)\s*->\s*(\w+)\s*\((\w+)\)\s*\)").unwrap());

/// Parse a Diesel schema from a string.
///
/// Returns the intermediate `DieselSchema` representation.
pub fn parse_diesel_schema(input: &str) -> ImportResult<DieselSchema> {
    let tables = parse_tables(input)?;
    let joinables = parse_joinables(input)?;

    Ok(DieselSchema { tables, joinables })
}

/// Parse a Diesel schema from a file.
pub fn parse_diesel_file<P: AsRef<Path>>(path: P) -> ImportResult<DieselSchema> {
    let content = fs::read_to_string(path)?;
    parse_diesel_schema(&content)
}

/// Convert Diesel schema to Prax schema.
pub fn import_diesel_schema(input: &str) -> ImportResult<Schema> {
    let diesel_schema = parse_diesel_schema(input)?;
    convert_diesel_to_prax(diesel_schema)
}

/// Convert a Diesel schema file to Prax schema.
pub fn import_diesel_schema_file<P: AsRef<Path>>(path: P) -> ImportResult<Schema> {
    let diesel_schema = parse_diesel_file(path)?;
    convert_diesel_to_prax(diesel_schema)
}

/// Parse all table! macros from the schema.
fn parse_tables(input: &str) -> ImportResult<Vec<DieselTable>> {
    let mut tables = vec![];

    for caps in TABLE_RE.captures_iter(input) {
        let name = caps.get(1).unwrap().as_str().to_string();
        let pk_str = caps.get(2).unwrap().as_str();
        let body = caps.get(3).unwrap().as_str();

        let primary_key = pk_str.split(',').map(|s| s.trim().to_string()).collect();

        let columns = parse_columns(body)?;

        tables.push(DieselTable {
            name,
            primary_key,
            columns,
        });
    }

    Ok(tables)
}

/// Parse columns from table body.
fn parse_columns(body: &str) -> ImportResult<Vec<DieselColumn>> {
    let mut columns = vec![];

    for caps in COLUMN_RE.captures_iter(body) {
        let name = caps.get(1).unwrap().as_str().to_string();
        let type_str = caps.get(2).unwrap().as_str();

        let (sql_type, is_nullable) = parse_sql_type(type_str)?;

        columns.push(DieselColumn {
            name,
            sql_type,
            is_nullable,
        });
    }

    Ok(columns)
}

/// Parse Diesel SQL type.
fn parse_sql_type(type_str: &str) -> ImportResult<(DieselSqlType, bool)> {
    // Check for Nullable wrapper
    if type_str.starts_with("Nullable<") {
        let inner = type_str
            .trim_start_matches("Nullable<")
            .trim_end_matches('>');
        let (inner_type, _) = parse_sql_type(inner)?;
        return Ok((DieselSqlType::Nullable(Box::new(inner_type)), true));
    }

    let sql_type = match type_str {
        "Int4" | "Integer" => DieselSqlType::Int4,
        "Int8" | "BigInt" | "Bigint" => DieselSqlType::Int8,
        "Int2" | "SmallInt" | "Smallint" => DieselSqlType::Int2,
        "Float4" | "Float" => DieselSqlType::Float4,
        "Float8" | "Double" => DieselSqlType::Float8,
        "Numeric" | "Decimal" => DieselSqlType::Numeric,
        "Text" => DieselSqlType::Text,
        "Varchar" | "VarChar" => DieselSqlType::Varchar,
        "Bool" | "Boolean" => DieselSqlType::Bool,
        "Timestamp" | "Timestamptz" => DieselSqlType::Timestamp,
        "Date" => DieselSqlType::Date,
        "Time" | "Timetz" => DieselSqlType::Time,
        "Json" => DieselSqlType::Json,
        "Jsonb" => DieselSqlType::Jsonb,
        "Bytea" | "Binary" => DieselSqlType::Bytea,
        "Uuid" => DieselSqlType::Uuid,
        custom => DieselSqlType::Custom(custom.to_string()),
    };

    Ok((sql_type, false))
}

/// Parse joinable! macros.
fn parse_joinables(input: &str) -> ImportResult<Vec<DieselJoinable>> {
    let mut joinables = vec![];

    for caps in JOINABLE_RE.captures_iter(input) {
        let child_table = caps.get(1).unwrap().as_str().to_string();
        let parent_table = caps.get(2).unwrap().as_str().to_string();
        let foreign_key = caps.get(3).unwrap().as_str().to_string();

        joinables.push(DieselJoinable {
            child_table,
            parent_table,
            foreign_key,
        });
    }

    Ok(joinables)
}

/// Convert Diesel schema to Prax schema.
fn convert_diesel_to_prax(diesel_schema: DieselSchema) -> ImportResult<Schema> {
    let mut builder = SchemaBuilder::new();

    // Convert tables to models
    for table in diesel_schema.tables {
        let model = convert_table(table, &diesel_schema.joinables)?;
        builder.add_model(model);
    }

    Ok(builder.build())
}

/// Convert a Diesel table to a Prax model.
fn convert_table(table: DieselTable, joinables: &[DieselJoinable]) -> ImportResult<Model> {
    let model_name = table_name_to_model_name(&table.name);
    let mut model_builder = ModelBuilder::new(&model_name).with_db_name(&table.name);

    // Convert columns
    for column in table.columns {
        let is_pk = table.primary_key.contains(&column.name);
        let field = convert_column(column, is_pk)?;
        model_builder.add_field(field);
    }

    // Add relation fields based on joinables
    for joinable in joinables {
        if joinable.child_table == table.name {
            // This table has a foreign key to parent_table
            let relation_name = table_name_to_model_name(&joinable.parent_table);
            let field_name = column_name_to_field_name(&joinable.parent_table);

            // Create relation field
            let relation_field = FieldBuilder::new(
                &field_name,
                FieldType::Model(SmolStr::from(&relation_name)),
                TypeModifier::Required,
            )
            .with_relation(
                None,
                vec![column_name_to_field_name(&joinable.foreign_key)],
                vec!["id".to_string()],
                None,
                None,
                None,
            )
            .build();

            model_builder.add_field(relation_field);
        }
    }

    Ok(model_builder.build())
}

/// Convert a Diesel column to a Prax field.
fn convert_column(column: DieselColumn, is_pk: bool) -> ImportResult<Field> {
    let (prax_type, modifier) = convert_sql_type(&column.sql_type, column.is_nullable)?;
    let field_name = column_name_to_field_name(&column.name);

    let mut field_builder = FieldBuilder::new(&field_name, prax_type, modifier);

    // Set @map if the field name differs from column name
    if field_name != column.name {
        field_builder = field_builder.with_db_name(&column.name);
    }

    // Mark primary keys
    if is_pk {
        field_builder = field_builder.with_id();

        // Auto-increment for integer primary keys
        if matches!(column.sql_type, DieselSqlType::Int4 | DieselSqlType::Int8) {
            field_builder = field_builder.with_auto();
        }
    }

    Ok(field_builder.build())
}

/// Convert Diesel SQL type to Prax field type and modifier.
fn convert_sql_type(
    sql_type: &DieselSqlType,
    is_nullable: bool,
) -> ImportResult<(FieldType, TypeModifier)> {
    let base_type = match sql_type {
        DieselSqlType::Int4 | DieselSqlType::Int2 => FieldType::Scalar(ScalarType::Int),
        DieselSqlType::Int8 => FieldType::Scalar(ScalarType::BigInt),
        DieselSqlType::Float4 | DieselSqlType::Float8 => FieldType::Scalar(ScalarType::Float),
        DieselSqlType::Numeric => FieldType::Scalar(ScalarType::Decimal),
        DieselSqlType::Text | DieselSqlType::Varchar => FieldType::Scalar(ScalarType::String),
        DieselSqlType::Bool => FieldType::Scalar(ScalarType::Boolean),
        DieselSqlType::Timestamp => FieldType::Scalar(ScalarType::DateTime),
        DieselSqlType::Date => FieldType::Scalar(ScalarType::Date),
        DieselSqlType::Time => FieldType::Scalar(ScalarType::Time),
        DieselSqlType::Json | DieselSqlType::Jsonb => FieldType::Scalar(ScalarType::Json),
        DieselSqlType::Bytea => FieldType::Scalar(ScalarType::Bytes),
        DieselSqlType::Uuid => FieldType::Scalar(ScalarType::Uuid),
        DieselSqlType::Nullable(inner) => {
            return convert_sql_type(inner, true);
        }
        DieselSqlType::Custom(name) => FieldType::Enum(SmolStr::from(name.as_str())),
    };

    let modifier = if is_nullable {
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
    fn test_parse_simple_table() {
        let schema = r#"
        table! {
            users (id) {
                id -> Int4,
                email -> Varchar,
                name -> Nullable<Varchar>,
            }
        }
        "#;

        let result = parse_diesel_schema(schema);
        assert!(result.is_ok());

        let diesel_schema = result.unwrap();
        assert_eq!(diesel_schema.tables.len(), 1);
        assert_eq!(diesel_schema.tables[0].columns.len(), 3);
    }

    #[test]
    fn test_import_simple_table() {
        let schema = r#"
        table! {
            users (id) {
                id -> Int4,
                email -> Varchar,
            }
        }
        "#;

        let result = import_diesel_schema(schema);
        assert!(result.is_ok());

        let prax_schema = result.unwrap();
        assert_eq!(prax_schema.models.len(), 1);
    }

    #[test]
    fn test_parse_with_joinable() {
        let schema = r#"
        table! {
            users (id) {
                id -> Int4,
            }
        }

        table! {
            posts (id) {
                id -> Int4,
                author_id -> Int4,
            }
        }

        joinable!(posts -> users (author_id));
        "#;

        let result = parse_diesel_schema(schema);
        assert!(result.is_ok());

        let diesel_schema = result.unwrap();
        assert_eq!(diesel_schema.tables.len(), 2);
        assert_eq!(diesel_schema.joinables.len(), 1);
    }
}
