//! Relation specification types.

use std::collections::HashMap;

/// Type of relation between models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationType {
    /// One-to-one relation (e.g., User has one Profile).
    OneToOne,
    /// One-to-many relation (e.g., User has many Posts).
    OneToMany,
    /// Many-to-one relation (e.g., Post belongs to User).
    ManyToOne,
    /// Many-to-many relation (e.g., Post has many Tags).
    ManyToMany,
}

impl RelationType {
    /// Check if this relation returns multiple records.
    pub fn is_many(&self) -> bool {
        matches!(self, Self::OneToMany | Self::ManyToMany)
    }

    /// Check if this relation returns a single record.
    pub fn is_one(&self) -> bool {
        matches!(self, Self::OneToOne | Self::ManyToOne)
    }
}

/// Specification for a relation between models.
#[derive(Debug, Clone)]
pub struct RelationSpec {
    /// Name of the relation (field name).
    pub name: String,
    /// Type of relation.
    pub relation_type: RelationType,
    /// Name of the related model.
    pub related_model: String,
    /// Name of the related table.
    pub related_table: String,
    /// Foreign key fields on this model.
    pub fields: Vec<String>,
    /// Referenced fields on the related model.
    pub references: Vec<String>,
    /// Join table for many-to-many relations.
    pub join_table: Option<JoinTableSpec>,
    /// On delete action.
    pub on_delete: Option<ReferentialAction>,
    /// On update action.
    pub on_update: Option<ReferentialAction>,
    /// Custom foreign key constraint name in the database.
    pub map: Option<String>,
}

impl RelationSpec {
    /// Create a one-to-one relation spec.
    pub fn one_to_one(
        name: impl Into<String>,
        related_model: impl Into<String>,
        related_table: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            relation_type: RelationType::OneToOne,
            related_model: related_model.into(),
            related_table: related_table.into(),
            fields: Vec::new(),
            references: Vec::new(),
            join_table: None,
            on_delete: None,
            on_update: None,
            map: None,
        }
    }

    /// Create a one-to-many relation spec.
    pub fn one_to_many(
        name: impl Into<String>,
        related_model: impl Into<String>,
        related_table: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            relation_type: RelationType::OneToMany,
            related_model: related_model.into(),
            related_table: related_table.into(),
            fields: Vec::new(),
            references: Vec::new(),
            join_table: None,
            on_delete: None,
            on_update: None,
            map: None,
        }
    }

    /// Create a many-to-one relation spec.
    pub fn many_to_one(
        name: impl Into<String>,
        related_model: impl Into<String>,
        related_table: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            relation_type: RelationType::ManyToOne,
            related_model: related_model.into(),
            related_table: related_table.into(),
            fields: Vec::new(),
            references: Vec::new(),
            join_table: None,
            on_delete: None,
            on_update: None,
            map: None,
        }
    }

    /// Create a many-to-many relation spec.
    pub fn many_to_many(
        name: impl Into<String>,
        related_model: impl Into<String>,
        related_table: impl Into<String>,
        join_table: JoinTableSpec,
    ) -> Self {
        Self {
            name: name.into(),
            relation_type: RelationType::ManyToMany,
            related_model: related_model.into(),
            related_table: related_table.into(),
            fields: Vec::new(),
            references: Vec::new(),
            join_table: Some(join_table),
            on_delete: None,
            on_update: None,
            map: None,
        }
    }

    /// Set the foreign key fields.
    pub fn fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Set the referenced fields.
    pub fn references(mut self, refs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.references = refs.into_iter().map(Into::into).collect();
        self
    }

    /// Set the on delete action.
    pub fn on_delete(mut self, action: ReferentialAction) -> Self {
        self.on_delete = Some(action);
        self
    }

    /// Set the on update action.
    pub fn on_update(mut self, action: ReferentialAction) -> Self {
        self.on_update = Some(action);
        self
    }

    /// Set the custom foreign key constraint name.
    pub fn map(mut self, name: impl Into<String>) -> Self {
        self.map = Some(name.into());
        self
    }

    /// Generate the JOIN clause for this relation.
    pub fn to_join_clause(&self, parent_alias: &str, child_alias: &str) -> String {
        if let Some(ref jt) = self.join_table {
            // Many-to-many join through join table
            format!(
                "JOIN {} ON {}.{} = {}.{} JOIN {} AS {} ON {}.{} = {}.{}",
                jt.table_name,
                parent_alias,
                self.fields.first().unwrap_or(&"id".to_string()),
                jt.table_name,
                jt.source_column,
                self.related_table,
                child_alias,
                jt.table_name,
                jt.target_column,
                child_alias,
                self.references.first().unwrap_or(&"id".to_string()),
            )
        } else {
            // Direct join
            let join_conditions: Vec<_> = self
                .fields
                .iter()
                .zip(self.references.iter())
                .map(|(f, r)| format!("{}.{} = {}.{}", parent_alias, f, child_alias, r))
                .collect();

            format!(
                "JOIN {} AS {} ON {}",
                self.related_table,
                child_alias,
                join_conditions.join(" AND ")
            )
        }
    }
}

/// Specification for a join table (many-to-many).
#[derive(Debug, Clone)]
pub struct JoinTableSpec {
    /// Name of the join table.
    pub table_name: String,
    /// Column referencing the source model.
    pub source_column: String,
    /// Column referencing the target model.
    pub target_column: String,
}

impl JoinTableSpec {
    /// Create a new join table spec.
    pub fn new(
        table_name: impl Into<String>,
        source_column: impl Into<String>,
        target_column: impl Into<String>,
    ) -> Self {
        Self {
            table_name: table_name.into(),
            source_column: source_column.into(),
            target_column: target_column.into(),
        }
    }
}

/// Referential action for cascading operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferentialAction {
    /// Cascade the operation to related records.
    Cascade,
    /// Set the foreign key to null.
    SetNull,
    /// Set the foreign key to default value.
    SetDefault,
    /// Restrict the operation if related records exist.
    Restrict,
    /// No action (let database handle).
    NoAction,
}

impl ReferentialAction {
    /// Get the SQL keyword for this action.
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
            Self::Restrict => "RESTRICT",
            Self::NoAction => "NO ACTION",
        }
    }
}

/// Registry of relation specifications for a model.
#[derive(Debug, Clone, Default)]
pub struct RelationRegistry {
    relations: HashMap<String, RelationSpec>,
}

impl RelationRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a relation.
    pub fn register(&mut self, spec: RelationSpec) {
        self.relations.insert(spec.name.clone(), spec);
    }

    /// Get a relation by name.
    pub fn get(&self, name: &str) -> Option<&RelationSpec> {
        self.relations.get(name)
    }

    /// Get all relations.
    pub fn all(&self) -> impl Iterator<Item = &RelationSpec> {
        self.relations.values()
    }

    /// Get all one-to-many relations.
    pub fn one_to_many(&self) -> impl Iterator<Item = &RelationSpec> {
        self.relations
            .values()
            .filter(|r| r.relation_type == RelationType::OneToMany)
    }

    /// Get all many-to-one relations.
    pub fn many_to_one(&self) -> impl Iterator<Item = &RelationSpec> {
        self.relations
            .values()
            .filter(|r| r.relation_type == RelationType::ManyToOne)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relation_type() {
        assert!(RelationType::OneToMany.is_many());
        assert!(RelationType::ManyToMany.is_many());
        assert!(!RelationType::OneToOne.is_many());
        assert!(RelationType::OneToOne.is_one());
    }

    #[test]
    fn test_relation_spec() {
        let spec = RelationSpec::one_to_many("posts", "Post", "posts")
            .fields(["id"])
            .references(["author_id"]);

        assert_eq!(spec.name, "posts");
        assert_eq!(spec.relation_type, RelationType::OneToMany);
        assert_eq!(spec.fields, vec!["id"]);
        assert_eq!(spec.references, vec!["author_id"]);
    }

    #[test]
    fn test_join_table_spec() {
        let jt = JoinTableSpec::new("_post_tags", "post_id", "tag_id");
        assert_eq!(jt.table_name, "_post_tags");
        assert_eq!(jt.source_column, "post_id");
        assert_eq!(jt.target_column, "tag_id");
    }

    #[test]
    fn test_referential_action() {
        assert_eq!(ReferentialAction::Cascade.as_sql(), "CASCADE");
        assert_eq!(ReferentialAction::SetNull.as_sql(), "SET NULL");
    }

    #[test]
    fn test_relation_registry() {
        let mut registry = RelationRegistry::new();
        registry.register(RelationSpec::one_to_many("posts", "Post", "posts"));
        registry.register(RelationSpec::many_to_one("author", "User", "users"));

        assert!(registry.get("posts").is_some());
        assert!(registry.get("author").is_some());
        assert!(registry.get("nonexistent").is_none());
    }
}
