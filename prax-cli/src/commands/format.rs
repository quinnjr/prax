//! `prax format` command - Format Prax schema file(s).
//!
//! Bypasses `prax_schema::load` deliberately: formatting is per-file and
//! syntactic, so cross-file merge/validation would only get in the way.
//!
//! Note: plain `//` line comments are not preserved — the schema parser
//! discards them as trivia, so only `///` documentation comments survive
//! a round-trip through the formatter.

use std::path::Path;

use crate::cli::FormatArgs;
use crate::config::SCHEMA_FILE_PATH;
use crate::error::{CliError, CliResult};
use crate::output::{self, success};

/// Run the format command
pub async fn run(args: FormatArgs) -> CliResult<()> {
    output::header("Format Schema");

    let cwd = std::env::current_dir()?;
    let schema_path = args.schema.unwrap_or_else(|| cwd.join(SCHEMA_FILE_PATH));

    if !schema_path.exists() {
        return Err(CliError::Config(format!(
            "Schema path not found: {}",
            schema_path.display()
        )));
    }

    output::kv("Schema", &schema_path.display().to_string());
    output::newline();

    let files: Vec<std::path::PathBuf> = if schema_path.is_dir() {
        let discovered = prax_schema::loader::discover(&schema_path).map_err(CliError::from)?;
        if discovered.is_empty() {
            return Err(CliError::Config(format!(
                "No .prax files found under {}",
                schema_path.display()
            )));
        }
        discovered.into_iter().map(|d| d.absolute).collect()
    } else {
        vec![schema_path.clone()]
    };

    let mut any_changed = false;
    let mut any_needs_format = false;
    for file in &files {
        match format_one(file, args.check)? {
            FormatOutcome::Unchanged => {}
            FormatOutcome::Reformatted => any_changed = true,
            FormatOutcome::NeedsFormatting => any_needs_format = true,
        }
    }

    output::newline();
    if args.check {
        if any_needs_format {
            output::error("Some schema files are not formatted correctly.");
            output::info("Run `prax format` to fix formatting.");
            return Err(CliError::Format(
                "One or more schema files need formatting".to_string(),
            ));
        }
        success(&format!(
            "All {} schema file(s) are formatted!",
            files.len()
        ));
    } else if any_changed {
        success(&format!("Formatted {} schema file(s).", files.len()));
    } else {
        success(&format!(
            "All {} schema file(s) are already formatted!",
            files.len()
        ));
    }

    Ok(())
}

enum FormatOutcome {
    Unchanged,
    Reformatted,
    NeedsFormatting,
}

fn format_one(path: &Path, check: bool) -> CliResult<FormatOutcome> {
    let content = std::fs::read_to_string(path)?;
    let schema = parse_schema(&content)?;
    let formatted = format_schema(&schema);
    let changed = formatted != content;

    if check {
        return Ok(if changed {
            output::error(&format!("Needs formatting: {}", path.display()));
            FormatOutcome::NeedsFormatting
        } else {
            FormatOutcome::Unchanged
        });
    }

    if changed {
        std::fs::write(path, &formatted)?;
        output::list_item(&format!("Formatted {}", path.display()));
        Ok(FormatOutcome::Reformatted)
    } else {
        Ok(FormatOutcome::Unchanged)
    }
}

fn parse_schema(content: &str) -> CliResult<prax_schema::Schema> {
    // Formatting is per-file and syntactic: parse only, without
    // validation, so files whose relations resolve only after a
    // multi-file merge still format cleanly. (Validation would also
    // resolve Model->Enum/Composite field types, but the formatter
    // renders FieldType::Model/Enum/Composite identically as the bare
    // name, so the output is unaffected.)
    prax_schema::parse_schema(content).map_err(|e| CliError::Schema(format!("Syntax error: {}", e)))
}

/// Format a schema AST into a formatted string
fn format_schema(schema: &prax_schema::ast::Schema) -> String {
    let mut output = String::new();
    let mut wrote_section = false;

    // Serialize the datasource declared in the schema, preserving the
    // actual provider/url instead of injecting a hardcoded default.
    if let Some(datasource) = &schema.datasource {
        format_datasource(&mut output, datasource);
        wrote_section = true;
    }

    // Serialize the generator blocks declared in the schema.
    for generator in schema.generators.values() {
        if wrote_section {
            output.push('\n');
        }
        format_generator(&mut output, generator);
        wrote_section = true;
    }

    // Format enums first (since they're used by models)
    for enum_def in schema.enums.values() {
        if wrote_section {
            output.push('\n');
        }
        format_enum(&mut output, enum_def);
        wrote_section = true;
    }

    // Format models
    for model in schema.models.values() {
        if wrote_section {
            output.push('\n');
        }
        format_model(&mut output, model);
        wrote_section = true;
    }

    // Format views
    for view in schema.views.values() {
        if wrote_section {
            output.push('\n');
        }
        format_view(&mut output, view);
        wrote_section = true;
    }

    // Format composite types
    for composite in schema.types.values() {
        if wrote_section {
            output.push('\n');
        }
        format_composite(&mut output, composite);
        wrote_section = true;
    }

    output
}

fn format_datasource(output: &mut String, datasource: &prax_schema::ast::Datasource) {
    output.push_str(&format!("datasource {} {{\n", datasource.name));
    output.push_str(&format!(
        "    provider = \"{}\"\n",
        datasource.provider.as_str()
    ));

    if let Some(url_env) = &datasource.url_env {
        output.push_str(&format!("    url      = env(\"{}\")\n", url_env));
    } else if let Some(url) = &datasource.url {
        output.push_str(&format!("    url      = \"{}\"\n", url));
    }

    if !datasource.extensions.is_empty() {
        let extensions: Vec<String> = datasource
            .extensions
            .iter()
            .map(|ext| {
                let mut args = Vec::new();
                if let Some(schema) = &ext.schema {
                    args.push(format!("schema: \"{}\"", schema));
                }
                if let Some(version) = &ext.version {
                    args.push(format!("version: \"{}\"", version));
                }
                if args.is_empty() {
                    ext.name().to_string()
                } else {
                    format!("{}({})", ext.name(), args.join(", "))
                }
            })
            .collect();
        output.push_str(&format!("    extensions = [{}]\n", extensions.join(", ")));
    }

    for (key, value) in &datasource.properties {
        // env("VAR") values are stored verbatim; re-emit them unquoted
        // so the formatted output stays parseable.
        if value.starts_with("env(") {
            output.push_str(&format!("    {} = {}\n", key, value));
        } else {
            output.push_str(&format!("    {} = \"{}\"\n", key, value));
        }
    }

    output.push_str("}\n");
}

fn format_generator(output: &mut String, generator: &prax_schema::ast::Generator) {
    use prax_schema::ast::{GeneratorToggle, GeneratorValue};

    output.push_str(&format!("generator {} {{\n", generator.name()));

    if let Some(provider) = &generator.provider {
        output.push_str(&format!("    provider = \"{}\"\n", provider));
    }

    if let Some(out) = &generator.output {
        output.push_str(&format!("    output   = \"{}\"\n", out));
    }

    match &generator.generate {
        GeneratorToggle::Always => {}
        GeneratorToggle::Never | GeneratorToggle::Literal(false) => {
            output.push_str("    generate = false\n");
        }
        GeneratorToggle::Literal(true) => {
            output.push_str("    generate = true\n");
        }
        GeneratorToggle::Env(var) => {
            output.push_str(&format!("    generate = env(\"{}\")\n", var));
        }
    }

    for (key, value) in &generator.properties {
        let formatted = match value {
            GeneratorValue::String(s) => format!("\"{}\"", s),
            GeneratorValue::Bool(b) => b.to_string(),
            GeneratorValue::Env(var) => format!("env(\"{}\")", var),
            GeneratorValue::Ident(s) => s.to_string(),
        };
        output.push_str(&format!("    {} = {}\n", key, formatted));
    }

    output.push_str("}\n");
}

fn format_enum(output: &mut String, enum_def: &prax_schema::ast::Enum) {
    // Documentation
    if let Some(doc) = &enum_def.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("enum {} {{\n", enum_def.name()));

    for variant in &enum_def.variants {
        // Documentation
        if let Some(doc) = &variant.documentation {
            for line in doc.text.lines() {
                output.push_str(&format!("    /// {}\n", line));
            }
        }

        output.push_str(&format!("    {}", variant.name()));

        // Format attributes
        for attr in &variant.attributes {
            output.push_str(&format!(" {}", format_attribute(attr, AttrLevel::Field)));
        }

        output.push('\n');
    }

    // Enum-level attributes
    for attr in &enum_def.attributes {
        output.push_str(&format!(
            "\n    {}",
            format_attribute(attr, AttrLevel::Block)
        ));
    }

    output.push_str("}\n");
}

fn format_model(output: &mut String, model: &prax_schema::ast::Model) {
    // Documentation
    if let Some(doc) = &model.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("model {} {{\n", model.name()));

    // Calculate alignment for fields
    let max_name_len = model
        .fields
        .values()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let max_type_len = model
        .fields
        .values()
        .map(|f| format_field_type(&f.field_type, f.modifier).len())
        .max()
        .unwrap_or(0);

    for field in model.fields.values() {
        // Documentation
        if let Some(doc) = &field.documentation {
            for line in doc.text.lines() {
                output.push_str(&format!("    /// {}\n", line));
            }
        }

        let type_str = format_field_type(&field.field_type, field.modifier);

        // Pad name and type for alignment
        let padded_name = format!("{:width$}", field.name(), width = max_name_len);
        let padded_type = format!("{:width$}", type_str, width = max_type_len);

        output.push_str(&format!("    {} {}", padded_name, padded_type));

        // Format attributes
        for attr in &field.attributes {
            output.push_str(&format!(" {}", format_attribute(attr, AttrLevel::Field)));
        }

        output.push('\n');
    }

    // Model-level attributes
    let model_attrs: Vec<_> = model.attributes.iter().collect();
    if !model_attrs.is_empty() {
        output.push('\n');
        for attr in model_attrs {
            output.push_str(&format!(
                "    {}\n",
                format_attribute(attr, AttrLevel::Block)
            ));
        }
    }

    output.push_str("}\n");
}

fn format_view(output: &mut String, view: &prax_schema::ast::View) {
    // Documentation
    if let Some(doc) = &view.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("view {} {{\n", view.name()));

    // Calculate alignment for fields
    let max_name_len = view
        .fields
        .values()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let max_type_len = view
        .fields
        .values()
        .map(|f| format_field_type(&f.field_type, f.modifier).len())
        .max()
        .unwrap_or(0);

    for field in view.fields.values() {
        let type_str = format_field_type(&field.field_type, field.modifier);
        let padded_name = format!("{:width$}", field.name(), width = max_name_len);
        let padded_type = format!("{:width$}", type_str, width = max_type_len);

        output.push_str(&format!("    {} {}", padded_name, padded_type));

        for attr in &field.attributes {
            output.push_str(&format!(" {}", format_attribute(attr, AttrLevel::Field)));
        }

        output.push('\n');
    }

    // View-level attributes
    let view_attrs: Vec<_> = view.attributes.iter().collect();
    if !view_attrs.is_empty() {
        output.push('\n');
        for attr in view_attrs {
            output.push_str(&format!(
                "    {}\n",
                format_attribute(attr, AttrLevel::Block)
            ));
        }
    }

    output.push_str("}\n");
}

fn format_composite(output: &mut String, composite: &prax_schema::ast::CompositeType) {
    // Documentation
    if let Some(doc) = &composite.documentation {
        for line in doc.text.lines() {
            output.push_str(&format!("/// {}\n", line));
        }
    }

    output.push_str(&format!("type {} {{\n", composite.name()));

    // Calculate alignment for fields
    let max_name_len = composite
        .fields
        .values()
        .map(|f| f.name().len())
        .max()
        .unwrap_or(0);

    let max_type_len = composite
        .fields
        .values()
        .map(|f| format_field_type(&f.field_type, f.modifier).len())
        .max()
        .unwrap_or(0);

    for field in composite.fields.values() {
        let type_str = format_field_type(&field.field_type, field.modifier);
        let padded_name = format!("{:width$}", field.name(), width = max_name_len);
        let padded_type = format!("{:width$}", type_str, width = max_type_len);

        output.push_str(&format!("    {} {}", padded_name, padded_type));

        for attr in &field.attributes {
            output.push_str(&format!(" {}", format_attribute(attr, AttrLevel::Field)));
        }

        output.push('\n');
    }

    output.push_str("}\n");
}

fn format_field_type(
    field_type: &prax_schema::ast::FieldType,
    modifier: prax_schema::ast::TypeModifier,
) -> String {
    use prax_schema::ast::{FieldType, ScalarType, TypeModifier};

    let base = match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int => "Int",
            ScalarType::BigInt => "BigInt",
            ScalarType::Float => "Float",
            ScalarType::String => "String",
            ScalarType::Boolean => "Boolean",
            ScalarType::DateTime => "DateTime",
            ScalarType::Date => "Date",
            ScalarType::Time => "Time",
            ScalarType::Json => "Json",
            ScalarType::Bytes => "Bytes",
            ScalarType::Decimal => "Decimal",
            ScalarType::Uuid => "Uuid",
            ScalarType::Cuid => "Cuid",
            ScalarType::Cuid2 => "Cuid2",
            ScalarType::NanoId => "NanoId",
            ScalarType::Ulid => "Ulid",
            ScalarType::Vector(_) => "Vector",
            ScalarType::HalfVector(_) => "HalfVector",
            ScalarType::SparseVector(_) => "SparseVector",
            ScalarType::Bit(_) => "Bit",
        }
        .to_string(),
        FieldType::Model(name) => name.to_string(),
        FieldType::Enum(name) => name.to_string(),
        FieldType::Composite(name) => name.to_string(),
        FieldType::Unsupported(name) => format!("Unsupported(\"{}\")", name),
    };

    match modifier {
        TypeModifier::Optional => format!("{}?", base),
        TypeModifier::List => format!("{}[]", base),
        TypeModifier::OptionalList => format!("{}[]?", base),
        TypeModifier::Required => base,
    }
}

/// Attribute position: selects the `@` vs `@@` prefix.
///
/// Several attributes (`id`, `unique`, `map`, `index`) are legal in both
/// positions, so the prefix must come from position, never from the name.
#[derive(Clone, Copy)]
enum AttrLevel {
    Field,
    Block,
}

fn format_attribute(attr: &prax_schema::ast::Attribute, level: AttrLevel) -> String {
    // The prefix comes from the attribute's position (block-level `@@`
    // vs field-level `@`), never from its name: name-based guessing
    // corrupts field-level `@id` into `@@id`.
    let prefix = match level {
        AttrLevel::Block => "@@",
        AttrLevel::Field => "@",
    };

    if attr.args.is_empty() {
        format!("{}{}", prefix, attr.name())
    } else {
        let args: Vec<String> = attr
            .args
            .iter()
            .map(|arg| {
                if let Some(name) = &arg.name {
                    format!("{}: {}", name.as_str(), format_attribute_value(&arg.value))
                } else {
                    format_attribute_value(&arg.value)
                }
            })
            .collect();

        format!("{}{}({})", prefix, attr.name(), args.join(", "))
    }
}

fn format_attribute_value(value: &prax_schema::ast::AttributeValue) -> String {
    use prax_schema::ast::AttributeValue;

    match value {
        AttributeValue::String(s) => format!("\"{}\"", s),
        AttributeValue::Int(i) => i.to_string(),
        AttributeValue::Float(f) => f.to_string(),
        AttributeValue::Boolean(b) => b.to_string(),
        AttributeValue::Ident(id) => id.to_string(),
        AttributeValue::Function(name, args) => {
            if args.is_empty() {
                format!("{}()", name)
            } else {
                let arg_strs: Vec<String> = args.iter().map(format_attribute_value).collect();
                format!("{}({})", name, arg_strs.join(", "))
            }
        }
        AttributeValue::Array(items) => {
            let item_strs: Vec<String> = items.iter().map(format_attribute_value).collect();
            format!("[{}]", item_strs.join(", "))
        }
        AttributeValue::FieldRef(field) => field.to_string(),
        AttributeValue::FieldRefList(fields) => {
            format!(
                "[{}]",
                fields
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}
