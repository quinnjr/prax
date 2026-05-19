//! Model definitions for the Prax schema AST.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Attribute, Documentation, Field, Ident, Span};

/// A model definition (maps to a database table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// Model name.
    pub name: Ident,
    /// Model fields.
    pub fields: IndexMap<SmolStr, Field>,
    /// Model-level attributes (prefixed with `@@`).
    pub attributes: Vec<Attribute>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
    /// Source file this model was parsed from (None for single-file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<crate::loader::SourceId>,
}

impl Model {
    /// Create a new model.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            fields: IndexMap::new(),
            attributes: vec![],
            documentation: None,
            span,
            source_id: None,
        }
    }

    /// Get the model name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Add a field to the model.
    pub fn add_field(&mut self, field: Field) {
        self.fields.insert(field.name.name.clone(), field);
    }

    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }

    /// Get the primary key field(s).
    pub fn id_fields(&self) -> Vec<&Field> {
        self.fields.values().filter(|f| f.is_id()).collect()
    }

    /// Get all relation fields.
    pub fn relation_fields(&self) -> Vec<&Field> {
        self.fields.values().filter(|f| f.is_relation()).collect()
    }

    /// Get all scalar (non-relation) fields.
    pub fn scalar_fields(&self) -> Vec<&Field> {
        self.fields.values().filter(|f| !f.is_relation()).collect()
    }

    /// Check if this model has a specific model-level attribute.
    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.iter().any(|a| a.is(name))
    }

    /// Get a model-level attribute by name.
    pub fn get_attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.is(name))
    }

    /// Get the database table name (from `@@map` or model name).
    pub fn table_name(&self) -> &str {
        self.get_attribute("map")
            .and_then(|a| a.first_arg())
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| self.name())
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: Documentation) -> Self {
        self.documentation = Some(doc);
        self
    }
}

/// An enum definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enum {
    /// Enum name.
    pub name: Ident,
    /// Enum variants.
    pub variants: Vec<EnumVariant>,
    /// Enum-level attributes.
    pub attributes: Vec<Attribute>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
    /// Source file this enum was parsed from (None for single-file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<crate::loader::SourceId>,
}

impl Enum {
    /// Create a new enum.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            variants: vec![],
            attributes: vec![],
            documentation: None,
            span,
            source_id: None,
        }
    }

    /// Get the enum name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Add a variant to the enum.
    pub fn add_variant(&mut self, variant: EnumVariant) {
        self.variants.push(variant);
    }

    /// Get a variant by name.
    pub fn get_variant(&self, name: &str) -> Option<&EnumVariant> {
        self.variants.iter().find(|v| v.name.as_str() == name)
    }

    /// Get the database type name (from `@@map` or enum name).
    pub fn db_name(&self) -> &str {
        self.attributes
            .iter()
            .find(|a| a.is("map"))
            .and_then(|a| a.first_arg())
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| self.name())
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: Documentation) -> Self {
        self.documentation = Some(doc);
        self
    }
}

/// An enum variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumVariant {
    /// Variant name.
    pub name: Ident,
    /// Variant-level attributes.
    pub attributes: Vec<Attribute>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
}

impl EnumVariant {
    /// Create a new enum variant.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            attributes: vec![],
            documentation: None,
            span,
        }
    }

    /// Get the variant name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Get the database value (from `@map` or variant name).
    pub fn db_value(&self) -> &str {
        self.attributes
            .iter()
            .find(|a| a.is("map"))
            .and_then(|a| a.first_arg())
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| self.name())
    }
}

/// A composite type definition (for embedded documents / JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompositeType {
    /// Type name.
    pub name: Ident,
    /// Type fields.
    pub fields: IndexMap<SmolStr, Field>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
    /// Source file this type was parsed from (None for single-file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<crate::loader::SourceId>,
}

impl CompositeType {
    /// Create a new composite type.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            fields: IndexMap::new(),
            documentation: None,
            span,
            source_id: None,
        }
    }

    /// Get the type name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Add a field to the type.
    pub fn add_field(&mut self, field: Field) {
        self.fields.insert(field.name.name.clone(), field);
    }

    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&Field> {
        self.fields.get(name)
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: Documentation) -> Self {
        self.documentation = Some(doc);
        self
    }
}

/// A view definition (read-only model mapping to a database view).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// View name.
    pub name: Ident,
    /// View fields.
    pub fields: IndexMap<SmolStr, Field>,
    /// View-level attributes.
    pub attributes: Vec<Attribute>,
    /// Documentation comment.
    pub documentation: Option<Documentation>,
    /// Source location.
    pub span: Span,
    /// Source file this view was parsed from (None for single-file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<crate::loader::SourceId>,
}

impl View {
    /// Create a new view.
    pub fn new(name: Ident, span: Span) -> Self {
        Self {
            name,
            fields: IndexMap::new(),
            attributes: vec![],
            documentation: None,
            span,
            source_id: None,
        }
    }

    /// Get the view name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Add a field to the view.
    pub fn add_field(&mut self, field: Field) {
        self.fields.insert(field.name.name.clone(), field);
    }

    /// Get the database view name (from `@@map` or view name).
    pub fn view_name(&self) -> &str {
        self.attributes
            .iter()
            .find(|a| a.is("map"))
            .and_then(|a| a.first_arg())
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| self.name())
    }

    /// Get the SQL query that defines the view (from `@@sql` attribute).
    pub fn sql_query(&self) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.is("sql"))
            .and_then(|a| a.first_arg())
            .and_then(|v| v.as_string())
    }

    /// Check if the view is materialized (has `@@materialized` attribute).
    pub fn is_materialized(&self) -> bool {
        self.attributes.iter().any(|a| a.is("materialized"))
    }

    /// Get the refresh interval for materialized views (from `@@refreshInterval`).
    pub fn refresh_interval(&self) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.is("refreshInterval"))
            .and_then(|a| a.first_arg())
            .and_then(|v| v.as_string())
    }

    /// Set documentation.
    pub fn with_documentation(mut self, doc: Documentation) -> Self {
        self.documentation = Some(doc);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        Attribute, AttributeArg, AttributeValue, FieldType, ScalarType, TypeModifier,
    };

    fn make_span() -> Span {
        Span::new(0, 10)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_field(name: &str, field_type: FieldType, modifier: TypeModifier) -> Field {
        Field::new(make_ident(name), field_type, modifier, vec![], make_span())
    }

    fn make_id_field() -> Field {
        let mut field = make_field(
            "id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        );
        field
            .attributes
            .push(Attribute::simple(make_ident("id"), make_span()));
        field
            .attributes
            .push(Attribute::simple(make_ident("auto"), make_span()));
        field
    }

    fn make_attribute(name: &str) -> Attribute {
        Attribute::simple(make_ident(name), make_span())
    }

    fn make_attribute_with_string(name: &str, value: &str) -> Attribute {
        Attribute::new(
            make_ident(name),
            vec![AttributeArg::positional(
                AttributeValue::String(value.into()),
                make_span(),
            )],
            make_span(),
        )
    }

    // ==================== Model Tests ====================

    #[test]
    fn test_model_new() {
        let model = Model::new(make_ident("User"), make_span());

        assert_eq!(model.name(), "User");
        assert!(model.fields.is_empty());
        assert!(model.attributes.is_empty());
        assert!(model.documentation.is_none());
    }

    #[test]
    fn test_model_name() {
        let model = Model::new(make_ident("BlogPost"), make_span());
        assert_eq!(model.name(), "BlogPost");
    }

    #[test]
    fn test_model_add_field() {
        let mut model = Model::new(make_ident("User"), make_span());
        let field = make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        );

        model.add_field(field);

        assert_eq!(model.fields.len(), 1);
        assert!(model.fields.contains_key("email"));
    }

    #[test]
    fn test_model_add_multiple_fields() {
        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(make_id_field());
        model.add_field(make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));
        model.add_field(make_field(
            "name",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Optional,
        ));

        assert_eq!(model.fields.len(), 3);
    }

    #[test]
    fn test_model_get_field() {
        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));

        let field = model.get_field("email");
        assert!(field.is_some());
        assert_eq!(field.unwrap().name(), "email");

        assert!(model.get_field("nonexistent").is_none());
    }

    #[test]
    fn test_model_id_fields() {
        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(make_id_field());
        model.add_field(make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));

        let id_fields = model.id_fields();
        assert_eq!(id_fields.len(), 1);
        assert_eq!(id_fields[0].name(), "id");
    }

    #[test]
    fn test_model_id_fields_none() {
        let mut model = Model::new(make_ident("User"), make_span());
        model.add_field(make_field(
            "email",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));

        let id_fields = model.id_fields();
        assert!(id_fields.is_empty());
    }

    #[test]
    fn test_model_relation_fields() {
        let mut model = Model::new(make_ident("Post"), make_span());
        model.add_field(make_id_field());
        model.add_field(make_field(
            "title",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));
        model.add_field(make_field(
            "author",
            FieldType::Model("User".into()),
            TypeModifier::Required,
        ));

        let rel_fields = model.relation_fields();
        assert_eq!(rel_fields.len(), 1);
        assert_eq!(rel_fields[0].name(), "author");
    }

    #[test]
    fn test_model_scalar_fields() {
        let mut model = Model::new(make_ident("Post"), make_span());
        model.add_field(make_id_field());
        model.add_field(make_field(
            "title",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));
        model.add_field(make_field(
            "author",
            FieldType::Model("User".into()),
            TypeModifier::Required,
        ));

        let scalar_fields = model.scalar_fields();
        assert_eq!(scalar_fields.len(), 2);
    }

    #[test]
    fn test_model_has_attribute() {
        let mut model = Model::new(make_ident("User"), make_span());
        model.attributes.push(make_attribute("map"));

        assert!(model.has_attribute("map"));
        assert!(!model.has_attribute("index"));
    }

    #[test]
    fn test_model_get_attribute() {
        let mut model = Model::new(make_ident("User"), make_span());
        model
            .attributes
            .push(make_attribute_with_string("map", "users"));

        let attr = model.get_attribute("map");
        assert!(attr.is_some());
        assert!(attr.unwrap().is("map"));

        assert!(model.get_attribute("index").is_none());
    }

    #[test]
    fn test_model_table_name_default() {
        let model = Model::new(make_ident("User"), make_span());
        assert_eq!(model.table_name(), "User");
    }

    #[test]
    fn test_model_table_name_mapped() {
        let mut model = Model::new(make_ident("User"), make_span());
        model
            .attributes
            .push(make_attribute_with_string("map", "app_users"));

        assert_eq!(model.table_name(), "app_users");
    }

    #[test]
    fn test_model_with_documentation() {
        let model = Model::new(make_ident("User"), make_span())
            .with_documentation(Documentation::new("Represents a user", make_span()));

        assert!(model.documentation.is_some());
        assert_eq!(model.documentation.unwrap().text, "Represents a user");
    }

    // ==================== Enum Tests ====================

    #[test]
    fn test_enum_new() {
        let e = Enum::new(make_ident("Role"), make_span());

        assert_eq!(e.name(), "Role");
        assert!(e.variants.is_empty());
        assert!(e.attributes.is_empty());
        assert!(e.documentation.is_none());
    }

    #[test]
    fn test_enum_add_variant() {
        let mut e = Enum::new(make_ident("Role"), make_span());
        e.add_variant(EnumVariant::new(make_ident("Admin"), make_span()));
        e.add_variant(EnumVariant::new(make_ident("User"), make_span()));

        assert_eq!(e.variants.len(), 2);
    }

    #[test]
    fn test_enum_get_variant() {
        let mut e = Enum::new(make_ident("Role"), make_span());
        e.add_variant(EnumVariant::new(make_ident("Admin"), make_span()));
        e.add_variant(EnumVariant::new(make_ident("User"), make_span()));

        let variant = e.get_variant("Admin");
        assert!(variant.is_some());
        assert_eq!(variant.unwrap().name(), "Admin");

        assert!(e.get_variant("Moderator").is_none());
    }

    #[test]
    fn test_enum_db_name_default() {
        let e = Enum::new(make_ident("Role"), make_span());
        assert_eq!(e.db_name(), "Role");
    }

    #[test]
    fn test_enum_db_name_mapped() {
        let mut e = Enum::new(make_ident("Role"), make_span());
        e.attributes
            .push(make_attribute_with_string("map", "user_role"));

        assert_eq!(e.db_name(), "user_role");
    }

    #[test]
    fn test_enum_with_documentation() {
        let e = Enum::new(make_ident("Role"), make_span())
            .with_documentation(Documentation::new("User roles", make_span()));

        assert!(e.documentation.is_some());
    }

    // ==================== EnumVariant Tests ====================

    #[test]
    fn test_enum_variant_new() {
        let variant = EnumVariant::new(make_ident("Admin"), make_span());

        assert_eq!(variant.name(), "Admin");
        assert!(variant.attributes.is_empty());
        assert!(variant.documentation.is_none());
    }

    #[test]
    fn test_enum_variant_db_value_default() {
        let variant = EnumVariant::new(make_ident("Admin"), make_span());
        assert_eq!(variant.db_value(), "Admin");
    }

    #[test]
    fn test_enum_variant_db_value_mapped() {
        let mut variant = EnumVariant::new(make_ident("Admin"), make_span());
        variant
            .attributes
            .push(make_attribute_with_string("map", "ADMIN_USER"));

        assert_eq!(variant.db_value(), "ADMIN_USER");
    }

    // ==================== CompositeType Tests ====================

    #[test]
    fn test_composite_type_new() {
        let ct = CompositeType::new(make_ident("Address"), make_span());

        assert_eq!(ct.name(), "Address");
        assert!(ct.fields.is_empty());
        assert!(ct.documentation.is_none());
    }

    #[test]
    fn test_composite_type_add_field() {
        let mut ct = CompositeType::new(make_ident("Address"), make_span());
        ct.add_field(make_field(
            "street",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));
        ct.add_field(make_field(
            "city",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));

        assert_eq!(ct.fields.len(), 2);
    }

    #[test]
    fn test_composite_type_get_field() {
        let mut ct = CompositeType::new(make_ident("Address"), make_span());
        ct.add_field(make_field(
            "city",
            FieldType::Scalar(ScalarType::String),
            TypeModifier::Required,
        ));

        let field = ct.get_field("city");
        assert!(field.is_some());
        assert_eq!(field.unwrap().name(), "city");

        assert!(ct.get_field("country").is_none());
    }

    #[test]
    fn test_composite_type_with_documentation() {
        let ct = CompositeType::new(make_ident("Address"), make_span())
            .with_documentation(Documentation::new("Mailing address", make_span()));

        assert!(ct.documentation.is_some());
    }

    // ==================== View Tests ====================

    #[test]
    fn test_view_new() {
        let view = View::new(make_ident("UserStats"), make_span());

        assert_eq!(view.name(), "UserStats");
        assert!(view.fields.is_empty());
        assert!(view.attributes.is_empty());
        assert!(view.documentation.is_none());
    }

    #[test]
    fn test_view_add_field() {
        let mut view = View::new(make_ident("UserStats"), make_span());
        view.add_field(make_field(
            "user_id",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        ));
        view.add_field(make_field(
            "post_count",
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
        ));

        assert_eq!(view.fields.len(), 2);
    }

    #[test]
    fn test_view_view_name_default() {
        let view = View::new(make_ident("UserStats"), make_span());
        assert_eq!(view.view_name(), "UserStats");
    }

    #[test]
    fn test_view_view_name_mapped() {
        let mut view = View::new(make_ident("UserStats"), make_span());
        view.attributes
            .push(make_attribute_with_string("map", "v_user_statistics"));

        assert_eq!(view.view_name(), "v_user_statistics");
    }

    #[test]
    fn test_view_with_documentation() {
        let view = View::new(make_ident("UserStats"), make_span()).with_documentation(
            Documentation::new("Aggregated user statistics", make_span()),
        );

        assert!(view.documentation.is_some());
    }

    // ==================== Equality Tests ====================

    #[test]
    fn test_model_equality() {
        let model1 = Model::new(make_ident("User"), make_span());
        let model2 = Model::new(make_ident("User"), make_span());

        assert_eq!(model1, model2);
    }

    #[test]
    fn test_model_inequality() {
        let model1 = Model::new(make_ident("User"), make_span());
        let model2 = Model::new(make_ident("Post"), make_span());

        assert_ne!(model1, model2);
    }

    #[test]
    fn test_enum_equality() {
        let enum1 = Enum::new(make_ident("Role"), make_span());
        let enum2 = Enum::new(make_ident("Role"), make_span());

        assert_eq!(enum1, enum2);
    }

    #[test]
    fn test_enum_variant_equality() {
        let v1 = EnumVariant::new(make_ident("Admin"), make_span());
        let v2 = EnumVariant::new(make_ident("Admin"), make_span());
        let v3 = EnumVariant::new(make_ident("User"), make_span());

        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    #[test]
    fn test_composite_type_equality() {
        let ct1 = CompositeType::new(make_ident("Address"), make_span());
        let ct2 = CompositeType::new(make_ident("Address"), make_span());

        assert_eq!(ct1, ct2);
    }

    #[test]
    fn test_view_equality() {
        let v1 = View::new(make_ident("Stats"), make_span());
        let v2 = View::new(make_ident("Stats"), make_span());

        assert_eq!(v1, v2);
    }
}
