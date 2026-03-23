//! Top-level schema definition.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{
    CompositeType, Datasource, Enum, Generator, Model, Policy, Relation, ServerGroup, View,
};

/// A complete Prax schema.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Datasource configuration (database connection and extensions).
    pub datasource: Option<Datasource>,
    /// Generator configurations.
    pub generators: IndexMap<SmolStr, Generator>,
    /// All models in the schema.
    pub models: IndexMap<SmolStr, Model>,
    /// All enums in the schema.
    pub enums: IndexMap<SmolStr, Enum>,
    /// All composite types in the schema.
    pub types: IndexMap<SmolStr, CompositeType>,
    /// All views in the schema.
    pub views: IndexMap<SmolStr, View>,
    /// Server groups for multi-server configurations.
    pub server_groups: IndexMap<SmolStr, ServerGroup>,
    /// PostgreSQL Row-Level Security policies.
    pub policies: Vec<Policy>,
    /// Raw SQL definitions.
    pub raw_sql: Vec<RawSql>,
    /// Resolved relations (populated after validation).
    pub relations: Vec<Relation>,
}

impl Schema {
    /// Create a new empty schema.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the datasource configuration.
    pub fn set_datasource(&mut self, datasource: Datasource) {
        self.datasource = Some(datasource);
    }

    /// Get the datasource configuration.
    pub fn datasource(&self) -> Option<&Datasource> {
        self.datasource.as_ref()
    }

    /// Check if the schema has vector extension enabled.
    pub fn has_vector_support(&self) -> bool {
        self.datasource
            .as_ref()
            .is_some_and(|ds| ds.has_vector_support())
    }

    /// Get all required PostgreSQL extensions from the datasource.
    pub fn required_extensions(&self) -> Vec<&super::PostgresExtension> {
        self.datasource
            .as_ref()
            .map(|ds| ds.extensions.iter().collect())
            .unwrap_or_default()
    }

    /// Add a generator to the schema.
    pub fn add_generator(&mut self, generator: Generator) {
        self.generators.insert(generator.name.clone(), generator);
    }

    /// Get a generator by name.
    pub fn get_generator(&self, name: &str) -> Option<&Generator> {
        self.generators.get(name)
    }

    /// Get all enabled generators.
    pub fn enabled_generators(&self) -> Vec<&Generator> {
        self.generators
            .values()
            .filter(|g| g.is_enabled())
            .collect()
    }

    /// Add a model to the schema.
    pub fn add_model(&mut self, model: Model) {
        self.models.insert(model.name.name.clone(), model);
    }

    /// Add an enum to the schema.
    pub fn add_enum(&mut self, e: Enum) {
        self.enums.insert(e.name.name.clone(), e);
    }

    /// Add a composite type to the schema.
    pub fn add_type(&mut self, t: CompositeType) {
        self.types.insert(t.name.name.clone(), t);
    }

    /// Add a view to the schema.
    pub fn add_view(&mut self, v: View) {
        self.views.insert(v.name.name.clone(), v);
    }

    /// Add a server group to the schema.
    pub fn add_server_group(&mut self, sg: ServerGroup) {
        self.server_groups.insert(sg.name.name.clone(), sg);
    }

    /// Add a PostgreSQL Row-Level Security policy.
    pub fn add_policy(&mut self, policy: Policy) {
        self.policies.push(policy);
    }

    /// Add a raw SQL definition.
    pub fn add_raw_sql(&mut self, sql: RawSql) {
        self.raw_sql.push(sql);
    }

    /// Get a model by name.
    pub fn get_model(&self, name: &str) -> Option<&Model> {
        self.models.get(name)
    }

    /// Get a mutable model by name.
    pub fn get_model_mut(&mut self, name: &str) -> Option<&mut Model> {
        self.models.get_mut(name)
    }

    /// Get an enum by name.
    pub fn get_enum(&self, name: &str) -> Option<&Enum> {
        self.enums.get(name)
    }

    /// Get a composite type by name.
    pub fn get_type(&self, name: &str) -> Option<&CompositeType> {
        self.types.get(name)
    }

    /// Get a view by name.
    pub fn get_view(&self, name: &str) -> Option<&View> {
        self.views.get(name)
    }

    /// Get a server group by name.
    pub fn get_server_group(&self, name: &str) -> Option<&ServerGroup> {
        self.server_groups.get(name)
    }

    /// Get all server group names.
    pub fn server_group_names(&self) -> impl Iterator<Item = &str> {
        self.server_groups.keys().map(|s| s.as_str())
    }

    /// Get a policy by name.
    pub fn get_policy(&self, name: &str) -> Option<&Policy> {
        self.policies.iter().find(|p| p.name() == name)
    }

    /// Get all policies for a specific model/table.
    pub fn policies_for(&self, model: &str) -> Vec<&Policy> {
        self.policies
            .iter()
            .filter(|p| p.table() == model)
            .collect()
    }

    /// Check if a model has Row-Level Security policies.
    pub fn has_policies(&self, model: &str) -> bool {
        self.policies.iter().any(|p| p.table() == model)
    }

    /// Get all policy names.
    pub fn policy_names(&self) -> impl Iterator<Item = &str> {
        self.policies.iter().map(|p| p.name())
    }

    /// Check if a type name exists (model, enum, type, or view).
    pub fn type_exists(&self, name: &str) -> bool {
        self.models.contains_key(name)
            || self.enums.contains_key(name)
            || self.types.contains_key(name)
            || self.views.contains_key(name)
    }

    /// Get all model names.
    pub fn model_names(&self) -> impl Iterator<Item = &str> {
        self.models.keys().map(|s| s.as_str())
    }

    /// Get all enum names.
    pub fn enum_names(&self) -> impl Iterator<Item = &str> {
        self.enums.keys().map(|s| s.as_str())
    }

    /// Get relations for a specific model.
    pub fn relations_for(&self, model: &str) -> Vec<&Relation> {
        self.relations
            .iter()
            .filter(|r| r.from_model == model || r.to_model == model)
            .collect()
    }

    /// Get relations originating from a specific model.
    pub fn relations_from(&self, model: &str) -> Vec<&Relation> {
        self.relations
            .iter()
            .filter(|r| r.from_model == model)
            .collect()
    }

    /// Merge another schema into this one.
    pub fn merge(&mut self, other: Schema) {
        self.models.extend(other.models);
        self.enums.extend(other.enums);
        self.types.extend(other.types);
        self.views.extend(other.views);
        self.server_groups.extend(other.server_groups);
        self.policies.extend(other.policies);
        self.raw_sql.extend(other.raw_sql);
    }
}

/// A raw SQL definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSql {
    /// Name/identifier for the SQL (e.g., view name).
    pub name: SmolStr,
    /// The raw SQL content.
    pub sql: String,
}

impl RawSql {
    /// Create a new raw SQL definition.
    pub fn new(name: impl Into<SmolStr>, sql: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sql: sql.into(),
        }
    }
}

/// Schema statistics for debugging/info.
#[derive(Debug, Clone, Default)]
pub struct SchemaStats {
    /// Number of models.
    pub model_count: usize,
    /// Number of enums.
    pub enum_count: usize,
    /// Number of composite types.
    pub type_count: usize,
    /// Number of views.
    pub view_count: usize,
    /// Number of server groups.
    pub server_group_count: usize,
    /// Number of RLS policies.
    pub policy_count: usize,
    /// Total number of fields across all models.
    pub field_count: usize,
    /// Number of relations.
    pub relation_count: usize,
}

impl Schema {
    /// Get statistics about the schema.
    pub fn stats(&self) -> SchemaStats {
        SchemaStats {
            model_count: self.models.len(),
            enum_count: self.enums.len(),
            type_count: self.types.len(),
            view_count: self.views.len(),
            server_group_count: self.server_groups.len(),
            policy_count: self.policies.len(),
            field_count: self.models.values().map(|m| m.fields.len()).sum(),
            relation_count: self.relations.len(),
        }
    }
}

impl std::fmt::Display for Schema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        write!(
            f,
            "Schema({} models, {} enums, {} types, {} views, {} server groups, {} policies, {} fields, {} relations)",
            stats.model_count,
            stats.enum_count,
            stats.type_count,
            stats.view_count,
            stats.server_group_count,
            stats.policy_count,
            stats.field_count,
            stats.relation_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        Attribute, EnumVariant, Field, FieldType, Ident, Policy, RelationType, ScalarType, Span,
        TypeModifier,
    };

    fn make_span() -> Span {
        Span::new(0, 10)
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name, make_span())
    }

    fn make_model(name: &str) -> Model {
        let mut model = Model::new(make_ident(name), make_span());
        let id_field = make_id_field();
        model.add_field(id_field);
        model
    }

    fn make_id_field() -> Field {
        let mut field = Field::new(
            make_ident("id"),
            FieldType::Scalar(ScalarType::Int),
            TypeModifier::Required,
            vec![],
            make_span(),
        );
        field
            .attributes
            .push(Attribute::simple(make_ident("id"), make_span()));
        field
    }

    fn make_field(name: &str, field_type: FieldType) -> Field {
        Field::new(
            make_ident(name),
            field_type,
            TypeModifier::Required,
            vec![],
            make_span(),
        )
    }

    fn make_enum(name: &str, variants: &[&str]) -> Enum {
        let mut e = Enum::new(make_ident(name), make_span());
        for v in variants {
            e.add_variant(EnumVariant::new(make_ident(v), make_span()));
        }
        e
    }

    // ==================== Schema Tests ====================

    #[test]
    fn test_schema_new() {
        let schema = Schema::new();
        assert!(schema.models.is_empty());
        assert!(schema.enums.is_empty());
        assert!(schema.types.is_empty());
        assert!(schema.views.is_empty());
        assert!(schema.policies.is_empty());
        assert!(schema.raw_sql.is_empty());
        assert!(schema.relations.is_empty());
    }

    #[test]
    fn test_schema_default() {
        let schema = Schema::default();
        assert!(schema.models.is_empty());
    }

    #[test]
    fn test_schema_add_model() {
        let mut schema = Schema::new();
        let model = make_model("User");

        schema.add_model(model);

        assert_eq!(schema.models.len(), 1);
        assert!(schema.models.contains_key("User"));
    }

    #[test]
    fn test_schema_add_multiple_models() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));
        schema.add_model(make_model("Post"));
        schema.add_model(make_model("Comment"));

        assert_eq!(schema.models.len(), 3);
    }

    #[test]
    fn test_schema_add_enum() {
        let mut schema = Schema::new();
        let e = make_enum("Role", &["User", "Admin"]);

        schema.add_enum(e);

        assert_eq!(schema.enums.len(), 1);
        assert!(schema.enums.contains_key("Role"));
    }

    #[test]
    fn test_schema_add_type() {
        let mut schema = Schema::new();
        let ct = CompositeType::new(make_ident("Address"), make_span());

        schema.add_type(ct);

        assert_eq!(schema.types.len(), 1);
        assert!(schema.types.contains_key("Address"));
    }

    #[test]
    fn test_schema_add_view() {
        let mut schema = Schema::new();
        let view = View::new(make_ident("UserStats"), make_span());

        schema.add_view(view);

        assert_eq!(schema.views.len(), 1);
        assert!(schema.views.contains_key("UserStats"));
    }

    #[test]
    fn test_schema_add_raw_sql() {
        let mut schema = Schema::new();
        let sql = RawSql::new("migration_1", "CREATE TABLE test ();");

        schema.add_raw_sql(sql);

        assert_eq!(schema.raw_sql.len(), 1);
    }

    #[test]
    fn test_schema_get_model() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));

        let model = schema.get_model("User");
        assert!(model.is_some());
        assert_eq!(model.unwrap().name(), "User");

        assert!(schema.get_model("NonExistent").is_none());
    }

    #[test]
    fn test_schema_get_model_mut() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));

        let model = schema.get_model_mut("User");
        assert!(model.is_some());

        // Modify the model
        let model = model.unwrap();
        model.add_field(make_field("email", FieldType::Scalar(ScalarType::String)));

        // Verify modification persisted
        assert_eq!(schema.get_model("User").unwrap().fields.len(), 2);
    }

    #[test]
    fn test_schema_get_enum() {
        let mut schema = Schema::new();
        schema.add_enum(make_enum("Role", &["User", "Admin"]));

        let e = schema.get_enum("Role");
        assert!(e.is_some());
        assert_eq!(e.unwrap().name(), "Role");

        assert!(schema.get_enum("NonExistent").is_none());
    }

    #[test]
    fn test_schema_get_type() {
        let mut schema = Schema::new();
        schema.add_type(CompositeType::new(make_ident("Address"), make_span()));

        let ct = schema.get_type("Address");
        assert!(ct.is_some());

        assert!(schema.get_type("NonExistent").is_none());
    }

    #[test]
    fn test_schema_get_view() {
        let mut schema = Schema::new();
        schema.add_view(View::new(make_ident("Stats"), make_span()));

        let v = schema.get_view("Stats");
        assert!(v.is_some());

        assert!(schema.get_view("NonExistent").is_none());
    }

    #[test]
    fn test_schema_type_exists() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));
        schema.add_enum(make_enum("Role", &["User"]));
        schema.add_type(CompositeType::new(make_ident("Address"), make_span()));
        schema.add_view(View::new(make_ident("Stats"), make_span()));

        assert!(schema.type_exists("User")); // model
        assert!(schema.type_exists("Role")); // enum
        assert!(schema.type_exists("Address")); // type
        assert!(schema.type_exists("Stats")); // view
        assert!(!schema.type_exists("NonExistent"));
    }

    #[test]
    fn test_schema_model_names() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));
        schema.add_model(make_model("Post"));

        let names: Vec<_> = schema.model_names().collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"User"));
        assert!(names.contains(&"Post"));
    }

    #[test]
    fn test_schema_enum_names() {
        let mut schema = Schema::new();
        schema.add_enum(make_enum("Role", &["User"]));
        schema.add_enum(make_enum("Status", &["Active"]));

        let names: Vec<_> = schema.enum_names().collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Role"));
        assert!(names.contains(&"Status"));
    }

    #[test]
    fn test_schema_relations_for() {
        let mut schema = Schema::new();
        schema.relations.push(Relation::new(
            "Post",
            "author",
            "User",
            RelationType::ManyToOne,
        ));
        schema.relations.push(Relation::new(
            "Comment",
            "user",
            "User",
            RelationType::ManyToOne,
        ));
        schema.relations.push(Relation::new(
            "Post",
            "tags",
            "Tag",
            RelationType::ManyToMany,
        ));

        let user_relations = schema.relations_for("User");
        assert_eq!(user_relations.len(), 2);

        let post_relations = schema.relations_for("Post");
        assert_eq!(post_relations.len(), 2);

        let tag_relations = schema.relations_for("Tag");
        assert_eq!(tag_relations.len(), 1);
    }

    #[test]
    fn test_schema_relations_from() {
        let mut schema = Schema::new();
        schema.relations.push(Relation::new(
            "Post",
            "author",
            "User",
            RelationType::ManyToOne,
        ));
        schema.relations.push(Relation::new(
            "Post",
            "tags",
            "Tag",
            RelationType::ManyToMany,
        ));
        schema.relations.push(Relation::new(
            "User",
            "posts",
            "Post",
            RelationType::OneToMany,
        ));

        let post_relations = schema.relations_from("Post");
        assert_eq!(post_relations.len(), 2);

        let user_relations = schema.relations_from("User");
        assert_eq!(user_relations.len(), 1);

        let tag_relations = schema.relations_from("Tag");
        assert_eq!(tag_relations.len(), 0);
    }

    #[test]
    fn test_schema_merge() {
        let mut schema1 = Schema::new();
        schema1.add_model(make_model("User"));
        schema1.add_enum(make_enum("Role", &["User"]));

        let mut schema2 = Schema::new();
        schema2.add_model(make_model("Post"));
        schema2.add_enum(make_enum("Status", &["Active"]));
        schema2.add_raw_sql(RawSql::new("init", "-- init"));

        schema1.merge(schema2);

        assert_eq!(schema1.models.len(), 2);
        assert_eq!(schema1.enums.len(), 2);
        assert_eq!(schema1.raw_sql.len(), 1);
    }

    #[test]
    fn test_schema_stats() {
        let mut schema = Schema::new();

        let mut user = make_model("User");
        user.add_field(make_field("email", FieldType::Scalar(ScalarType::String)));
        user.add_field(make_field("name", FieldType::Scalar(ScalarType::String)));
        schema.add_model(user);

        let mut post = make_model("Post");
        post.add_field(make_field("title", FieldType::Scalar(ScalarType::String)));
        schema.add_model(post);

        schema.add_enum(make_enum("Role", &["User", "Admin"]));
        schema.add_type(CompositeType::new(make_ident("Address"), make_span()));
        schema.add_view(View::new(make_ident("Stats"), make_span()));
        schema.relations.push(Relation::new(
            "Post",
            "author",
            "User",
            RelationType::ManyToOne,
        ));

        let stats = schema.stats();
        assert_eq!(stats.model_count, 2);
        assert_eq!(stats.enum_count, 1);
        assert_eq!(stats.type_count, 1);
        assert_eq!(stats.view_count, 1);
        assert_eq!(stats.field_count, 5); // 3 in User + 2 in Post
        assert_eq!(stats.relation_count, 1);
    }

    #[test]
    fn test_schema_display() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));
        schema.add_enum(make_enum("Role", &["User"]));

        let display = format!("{}", schema);
        assert!(display.contains("1 models"));
        assert!(display.contains("1 enums"));
        assert!(display.contains("0 policies"));
    }

    #[test]
    fn test_schema_equality() {
        let schema1 = Schema::new();
        let schema2 = Schema::new();
        assert_eq!(schema1, schema2);
    }

    #[test]
    fn test_schema_clone() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));

        let cloned = schema.clone();
        assert_eq!(cloned.models.len(), 1);
    }

    // ==================== RawSql Tests ====================

    #[test]
    fn test_raw_sql_new() {
        let sql = RawSql::new("create_users", "CREATE TABLE users ();");

        assert_eq!(sql.name.as_str(), "create_users");
        assert_eq!(sql.sql, "CREATE TABLE users ();");
    }

    #[test]
    fn test_raw_sql_from_strings() {
        let name = String::from("migration");
        let content = String::from("ALTER TABLE users ADD COLUMN age INT;");
        let sql = RawSql::new(name, content);

        assert_eq!(sql.name.as_str(), "migration");
    }

    #[test]
    fn test_raw_sql_equality() {
        let sql1 = RawSql::new("test", "SELECT 1;");
        let sql2 = RawSql::new("test", "SELECT 1;");
        let sql3 = RawSql::new("test", "SELECT 2;");

        assert_eq!(sql1, sql2);
        assert_ne!(sql1, sql3);
    }

    #[test]
    fn test_raw_sql_clone() {
        let sql = RawSql::new("test", "SELECT 1;");
        let cloned = sql.clone();
        assert_eq!(sql, cloned);
    }

    // ==================== SchemaStats Tests ====================

    #[test]
    fn test_schema_stats_default() {
        let stats = SchemaStats::default();
        assert_eq!(stats.model_count, 0);
        assert_eq!(stats.enum_count, 0);
        assert_eq!(stats.type_count, 0);
        assert_eq!(stats.view_count, 0);
        assert_eq!(stats.policy_count, 0);
        assert_eq!(stats.field_count, 0);
        assert_eq!(stats.relation_count, 0);
    }

    #[test]
    fn test_schema_stats_debug() {
        let stats = SchemaStats::default();
        let debug = format!("{:?}", stats);
        assert!(debug.contains("SchemaStats"));
    }

    #[test]
    fn test_schema_stats_clone() {
        let stats = SchemaStats {
            model_count: 5,
            enum_count: 2,
            type_count: 1,
            view_count: 3,
            server_group_count: 2,
            policy_count: 4,
            field_count: 25,
            relation_count: 10,
        };
        let cloned = stats.clone();
        assert_eq!(cloned.model_count, 5);
        assert_eq!(cloned.field_count, 25);
        assert_eq!(cloned.policy_count, 4);
    }

    // ==================== Policy Schema Tests ====================

    #[test]
    fn test_schema_add_policy() {
        let mut schema = Schema::new();
        let policy = Policy::new(make_ident("read_own"), make_ident("User"), make_span());

        schema.add_policy(policy);

        assert_eq!(schema.policies.len(), 1);
    }

    #[test]
    fn test_schema_get_policy() {
        let mut schema = Schema::new();
        schema.add_policy(Policy::new(
            make_ident("read_own"),
            make_ident("User"),
            make_span(),
        ));

        let policy = schema.get_policy("read_own");
        assert!(policy.is_some());
        assert_eq!(policy.unwrap().name(), "read_own");

        assert!(schema.get_policy("nonexistent").is_none());
    }

    #[test]
    fn test_schema_policies_for_model() {
        let mut schema = Schema::new();
        schema.add_policy(Policy::new(
            make_ident("user_read"),
            make_ident("User"),
            make_span(),
        ));
        schema.add_policy(Policy::new(
            make_ident("user_write"),
            make_ident("User"),
            make_span(),
        ));
        schema.add_policy(Policy::new(
            make_ident("post_read"),
            make_ident("Post"),
            make_span(),
        ));

        let user_policies = schema.policies_for("User");
        assert_eq!(user_policies.len(), 2);

        let post_policies = schema.policies_for("Post");
        assert_eq!(post_policies.len(), 1);

        let comment_policies = schema.policies_for("Comment");
        assert!(comment_policies.is_empty());
    }

    #[test]
    fn test_schema_has_policies() {
        let mut schema = Schema::new();
        schema.add_policy(Policy::new(
            make_ident("test"),
            make_ident("User"),
            make_span(),
        ));

        assert!(schema.has_policies("User"));
        assert!(!schema.has_policies("Post"));
    }

    #[test]
    fn test_schema_policy_names() {
        let mut schema = Schema::new();
        schema.add_policy(Policy::new(
            make_ident("policy1"),
            make_ident("User"),
            make_span(),
        ));
        schema.add_policy(Policy::new(
            make_ident("policy2"),
            make_ident("Post"),
            make_span(),
        ));

        let names: Vec<_> = schema.policy_names().collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"policy1"));
        assert!(names.contains(&"policy2"));
    }

    #[test]
    fn test_schema_merge_with_policies() {
        let mut schema1 = Schema::new();
        schema1.add_policy(Policy::new(
            make_ident("policy1"),
            make_ident("User"),
            make_span(),
        ));

        let mut schema2 = Schema::new();
        schema2.add_policy(Policy::new(
            make_ident("policy2"),
            make_ident("Post"),
            make_span(),
        ));

        schema1.merge(schema2);

        assert_eq!(schema1.policies.len(), 2);
    }

    #[test]
    fn test_schema_stats_with_policies() {
        let mut schema = Schema::new();
        schema.add_model(make_model("User"));
        schema.add_policy(Policy::new(
            make_ident("policy1"),
            make_ident("User"),
            make_span(),
        ));
        schema.add_policy(Policy::new(
            make_ident("policy2"),
            make_ident("User"),
            make_span(),
        ));

        let stats = schema.stats();
        assert_eq!(stats.model_count, 1);
        assert_eq!(stats.policy_count, 2);
    }
}
