//! Common conversion utilities for transforming schemas to Prax AST.

// Builder helpers (`with_documentation`, `is_id_field`) are part of the
// internal converter API surface used by future importers; some are
// covered by tests but not yet wired into the active prisma/diesel/seaorm
// importers, so keep them around without tripping the dead_code lint.
#![allow(dead_code)]

use convert_case::{Case, Casing};
use prax_schema::ast::*;
use smol_str::SmolStr;

/// Helper to convert snake_case table names to PascalCase model names.
pub fn table_name_to_model_name(table_name: &str) -> String {
    // Handle plural to singular conversion for common cases
    let singular = if let Some(stem) = table_name.strip_suffix("ies") {
        format!("{}y", stem)
    } else if let Some(stem) = table_name.strip_suffix('s')
        && !table_name.ends_with("ss")
    {
        stem.to_string()
    } else {
        table_name.to_string()
    };

    singular.to_case(Case::Pascal)
}

/// Helper to convert column names to field names.
pub fn column_name_to_field_name(column_name: &str) -> String {
    column_name.to_case(Case::Camel)
}

/// Helper to determine if a field should be marked as @id.
pub fn is_id_field(name: &str) -> bool {
    name == "id" || name.ends_with("_id")
}

/// Create a dummy span (used when we don't have source location info).
pub fn dummy_span() -> Span {
    Span::new(0, 0)
}

/// Builder for constructing Prax Schema AST.
pub struct SchemaBuilder {
    schema: Schema,
}

impl SchemaBuilder {
    /// Create a new schema builder.
    pub fn new() -> Self {
        Self {
            schema: Schema::new(),
        }
    }

    /// Set the datasource configuration.
    pub fn with_datasource(mut self, provider_str: String, url: String) -> Self {
        // Parse provider
        let provider =
            DatabaseProvider::from_str(&provider_str).unwrap_or(DatabaseProvider::PostgreSQL); // Default to PostgreSQL if unknown

        let datasource = Datasource {
            name: SmolStr::from("db"),
            provider,
            url: Some(SmolStr::from(url)),
            url_env: None,
            extensions: vec![],
            properties: vec![],
            span: dummy_span(),
        };
        self.schema.set_datasource(datasource);
        self
    }

    /// Add a model to the schema.
    pub fn add_model(&mut self, model: Model) {
        self.schema.add_model(model);
    }

    /// Add an enum to the schema.
    pub fn add_enum(&mut self, enum_def: Enum) {
        self.schema.add_enum(enum_def);
    }

    /// Build the final schema.
    pub fn build(self) -> Schema {
        self.schema
    }
}

impl Default for SchemaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing Model AST nodes.
pub struct ModelBuilder {
    name: String,
    fields: Vec<Field>,
    attributes: Vec<Attribute>,
    documentation: Option<String>,
    db_name: Option<String>,
}

impl ModelBuilder {
    /// Create a new model builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: vec![],
            attributes: vec![],
            documentation: None,
            db_name: None,
        }
    }

    /// Set the database table name (if different from model name).
    pub fn with_db_name(mut self, db_name: impl Into<String>) -> Self {
        self.db_name = Some(db_name.into());
        self
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }

    /// Add a field to the model.
    pub fn add_field(&mut self, field: Field) {
        self.fields.push(field);
    }

    /// Add a @@map attribute for the table name.
    fn add_map_attribute(&mut self, table_name: String) {
        let attr = Attribute {
            name: Ident::new("map", dummy_span()),
            args: vec![AttributeArg::positional(
                AttributeValue::String(table_name),
                dummy_span(),
            )],
            span: dummy_span(),
        };
        self.attributes.push(attr);
    }

    /// Add a @@index attribute.
    pub fn add_index(&mut self, fields: Vec<String>, name: Option<String>) {
        let field_refs: Vec<SmolStr> = fields.into_iter().map(SmolStr::from).collect();
        let mut args = vec![AttributeArg::positional(
            AttributeValue::FieldRefList(field_refs),
            dummy_span(),
        )];

        if let Some(name) = name {
            args.push(AttributeArg::named(
                Ident::new("name", dummy_span()),
                AttributeValue::String(name),
                dummy_span(),
            ));
        }

        let attr = Attribute {
            name: Ident::new("index", dummy_span()),
            args,
            span: dummy_span(),
        };
        self.attributes.push(attr);
    }

    /// Add a @@unique attribute.
    pub fn add_unique(&mut self, fields: Vec<String>, name: Option<String>) {
        let field_refs: Vec<SmolStr> = fields.into_iter().map(SmolStr::from).collect();
        let mut args = vec![AttributeArg::positional(
            AttributeValue::FieldRefList(field_refs),
            dummy_span(),
        )];

        if let Some(name) = name {
            args.push(AttributeArg::named(
                Ident::new("name", dummy_span()),
                AttributeValue::String(name),
                dummy_span(),
            ));
        }

        let attr = Attribute {
            name: Ident::new("unique", dummy_span()),
            args,
            span: dummy_span(),
        };
        self.attributes.push(attr);
    }

    /// Build the final model.
    pub fn build(mut self) -> Model {
        // Add @@map attribute if db_name is set and different from model name
        if let Some(db_name) = &self.db_name
            && db_name != &self.name.to_case(Case::Snake)
        {
            self.add_map_attribute(db_name.clone());
        }

        let mut model = Model::new(Ident::new(&self.name, dummy_span()), dummy_span());

        // Add fields
        for field in self.fields {
            model.add_field(field);
        }

        // Add attributes
        model.attributes = self.attributes;

        // Add documentation
        if let Some(doc) = self.documentation {
            model.documentation = Some(Documentation::new(doc, dummy_span()));
        }

        model
    }
}

/// Builder for constructing Field AST nodes.
pub struct FieldBuilder {
    name: String,
    field_type: FieldType,
    modifier: TypeModifier,
    attributes: Vec<Attribute>,
    documentation: Option<String>,
    db_name: Option<String>,
}

impl FieldBuilder {
    /// Create a new field builder.
    pub fn new(name: impl Into<String>, field_type: FieldType, modifier: TypeModifier) -> Self {
        Self {
            name: name.into(),
            field_type,
            modifier,
            attributes: vec![],
            documentation: None,
            db_name: None,
        }
    }

    /// Set the database column name (if different from field name).
    pub fn with_db_name(mut self, db_name: impl Into<String>) -> Self {
        self.db_name = Some(db_name.into());
        self
    }

    /// Mark this field as the primary key.
    pub fn with_id(mut self) -> Self {
        let attr = Attribute {
            name: Ident::new("id", dummy_span()),
            args: vec![],
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Mark this field as auto-incrementing.
    pub fn with_auto(mut self) -> Self {
        let attr = Attribute {
            name: Ident::new("auto", dummy_span()),
            args: vec![],
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Mark this field as updated_at (auto-updating timestamp).
    pub fn with_updated_at(mut self) -> Self {
        let attr = Attribute {
            name: Ident::new("updated_at", dummy_span()),
            args: vec![],
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Mark this field as unique.
    pub fn with_unique(mut self) -> Self {
        let attr = Attribute {
            name: Ident::new("unique", dummy_span()),
            args: vec![],
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Set a default value.
    pub fn with_default(mut self, value: AttributeValue) -> Self {
        let attr = Attribute {
            name: Ident::new("default", dummy_span()),
            args: vec![AttributeArg::positional(value, dummy_span())],
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Add a @map attribute for the column name.
    pub fn with_map(mut self, column_name: String) -> Self {
        let attr = Attribute {
            name: Ident::new("map", dummy_span()),
            args: vec![AttributeArg::positional(
                AttributeValue::String(column_name),
                dummy_span(),
            )],
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Add a @relation attribute.
    pub fn with_relation(
        mut self,
        name: Option<String>,
        fields: Vec<String>,
        references: Vec<String>,
        on_delete: Option<String>,
        on_update: Option<String>,
        map: Option<String>,
    ) -> Self {
        let mut args = vec![];

        // First positional arg: relation name (for disambiguation)
        if let Some(rel_name) = name {
            args.push(AttributeArg::positional(
                AttributeValue::String(rel_name),
                dummy_span(),
            ));
        }

        // Add fields argument
        let field_refs: Vec<SmolStr> = fields.into_iter().map(SmolStr::from).collect();
        args.push(AttributeArg::named(
            Ident::new("fields", dummy_span()),
            AttributeValue::FieldRefList(field_refs),
            dummy_span(),
        ));

        // Add references argument
        let ref_refs: Vec<SmolStr> = references.into_iter().map(SmolStr::from).collect();
        args.push(AttributeArg::named(
            Ident::new("references", dummy_span()),
            AttributeValue::FieldRefList(ref_refs),
            dummy_span(),
        ));

        // Add onDelete if specified
        if let Some(action) = on_delete {
            args.push(AttributeArg::named(
                Ident::new("onDelete", dummy_span()),
                AttributeValue::Ident(SmolStr::from(action)),
                dummy_span(),
            ));
        }

        // Add onUpdate if specified
        if let Some(action) = on_update {
            args.push(AttributeArg::named(
                Ident::new("onUpdate", dummy_span()),
                AttributeValue::Ident(SmolStr::from(action)),
                dummy_span(),
            ));
        }

        // Add map if specified (custom FK constraint name)
        if let Some(constraint_name) = map {
            args.push(AttributeArg::named(
                Ident::new("map", dummy_span()),
                AttributeValue::String(constraint_name),
                dummy_span(),
            ));
        }

        let attr = Attribute {
            name: Ident::new("relation", dummy_span()),
            args,
            span: dummy_span(),
        };
        self.attributes.push(attr);
        self
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }

    /// Build the final field.
    pub fn build(self) -> Field {
        // Add @map attribute if db_name is set and different from field name
        let mut builder = self;
        if let Some(db_name) = builder.db_name.clone()
            && db_name != builder.name.to_case(Case::Snake)
        {
            builder = builder.with_map(db_name);
        }

        let mut field = Field::new(
            Ident::new(&builder.name, dummy_span()),
            builder.field_type,
            builder.modifier,
            builder.attributes,
            dummy_span(),
        );

        // Add documentation
        if let Some(doc) = builder.documentation {
            field.documentation = Some(Documentation::new(doc, dummy_span()));
        }

        field
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_name_to_model_name() {
        assert_eq!(table_name_to_model_name("users"), "User");
        assert_eq!(table_name_to_model_name("blog_posts"), "BlogPost");
        assert_eq!(table_name_to_model_name("categories"), "Category");
        assert_eq!(table_name_to_model_name("user_profiles"), "UserProfile");
    }

    #[test]
    fn test_column_name_to_field_name() {
        assert_eq!(column_name_to_field_name("id"), "id");
        assert_eq!(column_name_to_field_name("created_at"), "createdAt");
        assert_eq!(column_name_to_field_name("user_id"), "userId");
    }

    #[test]
    fn test_is_id_field() {
        assert!(is_id_field("id"));
        assert!(is_id_field("user_id"));
        assert!(!is_id_field("email"));
    }

    #[test]
    fn test_schema_builder() {
        let builder = SchemaBuilder::new();
        let builder = builder.with_datasource(
            "postgresql".to_string(),
            "postgres://localhost/test".to_string(),
        );

        let model = ModelBuilder::new("User").with_db_name("users").build();

        let mut builder_mut = builder;
        builder_mut.add_model(model);

        let schema = builder_mut.build();
        assert!(schema.datasource.is_some());
        assert_eq!(schema.models.len(), 1);
    }

    #[test]
    fn test_model_builder() {
        let mut model_builder = ModelBuilder::new("User").with_db_name("users");

        let id_field = FieldBuilder::new(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        )
        .with_id()
        .with_auto()
        .build();

        model_builder.add_field(id_field);

        let model = model_builder.build();
        assert_eq!(model.name().to_string(), "User");
        assert_eq!(model.fields.len(), 1);
    }

    #[test]
    fn test_field_builder() {
        let field = FieldBuilder::new(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        )
        .with_unique()
        .with_db_name("email_address")
        .build();

        assert_eq!(field.name(), "email");
        assert!(matches!(
            field.field_type,
            FieldType::Scalar(ScalarType::String)
        ));
        assert!(field.has_attribute("unique"));
    }
}
