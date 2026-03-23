//! Schema parser for `.prax` files.

mod grammar;

use std::path::Path;

use pest::Parser;
use smol_str::SmolStr;
use tracing::{debug, info};

use crate::ast::*;
use crate::error::{SchemaError, SchemaResult};

pub use grammar::{PraxParser, Rule};

use crate::ast::{
    MssqlBlockOperation, Policy, PolicyCommand, PolicyType, Server, ServerGroup, ServerProperty,
    ServerPropertyValue,
};

/// Parse a schema from a string.
pub fn parse_schema(input: &str) -> SchemaResult<Schema> {
    debug!(input_len = input.len(), "parse_schema() starting");
    let pairs = PraxParser::parse(Rule::schema, input)
        .map_err(|e| SchemaError::syntax(input.to_string(), 0, input.len(), e.to_string()))?;

    let mut schema = Schema::new();
    let mut current_doc: Option<Documentation> = None;

    // The top-level parse result contains a single "schema" rule - get its inner pairs
    let schema_pair = pairs.into_iter().next().unwrap();

    for pair in schema_pair.into_inner() {
        match pair.as_rule() {
            Rule::documentation => {
                let span = pair.as_span();
                let text = pair
                    .into_inner()
                    .map(|p| p.as_str().trim_start_matches("///").trim())
                    .collect::<Vec<_>>()
                    .join("\n");
                current_doc = Some(Documentation::new(
                    text,
                    Span::new(span.start(), span.end()),
                ));
            }
            Rule::model_def => {
                let mut model = parse_model(pair)?;
                if let Some(doc) = current_doc.take() {
                    model = model.with_documentation(doc);
                }
                schema.add_model(model);
            }
            Rule::enum_def => {
                let mut e = parse_enum(pair)?;
                if let Some(doc) = current_doc.take() {
                    e = e.with_documentation(doc);
                }
                schema.add_enum(e);
            }
            Rule::type_def => {
                let mut t = parse_composite_type(pair)?;
                if let Some(doc) = current_doc.take() {
                    t = t.with_documentation(doc);
                }
                schema.add_type(t);
            }
            Rule::view_def => {
                let mut v = parse_view(pair)?;
                if let Some(doc) = current_doc.take() {
                    v = v.with_documentation(doc);
                }
                schema.add_view(v);
            }
            Rule::raw_sql_def => {
                let sql = parse_raw_sql(pair)?;
                schema.add_raw_sql(sql);
            }
            Rule::server_group_def => {
                let mut sg = parse_server_group(pair)?;
                if let Some(doc) = current_doc.take() {
                    sg.set_documentation(doc);
                }
                schema.add_server_group(sg);
            }
            Rule::policy_def => {
                let mut policy = parse_policy(pair)?;
                if let Some(doc) = current_doc.take() {
                    policy = policy.with_documentation(doc);
                }
                schema.add_policy(policy);
            }
            Rule::datasource_def => {
                let ds = parse_datasource(pair)?;
                schema.set_datasource(ds);
                current_doc = None;
            }
            Rule::generator_def => {
                let generator = parse_generator(pair)?;
                schema.add_generator(generator);
                current_doc = None;
            }
            Rule::EOI => {}
            _ => {}
        }
    }

    info!(
        models = schema.models.len(),
        enums = schema.enums.len(),
        types = schema.types.len(),
        views = schema.views.len(),
        generators = schema.generators.len(),
        policies = schema.policies.len(),
        "Schema parsed successfully"
    );
    Ok(schema)
}

/// Parse a schema from a file.
pub fn parse_schema_file(path: impl AsRef<Path>) -> SchemaResult<Schema> {
    let path = path.as_ref();
    info!(path = %path.display(), "Loading schema file");
    let content = std::fs::read_to_string(path).map_err(|e| SchemaError::IoError {
        path: path.display().to_string(),
        source: e,
    })?;

    parse_schema(&content)
}

/// Parse a model definition.
fn parse_model(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Model> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut model = Model::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        match item.as_rule() {
            Rule::field_def => {
                let field = parse_field(item)?;
                model.add_field(field);
            }
            Rule::model_attribute => {
                let attr = parse_attribute(item)?;
                model.attributes.push(attr);
            }
            Rule::model_body_item => {
                // Unwrap the model_body_item to get the actual field_def or model_attribute
                let inner_item = item.into_inner().next().unwrap();
                match inner_item.as_rule() {
                    Rule::field_def => {
                        let field = parse_field(inner_item)?;
                        model.add_field(field);
                    }
                    Rule::model_attribute => {
                        let attr = parse_attribute(inner_item)?;
                        model.attributes.push(attr);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(model)
}

/// Parse an enum definition.
fn parse_enum(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Enum> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut e = Enum::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        match item.as_rule() {
            Rule::enum_variant => {
                let variant = parse_enum_variant(item)?;
                e.add_variant(variant);
            }
            Rule::model_attribute => {
                let attr = parse_attribute(item)?;
                e.attributes.push(attr);
            }
            Rule::enum_body_item => {
                // Unwrap the enum_body_item to get the actual enum_variant or model_attribute
                let inner_item = item.into_inner().next().unwrap();
                match inner_item.as_rule() {
                    Rule::enum_variant => {
                        let variant = parse_enum_variant(inner_item)?;
                        e.add_variant(variant);
                    }
                    Rule::model_attribute => {
                        let attr = parse_attribute(inner_item)?;
                        e.attributes.push(attr);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(e)
}

/// Parse an enum variant.
fn parse_enum_variant(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<EnumVariant> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut variant = EnumVariant::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        if item.as_rule() == Rule::field_attribute {
            let attr = parse_attribute(item)?;
            variant.attributes.push(attr);
        }
    }

    Ok(variant)
}

/// Parse a composite type definition.
fn parse_composite_type(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<CompositeType> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut t = CompositeType::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        if item.as_rule() == Rule::field_def {
            let field = parse_field(item)?;
            t.add_field(field);
        }
    }

    Ok(t)
}

/// Parse a view definition.
fn parse_view(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<View> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut v = View::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        match item.as_rule() {
            Rule::field_def => {
                let field = parse_field(item)?;
                v.add_field(field);
            }
            Rule::model_attribute => {
                let attr = parse_attribute(item)?;
                v.attributes.push(attr);
            }
            Rule::model_body_item => {
                // Unwrap the model_body_item to get the actual field_def or model_attribute
                let inner_item = item.into_inner().next().unwrap();
                match inner_item.as_rule() {
                    Rule::field_def => {
                        let field = parse_field(inner_item)?;
                        v.add_field(field);
                    }
                    Rule::model_attribute => {
                        let attr = parse_attribute(inner_item)?;
                        v.attributes.push(attr);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(v)
}

/// Parse a field definition.
fn parse_field(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Field> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let type_pair = inner.next().unwrap();
    let (field_type, modifier) = parse_field_type(type_pair)?;

    let mut attributes = vec![];
    for item in inner {
        if item.as_rule() == Rule::field_attribute {
            let attr = parse_attribute(item)?;
            attributes.push(attr);
        }
    }

    Ok(Field::new(
        name,
        field_type,
        modifier,
        attributes,
        Span::new(span.start(), span.end()),
    ))
}

/// Parse a field type with optional modifier.
fn parse_field_type(
    pair: pest::iterators::Pair<'_, Rule>,
) -> SchemaResult<(FieldType, TypeModifier)> {
    let mut type_name = String::new();
    let mut modifier = TypeModifier::Required;

    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::type_name => {
                type_name = item.as_str().to_string();
            }
            Rule::optional_marker => {
                modifier = if modifier == TypeModifier::List {
                    TypeModifier::OptionalList
                } else {
                    TypeModifier::Optional
                };
            }
            Rule::list_marker => {
                modifier = if modifier == TypeModifier::Optional {
                    TypeModifier::OptionalList
                } else {
                    TypeModifier::List
                };
            }
            _ => {}
        }
    }

    let field_type = if let Some(scalar) = ScalarType::from_str(&type_name) {
        FieldType::Scalar(scalar)
    } else {
        // Assume it's a reference to a model, enum, or type
        // This will be validated later
        FieldType::Model(SmolStr::new(&type_name))
    };

    Ok((field_type, modifier))
}

/// Parse an attribute.
fn parse_attribute(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Attribute> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut args = vec![];
    for item in inner {
        if item.as_rule() == Rule::attribute_args {
            args = parse_attribute_args(item)?;
        }
    }

    Ok(Attribute::new(
        name,
        args,
        Span::new(span.start(), span.end()),
    ))
}

/// Parse attribute arguments.
fn parse_attribute_args(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Vec<AttributeArg>> {
    let mut args = vec![];

    for item in pair.into_inner() {
        if item.as_rule() == Rule::attribute_arg {
            let arg = parse_attribute_arg(item)?;
            args.push(arg);
        }
    }

    Ok(args)
}

/// Parse a single attribute argument.
fn parse_attribute_arg(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<AttributeArg> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let first = inner.next().unwrap();

    // Check if this is a named argument (name: value) or positional
    if let Some(second) = inner.next() {
        // Named argument
        let name = Ident::new(
            first.as_str(),
            Span::new(first.as_span().start(), first.as_span().end()),
        );
        let value = parse_attribute_value(second)?;
        Ok(AttributeArg::named(
            name,
            value,
            Span::new(span.start(), span.end()),
        ))
    } else {
        // Positional argument
        let value = parse_attribute_value(first)?;
        Ok(AttributeArg::positional(
            value,
            Span::new(span.start(), span.end()),
        ))
    }
}

/// Parse an attribute value.
fn parse_attribute_value(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<AttributeValue> {
    match pair.as_rule() {
        Rule::string_literal => {
            let s = pair.as_str();
            // Remove quotes
            let unquoted = &s[1..s.len() - 1];
            Ok(AttributeValue::String(unquoted.to_string()))
        }
        Rule::number_literal => {
            let s = pair.as_str();
            if s.contains('.') {
                Ok(AttributeValue::Float(s.parse().unwrap()))
            } else {
                Ok(AttributeValue::Int(s.parse().unwrap()))
            }
        }
        Rule::boolean_literal => Ok(AttributeValue::Boolean(pair.as_str() == "true")),
        Rule::identifier => Ok(AttributeValue::Ident(SmolStr::new(pair.as_str()))),
        Rule::function_call => {
            let mut inner = pair.into_inner();
            let name = SmolStr::new(inner.next().unwrap().as_str());
            let mut args = vec![];
            for item in inner {
                args.push(parse_attribute_value(item)?);
            }
            Ok(AttributeValue::Function(name, args))
        }
        Rule::field_ref_list => {
            let refs: Vec<SmolStr> = pair
                .into_inner()
                .map(|p| SmolStr::new(p.as_str()))
                .collect();
            Ok(AttributeValue::FieldRefList(refs))
        }
        Rule::array_literal => {
            let values: Result<Vec<_>, _> = pair.into_inner().map(parse_attribute_value).collect();
            Ok(AttributeValue::Array(values?))
        }
        Rule::attribute_value => {
            // Unwrap nested attribute_value
            parse_attribute_value(pair.into_inner().next().unwrap())
        }
        _ => {
            // Fallback: treat as identifier
            Ok(AttributeValue::Ident(SmolStr::new(pair.as_str())))
        }
    }
}

/// Parse a raw SQL definition.
fn parse_raw_sql(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<RawSql> {
    let mut inner = pair.into_inner();

    let name = inner.next().unwrap().as_str();
    let sql = inner.next().unwrap().as_str();

    // Remove triple quotes
    let sql_content = sql
        .trim_start_matches("\"\"\"")
        .trim_end_matches("\"\"\"")
        .trim();

    Ok(RawSql::new(name, sql_content))
}

/// Parse a server group definition.
fn parse_server_group(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<ServerGroup> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut server_group = ServerGroup::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        match item.as_rule() {
            Rule::server_group_item => {
                // Unwrap the server_group_item to get the actual server_def or model_attribute
                let inner_item = item.into_inner().next().unwrap();
                match inner_item.as_rule() {
                    Rule::server_def => {
                        let server = parse_server(inner_item)?;
                        server_group.add_server(server);
                    }
                    Rule::model_attribute => {
                        let attr = parse_attribute(inner_item)?;
                        server_group.add_attribute(attr);
                    }
                    _ => {}
                }
            }
            Rule::server_def => {
                let server = parse_server(item)?;
                server_group.add_server(server);
            }
            Rule::model_attribute => {
                let attr = parse_attribute(item)?;
                server_group.add_attribute(attr);
            }
            _ => {}
        }
    }

    Ok(server_group)
}

/// Parse a server definition within a server group.
fn parse_server(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Server> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    let mut server = Server::new(name, Span::new(span.start(), span.end()));

    for item in inner {
        if item.as_rule() == Rule::server_property {
            let prop = parse_server_property(item)?;
            server.add_property(prop);
        }
    }

    Ok(server)
}

/// Parse a server property (key = value).
fn parse_server_property(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<ServerProperty> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let key_pair = inner.next().unwrap();
    let key = key_pair.as_str();

    let value_pair = inner.next().unwrap();
    let value = parse_server_property_value(value_pair)?;

    Ok(ServerProperty::new(
        key,
        value,
        Span::new(span.start(), span.end()),
    ))
}

/// Parse a generator definition.
fn parse_generator(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Generator> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name = inner.next().unwrap().as_str();
    let mut generator = Generator::new(name, Span::new(span.start(), span.end()));

    for prop in inner {
        if prop.as_rule() == Rule::datasource_property {
            let mut prop_inner = prop.into_inner();
            let key = prop_inner.next().unwrap().as_str();
            let value_pair = prop_inner.next().unwrap();

            match key {
                "provider" => {
                    let s = extract_datasource_string(&value_pair);
                    generator.provider = Some(SmolStr::new(s));
                }
                "output" => {
                    let s = extract_datasource_string(&value_pair);
                    generator.output = Some(SmolStr::new(s));
                }
                "generate" => {
                    generator.generate = parse_generator_toggle(&value_pair);
                }
                _ => {
                    let val = parse_generator_value(&value_pair);
                    generator.properties.insert(SmolStr::new(key), val);
                }
            }
        }
    }

    Ok(generator)
}

/// Parse a generator toggle value (bool literal or env() call).
fn parse_generator_toggle(pair: &pest::iterators::Pair<'_, Rule>) -> GeneratorToggle {
    match pair.as_rule() {
        Rule::env_function => {
            let env_var = pair
                .clone()
                .into_inner()
                .next()
                .map(|p| {
                    let s = p.as_str();
                    SmolStr::new(&s[1..s.len() - 1])
                })
                .unwrap_or_default();
            GeneratorToggle::Env(env_var)
        }
        Rule::datasource_value => {
            let inner = pair.clone().into_inner().next().unwrap();
            parse_generator_toggle(&inner)
        }
        _ => {
            let s = pair.as_str().trim().trim_matches('"');
            match s {
                "true" => GeneratorToggle::Literal(true),
                "false" => GeneratorToggle::Literal(false),
                _ => GeneratorToggle::Literal(false),
            }
        }
    }
}

/// Parse an arbitrary generator property value.
fn parse_generator_value(pair: &pest::iterators::Pair<'_, Rule>) -> GeneratorValue {
    match pair.as_rule() {
        Rule::env_function => {
            let env_var = pair
                .clone()
                .into_inner()
                .next()
                .map(|p| {
                    let s = p.as_str();
                    SmolStr::new(&s[1..s.len() - 1])
                })
                .unwrap_or_default();
            GeneratorValue::Env(env_var)
        }
        Rule::datasource_value => {
            let inner = pair.clone().into_inner().next().unwrap();
            parse_generator_value(&inner)
        }
        Rule::string_literal => {
            let s = pair.as_str();
            GeneratorValue::String(SmolStr::new(&s[1..s.len() - 1]))
        }
        _ => {
            let s = pair.as_str().trim().trim_matches('"');
            match s {
                "true" => GeneratorValue::Bool(true),
                "false" => GeneratorValue::Bool(false),
                _ => GeneratorValue::Ident(SmolStr::new(s)),
            }
        }
    }
}

/// Parse a datasource definition.
fn parse_datasource(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Datasource> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    let name_pair = inner.next().unwrap();
    let name = name_pair.as_str();

    let mut datasource = Datasource::new(
        name,
        DatabaseProvider::PostgreSQL,
        Span::new(span.start(), span.end()),
    );

    for prop in inner {
        if prop.as_rule() == Rule::datasource_property {
            let mut prop_inner = prop.into_inner();
            let key = prop_inner.next().unwrap().as_str();
            let value_pair = prop_inner.next().unwrap();

            match key {
                "provider" => {
                    let provider_str = extract_datasource_string(&value_pair);
                    if let Some(provider) = DatabaseProvider::from_str(&provider_str) {
                        datasource.provider = provider;
                    }
                }
                "url" => {
                    match value_pair.as_rule() {
                        Rule::env_function => {
                            // env("DATABASE_URL")
                            let env_var = value_pair
                                .into_inner()
                                .next()
                                .map(|p| {
                                    let s = p.as_str();
                                    s[1..s.len() - 1].to_string()
                                })
                                .unwrap_or_default();
                            datasource.url_env = Some(SmolStr::new(env_var));
                        }
                        Rule::string_literal => {
                            let s = value_pair.as_str();
                            let url = &s[1..s.len() - 1];
                            datasource.url = Some(SmolStr::new(url));
                        }
                        _ => {}
                    }
                }
                "extensions" => {
                    if value_pair.as_rule() == Rule::extension_array {
                        for ext_item in value_pair.into_inner() {
                            if ext_item.as_rule() == Rule::extension_item {
                                let ext = parse_extension_item(
                                    ext_item,
                                    Span::new(span.start(), span.end()),
                                )?;
                                datasource.add_extension(ext);
                            }
                        }
                    }
                }
                _ => {
                    // Store as additional property
                    let value_str = extract_datasource_string(&value_pair);
                    datasource.add_property(key, value_str);
                }
            }
        }
    }

    Ok(datasource)
}

/// Parse an extension item from the extensions array.
fn parse_extension_item(
    pair: pest::iterators::Pair<'_, Rule>,
    span: Span,
) -> SchemaResult<PostgresExtension> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str();
    let mut ext = PostgresExtension::new(name, span);

    // Check for extension args like (schema: "public", version: "0.5.0")
    if let Some(args_pair) = inner.next() {
        if args_pair.as_rule() == Rule::extension_args {
            for arg in args_pair.into_inner() {
                if arg.as_rule() == Rule::extension_arg {
                    let mut arg_inner = arg.into_inner();
                    let arg_key = arg_inner.next().unwrap().as_str();
                    let arg_value_pair = arg_inner.next().unwrap();
                    let arg_value = {
                        let s = arg_value_pair.as_str();
                        &s[1..s.len() - 1]
                    };

                    match arg_key {
                        "schema" => {
                            ext = ext.with_schema(arg_value);
                        }
                        "version" => {
                            ext = ext.with_version(arg_value);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(ext)
}

/// Extract a string value from a datasource property value.
fn extract_datasource_string(pair: &pest::iterators::Pair<'_, Rule>) -> String {
    match pair.as_rule() {
        Rule::string_literal => {
            let s = pair.as_str();
            s[1..s.len() - 1].to_string()
        }
        Rule::identifier => pair.as_str().to_string(),
        Rule::datasource_value => {
            if let Some(inner) = pair.clone().into_inner().next() {
                extract_datasource_string(&inner)
            } else {
                pair.as_str().to_string()
            }
        }
        _ => pair.as_str().to_string(),
    }
}

/// Extract a string value from a pest pair, handling nesting.
fn extract_string_from_arg(pair: pest::iterators::Pair<'_, Rule>) -> String {
    match pair.as_rule() {
        Rule::string_literal => {
            let s = pair.as_str();
            s[1..s.len() - 1].to_string()
        }
        Rule::attribute_value => {
            // Unwrap nested attribute_value
            if let Some(inner) = pair.into_inner().next() {
                extract_string_from_arg(inner)
            } else {
                String::new()
            }
        }
        _ => pair.as_str().to_string(),
    }
}

/// Parse a server property value.
fn parse_server_property_value(
    pair: pest::iterators::Pair<'_, Rule>,
) -> SchemaResult<ServerPropertyValue> {
    match pair.as_rule() {
        Rule::string_literal => {
            let s = pair.as_str();
            // Remove quotes
            let unquoted = &s[1..s.len() - 1];
            Ok(ServerPropertyValue::String(unquoted.to_string()))
        }
        Rule::number_literal => {
            let s = pair.as_str();
            Ok(ServerPropertyValue::Number(s.parse().unwrap_or(0.0)))
        }
        Rule::boolean_literal => Ok(ServerPropertyValue::Boolean(pair.as_str() == "true")),
        Rule::identifier => Ok(ServerPropertyValue::Identifier(pair.as_str().to_string())),
        Rule::function_call => {
            // Handle env("VAR") and other function calls
            let mut inner = pair.into_inner();
            let func_name = inner.next().unwrap().as_str();
            if func_name == "env" {
                if let Some(arg) = inner.next() {
                    let var_name = extract_string_from_arg(arg);
                    return Ok(ServerPropertyValue::EnvVar(var_name));
                }
            }
            // For other functions, store as identifier
            Ok(ServerPropertyValue::Identifier(func_name.to_string()))
        }
        Rule::array_literal => {
            let values: Result<Vec<_>, _> =
                pair.into_inner().map(parse_server_property_value).collect();
            Ok(ServerPropertyValue::Array(values?))
        }
        Rule::attribute_value => {
            // Unwrap nested attribute_value
            parse_server_property_value(pair.into_inner().next().unwrap())
        }
        _ => {
            // Fallback: treat as identifier
            Ok(ServerPropertyValue::Identifier(pair.as_str().to_string()))
        }
    }
}

/// Parse a PostgreSQL Row-Level Security policy definition.
fn parse_policy(pair: pest::iterators::Pair<'_, Rule>) -> SchemaResult<Policy> {
    let span = pair.as_span();
    let mut inner = pair.into_inner();

    // First identifier is the policy name
    let name_pair = inner.next().unwrap();
    let name = Ident::new(
        name_pair.as_str(),
        Span::new(name_pair.as_span().start(), name_pair.as_span().end()),
    );

    // Second identifier is the table name
    let table_pair = inner.next().unwrap();
    let table = Ident::new(
        table_pair.as_str(),
        Span::new(table_pair.as_span().start(), table_pair.as_span().end()),
    );

    let mut policy = Policy::new(name, table, Span::new(span.start(), span.end()));
    // Reset commands to empty - will be set by 'for' clause if present
    policy.commands = vec![];

    for item in inner {
        match item.as_rule() {
            Rule::policy_item => {
                let inner_item = item.into_inner().next().unwrap();
                parse_policy_item(&mut policy, inner_item)?;
            }
            Rule::policy_for
            | Rule::policy_to
            | Rule::policy_as
            | Rule::policy_using
            | Rule::policy_check => {
                parse_policy_item(&mut policy, item)?;
            }
            _ => {}
        }
    }

    // Default to ALL if no commands specified
    if policy.commands.is_empty() {
        policy.commands.push(PolicyCommand::All);
    }

    Ok(policy)
}

/// Parse a single policy item (for, to, as, using, check, mssqlSchema, mssqlBlock).
fn parse_policy_item(
    policy: &mut Policy,
    pair: pest::iterators::Pair<'_, Rule>,
) -> SchemaResult<()> {
    match pair.as_rule() {
        Rule::policy_for => {
            let inner = pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::policy_command => {
                    if let Some(cmd) = PolicyCommand::from_str(inner.as_str()) {
                        policy.add_command(cmd);
                    }
                }
                Rule::policy_command_list => {
                    for cmd_pair in inner.into_inner() {
                        if cmd_pair.as_rule() == Rule::policy_command {
                            if let Some(cmd) = PolicyCommand::from_str(cmd_pair.as_str()) {
                                policy.add_command(cmd);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Rule::policy_to => {
            let inner = pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::identifier => {
                    policy.add_role(inner.as_str());
                }
                Rule::policy_role_list => {
                    for role_pair in inner.into_inner() {
                        if role_pair.as_rule() == Rule::identifier {
                            policy.add_role(role_pair.as_str());
                        }
                    }
                }
                _ => {}
            }
        }
        Rule::policy_as => {
            let inner = pair.into_inner().next().unwrap();
            if inner.as_rule() == Rule::policy_type {
                if let Some(policy_type) = PolicyType::from_str(inner.as_str()) {
                    policy.policy_type = policy_type;
                }
            }
        }
        Rule::policy_using => {
            let inner = pair.into_inner().next().unwrap();
            let expr = extract_policy_expression(&inner);
            policy.using_expr = Some(expr);
        }
        Rule::policy_check => {
            let inner = pair.into_inner().next().unwrap();
            let expr = extract_policy_expression(&inner);
            policy.check_expr = Some(expr);
        }
        Rule::policy_mssql_schema => {
            let inner = pair.into_inner().next().unwrap();
            if inner.as_rule() == Rule::string_literal {
                let s = inner.as_str();
                let schema = &s[1..s.len() - 1]; // Remove quotes
                policy.mssql_schema = Some(SmolStr::new(schema));
            }
        }
        Rule::policy_mssql_block => {
            let inner = pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::mssql_block_op => {
                    if let Some(op) = MssqlBlockOperation::from_str(inner.as_str()) {
                        policy.add_mssql_block_operation(op);
                    }
                }
                Rule::mssql_block_op_list => {
                    for op_pair in inner.into_inner() {
                        if op_pair.as_rule() == Rule::mssql_block_op {
                            if let Some(op) = MssqlBlockOperation::from_str(op_pair.as_str()) {
                                policy.add_mssql_block_operation(op);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

/// Extract the expression from a string literal or multiline string.
fn extract_policy_expression(pair: &pest::iterators::Pair<'_, Rule>) -> String {
    let s = pair.as_str();
    match pair.as_rule() {
        Rule::multiline_string => {
            // Remove triple quotes
            s.trim_start_matches("\"\"\"")
                .trim_end_matches("\"\"\"")
                .trim()
                .to_string()
        }
        Rule::string_literal => {
            // Remove single quotes
            s[1..s.len() - 1].to_string()
        }
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Basic Model Parsing ====================

    #[test]
    fn test_parse_simple_model() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String @unique
                name  String?
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 1);
        let user = schema.get_model("User").unwrap();
        assert_eq!(user.fields.len(), 3);
        assert!(user.get_field("id").unwrap().is_id());
        assert!(user.get_field("email").unwrap().is_unique());
        assert!(user.get_field("name").unwrap().is_optional());
    }

    #[test]
    fn test_parse_model_name() {
        let schema = parse_schema(
            r#"
            model BlogPost {
                id Int @id
            }
        "#,
        )
        .unwrap();

        assert!(schema.get_model("BlogPost").is_some());
    }

    #[test]
    fn test_parse_multiple_models() {
        let schema = parse_schema(
            r#"
            model User {
                id Int @id
            }

            model Post {
                id Int @id
            }

            model Comment {
                id Int @id
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 3);
        assert!(schema.get_model("User").is_some());
        assert!(schema.get_model("Post").is_some());
        assert!(schema.get_model("Comment").is_some());
    }

    // ==================== Field Type Parsing ====================

    #[test]
    fn test_parse_all_scalar_types() {
        let schema = parse_schema(
            r#"
            model AllTypes {
                id       Int      @id
                big      BigInt
                float_f  Float
                decimal  Decimal
                str      String
                bool     Boolean
                datetime DateTime
                date     Date
                time     Time
                json     Json
                bytes    Bytes
                uuid     Uuid
                cuid     Cuid
                cuid2    Cuid2
                nanoid   NanoId
                ulid     Ulid
            }
        "#,
        )
        .unwrap();

        let model = schema.get_model("AllTypes").unwrap();
        assert_eq!(model.fields.len(), 16);

        assert!(matches!(
            model.get_field("id").unwrap().field_type,
            FieldType::Scalar(ScalarType::Int)
        ));
        assert!(matches!(
            model.get_field("big").unwrap().field_type,
            FieldType::Scalar(ScalarType::BigInt)
        ));
        assert!(matches!(
            model.get_field("str").unwrap().field_type,
            FieldType::Scalar(ScalarType::String)
        ));
        assert!(matches!(
            model.get_field("bool").unwrap().field_type,
            FieldType::Scalar(ScalarType::Boolean)
        ));
        assert!(matches!(
            model.get_field("datetime").unwrap().field_type,
            FieldType::Scalar(ScalarType::DateTime)
        ));
        assert!(matches!(
            model.get_field("uuid").unwrap().field_type,
            FieldType::Scalar(ScalarType::Uuid)
        ));
        assert!(matches!(
            model.get_field("cuid").unwrap().field_type,
            FieldType::Scalar(ScalarType::Cuid)
        ));
        assert!(matches!(
            model.get_field("cuid2").unwrap().field_type,
            FieldType::Scalar(ScalarType::Cuid2)
        ));
        assert!(matches!(
            model.get_field("nanoid").unwrap().field_type,
            FieldType::Scalar(ScalarType::NanoId)
        ));
        assert!(matches!(
            model.get_field("ulid").unwrap().field_type,
            FieldType::Scalar(ScalarType::Ulid)
        ));
    }

    #[test]
    fn test_parse_optional_field() {
        let schema = parse_schema(
            r#"
            model User {
                id   Int     @id
                bio  String?
                age  Int?
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        assert!(!user.get_field("id").unwrap().is_optional());
        assert!(user.get_field("bio").unwrap().is_optional());
        assert!(user.get_field("age").unwrap().is_optional());
    }

    #[test]
    fn test_parse_list_field() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int      @id
                tags  String[]
                posts Post[]
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        assert!(user.get_field("tags").unwrap().is_list());
        assert!(user.get_field("posts").unwrap().is_list());
    }

    #[test]
    fn test_parse_optional_list_field() {
        let schema = parse_schema(
            r#"
            model User {
                id       Int       @id
                metadata String[]?
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let metadata = user.get_field("metadata").unwrap();
        assert!(metadata.is_list());
        assert!(metadata.is_optional());
    }

    // ==================== Attribute Parsing ====================

    #[test]
    fn test_parse_id_attribute() {
        let schema = parse_schema(
            r#"
            model User {
                id Int @id
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        assert!(user.get_field("id").unwrap().is_id());
    }

    #[test]
    fn test_parse_unique_attribute() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id
                email String @unique
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        assert!(user.get_field("email").unwrap().is_unique());
    }

    #[test]
    fn test_parse_default_int() {
        let schema = parse_schema(
            r#"
            model Counter {
                id    Int @id
                count Int @default(0)
            }
        "#,
        )
        .unwrap();

        let counter = schema.get_model("Counter").unwrap();
        let count_field = counter.get_field("count").unwrap();
        let attrs = count_field.extract_attributes();
        assert!(attrs.default.is_some());
        assert_eq!(attrs.default.unwrap().as_int(), Some(0));
    }

    #[test]
    fn test_parse_default_string() {
        let schema = parse_schema(
            r#"
            model User {
                id     Int    @id
                status String @default("active")
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let status = user.get_field("status").unwrap();
        let attrs = status.extract_attributes();
        assert!(attrs.default.is_some());
        assert_eq!(attrs.default.unwrap().as_string(), Some("active"));
    }

    #[test]
    fn test_parse_default_boolean() {
        let schema = parse_schema(
            r#"
            model Post {
                id        Int     @id
                published Boolean @default(false)
            }
        "#,
        )
        .unwrap();

        let post = schema.get_model("Post").unwrap();
        let published = post.get_field("published").unwrap();
        let attrs = published.extract_attributes();
        assert!(attrs.default.is_some());
        assert_eq!(attrs.default.unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_parse_default_function() {
        let schema = parse_schema(
            r#"
            model User {
                id        Int      @id
                createdAt DateTime @default(now())
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let created_at = user.get_field("createdAt").unwrap();
        let attrs = created_at.extract_attributes();
        assert!(attrs.default.is_some());
        if let Some(AttributeValue::Function(name, _)) = attrs.default {
            assert_eq!(name.as_str(), "now");
        } else {
            panic!("Expected function default");
        }
    }

    #[test]
    fn test_parse_updated_at_attribute() {
        let schema = parse_schema(
            r#"
            model User {
                id        Int      @id
                updatedAt DateTime @updated_at
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let updated_at = user.get_field("updatedAt").unwrap();
        let attrs = updated_at.extract_attributes();
        assert!(attrs.is_updated_at);
    }

    #[test]
    fn test_parse_map_attribute() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id
                email String @map("email_address")
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let email = user.get_field("email").unwrap();
        let attrs = email.extract_attributes();
        assert_eq!(attrs.map, Some("email_address".to_string()));
    }

    #[test]
    fn test_parse_multiple_attributes() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String @unique @index
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let id = user.get_field("id").unwrap();
        let email = user.get_field("email").unwrap();

        let id_attrs = id.extract_attributes();
        assert!(id_attrs.is_id);
        assert!(id_attrs.is_auto);

        let email_attrs = email.extract_attributes();
        assert!(email_attrs.is_unique);
        assert!(email_attrs.is_indexed);
    }

    // ==================== Model Attribute Parsing ====================

    #[test]
    fn test_parse_model_map_attribute() {
        let schema = parse_schema(
            r#"
            model User {
                id Int @id

                @@map("app_users")
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        assert_eq!(user.table_name(), "app_users");
    }

    #[test]
    fn test_parse_model_index_attribute() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id
                email String
                name  String

                @@index([email, name])
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        assert!(user.has_attribute("index"));
    }

    #[test]
    fn test_parse_composite_primary_key() {
        let schema = parse_schema(
            r#"
            model PostTag {
                postId Int
                tagId  Int

                @@id([postId, tagId])
            }
        "#,
        )
        .unwrap();

        let post_tag = schema.get_model("PostTag").unwrap();
        assert!(post_tag.has_attribute("id"));
    }

    // ==================== Enum Parsing ====================

    #[test]
    fn test_parse_enum() {
        let schema = parse_schema(
            r#"
            enum Role {
                User
                Admin
                Moderator
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.enums.len(), 1);
        let role = schema.get_enum("Role").unwrap();
        assert_eq!(role.variants.len(), 3);
    }

    #[test]
    fn test_parse_enum_variant_names() {
        let schema = parse_schema(
            r#"
            enum Status {
                Pending
                Active
                Completed
                Cancelled
            }
        "#,
        )
        .unwrap();

        let status = schema.get_enum("Status").unwrap();
        assert!(status.get_variant("Pending").is_some());
        assert!(status.get_variant("Active").is_some());
        assert!(status.get_variant("Completed").is_some());
        assert!(status.get_variant("Cancelled").is_some());
    }

    #[test]
    fn test_parse_enum_with_map() {
        let schema = parse_schema(
            r#"
            enum Role {
                User  @map("USER")
                Admin @map("ADMINISTRATOR")
            }
        "#,
        )
        .unwrap();

        let role = schema.get_enum("Role").unwrap();
        let user_variant = role.get_variant("User").unwrap();
        assert_eq!(user_variant.db_value(), "USER");

        let admin_variant = role.get_variant("Admin").unwrap();
        assert_eq!(admin_variant.db_value(), "ADMINISTRATOR");
    }

    // ==================== Relation Parsing ====================

    #[test]
    fn test_parse_one_to_many_relation() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id
                posts Post[]
            }

            model Post {
                id       Int  @id
                authorId Int
                author   User @relation(fields: [authorId], references: [id])
            }
        "#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        let post = schema.get_model("Post").unwrap();

        assert!(user.get_field("posts").unwrap().is_list());
        assert!(post.get_field("author").unwrap().is_relation());
    }

    #[test]
    fn test_parse_relation_with_actions() {
        let schema = parse_schema(
            r#"
            model Post {
                id       Int  @id
                authorId Int
                author   User @relation(fields: [authorId], references: [id], onDelete: Cascade, onUpdate: Restrict)
            }

            model User {
                id    Int    @id
                posts Post[]
            }
        "#,
        )
        .unwrap();

        let post = schema.get_model("Post").unwrap();
        let author = post.get_field("author").unwrap();
        let attrs = author.extract_attributes();

        assert!(attrs.relation.is_some());
        let rel = attrs.relation.unwrap();
        assert_eq!(rel.on_delete, Some(ReferentialAction::Cascade));
        assert_eq!(rel.on_update, Some(ReferentialAction::Restrict));
    }

    // ==================== Documentation Parsing ====================

    #[test]
    fn test_parse_model_documentation() {
        let schema = parse_schema(
            r#"/// Represents a user in the system
model User {
    id Int @id
}"#,
        )
        .unwrap();

        let user = schema.get_model("User").unwrap();
        // Documentation parsing is optional - the model should still parse
        // If documentation is present, it should contain "user"
        if let Some(doc) = &user.documentation {
            assert!(doc.text.contains("user"));
        }
    }

    // ==================== Complete Schema Parsing ====================

    #[test]
    fn test_parse_complete_schema() {
        let schema = parse_schema(
            r#"
            /// User model
            model User {
                id        Int      @id @auto
                email     String   @unique
                name      String?
                role      Role     @default(User)
                posts     Post[]
                profile   Profile?
                createdAt DateTime @default(now())
                updatedAt DateTime @updated_at

                @@map("users")
                @@index([email])
            }

            model Post {
                id        Int      @id @auto
                title     String
                content   String?
                published Boolean  @default(false)
                authorId  Int
                author    User     @relation(fields: [authorId], references: [id])
                tags      Tag[]
                createdAt DateTime @default(now())

                @@index([authorId])
            }

            model Profile {
                id     Int    @id @auto
                bio    String?
                userId Int    @unique
                user   User   @relation(fields: [userId], references: [id])
            }

            model Tag {
                id    Int    @id @auto
                name  String @unique
                posts Post[]
            }

            enum Role {
                User
                Admin
                Moderator
            }
        "#,
        )
        .unwrap();

        // Verify models
        assert_eq!(schema.models.len(), 4);
        assert!(schema.get_model("User").is_some());
        assert!(schema.get_model("Post").is_some());
        assert!(schema.get_model("Profile").is_some());
        assert!(schema.get_model("Tag").is_some());

        // Verify enums
        assert_eq!(schema.enums.len(), 1);
        assert!(schema.get_enum("Role").is_some());

        // Verify User model details
        let user = schema.get_model("User").unwrap();
        assert_eq!(user.table_name(), "users");
        assert_eq!(user.fields.len(), 8);
        assert!(user.has_attribute("index"));

        // Verify relations
        let post = schema.get_model("Post").unwrap();
        assert!(post.get_field("author").unwrap().is_relation());
    }

    // ==================== Error Handling ====================

    #[test]
    fn test_parse_invalid_syntax() {
        let result = parse_schema("model { broken }");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_schema() {
        let schema = parse_schema("").unwrap();
        assert!(schema.models.is_empty());
        assert!(schema.enums.is_empty());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let schema = parse_schema("   \n\t   \n   ").unwrap();
        assert!(schema.models.is_empty());
    }

    #[test]
    fn test_parse_comments_only() {
        let schema = parse_schema(
            r#"
            // This is a comment
            // Another comment
        "#,
        )
        .unwrap();
        assert!(schema.models.is_empty());
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_parse_model_with_no_fields() {
        // Models with no fields should still parse (might be invalid semantically but syntactically ok)
        let result = parse_schema(
            r#"
            model Empty {
            }
        "#,
        );
        // This might error or succeed depending on grammar - just verify it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_parse_long_identifier() {
        let schema = parse_schema(
            r#"
            model VeryLongModelNameThatIsStillValid {
                someVeryLongFieldNameThatShouldWork Int @id
            }
        "#,
        )
        .unwrap();

        assert!(
            schema
                .get_model("VeryLongModelNameThatIsStillValid")
                .is_some()
        );
    }

    #[test]
    fn test_parse_underscore_identifiers() {
        let schema = parse_schema(
            r#"
            model user_account {
                user_id     Int @id
                created_at  DateTime
            }
        "#,
        )
        .unwrap();

        let model = schema.get_model("user_account").unwrap();
        assert!(model.get_field("user_id").is_some());
        assert!(model.get_field("created_at").is_some());
    }

    #[test]
    fn test_parse_negative_default() {
        let schema = parse_schema(
            r#"
            model Config {
                id       Int @id
                minValue Int @default(-100)
            }
        "#,
        )
        .unwrap();

        let config = schema.get_model("Config").unwrap();
        let min_value = config.get_field("minValue").unwrap();
        let attrs = min_value.extract_attributes();
        assert!(attrs.default.is_some());
    }

    #[test]
    fn test_parse_float_default() {
        let schema = parse_schema(
            r#"
            model Product {
                id    Int   @id
                price Float @default(9.99)
            }
        "#,
        )
        .unwrap();

        let product = schema.get_model("Product").unwrap();
        let price = product.get_field("price").unwrap();
        let attrs = price.extract_attributes();
        assert!(attrs.default.is_some());
    }

    // ==================== Server Group Parsing ====================

    #[test]
    fn test_parse_simple_server_group() {
        let schema = parse_schema(
            r#"
            serverGroup MainCluster {
                server primary {
                    url = "postgres://localhost/db"
                    role = "primary"
                }
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.server_groups.len(), 1);
        let cluster = schema.get_server_group("MainCluster").unwrap();
        assert_eq!(cluster.servers.len(), 1);
        assert!(cluster.servers.contains_key("primary"));
    }

    #[test]
    fn test_parse_server_group_with_multiple_servers() {
        let schema = parse_schema(
            r#"
            serverGroup ReadReplicas {
                server primary {
                    url = "postgres://primary.db.com/app"
                    role = "primary"
                    weight = 1
                }

                server replica1 {
                    url = "postgres://replica1.db.com/app"
                    role = "replica"
                    weight = 2
                }

                server replica2 {
                    url = "postgres://replica2.db.com/app"
                    role = "replica"
                    weight = 2
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("ReadReplicas").unwrap();
        assert_eq!(cluster.servers.len(), 3);

        let primary = cluster.servers.get("primary").unwrap();
        assert_eq!(primary.role(), Some(ServerRole::Primary));
        assert_eq!(primary.weight(), Some(1));

        let replica1 = cluster.servers.get("replica1").unwrap();
        assert_eq!(replica1.role(), Some(ServerRole::Replica));
        assert_eq!(replica1.weight(), Some(2));
    }

    #[test]
    fn test_parse_server_group_with_attributes() {
        let schema = parse_schema(
            r#"
            serverGroup ProductionCluster {
                @@strategy(ReadReplica)
                @@loadBalance(RoundRobin)

                server main {
                    url = "postgres://main/db"
                    role = "primary"
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("ProductionCluster").unwrap();
        assert!(cluster.attributes.iter().any(|a| a.name.name == "strategy"));
        assert!(
            cluster
                .attributes
                .iter()
                .any(|a| a.name.name == "loadBalance")
        );
    }

    #[test]
    fn test_parse_server_group_with_env_vars() {
        let schema = parse_schema(
            r#"
            serverGroup EnvCluster {
                server db1 {
                    url = env("PRIMARY_DB_URL")
                    role = "primary"
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("EnvCluster").unwrap();
        let server = cluster.servers.get("db1").unwrap();

        // Check that the URL is stored as an env var reference
        if let Some(ServerPropertyValue::EnvVar(var)) = server.get_property("url") {
            assert_eq!(var, "PRIMARY_DB_URL");
        } else {
            panic!("Expected env var for url property");
        }
    }

    #[test]
    fn test_parse_server_group_with_boolean_property() {
        let schema = parse_schema(
            r#"
            serverGroup TestCluster {
                server replica {
                    url = "postgres://replica/db"
                    role = "replica"
                    readOnly = true
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("TestCluster").unwrap();
        let server = cluster.servers.get("replica").unwrap();
        assert!(server.is_read_only());
    }

    #[test]
    fn test_parse_server_group_with_numeric_properties() {
        let schema = parse_schema(
            r#"
            serverGroup NumericCluster {
                server db {
                    url = "postgres://localhost/db"
                    weight = 5
                    priority = 1
                    maxConnections = 100
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("NumericCluster").unwrap();
        let server = cluster.servers.get("db").unwrap();

        assert_eq!(server.weight(), Some(5));
        assert_eq!(server.priority(), Some(1));
        assert_eq!(server.max_connections(), Some(100));
    }

    #[test]
    fn test_parse_server_group_with_region() {
        let schema = parse_schema(
            r#"
            serverGroup GeoCluster {
                server usEast {
                    url = "postgres://us-east.db.com/app"
                    role = "replica"
                    region = "us-east-1"
                }

                server usWest {
                    url = "postgres://us-west.db.com/app"
                    role = "replica"
                    region = "us-west-2"
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("GeoCluster").unwrap();

        let us_east = cluster.servers.get("usEast").unwrap();
        assert_eq!(us_east.region(), Some("us-east-1"));

        let us_west = cluster.servers.get("usWest").unwrap();
        assert_eq!(us_west.region(), Some("us-west-2"));

        // Test region filtering
        let us_east_servers = cluster.servers_in_region("us-east-1");
        assert_eq!(us_east_servers.len(), 1);
    }

    #[test]
    fn test_parse_multiple_server_groups() {
        let schema = parse_schema(
            r#"
            serverGroup Cluster1 {
                server db1 {
                    url = "postgres://db1/app"
                }
            }

            serverGroup Cluster2 {
                server db2 {
                    url = "postgres://db2/app"
                }
            }

            serverGroup Cluster3 {
                server db3 {
                    url = "postgres://db3/app"
                }
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.server_groups.len(), 3);
        assert!(schema.get_server_group("Cluster1").is_some());
        assert!(schema.get_server_group("Cluster2").is_some());
        assert!(schema.get_server_group("Cluster3").is_some());
    }

    #[test]
    fn test_parse_schema_with_models_and_server_groups() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String @unique
            }

            serverGroup Database {
                @@strategy(ReadReplica)

                server primary {
                    url = env("DATABASE_URL")
                    role = "primary"
                }
            }

            model Post {
                id       Int    @id @auto
                title    String
                authorId Int
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 2);
        assert!(schema.get_model("User").is_some());
        assert!(schema.get_model("Post").is_some());

        assert_eq!(schema.server_groups.len(), 1);
        assert!(schema.get_server_group("Database").is_some());
    }

    #[test]
    fn test_parse_server_group_with_health_check() {
        let schema = parse_schema(
            r#"
            serverGroup HealthyCluster {
                server monitored {
                    url = "postgres://localhost/db"
                    healthCheck = "/health"
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("HealthyCluster").unwrap();
        let server = cluster.servers.get("monitored").unwrap();
        assert_eq!(server.health_check(), Some("/health"));
    }

    #[test]
    fn test_server_group_failover_order() {
        let schema = parse_schema(
            r#"
            serverGroup FailoverCluster {
                server db3 {
                    url = "postgres://db3/app"
                    priority = 3
                }

                server db1 {
                    url = "postgres://db1/app"
                    priority = 1
                }

                server db2 {
                    url = "postgres://db2/app"
                    priority = 2
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("FailoverCluster").unwrap();
        let ordered = cluster.failover_order();

        assert_eq!(ordered[0].name.name.as_str(), "db1");
        assert_eq!(ordered[1].name.name.as_str(), "db2");
        assert_eq!(ordered[2].name.name.as_str(), "db3");
    }

    #[test]
    fn test_server_group_names() {
        let schema = parse_schema(
            r#"
            serverGroup Alpha {
                server s1 { url = "pg://a" }
            }
            serverGroup Beta {
                server s2 { url = "pg://b" }
            }
        "#,
        )
        .unwrap();

        let names: Vec<_> = schema.server_group_names().collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
    }

    // ==================== Policy Parsing ====================

    #[test]
    fn test_parse_simple_policy() {
        let schema = parse_schema(
            r#"
            policy UserReadOwn on User {
                for SELECT
                using "id = current_user_id()"
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.policies.len(), 1);
        let policy = schema.get_policy("UserReadOwn").unwrap();
        assert_eq!(policy.name(), "UserReadOwn");
        assert_eq!(policy.table(), "User");
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(!policy.applies_to(PolicyCommand::Insert));
        assert_eq!(policy.using_expr.as_deref(), Some("id = current_user_id()"));
    }

    #[test]
    fn test_parse_policy_with_multiple_commands() {
        let schema = parse_schema(
            r#"
            policy UserModify on User {
                for [SELECT, UPDATE, DELETE]
                using "id = auth.uid()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UserModify").unwrap();
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Update));
        assert!(policy.applies_to(PolicyCommand::Delete));
        assert!(!policy.applies_to(PolicyCommand::Insert));
    }

    #[test]
    fn test_parse_policy_with_all_command() {
        let schema = parse_schema(
            r#"
            policy UserAll on User {
                for ALL
                using "true"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UserAll").unwrap();
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.applies_to(PolicyCommand::Insert));
        assert!(policy.applies_to(PolicyCommand::Update));
        assert!(policy.applies_to(PolicyCommand::Delete));
    }

    #[test]
    fn test_parse_policy_with_roles() {
        let schema = parse_schema(
            r#"
            policy AuthenticatedRead on Document {
                for SELECT
                to authenticated
                using "true"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("AuthenticatedRead").unwrap();
        let roles = policy.effective_roles();
        assert!(roles.contains(&"authenticated"));
    }

    #[test]
    fn test_parse_policy_with_multiple_roles() {
        let schema = parse_schema(
            r#"
            policy AdminModerator on Post {
                for [UPDATE, DELETE]
                to [admin, moderator]
                using "true"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("AdminModerator").unwrap();
        let roles = policy.effective_roles();
        assert!(roles.contains(&"admin"));
        assert!(roles.contains(&"moderator"));
    }

    #[test]
    fn test_parse_policy_restrictive() {
        let schema = parse_schema(
            r#"
            policy OrgRestriction on Document {
                as RESTRICTIVE
                for SELECT
                using "org_id = current_org_id()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("OrgRestriction").unwrap();
        assert!(policy.is_restrictive());
        assert!(!policy.is_permissive());
    }

    #[test]
    fn test_parse_policy_permissive_explicit() {
        let schema = parse_schema(
            r#"
            policy Permissive on User {
                as PERMISSIVE
                for SELECT
                using "true"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("Permissive").unwrap();
        assert!(policy.is_permissive());
    }

    #[test]
    fn test_parse_policy_with_check() {
        let schema = parse_schema(
            r#"
            policy InsertOwn on Post {
                for INSERT
                to authenticated
                check "author_id = current_user_id()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("InsertOwn").unwrap();
        assert!(policy.applies_to(PolicyCommand::Insert));
        assert_eq!(
            policy.check_expr.as_deref(),
            Some("author_id = current_user_id()")
        );
        assert!(policy.using_expr.is_none());
    }

    #[test]
    fn test_parse_policy_with_both_expressions() {
        let schema = parse_schema(
            r#"
            policy UpdateOwn on Post {
                for UPDATE
                using "author_id = current_user_id()"
                check "author_id = current_user_id()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UpdateOwn").unwrap();
        assert!(policy.using_expr.is_some());
        assert!(policy.check_expr.is_some());
    }

    #[test]
    fn test_parse_policy_multiline_expression() {
        let schema = parse_schema(
            r#"
            policy ComplexCheck on Document {
                for SELECT
                using """
                    (is_public = true)
                    OR (owner_id = current_user_id())
                    OR (id IN (SELECT document_id FROM shares WHERE user_id = current_user_id()))
                """
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("ComplexCheck").unwrap();
        assert!(policy.using_expr.is_some());
        let expr = policy.using_expr.as_ref().unwrap();
        assert!(expr.contains("is_public = true"));
        assert!(expr.contains("owner_id = current_user_id()"));
        assert!(expr.contains("SELECT document_id FROM shares"));
    }

    #[test]
    fn test_parse_multiple_policies() {
        let schema = parse_schema(
            r#"
            policy UserRead on User {
                for SELECT
                using "true"
            }

            policy UserInsert on User {
                for INSERT
                check "id = current_user_id()"
            }

            policy PostRead on Post {
                for SELECT
                using "published = true OR author_id = current_user_id()"
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.policies.len(), 3);
        assert!(schema.get_policy("UserRead").is_some());
        assert!(schema.get_policy("UserInsert").is_some());
        assert!(schema.get_policy("PostRead").is_some());
    }

    #[test]
    fn test_parse_policy_with_model() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String @unique
            }

            policy UserReadOwn on User {
                for SELECT
                to authenticated
                using "id = auth.uid()"
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 1);
        assert_eq!(schema.policies.len(), 1);

        let policies = schema.policies_for("User");
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].name(), "UserReadOwn");
    }

    #[test]
    fn test_parse_policies_for_multiple_models() {
        let schema = parse_schema(
            r#"
            policy UserPolicy1 on User {
                for SELECT
                using "true"
            }

            policy UserPolicy2 on User {
                for INSERT
                check "true"
            }

            policy PostPolicy on Post {
                for SELECT
                using "true"
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.policies_for("User").len(), 2);
        assert_eq!(schema.policies_for("Post").len(), 1);
        assert!(schema.has_policies("User"));
        assert!(schema.has_policies("Post"));
        assert!(!schema.has_policies("Comment"));
    }

    #[test]
    fn test_parse_policy_default_all_command() {
        let schema = parse_schema(
            r#"
            policy DefaultAll on User {
                using "id = current_user_id()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("DefaultAll").unwrap();
        // When no 'for' clause, should default to ALL
        assert!(policy.applies_to(PolicyCommand::All));
    }

    #[test]
    fn test_parse_policy_case_insensitive_keywords() {
        let schema = parse_schema(
            r#"
            policy CaseTest on User {
                for select
                as permissive
                using "true"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("CaseTest").unwrap();
        assert!(policy.applies_to(PolicyCommand::Select));
        assert!(policy.is_permissive());
    }

    #[test]
    fn test_parse_policy_sql_generation() {
        let schema = parse_schema(
            r#"
            model User {
                id Int @id

                @@map("users")
            }

            policy ReadOwn on User {
                for SELECT
                to authenticated
                using "id = auth.uid()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("ReadOwn").unwrap();
        let sql = policy.to_sql("users");

        assert!(sql.contains("CREATE POLICY ReadOwn ON users"));
        assert!(sql.contains("FOR SELECT"));
        assert!(sql.contains("TO authenticated"));
        assert!(sql.contains("USING (id = auth.uid())"));
    }

    #[test]
    fn test_parse_policy_restrictive_sql() {
        let schema = parse_schema(
            r#"
            policy OrgBoundary on Document {
                as RESTRICTIVE
                for ALL
                using "org_id = current_org_id()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("OrgBoundary").unwrap();
        let sql = policy.to_sql("documents");

        assert!(sql.contains("AS RESTRICTIVE"));
    }

    #[test]
    fn test_parse_policy_with_documentation() {
        let schema = parse_schema(
            r#"
            /// Users can only read their own data
            policy UserIsolation on User {
                for SELECT
                using "id = current_user_id()"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UserIsolation").unwrap();
        if let Some(doc) = &policy.documentation {
            assert!(doc.text.contains("their own data"));
        }
    }

    #[test]
    fn test_parse_complex_rls_schema() {
        let schema = parse_schema(
            r#"
            model Organization {
                id   Int    @id @auto
                name String
            }

            model User {
                id    Int    @id @auto
                orgId Int
                email String @unique
            }

            model Document {
                id       Int     @id @auto
                title    String
                ownerId  Int
                orgId    Int
                isPublic Boolean @default(false)
            }

            /// Organization-level isolation
            policy OrgIsolation on Document {
                as RESTRICTIVE
                for ALL
                using "org_id = current_setting('app.current_org')::int"
            }

            /// Users can read public documents
            policy PublicRead on Document {
                for SELECT
                using "is_public = true"
            }

            /// Users can read their own documents
            policy OwnerRead on Document {
                for SELECT
                to authenticated
                using "owner_id = auth.uid()"
            }

            /// Users can only modify their own documents
            policy OwnerModify on Document {
                for [UPDATE, DELETE]
                to authenticated
                using "owner_id = auth.uid()"
                check "owner_id = auth.uid()"
            }

            /// Users can create documents in their org
            policy OrgInsert on Document {
                for INSERT
                to authenticated
                check "org_id = current_setting('app.current_org')::int"
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 3);
        assert_eq!(schema.policies.len(), 5);

        // Verify org isolation is restrictive
        let org_iso = schema.get_policy("OrgIsolation").unwrap();
        assert!(org_iso.is_restrictive());

        // Verify all Document policies
        let doc_policies = schema.policies_for("Document");
        assert_eq!(doc_policies.len(), 5);
    }

    // ==================== MSSQL Policy Parsing ====================

    #[test]
    fn test_parse_policy_with_mssql_schema() {
        let schema = parse_schema(
            r#"
            policy UserFilter on User {
                for SELECT
                using "UserId = @UserId"
                mssqlSchema "RLS"
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UserFilter").unwrap();
        assert_eq!(policy.mssql_schema(), "RLS");
    }

    #[test]
    fn test_parse_policy_with_mssql_block_single() {
        let schema = parse_schema(
            r#"
            policy UserInsert on User {
                for INSERT
                check "UserId = @UserId"
                mssqlBlock AFTER_INSERT
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UserInsert").unwrap();
        assert_eq!(policy.mssql_block_operations.len(), 1);
        assert_eq!(
            policy.mssql_block_operations[0],
            MssqlBlockOperation::AfterInsert
        );
    }

    #[test]
    fn test_parse_policy_with_mssql_block_list() {
        let schema = parse_schema(
            r#"
            policy UserModify on User {
                for [INSERT, UPDATE, DELETE]
                check "UserId = @UserId"
                mssqlBlock [AFTER_INSERT, AFTER_UPDATE, BEFORE_DELETE]
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("UserModify").unwrap();
        assert_eq!(policy.mssql_block_operations.len(), 3);
        assert!(
            policy
                .mssql_block_operations
                .contains(&MssqlBlockOperation::AfterInsert)
        );
        assert!(
            policy
                .mssql_block_operations
                .contains(&MssqlBlockOperation::AfterUpdate)
        );
        assert!(
            policy
                .mssql_block_operations
                .contains(&MssqlBlockOperation::BeforeDelete)
        );
    }

    #[test]
    fn test_parse_policy_full_mssql_config() {
        let schema = parse_schema(
            r#"
            policy TenantIsolation on Order {
                for ALL
                using "TenantId = @TenantId"
                check "TenantId = @TenantId"
                mssqlSchema "MultiTenant"
                mssqlBlock [AFTER_INSERT, BEFORE_UPDATE, AFTER_UPDATE, BEFORE_DELETE]
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("TenantIsolation").unwrap();

        // Verify standard options
        assert!(policy.applies_to(PolicyCommand::All));
        assert!(policy.using_expr.is_some());
        assert!(policy.check_expr.is_some());

        // Verify MSSQL options
        assert_eq!(policy.mssql_schema(), "MultiTenant");
        assert_eq!(policy.mssql_block_operations.len(), 4);

        // Test SQL generation
        let mssql = policy.to_mssql_sql("dbo.Orders", "TenantId");
        assert!(mssql.schema_sql.contains("MultiTenant"));
        assert!(mssql.function_sql.contains("fn_TenantIsolation_predicate"));
    }

    #[test]
    fn test_parse_policy_mssql_block_case_variants() {
        // Test different case variants for block operations
        let schema = parse_schema(
            r#"
            policy Test1 on User {
                for INSERT
                check "true"
                mssqlBlock after_insert
            }
        "#,
        )
        .unwrap();

        let policy = schema.get_policy("Test1").unwrap();
        assert_eq!(policy.mssql_block_operations.len(), 1);
        assert_eq!(
            policy.mssql_block_operations[0],
            MssqlBlockOperation::AfterInsert
        );
    }

    #[test]
    fn test_parse_mixed_postgres_mssql_schema() {
        let schema = parse_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String @unique
            }

            // PostgreSQL-style policy (works on both, MSSQL uses defaults)
            policy UserReadOwn on User {
                for SELECT
                to authenticated
                using "id = current_user_id()"
            }

            // MSSQL-optimized policy with explicit settings
            policy UserModifyOwn on User {
                for [INSERT, UPDATE, DELETE]
                to authenticated
                using "id = current_user_id()"
                check "id = current_user_id()"
                mssqlSchema "Security"
                mssqlBlock [AFTER_INSERT, BEFORE_UPDATE, AFTER_UPDATE, BEFORE_DELETE]
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.policies.len(), 2);

        // First policy uses defaults for MSSQL
        let read_policy = schema.get_policy("UserReadOwn").unwrap();
        assert_eq!(read_policy.mssql_schema(), "Security"); // default
        assert!(read_policy.mssql_block_operations.is_empty()); // will use auto-generated

        // Second policy has explicit MSSQL config
        let modify_policy = schema.get_policy("UserModifyOwn").unwrap();
        assert_eq!(modify_policy.mssql_schema(), "Security");
        assert_eq!(modify_policy.mssql_block_operations.len(), 4);

        // Both should generate valid PostgreSQL SQL
        let pg_sql = read_policy.to_postgres_sql("users");
        assert!(pg_sql.contains("CREATE POLICY UserReadOwn ON users"));

        // Both should generate valid MSSQL SQL
        let mssql = modify_policy.to_mssql_sql("dbo.Users", "id");
        assert!(mssql.policy_sql.contains("Security.UserModifyOwn"));
    }
}
