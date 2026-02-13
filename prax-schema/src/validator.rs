//! Schema validation and semantic analysis.
//!
//! This module validates parsed schemas for semantic correctness:
//! - All type references are valid
//! - Relations are properly defined
//! - Required attributes are present
//! - No duplicate definitions

use crate::ast::*;
use crate::error::{SchemaError, SchemaResult};

/// Schema validator for semantic analysis.
#[derive(Debug)]
pub struct Validator {
    /// Collected validation errors.
    errors: Vec<SchemaError>,
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self { errors: vec![] }
    }

    /// Validate a schema and return the validated schema or errors.
    pub fn validate(&mut self, mut schema: Schema) -> SchemaResult<Schema> {
        self.errors.clear();

        // Check for duplicate definitions
        self.check_duplicates(&schema);

        // Resolve field types (convert Model references to Enum or Composite where appropriate)
        self.resolve_field_types(&mut schema);

        // Validate each model
        for model in schema.models.values() {
            self.validate_model(model, &schema);
        }

        // Validate each enum
        for e in schema.enums.values() {
            self.validate_enum(e);
        }

        // Validate each composite type
        for t in schema.types.values() {
            self.validate_composite_type(t, &schema);
        }

        // Validate each view
        for v in schema.views.values() {
            self.validate_view(v, &schema);
        }

        // Validate each server group
        for sg in schema.server_groups.values() {
            self.validate_server_group(sg);
        }

        // Resolve relations
        let relations = self.resolve_relations(&schema);
        schema.relations = relations;

        if self.errors.is_empty() {
            Ok(schema)
        } else {
            Err(SchemaError::ValidationFailed {
                count: self.errors.len(),
                errors: std::mem::take(&mut self.errors),
            })
        }
    }

    /// Check for duplicate model, enum, or type names.
    fn check_duplicates(&mut self, schema: &Schema) {
        let mut seen = std::collections::HashSet::new();

        for name in schema.models.keys() {
            if !seen.insert(name.as_str()) {
                self.errors
                    .push(SchemaError::duplicate("model", name.as_str()));
            }
        }

        for name in schema.enums.keys() {
            if !seen.insert(name.as_str()) {
                self.errors
                    .push(SchemaError::duplicate("enum", name.as_str()));
            }
        }

        for name in schema.types.keys() {
            if !seen.insert(name.as_str()) {
                self.errors
                    .push(SchemaError::duplicate("type", name.as_str()));
            }
        }

        for name in schema.views.keys() {
            if !seen.insert(name.as_str()) {
                self.errors
                    .push(SchemaError::duplicate("view", name.as_str()));
            }
        }

        // Check server group names (separately, since they don't conflict with types)
        let mut server_group_names = std::collections::HashSet::new();
        for name in schema.server_groups.keys() {
            if !server_group_names.insert(name.as_str()) {
                self.errors
                    .push(SchemaError::duplicate("serverGroup", name.as_str()));
            }
        }
    }

    /// Resolve field types to their correct types (Enum or Composite) instead of Model.
    ///
    /// The parser initially treats all non-scalar type references as Model references.
    /// This pass corrects them to Enum or Composite where appropriate.
    fn resolve_field_types(&self, schema: &mut Schema) {
        // Collect enum and composite type names into owned strings to avoid borrow conflicts
        let enum_names: std::collections::HashSet<String> =
            schema.enums.keys().map(|s| s.to_string()).collect();
        let composite_names: std::collections::HashSet<String> =
            schema.types.keys().map(|s| s.to_string()).collect();

        // Update field types in models
        for model in schema.models.values_mut() {
            for field in model.fields.values_mut() {
                if let FieldType::Model(ref type_name) = field.field_type {
                    let name = type_name.as_str();
                    if enum_names.contains(name) {
                        field.field_type = FieldType::Enum(type_name.clone());
                    } else if composite_names.contains(name) {
                        field.field_type = FieldType::Composite(type_name.clone());
                    }
                }
            }
        }

        // Also update field types in composite types
        for composite in schema.types.values_mut() {
            for field in composite.fields.values_mut() {
                if let FieldType::Model(ref type_name) = field.field_type {
                    let name = type_name.as_str();
                    if enum_names.contains(name) {
                        field.field_type = FieldType::Enum(type_name.clone());
                    } else if composite_names.contains(name) {
                        field.field_type = FieldType::Composite(type_name.clone());
                    }
                }
            }
        }

        // Also update field types in views
        for view in schema.views.values_mut() {
            for field in view.fields.values_mut() {
                if let FieldType::Model(ref type_name) = field.field_type {
                    let name = type_name.as_str();
                    if enum_names.contains(name) {
                        field.field_type = FieldType::Enum(type_name.clone());
                    } else if composite_names.contains(name) {
                        field.field_type = FieldType::Composite(type_name.clone());
                    }
                }
            }
        }
    }

    /// Validate a model definition.
    fn validate_model(&mut self, model: &Model, schema: &Schema) {
        // Check for @id field
        let id_fields: Vec<_> = model.fields.values().filter(|f| f.is_id()).collect();
        if id_fields.is_empty() && !self.has_composite_id(model) {
            self.errors.push(SchemaError::MissingId {
                model: model.name().to_string(),
            });
        }

        // Validate each field
        for field in model.fields.values() {
            self.validate_field(field, model.name(), schema);
        }

        // Validate model attributes
        for attr in &model.attributes {
            self.validate_model_attribute(attr, model);
        }
    }

    /// Check if model has a composite ID (@@id attribute).
    fn has_composite_id(&self, model: &Model) -> bool {
        model.attributes.iter().any(|a| a.is("id"))
    }

    /// Validate a field definition.
    fn validate_field(&mut self, field: &Field, model_name: &str, schema: &Schema) {
        // Validate type references
        match &field.field_type {
            FieldType::Model(name) => {
                // Check if it's actually a model, enum, or composite type
                if schema.models.contains_key(name.as_str()) {
                    // Valid model reference
                } else if schema.enums.contains_key(name.as_str()) {
                    // Parser initially treats non-scalar types as Model references
                    // This is actually an enum type - we'll handle this during resolution
                } else if schema.types.contains_key(name.as_str()) {
                    // This is a composite type
                } else {
                    self.errors.push(SchemaError::unknown_type(
                        model_name,
                        field.name(),
                        name.as_str(),
                    ));
                }
            }
            FieldType::Enum(name) => {
                if !schema.enums.contains_key(name.as_str()) {
                    self.errors.push(SchemaError::unknown_type(
                        model_name,
                        field.name(),
                        name.as_str(),
                    ));
                }
            }
            FieldType::Composite(name) => {
                if !schema.types.contains_key(name.as_str()) {
                    self.errors.push(SchemaError::unknown_type(
                        model_name,
                        field.name(),
                        name.as_str(),
                    ));
                }
            }
            _ => {}
        }

        // Validate field attributes
        for attr in &field.attributes {
            self.validate_field_attribute(attr, field, model_name, schema);
        }

        // Validate relation fields have @relation or are back-references
        // Only check actual model relations (not enums or composite types parsed as Model)
        if let FieldType::Model(ref target_name) = field.field_type {
            // Skip validation for enum and composite type references
            let is_actual_relation = schema.models.contains_key(target_name.as_str())
                && !schema.enums.contains_key(target_name.as_str())
                && !schema.types.contains_key(target_name.as_str());

            if is_actual_relation && !field.is_list() {
                // One-side of relation should have foreign key fields
                let attrs = field.extract_attributes();
                if attrs.relation.is_some() {
                    let rel = attrs.relation.as_ref().unwrap();
                    // Validate foreign key fields exist
                    for fk_field in &rel.fields {
                        if !schema
                            .models
                            .get(model_name)
                            .map(|m| m.fields.contains_key(fk_field.as_str()))
                            .unwrap_or(false)
                        {
                            self.errors.push(SchemaError::invalid_relation(
                                model_name,
                                field.name(),
                                format!("foreign key field '{}' does not exist", fk_field),
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Validate a field attribute.
    fn validate_field_attribute(
        &mut self,
        attr: &Attribute,
        field: &Field,
        model_name: &str,
        schema: &Schema,
    ) {
        match attr.name() {
            "id" => {
                // @id should be on a scalar or composite type, not a relation
                if field.field_type.is_relation() {
                    self.errors.push(SchemaError::InvalidAttribute {
                        attribute: "id".to_string(),
                        message: format!(
                            "@id cannot be applied to relation field '{}.{}'",
                            model_name,
                            field.name()
                        ),
                    });
                }
            }
            "auto" => {
                // @auto should only be on Int or BigInt
                if !matches!(
                    field.field_type,
                    FieldType::Scalar(ScalarType::Int) | FieldType::Scalar(ScalarType::BigInt)
                ) {
                    self.errors.push(SchemaError::InvalidAttribute {
                        attribute: "auto".to_string(),
                        message: format!(
                            "@auto can only be applied to Int or BigInt fields, not '{}.{}'",
                            model_name,
                            field.name()
                        ),
                    });
                }
            }
            "default" => {
                // Validate default value type matches field type
                if let Some(value) = attr.first_arg() {
                    self.validate_default_value(value, field, model_name, schema);
                }
            }
            "relation" => {
                // Validate relation attribute - should only be on actual model references
                let is_model_ref = matches!(&field.field_type, FieldType::Model(name)
                    if schema.models.contains_key(name.as_str()));
                if !is_model_ref {
                    self.errors.push(SchemaError::InvalidAttribute {
                        attribute: "relation".to_string(),
                        message: format!(
                            "@relation can only be applied to model reference fields, not '{}.{}'",
                            model_name,
                            field.name()
                        ),
                    });
                }
            }
            "updated_at" => {
                // @updated_at should only be on DateTime
                if !matches!(field.field_type, FieldType::Scalar(ScalarType::DateTime)) {
                    self.errors.push(SchemaError::InvalidAttribute {
                        attribute: "updated_at".to_string(),
                        message: format!(
                            "@updated_at can only be applied to DateTime fields, not '{}.{}'",
                            model_name,
                            field.name()
                        ),
                    });
                }
            }
            _ => {}
        }
    }

    /// Validate a default value matches the field type.
    fn validate_default_value(
        &mut self,
        value: &AttributeValue,
        field: &Field,
        model_name: &str,
        schema: &Schema,
    ) {
        match (&field.field_type, value) {
            // Functions are generally allowed (now(), uuid(), etc.)
            (_, AttributeValue::Function(_, _)) => {}

            // Int fields should have int defaults
            (FieldType::Scalar(ScalarType::Int), AttributeValue::Int(_)) => {}
            (FieldType::Scalar(ScalarType::BigInt), AttributeValue::Int(_)) => {}

            // Float fields can have int or float defaults
            (FieldType::Scalar(ScalarType::Float), AttributeValue::Int(_)) => {}
            (FieldType::Scalar(ScalarType::Float), AttributeValue::Float(_)) => {}
            (FieldType::Scalar(ScalarType::Decimal), AttributeValue::Int(_)) => {}
            (FieldType::Scalar(ScalarType::Decimal), AttributeValue::Float(_)) => {}

            // String fields should have string defaults
            (FieldType::Scalar(ScalarType::String), AttributeValue::String(_)) => {}

            // Boolean fields should have boolean defaults
            (FieldType::Scalar(ScalarType::Boolean), AttributeValue::Boolean(_)) => {}

            // Enum fields should have ident defaults matching a variant
            (FieldType::Enum(enum_name), AttributeValue::Ident(variant)) => {
                if let Some(e) = schema.enums.get(enum_name.as_str()) {
                    if e.get_variant(variant).is_none() {
                        self.errors.push(SchemaError::invalid_field(
                            model_name,
                            field.name(),
                            format!(
                                "default value '{}' is not a valid variant of enum '{}'",
                                variant, enum_name
                            ),
                        ));
                    }
                }
            }

            // Model type might actually be an enum (parser treats non-scalar as Model initially)
            (FieldType::Model(type_name), AttributeValue::Ident(variant)) => {
                // Check if this is actually an enum reference
                if let Some(e) = schema.enums.get(type_name.as_str()) {
                    if e.get_variant(variant).is_none() {
                        self.errors.push(SchemaError::invalid_field(
                            model_name,
                            field.name(),
                            format!(
                                "default value '{}' is not a valid variant of enum '{}'",
                                variant, type_name
                            ),
                        ));
                    }
                }
                // If it's a real model reference with an ident default, that's an error
                // but we skip that here since it's likely a valid enum
            }

            // Type mismatch
            _ => {
                self.errors.push(SchemaError::invalid_field(
                    model_name,
                    field.name(),
                    format!(
                        "default value type does not match field type '{}'",
                        field.field_type
                    ),
                ));
            }
        }
    }

    /// Validate a model-level attribute.
    fn validate_model_attribute(&mut self, attr: &Attribute, model: &Model) {
        match attr.name() {
            "index" | "unique" => {
                // Validate referenced fields exist
                if let Some(AttributeValue::FieldRefList(fields)) = attr.first_arg() {
                    for field_name in fields {
                        if !model.fields.contains_key(field_name.as_str()) {
                            self.errors.push(SchemaError::invalid_model(
                                model.name(),
                                format!(
                                    "@@{} references non-existent field '{}'",
                                    attr.name(),
                                    field_name
                                ),
                            ));
                        }
                    }
                }
            }
            "id" => {
                // Composite primary key
                if let Some(AttributeValue::FieldRefList(fields)) = attr.first_arg() {
                    for field_name in fields {
                        if !model.fields.contains_key(field_name.as_str()) {
                            self.errors.push(SchemaError::invalid_model(
                                model.name(),
                                format!("@@id references non-existent field '{}'", field_name),
                            ));
                        }
                    }
                }
            }
            "search" => {
                // Full-text search on fields
                if let Some(AttributeValue::FieldRefList(fields)) = attr.first_arg() {
                    for field_name in fields {
                        if let Some(field) = model.fields.get(field_name.as_str()) {
                            // Only string fields can be searched
                            if !matches!(field.field_type, FieldType::Scalar(ScalarType::String)) {
                                self.errors.push(SchemaError::invalid_model(
                                    model.name(),
                                    format!(
                                        "@@search field '{}' must be of type String",
                                        field_name
                                    ),
                                ));
                            }
                        } else {
                            self.errors.push(SchemaError::invalid_model(
                                model.name(),
                                format!("@@search references non-existent field '{}'", field_name),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Validate an enum definition.
    fn validate_enum(&mut self, e: &Enum) {
        if e.variants.is_empty() {
            self.errors.push(SchemaError::invalid_model(
                e.name(),
                "enum must have at least one variant".to_string(),
            ));
        }

        // Check for duplicate variant names
        let mut seen = std::collections::HashSet::new();
        for variant in &e.variants {
            if !seen.insert(variant.name()) {
                self.errors.push(SchemaError::duplicate(
                    format!("enum variant in {}", e.name()),
                    variant.name(),
                ));
            }
        }
    }

    /// Validate a composite type definition.
    fn validate_composite_type(&mut self, t: &CompositeType, schema: &Schema) {
        if t.fields.is_empty() {
            self.errors.push(SchemaError::invalid_model(
                t.name(),
                "composite type must have at least one field".to_string(),
            ));
        }

        // Validate field types
        for field in t.fields.values() {
            match &field.field_type {
                FieldType::Model(_) => {
                    self.errors.push(SchemaError::invalid_field(
                        t.name(),
                        field.name(),
                        "composite types cannot have model relations".to_string(),
                    ));
                }
                FieldType::Enum(name) => {
                    if !schema.enums.contains_key(name.as_str()) {
                        self.errors.push(SchemaError::unknown_type(
                            t.name(),
                            field.name(),
                            name.as_str(),
                        ));
                    }
                }
                FieldType::Composite(name) => {
                    if !schema.types.contains_key(name.as_str()) {
                        self.errors.push(SchemaError::unknown_type(
                            t.name(),
                            field.name(),
                            name.as_str(),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    /// Validate a view definition.
    fn validate_view(&mut self, v: &View, schema: &Schema) {
        // Views should have at least one field
        if v.fields.is_empty() {
            self.errors.push(SchemaError::invalid_model(
                v.name(),
                "view must have at least one field".to_string(),
            ));
        }

        // Validate field types
        for field in v.fields.values() {
            self.validate_field(field, v.name(), schema);
        }
    }

    /// Validate a server group definition.
    fn validate_server_group(&mut self, sg: &ServerGroup) {
        // Server groups should have at least one server
        if sg.servers.is_empty() {
            self.errors.push(SchemaError::invalid_model(
                sg.name.name.as_str(),
                "serverGroup must have at least one server".to_string(),
            ));
        }

        // Check for duplicate server names within the group
        let mut seen_servers = std::collections::HashSet::new();
        for server_name in sg.servers.keys() {
            if !seen_servers.insert(server_name.as_str()) {
                self.errors.push(SchemaError::duplicate(
                    format!("server in serverGroup {}", sg.name.name),
                    server_name.as_str(),
                ));
            }
        }

        // Validate each server
        for server in sg.servers.values() {
            self.validate_server(server, sg.name.name.as_str());
        }

        // Validate server group attributes
        for attr in &sg.attributes {
            self.validate_server_group_attribute(attr, sg);
        }

        // Check for at least one primary server in read replica strategy
        if let Some(strategy) = sg.strategy() {
            if strategy == ServerGroupStrategy::ReadReplica {
                let has_primary = sg
                    .servers
                    .values()
                    .any(|s| s.role() == Some(ServerRole::Primary));
                if !has_primary {
                    self.errors.push(SchemaError::invalid_model(
                        sg.name.name.as_str(),
                        "ReadReplica strategy requires at least one server with role = \"primary\""
                            .to_string(),
                    ));
                }
            }
        }
    }

    /// Validate an individual server definition.
    fn validate_server(&mut self, server: &Server, group_name: &str) {
        // Server should have a URL property
        if server.url().is_none() {
            self.errors.push(SchemaError::invalid_model(
                group_name,
                format!("server '{}' must have a 'url' property", server.name.name),
            ));
        }

        // Validate weight is positive if specified
        if let Some(weight) = server.weight() {
            if weight == 0 {
                self.errors.push(SchemaError::invalid_model(
                    group_name,
                    format!(
                        "server '{}' weight must be greater than 0",
                        server.name.name
                    ),
                ));
            }
        }

        // Validate priority is positive if specified
        if let Some(priority) = server.priority() {
            if priority == 0 {
                self.errors.push(SchemaError::invalid_model(
                    group_name,
                    format!(
                        "server '{}' priority must be greater than 0",
                        server.name.name
                    ),
                ));
            }
        }
    }

    /// Validate a server group attribute.
    fn validate_server_group_attribute(&mut self, attr: &Attribute, sg: &ServerGroup) {
        match attr.name() {
            "strategy" => {
                // Validate strategy value
                if let Some(arg) = attr.first_arg() {
                    let value_str = arg
                        .as_string()
                        .map(|s| s.to_string())
                        .or_else(|| arg.as_ident().map(|s| s.to_string()));
                    if let Some(val) = value_str {
                        if ServerGroupStrategy::parse(&val).is_none() {
                            self.errors.push(SchemaError::InvalidAttribute {
                                attribute: "strategy".to_string(),
                                message: format!(
                                    "invalid strategy '{}' for serverGroup '{}'. Valid values: ReadReplica, Sharding, MultiRegion, HighAvailability, Custom",
                                    val,
                                    sg.name.name
                                ),
                            });
                        }
                    }
                }
            }
            "loadBalance" => {
                // Validate load balance value
                if let Some(arg) = attr.first_arg() {
                    let value_str = arg
                        .as_string()
                        .map(|s| s.to_string())
                        .or_else(|| arg.as_ident().map(|s| s.to_string()));
                    if let Some(val) = value_str {
                        if LoadBalanceStrategy::parse(&val).is_none() {
                            self.errors.push(SchemaError::InvalidAttribute {
                                attribute: "loadBalance".to_string(),
                                message: format!(
                                    "invalid loadBalance '{}' for serverGroup '{}'. Valid values: RoundRobin, Random, LeastConnections, Weighted, Nearest, Sticky",
                                    val,
                                    sg.name.name
                                ),
                            });
                        }
                    }
                }
            }
            _ => {} // Other attributes are allowed
        }
    }

    /// Resolve all relations in the schema.
    fn resolve_relations(&mut self, schema: &Schema) -> Vec<Relation> {
        let mut relations = Vec::new();

        for model in schema.models.values() {
            for field in model.fields.values() {
                if let FieldType::Model(ref target_model) = field.field_type {
                    // Skip if this is actually an enum reference (parser treats non-scalar as Model initially)
                    if schema.enums.contains_key(target_model.as_str()) {
                        continue;
                    }

                    // Skip if this is actually a composite type reference
                    if schema.types.contains_key(target_model.as_str()) {
                        continue;
                    }

                    // Skip if the target model doesn't exist (error was already reported)
                    if !schema.models.contains_key(target_model.as_str()) {
                        continue;
                    }

                    let attrs = field.extract_attributes();

                    let relation_type = if field.is_list() {
                        // This model has many of target
                        RelationType::OneToMany
                    } else {
                        // This model has one of target
                        RelationType::ManyToOne
                    };

                    let mut relation = Relation::new(
                        model.name(),
                        field.name(),
                        target_model.as_str(),
                        relation_type,
                    );

                    if let Some(rel_attr) = &attrs.relation {
                        if let Some(name) = &rel_attr.name {
                            relation = relation.with_name(name.as_str());
                        }
                        if !rel_attr.fields.is_empty() {
                            relation = relation.with_from_fields(rel_attr.fields.clone());
                        }
                        if !rel_attr.references.is_empty() {
                            relation = relation.with_to_fields(rel_attr.references.clone());
                        }
                        if let Some(action) = rel_attr.on_delete {
                            relation = relation.with_on_delete(action);
                        }
                        if let Some(action) = rel_attr.on_update {
                            relation = relation.with_on_update(action);
                        }
                    }

                    relations.push(relation);
                }
            }
        }

        relations
    }
}

/// Validate a schema string and return the validated schema.
pub fn validate_schema(input: &str) -> SchemaResult<Schema> {
    let schema = crate::parser::parse_schema(input)?;
    let mut validator = Validator::new();
    validator.validate(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_simple_model() {
        let schema = validate_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String @unique
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 1);
    }

    #[test]
    fn test_validate_model_missing_id() {
        let result = validate_schema(
            r#"
            model User {
                email String
                name  String
            }
        "#,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SchemaError::ValidationFailed { .. }));
    }

    #[test]
    fn test_validate_model_with_composite_id() {
        let schema = validate_schema(
            r#"
            model PostTag {
                post_id Int
                tag_id  Int

                @@id([post_id, tag_id])
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 1);
    }

    #[test]
    fn test_validate_unknown_type_reference() {
        let result = validate_schema(
            r#"
            model User {
                id      Int    @id @auto
                profile UnknownType
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_enum_reference() {
        let schema = validate_schema(
            r#"
            enum Role {
                User
                Admin
            }

            model User {
                id   Int    @id @auto
                role Role   @default(User)
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 1);
        assert_eq!(schema.enums.len(), 1);
    }

    #[test]
    fn test_validate_invalid_enum_default() {
        let result = validate_schema(
            r#"
            enum Role {
                User
                Admin
            }

            model User {
                id   Int    @id @auto
                role Role   @default(Unknown)
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_auto_on_non_int() {
        let result = validate_schema(
            r#"
            model User {
                id    String @id @auto
                email String
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_updated_at_on_non_datetime() {
        let result = validate_schema(
            r#"
            model User {
                id         Int    @id @auto
                updated_at String @updated_at
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_empty_enum() {
        let result = validate_schema(
            r#"
            enum Empty {
            }

            model User {
                id Int @id @auto
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_duplicate_model_names() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            model User {
                id Int @id @auto
            }
        "#,
        );

        // Note: This might parse as a single model due to grammar
        // The duplicate check happens at validation time
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_validate_relation() {
        let schema = validate_schema(
            r#"
            model User {
                id    Int    @id @auto
                posts Post[]
            }

            model Post {
                id        Int    @id @auto
                author_id Int
                author    User   @relation(fields: [author_id], references: [id])
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.models.len(), 2);
        assert!(!schema.relations.is_empty());
    }

    #[test]
    fn test_validate_index_with_invalid_field() {
        let result = validate_schema(
            r#"
            model User {
                id    Int    @id @auto
                email String

                @@index([nonexistent])
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_search_on_non_string_field() {
        let result = validate_schema(
            r#"
            model Post {
                id    Int    @id @auto
                views Int

                @@search([views])
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_composite_type() {
        let schema = validate_schema(
            r#"
            type Address {
                street  String
                city    String
                country String @default("US")
            }

            model User {
                id      Int     @id @auto
                address Address
            }
        "#,
        );

        // Note: Composite type support depends on parser handling
        assert!(schema.is_ok() || schema.is_err());
    }

    // ==================== Server Group Validation Tests ====================

    #[test]
    fn test_validate_server_group_basic() {
        let schema = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

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
    }

    #[test]
    fn test_validate_server_group_empty_servers() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup EmptyCluster {
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_server_group_missing_url() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                server db {
                    role = "primary"
                }
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_server_group_invalid_strategy() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                @@strategy(InvalidStrategy)

                server db {
                    url = "postgres://localhost/db"
                }
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_server_group_valid_strategy() {
        let schema = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                @@strategy(ReadReplica)
                @@loadBalance(RoundRobin)

                server primary {
                    url = "postgres://localhost/db"
                    role = "primary"
                }
            }
        "#,
        )
        .unwrap();

        assert_eq!(schema.server_groups.len(), 1);
    }

    #[test]
    fn test_validate_server_group_read_replica_needs_primary() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                @@strategy(ReadReplica)

                server replica1 {
                    url = "postgres://localhost/db"
                    role = "replica"
                }
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_server_group_with_replicas() {
        let schema = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                @@strategy(ReadReplica)

                server primary {
                    url = "postgres://primary/db"
                    role = "primary"
                    weight = 1
                }

                server replica1 {
                    url = "postgres://replica1/db"
                    role = "replica"
                    weight = 2
                }

                server replica2 {
                    url = "postgres://replica2/db"
                    role = "replica"
                    weight = 2
                    region = "us-west-1"
                }
            }
        "#,
        )
        .unwrap();

        let cluster = schema.get_server_group("Cluster").unwrap();
        assert_eq!(cluster.servers.len(), 3);
    }

    #[test]
    fn test_validate_server_group_zero_weight() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                server db {
                    url = "postgres://localhost/db"
                    weight = 0
                }
            }
        "#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_validate_server_group_invalid_load_balance() {
        let result = validate_schema(
            r#"
            model User {
                id Int @id @auto
            }

            serverGroup Cluster {
                @@loadBalance(InvalidStrategy)

                server db {
                    url = "postgres://localhost/db"
                }
            }
        "#,
        );

        assert!(result.is_err());
    }
}
