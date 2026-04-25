//! Prisma schema parser and converter.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use convert_case::{Case, Casing};
use once_cell::sync::Lazy;
use regex_lite::Regex;
use smol_str::SmolStr;

use crate::converter::{FieldBuilder, ModelBuilder, SchemaBuilder, dummy_span};
use crate::error::ImportResult;
use crate::prisma::types::*;
use prax_schema::ast::*;

// Pre-compiled regex patterns for better performance
static DATASOURCE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?s)datasource\s+\w+\s*\{[^}]*provider\s*=\s*"([^"]+)"[^}]*url\s*=\s*[^"}]*"([^"]+)""#,
    )
    .unwrap()
});

static MODEL_START_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"model\s+(\w+)\s*\{").unwrap());

// FIELD_RE removed — field parsing now uses split_field_line() for robust handling
// of complex types like Unsupported("vector(1024)")

// DEFAULT_RE and RELATION_RE removed — use extract_paren_content() for nested parens

static MAP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"@map\("([^"]+)"\)"#).unwrap());

static RELATION_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"name:\s*"([^"]+)""#).unwrap());

static RELATION_FIELDS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"fields:\s*\[([^\]]+)\]").unwrap());

static RELATION_REFS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"references:\s*\[([^\]]+)\]").unwrap());

static MODEL_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@@id\(\[([^\]]+)\]\)").unwrap());

static MODEL_UNIQUE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@@unique\(\[([^\]]+)\]").unwrap());

static MODEL_INDEX_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@@index\(\[([^\]]+)\]").unwrap());

static MODEL_MAP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"@@map\("([^"]+)"\)"#).unwrap());

static ATTR_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"name:\s*"([^"]+)""#).unwrap());

static ENUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)enum\s+(\w+)\s*\{([^}]+)\}").unwrap());

/// Parse a Prisma schema from a string.
///
/// Returns the intermediate `PrismaSchema` representation.
pub fn parse_prisma_schema(input: &str) -> ImportResult<PrismaSchema> {
    let mut schema = PrismaSchema {
        datasource: None,
        models: vec![],
        enums: vec![],
    };

    // Parse datasource
    if let Some(datasource) = parse_datasource(input)? {
        schema.datasource = Some(datasource);
    }

    // Parse models
    for model in parse_models(input)? {
        schema.models.push(model);
    }

    // Parse enums
    for enum_def in parse_enums(input)? {
        schema.enums.push(enum_def);
    }

    Ok(schema)
}

/// Parse a Prisma schema from a file.
pub fn parse_prisma_file<P: AsRef<Path>>(path: P) -> ImportResult<PrismaSchema> {
    let content = fs::read_to_string(path)?;
    parse_prisma_schema(&content)
}

/// Convert Prisma schema to Prax schema.
pub fn import_prisma_schema(input: &str) -> ImportResult<Schema> {
    let prisma_schema = parse_prisma_schema(input)?;
    convert_prisma_to_prax(prisma_schema)
}

/// Convert a Prisma schema file to Prax schema.
pub fn import_prisma_schema_file<P: AsRef<Path>>(path: P) -> ImportResult<Schema> {
    let prisma_schema = parse_prisma_file(path)?;
    convert_prisma_to_prax(prisma_schema)
}

/// Parse datasource block.
fn parse_datasource(input: &str) -> ImportResult<Option<PrismaDatasource>> {
    if let Some(caps) = DATASOURCE_RE.captures(input) {
        let provider = caps.get(1).unwrap().as_str().to_string();
        let url = caps.get(2).unwrap().as_str().to_string();

        Ok(Some(PrismaDatasource { provider, url }))
    } else {
        Ok(None)
    }
}

/// Parse all models from the schema using brace-counting to handle nested braces
/// (e.g. `@default(dbgenerated("..."))` contains `}` chars that break simple regex).
fn parse_models(input: &str) -> ImportResult<Vec<PrismaModel>> {
    let mut models = vec![];
    let mut search_from = 0;

    while let Some(caps) = MODEL_START_RE.captures(&input[search_from..]) {
        let m = caps.get(0).unwrap();
        let name = caps.get(1).unwrap().as_str().to_string();
        let brace_start = search_from + m.end(); // position right after '{'

        // Count braces to find the matching close brace
        let mut depth = 1;
        let mut end = brace_start;
        let bytes = input.as_bytes();
        let mut in_string = false;
        while end < bytes.len() && depth > 0 {
            match bytes[end] {
                b'"' if !in_string => in_string = true,
                b'"' if in_string => in_string = false,
                b'{' if !in_string => depth += 1,
                b'}' if !in_string => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                end += 1;
            }
        }

        let body = &input[brace_start..end];
        let fields = parse_fields(body)?;
        let attributes = parse_model_attributes(body)?;

        models.push(PrismaModel {
            name,
            fields,
            attributes,
            documentation: None,
        });

        search_from = end + 1;
    }

    Ok(models)
}

/// Parse fields from model body.
fn parse_fields(body: &str) -> ImportResult<Vec<PrismaField>> {
    let mut fields = vec![];

    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("@@") || line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some((name, type_str)) = split_field_line(line) {
            let (field_type, is_optional, is_list) = parse_field_type(&type_str)?;
            let attributes = parse_field_attributes(line)?;

            fields.push(PrismaField {
                name,
                field_type,
                is_optional,
                is_list,
                attributes,
                documentation: None,
            });
        }
    }

    Ok(fields)
}

/// Extract the content inside balanced parentheses after a given prefix.
///
/// Given `line = "... @default(dbgenerated(\"...\")) @map..."` and `prefix = "@default"`,
/// returns `Some("dbgenerated(\"...\")")`.
fn extract_paren_content<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let start = line.find(prefix)?;
    let after_prefix = &line[start + prefix.len()..];
    if !after_prefix.starts_with('(') {
        return None;
    }
    let mut depth = 0;
    let mut in_string = false;
    for (i, c) in after_prefix.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&after_prefix[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a Prisma field line into (field_name, type_string).
///
/// Handles standard types (`String`, `Int?`, `Post[]`) and complex types
/// like `Unsupported("vector(1024)")`.
fn split_field_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();

    // Skip lines that start with @ (model-level attributes) or /// (docs)
    if trimmed.starts_with('@') || trimmed.starts_with("///") {
        return None;
    }

    // First token is the field name (must be a word char)
    let mut chars = trimmed.chars().peekable();
    let name: String = chars.by_ref().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() {
        return None;
    }

    // Skip whitespace
    let rest: String = chars.collect();
    let rest = rest.trim_start();

    // Second token is the type. Handle Unsupported("...") specially.
    let type_str = if rest.starts_with("Unsupported(") {
        // Consume until matching close paren, accounting for nested parens and quotes
        let mut depth = 0;
        let mut in_string = false;
        let mut end = 0;
        for (i, c) in rest.char_indices() {
            match c {
                '"' => in_string = !in_string,
                '(' if !in_string => depth += 1,
                ')' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        &rest[..end]
    } else {
        // Standard type: word chars, [], ?
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '@')
            .unwrap_or(rest.len());
        &rest[..end]
    };

    if type_str.is_empty() {
        return None;
    }

    Some((name, type_str.to_string()))
}

/// Parse field type and modifiers.
fn parse_field_type(type_str: &str) -> ImportResult<(PrismaFieldType, bool, bool)> {
    let is_optional = type_str.contains('?');
    let is_list = type_str.contains("[]");
    let base_type = type_str.replace('?', "").replace("[]", "");

    let field_type = if base_type.starts_with("Unsupported(") {
        // Extract the inner type string: Unsupported("vector(1024)") → "vector(1024)"
        let inner = base_type
            .strip_prefix("Unsupported(\"")
            .and_then(|s| s.strip_suffix("\")"))
            .unwrap_or(&base_type);
        PrismaFieldType::Custom(format!("Unsupported:{}", inner))
    } else {
        match base_type.as_str() {
            "String" => PrismaFieldType::String,
            "Boolean" => PrismaFieldType::Boolean,
            "Int" => PrismaFieldType::Int,
            "BigInt" => PrismaFieldType::BigInt,
            "Float" => PrismaFieldType::Float,
            "Decimal" => PrismaFieldType::Decimal,
            "DateTime" => PrismaFieldType::DateTime,
            "Json" => PrismaFieldType::Json,
            "Bytes" => PrismaFieldType::Bytes,
            custom => PrismaFieldType::Custom(custom.to_string()),
        }
    };

    Ok((field_type, is_optional, is_list))
}

/// Parse field attributes.
fn parse_field_attributes(line: &str) -> ImportResult<Vec<PrismaFieldAttribute>> {
    let mut attributes = vec![];

    if line.contains("@id") {
        attributes.push(PrismaFieldAttribute::Id);
    }

    if line.contains("@unique") {
        attributes.push(PrismaFieldAttribute::Unique);
    }

    if line.contains("@updatedAt") {
        attributes.push(PrismaFieldAttribute::UpdatedAt);
    }

    // Parse @default — use paren-aware extraction for nested calls like dbgenerated("...")
    if let Some(default_val) = extract_paren_content(line, "@default") {
        let default = if default_val.contains('(') {
            // Function call: now(), uuid(), dbgenerated("..."), autoincrement()
            // Store the full expression including args for dbgenerated
            PrismaDefaultValue::Function(default_val.to_string())
        } else {
            PrismaDefaultValue::Literal(default_val.to_string())
        };
        attributes.push(PrismaFieldAttribute::Default(default));
    }

    // Parse @map
    if let Some(caps) = MAP_RE.captures(line) {
        let map_val = caps.get(1).unwrap().as_str().to_string();
        attributes.push(PrismaFieldAttribute::Map(map_val));
    }

    // Parse @relation
    if line.contains("@relation") {
        let relation = parse_relation_attribute(line)?;
        attributes.push(relation);
    }

    Ok(attributes)
}

/// Parse @relation attribute.
fn parse_relation_attribute(line: &str) -> ImportResult<PrismaFieldAttribute> {
    if let Some(args) = extract_paren_content(line, "@relation") {
        let name = extract_relation_name(args);
        let fields = extract_relation_fields(args);
        let references = extract_relation_references(args);
        let on_delete = extract_relation_action(args, "onDelete");
        let on_update = extract_relation_action(args, "onUpdate");
        let map = extract_relation_map(args);

        Ok(PrismaFieldAttribute::Relation {
            name,
            fields,
            references,
            on_delete,
            on_update,
            map,
        })
    } else {
        Ok(PrismaFieldAttribute::Relation {
            name: None,
            fields: None,
            references: None,
            on_delete: None,
            on_update: None,
            map: None,
        })
    }
}

fn extract_relation_name(args: &str) -> Option<String> {
    // Try named syntax first: name: "RelationName"
    if let Some(caps) = RELATION_NAME_RE.captures(args) {
        return caps.get(1).map(|m| m.as_str().to_string());
    }
    // Try positional syntax: first arg is a quoted string before any named args
    // e.g. @relation("batch_file", fields: [...])
    let trimmed = args.trim();
    if trimmed.starts_with('"') {
        let end = trimmed[1..].find('"').map(|i| i + 1)?;
        return Some(trimmed[1..end].to_string());
    }
    None
}

fn extract_relation_fields(args: &str) -> Option<Vec<String>> {
    RELATION_FIELDS_RE.captures(args).map(|caps| {
        caps.get(1)
            .unwrap()
            .as_str()
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    })
}

fn extract_relation_references(args: &str) -> Option<Vec<String>> {
    RELATION_REFS_RE.captures(args).map(|caps| {
        caps.get(1)
            .unwrap()
            .as_str()
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    })
}

fn extract_relation_action(args: &str, action: &str) -> Option<String> {
    let pattern = format!(r"{}:\s*(\w+)", action);
    let re = Regex::new(&pattern).unwrap();
    re.captures(args)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

fn extract_relation_map(args: &str) -> Option<String> {
    let re = Regex::new(r#"map:\s*"([^"]+)""#).unwrap();
    re.captures(args)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

/// Parse model-level attributes.
fn parse_model_attributes(body: &str) -> ImportResult<Vec<PrismaModelAttribute>> {
    let mut attributes = vec![];

    for line in body.lines() {
        let line = line.trim();

        // Parse @@id
        if line.starts_with("@@id") {
            if let Some(caps) = MODEL_ID_RE.captures(line) {
                let fields = caps
                    .get(1)
                    .unwrap()
                    .as_str()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                attributes.push(PrismaModelAttribute::Id(fields));
            }
        }

        // Parse @@unique
        if line.starts_with("@@unique") {
            if let Some(caps) = MODEL_UNIQUE_RE.captures(line) {
                let fields = caps
                    .get(1)
                    .unwrap()
                    .as_str()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();

                let name = ATTR_NAME_RE
                    .captures(line)
                    .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()));

                attributes.push(PrismaModelAttribute::Unique { fields, name });
            }
        }

        // Parse @@index
        if line.starts_with("@@index") {
            if let Some(caps) = MODEL_INDEX_RE.captures(line) {
                let fields = caps
                    .get(1)
                    .unwrap()
                    .as_str()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();

                let name = ATTR_NAME_RE
                    .captures(line)
                    .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()));

                attributes.push(PrismaModelAttribute::Index { fields, name });
            }
        }

        // Parse @@map
        if line.starts_with("@@map") {
            if let Some(caps) = MODEL_MAP_RE.captures(line) {
                let map_val = caps.get(1).unwrap().as_str().to_string();
                attributes.push(PrismaModelAttribute::Map(map_val));
            }
        }
    }

    Ok(attributes)
}

/// Parse enums from the schema.
fn parse_enums(input: &str) -> ImportResult<Vec<PrismaEnum>> {
    let mut enums = vec![];

    for caps in ENUM_RE.captures_iter(input) {
        let name = caps.get(1).unwrap().as_str().to_string();
        let body = caps.get(2).unwrap().as_str();

        let values = body
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .map(|l| l.to_string())
            .collect();

        enums.push(PrismaEnum {
            name,
            values,
            documentation: None,
        });
    }

    Ok(enums)
}

/// Convert Prisma schema to Prax schema.
fn convert_prisma_to_prax(prisma_schema: PrismaSchema) -> ImportResult<Schema> {
    let mut builder = SchemaBuilder::new();

    // Convert datasource
    if let Some(datasource) = prisma_schema.datasource {
        builder = builder.with_datasource(datasource.provider, datasource.url);
    }

    // Convert models
    for model in prisma_schema.models {
        let prax_model = convert_model(model)?;
        builder.add_model(prax_model);
    }

    // Convert enums
    for enum_def in prisma_schema.enums {
        let prax_enum = convert_enum(enum_def);
        builder.add_enum(prax_enum);
    }

    Ok(builder.build())
}

/// Convert a Prisma model to a Prax model.
fn convert_model(model: PrismaModel) -> ImportResult<Model> {
    let mut model_builder = ModelBuilder::new(&model.name);

    // Check for @@map attribute
    for attr in &model.attributes {
        if let PrismaModelAttribute::Map(table_name) = attr {
            model_builder = model_builder.with_db_name(table_name);
        }
    }

    // Build field name mapping: prisma_name → prax_name (snake_case / @map col)
    let field_name_map: HashMap<String, String> = model
        .fields
        .iter()
        .map(|f| {
            let prax_name = f
                .attributes
                .iter()
                .find_map(|a| {
                    if let PrismaFieldAttribute::Map(col) = a {
                        Some(col.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| f.name.to_case(Case::Snake));
            (f.name.clone(), prax_name)
        })
        .collect();

    // Convert fields
    for field in model.fields {
        let prax_field = convert_field(field, &field_name_map)?;
        model_builder.add_field(prax_field);
    }

    // Convert model attributes, remapping field references to snake_case
    for attr in model.attributes {
        match attr {
            PrismaModelAttribute::Unique { fields, name } => {
                let mapped: Vec<String> = fields
                    .into_iter()
                    .map(|f| remap_field_name(&f, &field_name_map))
                    .collect();
                model_builder.add_unique(mapped, name);
            }
            PrismaModelAttribute::Index { fields, name } => {
                let mapped: Vec<String> = fields
                    .into_iter()
                    .map(|f| remap_field_name(&f, &field_name_map))
                    .collect();
                model_builder.add_index(mapped, name);
            }
            PrismaModelAttribute::Map(_) => {
                // Already handled above
            }
            PrismaModelAttribute::Id(fields) => {
                let mapped: Vec<String> = fields
                    .into_iter()
                    .map(|f| remap_field_name(&f, &field_name_map))
                    .collect();
                model_builder.add_unique(mapped, Some("PRIMARY".to_string()));
            }
        }
    }

    Ok(model_builder.build())
}

/// Remap a Prisma field name to its Prax equivalent using the field name map.
fn remap_field_name(prisma_name: &str, map: &HashMap<String, String>) -> String {
    map.get(prisma_name)
        .cloned()
        .unwrap_or_else(|| prisma_name.to_case(Case::Snake))
}

/// Convert a Prisma field to a Prax field.
fn convert_field(
    field: PrismaField,
    field_name_map: &HashMap<String, String>,
) -> ImportResult<Field> {
    let (prax_type, modifier) =
        convert_field_type(&field.field_type, field.is_optional, field.is_list)?;

    // Determine the Prax field name: use @map column name if present, else snake_case
    let prax_name = field_name_map
        .get(&field.name)
        .cloned()
        .unwrap_or_else(|| field.name.to_case(Case::Snake));

    let mut field_builder = FieldBuilder::new(&prax_name, prax_type, modifier);

    // Convert field attributes
    for attr in field.attributes {
        match attr {
            PrismaFieldAttribute::Id => {
                field_builder = field_builder.with_id();
            }
            PrismaFieldAttribute::Unique => {
                field_builder = field_builder.with_unique();
            }
            PrismaFieldAttribute::Default(default_val) => match default_val {
                PrismaDefaultValue::Function(ref func)
                    if func == "autoincrement()" || func == "autoincrement" =>
                {
                    // Prisma @default(autoincrement()) → Prax @auto
                    field_builder = field_builder.with_auto();
                }
                _ => {
                    let prax_default = convert_default_value(default_val);
                    field_builder = field_builder.with_default(prax_default);
                }
            },
            PrismaFieldAttribute::Map(_) => {
                // Prax field name is already the column name — no @map needed
            }
            PrismaFieldAttribute::UpdatedAt => {
                // Prisma @updatedAt → Prax @updated_at
                field_builder = field_builder.with_updated_at();
            }
            PrismaFieldAttribute::Relation {
                name,
                fields,
                references,
                on_delete,
                on_update,
                map,
            } => {
                if let (Some(fk_fields), Some(ref_fields)) = (fields, references) {
                    // Remap field references from camelCase to snake_case
                    let mapped_fields: Vec<String> = fk_fields
                        .into_iter()
                        .map(|f| remap_field_name(&f, field_name_map))
                        .collect();
                    let mapped_refs: Vec<String> = ref_fields
                        .into_iter()
                        .map(|f| remap_field_name(&f, field_name_map))
                        .collect();
                    field_builder = field_builder.with_relation(
                        name,
                        mapped_fields,
                        mapped_refs,
                        on_delete,
                        on_update,
                        map,
                    );
                }
            }
        }
    }

    Ok(field_builder.build())
}

/// Convert Prisma field type to Prax field type and modifier.
fn convert_field_type(
    field_type: &PrismaFieldType,
    is_optional: bool,
    is_list: bool,
) -> ImportResult<(FieldType, TypeModifier)> {
    let base_type = match field_type {
        PrismaFieldType::String => FieldType::Scalar(ScalarType::String),
        PrismaFieldType::Boolean => FieldType::Scalar(ScalarType::Boolean),
        PrismaFieldType::Int => FieldType::Scalar(ScalarType::Int),
        PrismaFieldType::BigInt => FieldType::Scalar(ScalarType::BigInt),
        PrismaFieldType::Float => FieldType::Scalar(ScalarType::Float),
        PrismaFieldType::Decimal => FieldType::Scalar(ScalarType::Decimal),
        PrismaFieldType::DateTime => FieldType::Scalar(ScalarType::DateTime),
        PrismaFieldType::Json => FieldType::Scalar(ScalarType::Json),
        PrismaFieldType::Bytes => FieldType::Scalar(ScalarType::Bytes),
        PrismaFieldType::Custom(name) => {
            if let Some(inner) = name.strip_prefix("Unsupported:") {
                // Unsupported types become FieldType::Unsupported
                FieldType::Unsupported(SmolStr::from(inner))
            } else {
                // Could be an enum or a relation
                FieldType::Model(SmolStr::from(name.as_str()))
            }
        }
    };

    let modifier = match (is_optional, is_list) {
        (true, true) => TypeModifier::OptionalList,
        (false, true) => TypeModifier::List,
        (true, false) => TypeModifier::Optional,
        (false, false) => TypeModifier::Required,
    };

    Ok((base_type, modifier))
}

/// Convert Prisma default value to Prax attribute value.
fn convert_default_value(default: PrismaDefaultValue) -> AttributeValue {
    match default {
        PrismaDefaultValue::Literal(val) => {
            // Try to parse as different types
            if val == "true" {
                AttributeValue::Boolean(true)
            } else if val == "false" {
                AttributeValue::Boolean(false)
            } else if let Ok(n) = val.parse::<i64>() {
                AttributeValue::Int(n)
            } else if let Ok(f) = val.parse::<f64>() {
                AttributeValue::Float(f)
            } else {
                AttributeValue::String(val)
            }
        }
        PrismaDefaultValue::Function(func) => {
            // Parse "funcName(args)" into name and arguments
            if let Some(paren_pos) = func.find('(') {
                let name = &func[..paren_pos];
                let args_str = func[paren_pos + 1..].trim_end_matches(')');
                let args = if args_str.is_empty() {
                    vec![]
                } else {
                    vec![AttributeValue::String(args_str.to_string())]
                };
                AttributeValue::Function(SmolStr::from(name), args)
            } else {
                AttributeValue::Function(SmolStr::from(func.as_str()), vec![])
            }
        }
    }
}

/// Convert a Prisma enum to a Prax enum.
fn convert_enum(enum_def: PrismaEnum) -> Enum {
    let mut prax_enum = Enum::new(Ident::new(&enum_def.name, dummy_span()), dummy_span());

    for variant_name in enum_def.values {
        let variant = EnumVariant {
            name: Ident::new(&variant_name, dummy_span()),
            attributes: vec![],
            documentation: None,
            span: dummy_span(),
        };
        prax_enum.variants.push(variant);
    }

    if let Some(doc) = enum_def.documentation {
        prax_enum.documentation = Some(Documentation::new(doc, dummy_span()));
    }

    prax_enum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_model() {
        let schema = r#"
        model User {
            id    Int    @id @default(autoincrement())
            email String @unique
            name  String?
        }
        "#;

        let result = parse_prisma_schema(schema);
        assert!(result.is_ok());

        let prisma_schema = result.unwrap();
        assert_eq!(prisma_schema.models.len(), 1);
        assert_eq!(prisma_schema.models[0].fields.len(), 3);
    }

    #[test]
    fn test_import_simple_model() {
        let schema = r#"
        model User {
            id    Int    @id @default(autoincrement())
            email String @unique
        }
        "#;

        let result = import_prisma_schema(schema);
        assert!(result.is_ok());

        let prax_schema = result.unwrap();
        assert_eq!(prax_schema.models.len(), 1);
    }
}
