//! Schema diffing for generating migrations.

use std::collections::{HashMap, HashSet, VecDeque};

use prax_schema::Schema;
use prax_schema::ast::{Field, FieldType, IndexType, Model, VectorOps, View};

use crate::error::MigrateResult;

/// A diff between two schemas.
#[derive(Debug, Clone, Default)]
pub struct SchemaDiff {
    /// PostgreSQL extensions to create.
    pub create_extensions: Vec<ExtensionDiff>,
    /// PostgreSQL extensions to drop.
    pub drop_extensions: Vec<String>,
    /// Models to create.
    pub create_models: Vec<ModelDiff>,
    /// Models to drop.
    pub drop_models: Vec<String>,
    /// Models to alter.
    pub alter_models: Vec<ModelAlterDiff>,
    /// Enums to create.
    pub create_enums: Vec<EnumDiff>,
    /// Enums to drop.
    pub drop_enums: Vec<String>,
    /// Enums to alter.
    pub alter_enums: Vec<EnumAlterDiff>,
    /// Views to create.
    pub create_views: Vec<ViewDiff>,
    /// Views to drop.
    pub drop_views: Vec<String>,
    /// Views to alter (recreate with new definition).
    pub alter_views: Vec<ViewDiff>,
    /// Indexes to create.
    pub create_indexes: Vec<IndexDiff>,
    /// Indexes to drop.
    pub drop_indexes: Vec<IndexDiff>,
}

/// Diff for PostgreSQL extensions.
#[derive(Debug, Clone)]
pub struct ExtensionDiff {
    /// Extension name.
    pub name: String,
    /// Optional schema to install into.
    pub schema: Option<String>,
    /// Optional version.
    pub version: Option<String>,
}

impl SchemaDiff {
    /// Check if there are any differences.
    pub fn is_empty(&self) -> bool {
        self.create_extensions.is_empty()
            && self.drop_extensions.is_empty()
            && self.create_models.is_empty()
            && self.drop_models.is_empty()
            && self.alter_models.is_empty()
            && self.create_enums.is_empty()
            && self.drop_enums.is_empty()
            && self.alter_enums.is_empty()
            && self.create_views.is_empty()
            && self.drop_views.is_empty()
            && self.alter_views.is_empty()
            && self.create_indexes.is_empty()
            && self.drop_indexes.is_empty()
    }

    /// Get a human-readable summary of the diff.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if !self.create_extensions.is_empty() {
            parts.push(format!(
                "Create {} extensions",
                self.create_extensions.len()
            ));
        }
        if !self.drop_extensions.is_empty() {
            parts.push(format!("Drop {} extensions", self.drop_extensions.len()));
        }
        if !self.create_models.is_empty() {
            parts.push(format!("Create {} models", self.create_models.len()));
        }
        if !self.drop_models.is_empty() {
            parts.push(format!("Drop {} models", self.drop_models.len()));
        }
        if !self.alter_models.is_empty() {
            parts.push(format!("Alter {} models", self.alter_models.len()));
        }
        if !self.create_enums.is_empty() {
            parts.push(format!("Create {} enums", self.create_enums.len()));
        }
        if !self.drop_enums.is_empty() {
            parts.push(format!("Drop {} enums", self.drop_enums.len()));
        }
        if !self.create_views.is_empty() {
            parts.push(format!("Create {} views", self.create_views.len()));
        }
        if !self.drop_views.is_empty() {
            parts.push(format!("Drop {} views", self.drop_views.len()));
        }
        if !self.alter_views.is_empty() {
            parts.push(format!("Alter {} views", self.alter_views.len()));
        }
        if !self.create_indexes.is_empty() {
            parts.push(format!("Create {} indexes", self.create_indexes.len()));
        }
        if !self.drop_indexes.is_empty() {
            parts.push(format!("Drop {} indexes", self.drop_indexes.len()));
        }

        if parts.is_empty() {
            "No changes".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// Return `create_models` ordered so that referenced tables appear before
    /// the tables that reference them. Self-references and FKs that point at
    /// tables outside this batch (i.e. tables that already exist) do not
    /// constrain the ordering. If the FK graph contains a cycle, the remaining
    /// models are emitted in their original order — engines that need cycles
    /// resolved must use deferred constraints regardless of emission order.
    pub fn ordered_create_models(&self) -> Vec<&ModelDiff> {
        let in_batch: HashSet<&str> = self
            .create_models
            .iter()
            .map(|m| m.table_name.as_str())
            .collect();

        let mut indegree: HashMap<&str, usize> = self
            .create_models
            .iter()
            .map(|m| (m.table_name.as_str(), 0))
            .collect();
        let mut deps: HashMap<&str, Vec<&str>> = HashMap::new();

        for model in &self.create_models {
            let mut seen = HashSet::new();
            for fk in &model.foreign_keys {
                let target = fk.referenced_table.as_str();
                if target == model.table_name {
                    continue;
                }
                if !in_batch.contains(target) {
                    continue;
                }
                if !seen.insert(target) {
                    continue;
                }
                deps.entry(target)
                    .or_default()
                    .push(model.table_name.as_str());
                *indegree.entry(model.table_name.as_str()).or_insert(0) += 1;
            }
        }

        let by_name: HashMap<&str, &ModelDiff> = self
            .create_models
            .iter()
            .map(|m| (m.table_name.as_str(), m))
            .collect();

        let mut ready: VecDeque<&str> = self
            .create_models
            .iter()
            .filter(|m| indegree.get(m.table_name.as_str()).copied().unwrap_or(0) == 0)
            .map(|m| m.table_name.as_str())
            .collect();

        let mut ordered: Vec<&ModelDiff> = Vec::with_capacity(self.create_models.len());
        let mut emitted: HashSet<&str> = HashSet::new();

        while let Some(name) = ready.pop_front() {
            if !emitted.insert(name) {
                continue;
            }
            if let Some(model) = by_name.get(name) {
                ordered.push(*model);
            }
            if let Some(children) = deps.get(name) {
                for child in children {
                    if let Some(deg) = indegree.get_mut(child) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            ready.push_back(child);
                        }
                    }
                }
            }
        }

        if ordered.len() < self.create_models.len() {
            for model in &self.create_models {
                if !emitted.contains(model.table_name.as_str()) {
                    ordered.push(model);
                }
            }
        }

        ordered
    }
}

/// Diff for creating a model.
#[derive(Debug, Clone)]
pub struct ModelDiff {
    /// Model name.
    pub name: String,
    /// Table name.
    pub table_name: String,
    /// Fields to create.
    pub fields: Vec<FieldDiff>,
    /// Primary key columns.
    pub primary_key: Vec<String>,
    /// Indexes.
    pub indexes: Vec<IndexDiff>,
    /// Unique constraints.
    pub unique_constraints: Vec<UniqueConstraint>,
    /// Foreign key constraints.
    pub foreign_keys: Vec<ForeignKeyDiff>,
}

/// Diff for altering a model.
#[derive(Debug, Clone)]
pub struct ModelAlterDiff {
    /// Model name.
    pub name: String,
    /// Table name.
    pub table_name: String,
    /// Fields to add.
    pub add_fields: Vec<FieldDiff>,
    /// Fields to drop.
    pub drop_fields: Vec<String>,
    /// Fields to alter.
    pub alter_fields: Vec<FieldAlterDiff>,
    /// Indexes to add.
    pub add_indexes: Vec<IndexDiff>,
    /// Indexes to drop.
    pub drop_indexes: Vec<String>,
    /// Foreign keys to add.
    pub add_foreign_keys: Vec<ForeignKeyDiff>,
    /// Foreign keys to drop (by constraint name).
    pub drop_foreign_keys: Vec<String>,
}

/// A foreign key constraint diff.
#[derive(Debug, Clone)]
pub struct ForeignKeyDiff {
    /// Constraint name (from `map` or auto-generated).
    pub constraint_name: String,
    /// Columns on this table.
    pub columns: Vec<String>,
    /// Referenced table name.
    pub referenced_table: String,
    /// Referenced columns.
    pub referenced_columns: Vec<String>,
    /// On delete action.
    pub on_delete: Option<String>,
    /// On update action.
    pub on_update: Option<String>,
}

/// Diff for a field.
#[derive(Debug, Clone)]
pub struct FieldDiff {
    /// Field name.
    pub name: String,
    /// Column name.
    pub column_name: String,
    /// SQL type.
    pub sql_type: String,
    /// Whether the field is nullable.
    pub nullable: bool,
    /// Default value expression.
    pub default: Option<String>,
    /// Whether this is a primary key.
    pub is_primary_key: bool,
    /// Whether this has auto increment.
    pub is_auto_increment: bool,
    /// Whether this is unique.
    pub is_unique: bool,
    /// Optional vector column metadata. Only used by SQLite backends; other
    /// generators ignore this field. Populated by the schema parser when a
    /// field declares `Vector @dim(N)`.
    pub vector: Option<VectorColumnInfo>,
    /// If this field is an enum type, the enum name; otherwise None.
    /// Dialects can choose how to render: Postgres uses `"name"` as the column
    /// type referencing a pre-created enum type; SQLite/MySQL/MSSQL/DuckDB use
    /// TEXT (optionally with a CHECK constraint of valid variants).
    pub enum_name: Option<String>,
}

/// Diff for altering a field.
#[derive(Debug, Clone)]
pub struct FieldAlterDiff {
    /// Field name.
    pub name: String,
    /// Column name.
    pub column_name: String,
    /// Old SQL type (if changed).
    pub old_type: Option<String>,
    /// New SQL type (if changed).
    pub new_type: Option<String>,
    /// Old nullable (if changed).
    pub old_nullable: Option<bool>,
    /// New nullable (if changed).
    pub new_nullable: Option<bool>,
    /// Old default (if changed).
    pub old_default: Option<String>,
    /// New default (if changed).
    pub new_default: Option<String>,
}

/// Diff for an enum.
#[derive(Debug, Clone)]
pub struct EnumDiff {
    /// Enum name.
    pub name: String,
    /// Values.
    pub values: Vec<String>,
}

/// Diff for altering an enum.
#[derive(Debug, Clone)]
pub struct EnumAlterDiff {
    /// Enum name.
    pub name: String,
    /// Values to add.
    pub add_values: Vec<String>,
    /// Values to remove.
    pub remove_values: Vec<String>,
}

/// Index diff.
#[derive(Debug, Clone)]
pub struct IndexDiff {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table_name: String,
    /// Columns in the index.
    pub columns: Vec<String>,
    /// Whether this is a unique index.
    pub unique: bool,
    /// Index type (btree, hash, hnsw, ivfflat, etc.).
    pub index_type: Option<IndexType>,
    /// Vector distance operation (for HNSW/IVFFlat indexes).
    pub vector_ops: Option<VectorOps>,
    /// HNSW m parameter (max connections per layer).
    pub hnsw_m: Option<u32>,
    /// HNSW ef_construction parameter.
    pub hnsw_ef_construction: Option<u32>,
    /// IVFFlat lists parameter.
    pub ivfflat_lists: Option<u32>,
}

impl IndexDiff {
    /// Create a new index diff.
    pub fn new(
        name: impl Into<String>,
        table_name: impl Into<String>,
        columns: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            table_name: table_name.into(),
            columns,
            unique: false,
            index_type: None,
            vector_ops: None,
            hnsw_m: None,
            hnsw_ef_construction: None,
            ivfflat_lists: None,
        }
    }

    /// Set as unique index.
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    /// Set the index type.
    pub fn with_type(mut self, index_type: IndexType) -> Self {
        self.index_type = Some(index_type);
        self
    }

    /// Set vector options.
    pub fn with_vector_ops(mut self, ops: VectorOps) -> Self {
        self.vector_ops = Some(ops);
        self
    }

    /// Set HNSW m parameter.
    pub fn with_hnsw_m(mut self, m: u32) -> Self {
        self.hnsw_m = Some(m);
        self
    }

    /// Set HNSW ef_construction parameter.
    pub fn with_hnsw_ef_construction(mut self, ef: u32) -> Self {
        self.hnsw_ef_construction = Some(ef);
        self
    }

    /// Set IVFFlat lists parameter.
    pub fn with_ivfflat_lists(mut self, lists: u32) -> Self {
        self.ivfflat_lists = Some(lists);
        self
    }

    /// Check if this is a vector index.
    pub fn is_vector_index(&self) -> bool {
        self.index_type
            .as_ref()
            .is_some_and(|t| t.is_vector_index())
    }
}

/// Unique constraint.
#[derive(Debug, Clone)]
pub struct UniqueConstraint {
    /// Constraint name.
    pub name: Option<String>,
    /// Columns.
    pub columns: Vec<String>,
}

/// Diff for creating or altering a view.
#[derive(Debug, Clone)]
pub struct ViewDiff {
    /// View name.
    pub name: String,
    /// Database view name.
    pub view_name: String,
    /// SQL query that defines the view.
    pub sql_query: String,
    /// Whether the view is materialized.
    pub is_materialized: bool,
    /// Refresh interval for materialized views (if any).
    pub refresh_interval: Option<String>,
    /// Fields in the view (for documentation/validation).
    pub fields: Vec<ViewFieldDiff>,
}

/// Field in a view diff (for documentation purposes).
#[derive(Debug, Clone)]
pub struct ViewFieldDiff {
    /// Field name.
    pub name: String,
    /// Column name in the view.
    pub column_name: String,
    /// SQL type.
    pub sql_type: String,
    /// Whether the field is nullable.
    pub nullable: bool,
}

/// Schema differ for comparing schemas.
pub struct SchemaDiffer {
    /// Source schema (current database state).
    source: Option<Schema>,
    /// Target schema (desired state).
    target: Schema,
}

impl SchemaDiffer {
    /// Create a new differ with only the target schema.
    pub fn new(target: Schema) -> Self {
        Self {
            source: None,
            target,
        }
    }

    /// Set the source schema.
    pub fn with_source(mut self, source: Schema) -> Self {
        self.source = Some(source);
        self
    }

    /// Compute the diff between schemas.
    pub fn diff(&self) -> MigrateResult<SchemaDiff> {
        let mut result = SchemaDiff::default();

        let source_models: HashMap<&str, &Model> = self
            .source
            .as_ref()
            .map(|s| s.models.values().map(|m| (m.name(), m)).collect())
            .unwrap_or_default();

        let target_models: HashMap<&str, &Model> =
            self.target.models.values().map(|m| (m.name(), m)).collect();

        // Find models to create
        for (name, model) in &target_models {
            if !source_models.contains_key(name) {
                let model_diff = model_to_diff(model, &self.target);
                // Populate create_indexes from the model's indexes
                result.create_indexes.extend(model_diff.indexes.clone());
                result.create_models.push(model_diff);
            }
        }

        // Find models to drop
        for name in source_models.keys() {
            if !target_models.contains_key(name) {
                result.drop_models.push((*name).to_string());
            }
        }

        // Find models to alter
        for (name, target_model) in &target_models {
            if let Some(source_model) = source_models.get(name)
                && let Some(alter) = diff_models(source_model, target_model, &self.target)
            {
                result.alter_models.push(alter);
            }
        }

        // Diff enums similarly
        let source_enums: HashMap<&str, _> = self
            .source
            .as_ref()
            .map(|s| s.enums.values().map(|e| (e.name(), e)).collect())
            .unwrap_or_default();

        let target_enums: HashMap<&str, _> =
            self.target.enums.values().map(|e| (e.name(), e)).collect();

        for (name, enum_def) in &target_enums {
            if !source_enums.contains_key(name) {
                result.create_enums.push(EnumDiff {
                    name: (*name).to_string(),
                    values: enum_def
                        .variants
                        .iter()
                        .map(|v| v.name.to_string())
                        .collect(),
                });
            }
        }

        for name in source_enums.keys() {
            if !target_enums.contains_key(name) {
                result.drop_enums.push((*name).to_string());
            }
        }

        // Diff views
        let source_views: HashMap<&str, &View> = self
            .source
            .as_ref()
            .map(|s| s.views.values().map(|v| (v.name(), v)).collect())
            .unwrap_or_default();

        let target_views: HashMap<&str, &View> =
            self.target.views.values().map(|v| (v.name(), v)).collect();

        // Find views to create
        for (name, view) in &target_views {
            if !source_views.contains_key(name)
                && let Some(view_diff) = view_to_diff(view)
            {
                result.create_views.push(view_diff);
            }
        }

        // Find views to drop
        for name in source_views.keys() {
            if !target_views.contains_key(name) {
                result.drop_views.push((*name).to_string());
            }
        }

        // Find views to alter (if SQL changed)
        for (name, target_view) in &target_views {
            if let Some(source_view) = source_views.get(name) {
                // Views are altered by dropping and recreating
                let source_sql = source_view.sql_query();
                let target_sql = target_view.sql_query();

                // Check if SQL or materialized status changed
                let sql_changed = source_sql != target_sql;
                let materialized_changed =
                    source_view.is_materialized() != target_view.is_materialized();

                if (sql_changed || materialized_changed)
                    && let Some(view_diff) = view_to_diff(target_view)
                {
                    result.alter_views.push(view_diff);
                }
            }
        }

        Ok(result)
    }
}

/// Convert a model to a diff for creation.
fn model_to_diff(model: &Model, schema: &Schema) -> ModelDiff {
    let fields: Vec<FieldDiff> = model
        .fields
        .values()
        .filter(|f| !f.is_relation())
        .map(field_to_diff)
        .collect();

    let primary_key: Vec<String> = model
        .fields
        .values()
        .filter(|f| f.has_attribute("id"))
        .map(|f| f.name().to_string())
        .collect();

    let foreign_keys = extract_foreign_keys(model, schema);

    // Extract indexes from @@index attributes
    let mut indexes = Vec::new();
    let mut unique_constraints = Vec::new();

    for attr in &model.attributes {
        let attr_name = attr.name();
        if attr_name != "index" && attr_name != "unique" {
            continue;
        }

        // Extract column list from first positional arg
        let columns = if let Some(first_arg) = attr.first_arg() {
            match first_arg {
                prax_schema::ast::AttributeValue::FieldRef(col) => vec![col.to_string()],
                prax_schema::ast::AttributeValue::FieldRefList(cols) => {
                    cols.iter().map(|c| c.to_string()).collect()
                }
                _ => {
                    eprintln!(
                        "Warning: unexpected @@{} argument type - skipping",
                        attr_name
                    );
                    continue;
                }
            }
        } else {
            eprintln!("Warning: @@{} without column list - skipping", attr_name);
            continue;
        };

        // Map field names to column names (respecting @map)
        let column_names: Vec<String> = columns
            .iter()
            .map(|field_name| {
                model
                    .fields
                    .get(field_name.as_str())
                    .and_then(|f| {
                        f.get_attribute("map")
                            .and_then(|a| a.first_arg())
                            .and_then(|v| v.as_string())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| field_name.clone())
            })
            .collect();

        // Look for custom index name via `map` or `name` named arg
        let custom_name = attr
            .get_arg("map")
            .or_else(|| attr.get_arg("name"))
            .and_then(|v: &prax_schema::ast::AttributeValue| v.as_string());

        let index_name = custom_name.map(|s: &str| s.to_string()).unwrap_or_else(|| {
            let prefix = if attr_name == "unique" { "uq" } else { "idx" };
            format!(
                "{}_{}_{}",
                prefix,
                model.table_name(),
                column_names.join("_")
            )
        });

        if attr_name == "unique" {
            unique_constraints.push(UniqueConstraint {
                name: Some(index_name),
                columns: column_names,
            });
        } else {
            indexes.push(IndexDiff {
                name: index_name,
                table_name: model.table_name().to_string(),
                columns: column_names,
                unique: false,
                index_type: None,
                vector_ops: None,
                hnsw_m: None,
                hnsw_ef_construction: None,
                ivfflat_lists: None,
            });
        }
    }

    ModelDiff {
        name: model.name().to_string(),
        table_name: model.table_name().to_string(),
        fields,
        primary_key,
        indexes,
        unique_constraints,
        foreign_keys,
    }
}

/// Extract foreign key constraints from a model's relation fields.
fn extract_foreign_keys(model: &Model, schema: &Schema) -> Vec<ForeignKeyDiff> {
    let mut fks = Vec::new();

    for field in model.fields.values() {
        if !field.is_relation() {
            continue;
        }

        let attrs = field.extract_attributes();
        let Some(rel) = &attrs.relation else {
            continue;
        };

        // Only the side that holds the FK columns generates a constraint
        if rel.fields.is_empty() || rel.references.is_empty() {
            continue;
        }

        // Resolve the referenced table name from the field's model type
        // Use @@map-aware table name lookup
        let referenced_table = match &field.field_type {
            FieldType::Model(name) => {
                schema
                    .models
                    .get(name.as_str())
                    .map(|m| m.table_name().to_string())
                    .unwrap_or_else(|| {
                        eprintln!(
                            "Warning: referenced model '{}' not found in schema; using model name as table name",
                            name
                        );
                        name.to_string()
                    })
            }
            _ => continue,
        };

        let columns: Vec<String> = rel.fields.iter().map(|f| f.to_string()).collect();
        let referenced_columns: Vec<String> =
            rel.references.iter().map(|r| r.to_string()).collect();

        let constraint_name = rel
            .map
            .clone()
            .unwrap_or_else(|| format!("fk_{}_{}", model.table_name(), columns.join("_")));

        fks.push(ForeignKeyDiff {
            constraint_name,
            columns,
            referenced_table,
            referenced_columns,
            on_delete: rel.on_delete.map(|a| a.as_str().to_string()),
            on_update: rel.on_update.map(|a| a.as_str().to_string()),
        });
    }

    fks
}

/// Render an AttributeValue to SQL literal syntax (ANSI SQL with TRUE/FALSE/CURRENT_TIMESTAMP).
/// SQLite generators can post-process TRUE→1, FALSE→0.
fn render_default_sql_ansi(value: &prax_schema::ast::AttributeValue) -> Option<String> {
    use prax_schema::ast::AttributeValue;

    match value {
        AttributeValue::Int(i) => Some(i.to_string()),
        AttributeValue::Float(f) => Some(f.to_string()),
        AttributeValue::Boolean(true) => Some("TRUE".to_string()),
        AttributeValue::Boolean(false) => Some("FALSE".to_string()),
        AttributeValue::String(s) => {
            // SQL single-quoted literal with doubled quotes for escaping
            Some(format!("'{}'", s.replace('\'', "''")))
        }
        AttributeValue::Ident(name) => {
            // Treat as enum variant or constant - quote it
            Some(format!("'{}'", name))
        }
        AttributeValue::Function(name, args) => {
            // Map common functions to SQL builtins
            if name == "now" && args.is_empty() {
                Some("CURRENT_TIMESTAMP".to_string())
            } else if name == "uuid" && args.is_empty() {
                // UUID generation - dialect-specific; for now use a generic name
                Some("gen_random_uuid()".to_string())
            } else {
                // Other functions - attempt to render recursively
                let arg_strs: Vec<String> =
                    args.iter().filter_map(render_default_sql_ansi).collect();
                Some(format!("{}({})", name, arg_strs.join(", ")))
            }
        }
        AttributeValue::Array(_)
        | AttributeValue::FieldRef(_)
        | AttributeValue::FieldRefList(_) => {
            // Not valid in DEFAULT clauses
            eprintln!(
                "Warning: unsupported default value type {:?} - skipping default",
                value
            );
            None
        }
    }
}

/// Convert a field to a diff.
fn field_to_diff(field: &Field) -> FieldDiff {
    let sql_type = field_type_to_sql(&field.field_type);
    let nullable = field.is_optional();
    let is_primary_key = field.has_attribute("id");
    let is_auto_increment = field.has_attribute("auto");
    let is_unique = field.has_attribute("unique");

    let default = field
        .get_attribute("default")
        .and_then(|attr| attr.first_arg())
        .and_then(render_default_sql_ansi);

    // Get column name from @map attribute or use field name
    let column_name = field
        .get_attribute("map")
        .and_then(|attr| attr.first_arg())
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| field.name())
        .to_string();

    // Extract enum name if this is an enum type
    let enum_name = match &field.field_type {
        FieldType::Enum(name) => Some(name.to_string()),
        _ => None,
    };

    FieldDiff {
        name: field.name().to_string(),
        column_name,
        sql_type,
        nullable,
        default,
        is_primary_key,
        is_auto_increment,
        is_unique,
        vector: None,
        enum_name,
    }
}

/// Convert a field type to SQL.
fn field_type_to_sql(field_type: &prax_schema::ast::FieldType) -> String {
    use prax_schema::ast::{FieldType, ScalarType};

    match field_type {
        FieldType::Scalar(scalar) => match scalar {
            ScalarType::Int => "INTEGER".to_string(),
            ScalarType::BigInt => "BIGINT".to_string(),
            ScalarType::Float => "DOUBLE PRECISION".to_string(),
            ScalarType::Decimal => "DECIMAL".to_string(),
            ScalarType::String => "TEXT".to_string(),
            ScalarType::Boolean => "BOOLEAN".to_string(),
            ScalarType::DateTime => "TIMESTAMP WITH TIME ZONE".to_string(),
            ScalarType::Date => "DATE".to_string(),
            ScalarType::Time => "TIME".to_string(),
            ScalarType::Json => "JSONB".to_string(),
            ScalarType::Bytes => "BYTEA".to_string(),
            ScalarType::Uuid => "UUID".to_string(),
            // String-based ID types stored as TEXT
            ScalarType::Cuid | ScalarType::Cuid2 | ScalarType::NanoId | ScalarType::Ulid => {
                "TEXT".to_string()
            }
            // PostgreSQL vector extension types
            ScalarType::Vector(dim) => match dim {
                Some(d) => format!("vector({})", d),
                None => "vector".to_string(),
            },
            ScalarType::HalfVector(dim) => match dim {
                Some(d) => format!("halfvec({})", d),
                None => "halfvec".to_string(),
            },
            ScalarType::SparseVector(dim) => match dim {
                Some(d) => format!("sparsevec({})", d),
                None => "sparsevec".to_string(),
            },
            ScalarType::Bit(dim) => match dim {
                Some(d) => format!("bit({})", d),
                None => "bit".to_string(),
            },
        },
        FieldType::Model(name) => name.to_string(),
        FieldType::Enum(_name) => "TEXT".to_string(), // Dialects override via enum_name field
        FieldType::Composite(name) => name.to_string(),
        FieldType::Unsupported(name) => name.to_string(),
    }
}

/// Diff two models and return alterations if any.
fn diff_models(source: &Model, target: &Model, schema: &Schema) -> Option<ModelAlterDiff> {
    let source_fields: HashMap<&str, &Field> = source
        .fields
        .values()
        .filter(|f| !f.is_relation())
        .map(|f| (f.name(), f))
        .collect();

    let target_fields: HashMap<&str, &Field> = target
        .fields
        .values()
        .filter(|f| !f.is_relation())
        .map(|f| (f.name(), f))
        .collect();

    let mut add_fields = Vec::new();
    let mut drop_fields = Vec::new();
    let mut alter_fields = Vec::new();

    // Find fields to add
    for (name, field) in &target_fields {
        if !source_fields.contains_key(name) {
            add_fields.push(field_to_diff(field));
        }
    }

    // Find fields to drop
    for name in source_fields.keys() {
        if !target_fields.contains_key(name) {
            drop_fields.push((*name).to_string());
        }
    }

    // Find fields to alter
    for (name, target_field) in &target_fields {
        if let Some(source_field) = source_fields.get(name)
            && let Some(alter) = diff_fields(source_field, target_field)
        {
            alter_fields.push(alter);
        }
    }

    // Diff foreign keys
    let source_fks = extract_foreign_keys(source, schema);
    let target_fks = extract_foreign_keys(target, schema);

    let source_fk_names: std::collections::HashSet<&str> = source_fks
        .iter()
        .map(|fk| fk.constraint_name.as_str())
        .collect();
    let target_fk_names: std::collections::HashSet<&str> = target_fks
        .iter()
        .map(|fk| fk.constraint_name.as_str())
        .collect();

    let drop_foreign_keys: Vec<String> = source_fks
        .iter()
        .filter(|fk| !target_fk_names.contains(fk.constraint_name.as_str()))
        .map(|fk| fk.constraint_name.clone())
        .collect();
    let add_foreign_keys: Vec<ForeignKeyDiff> = target_fks
        .into_iter()
        .filter(|fk| !source_fk_names.contains(fk.constraint_name.as_str()))
        .collect();

    if add_fields.is_empty()
        && drop_fields.is_empty()
        && alter_fields.is_empty()
        && add_foreign_keys.is_empty()
        && drop_foreign_keys.is_empty()
    {
        None
    } else {
        Some(ModelAlterDiff {
            name: target.name().to_string(),
            table_name: target.table_name().to_string(),
            add_fields,
            drop_fields,
            alter_fields,
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys,
            drop_foreign_keys,
        })
    }
}

/// Convert a view to a diff for creation.
fn view_to_diff(view: &View) -> Option<ViewDiff> {
    // Views require a @@sql attribute to be migrated
    let sql_query = view.sql_query()?.to_string();

    let fields: Vec<ViewFieldDiff> = view
        .fields
        .values()
        .map(|field| {
            let column_name = field
                .get_attribute("map")
                .and_then(|attr| attr.first_arg())
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| field.name())
                .to_string();

            ViewFieldDiff {
                name: field.name().to_string(),
                column_name,
                sql_type: field_type_to_sql(&field.field_type),
                nullable: field.is_optional(),
            }
        })
        .collect();

    Some(ViewDiff {
        name: view.name().to_string(),
        view_name: view.view_name().to_string(),
        sql_query,
        is_materialized: view.is_materialized(),
        refresh_interval: view.refresh_interval().map(|s| s.to_string()),
        fields,
    })
}

/// Diff two fields and return alterations if any.
fn diff_fields(source: &Field, target: &Field) -> Option<FieldAlterDiff> {
    let source_type = field_type_to_sql(&source.field_type);
    let target_type = field_type_to_sql(&target.field_type);

    let source_nullable = source.is_optional();
    let target_nullable = target.is_optional();

    let type_changed = source_type != target_type;
    let nullable_changed = source_nullable != target_nullable;

    if !type_changed && !nullable_changed {
        return None;
    }

    // Get column name from @map attribute or use field name
    let column_name = target
        .get_attribute("map")
        .and_then(|attr| attr.first_arg())
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| target.name())
        .to_string();

    Some(FieldAlterDiff {
        name: target.name().to_string(),
        column_name,
        old_type: if type_changed {
            Some(source_type)
        } else {
            None
        },
        new_type: if type_changed {
            Some(target_type)
        } else {
            None
        },
        old_nullable: if nullable_changed {
            Some(source_nullable)
        } else {
            None
        },
        new_nullable: if nullable_changed {
            Some(target_nullable)
        } else {
            None
        },
        old_default: None,
        new_default: None,
    })
}

/// Metadata describing a vector column.
///
/// Populated by the schema parser when a field is declared with the
/// `Vector` type and the `@dim(...)`, `@vectorType(...)`, `@metric(...)`,
/// and `@index(...)` attributes. Only consumed by the SQLite generator;
/// Postgres/MySQL/MSSQL/DuckDB generators treat fields with `vector = Some(_)`
/// as an error (reported by the schema differ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorColumnInfo {
    /// Vector dimensionality (required).
    pub dimensions: u32,
    /// Element type (default: Float4).
    pub element_type: VectorElementType,
    /// Distance metric (default: Cosine).
    pub metric: VectorDistanceMetric,
    /// Optional HNSW index.
    pub index: Option<VectorIndexKind>,
}

/// Vector element types supported by sqlite-vector-rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorElementType {
    Float2,
    Float4,
    Float8,
    Int1,
    Int2,
    Int4,
}

impl VectorElementType {
    /// Lowercase string identifier used in sqlite-vector-rs DDL.
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorElementType::Float2 => "float2",
            VectorElementType::Float4 => "float4",
            VectorElementType::Float8 => "float8",
            VectorElementType::Int1 => "int1",
            VectorElementType::Int2 => "int2",
            VectorElementType::Int4 => "int4",
        }
    }
}

/// Vector distance metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorDistanceMetric {
    Cosine,
    L2,
    InnerProduct,
}

impl VectorDistanceMetric {
    /// Lowercase string identifier used in sqlite-vector-rs DDL.
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorDistanceMetric::Cosine => "cosine",
            VectorDistanceMetric::L2 => "l2",
            VectorDistanceMetric::InnerProduct => "inner",
        }
    }
}

/// Vector index kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorIndexKind {
    Hnsw,
}

impl VectorIndexKind {
    /// Lowercase string identifier used in sqlite-vector-rs DDL.
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorIndexKind::Hnsw => "hnsw",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_diff_empty() {
        let diff = SchemaDiff::default();
        assert!(diff.is_empty());
    }

    #[test]
    fn test_schema_diff_summary() {
        let mut diff = SchemaDiff::default();
        diff.create_models.push(ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: Vec::new(),
            primary_key: Vec::new(),
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        });

        let summary = diff.summary();
        assert!(summary.contains("Create 1 models"));
    }

    #[test]
    fn test_schema_diff_with_views() {
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) FROM users GROUP BY id".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        });

        assert!(!diff.is_empty());
        let summary = diff.summary();
        assert!(summary.contains("Create 1 views"));
    }

    #[test]
    fn test_schema_diff_summary_with_multiple() {
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "View1".to_string(),
            view_name: "view1".to_string(),
            sql_query: "SELECT 1".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        });
        diff.drop_views.push("old_view".to_string());
        diff.alter_views.push(ViewDiff {
            name: "View2".to_string(),
            view_name: "view2".to_string(),
            sql_query: "SELECT 2".to_string(),
            is_materialized: true,
            refresh_interval: Some("1h".to_string()),
            fields: vec![],
        });

        let summary = diff.summary();
        assert!(summary.contains("Create 1 views"));
        assert!(summary.contains("Drop 1 views"));
        assert!(summary.contains("Alter 1 views"));
    }

    #[test]
    fn test_view_diff_fields() {
        let view_diff = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, name FROM users".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![
                ViewFieldDiff {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    sql_type: "INTEGER".to_string(),
                    nullable: false,
                },
                ViewFieldDiff {
                    name: "name".to_string(),
                    column_name: "user_name".to_string(),
                    sql_type: "TEXT".to_string(),
                    nullable: true,
                },
            ],
        };

        assert_eq!(view_diff.fields.len(), 2);
        assert_eq!(view_diff.fields[0].name, "id");
        assert_eq!(view_diff.fields[1].column_name, "user_name");
    }

    #[test]
    fn test_field_diff_defaults_vector_to_none() {
        // Fields created the old way still compile after the new field is added.
        let f = FieldDiff {
            name: "id".to_string(),
            column_name: "id".to_string(),
            sql_type: "INTEGER".to_string(),
            nullable: false,
            default: None,
            is_primary_key: true,
            is_auto_increment: true,
            is_unique: false,
            vector: None,
            enum_name: None,
        };
        assert!(f.vector.is_none());
    }

    #[test]
    fn test_vector_column_info_populated() {
        let v = VectorColumnInfo {
            dimensions: 1536,
            element_type: VectorElementType::Float4,
            metric: VectorDistanceMetric::Cosine,
            index: Some(VectorIndexKind::Hnsw),
        };
        assert_eq!(v.dimensions, 1536);
        assert_eq!(v.element_type, VectorElementType::Float4);
        assert_eq!(v.metric, VectorDistanceMetric::Cosine);
        assert_eq!(v.index, Some(VectorIndexKind::Hnsw));
    }

    #[test]
    fn test_element_type_sql_strings() {
        assert_eq!(VectorElementType::Float2.as_sql(), "float2");
        assert_eq!(VectorElementType::Float4.as_sql(), "float4");
        assert_eq!(VectorElementType::Float8.as_sql(), "float8");
        assert_eq!(VectorElementType::Int1.as_sql(), "int1");
        assert_eq!(VectorElementType::Int2.as_sql(), "int2");
        assert_eq!(VectorElementType::Int4.as_sql(), "int4");
    }

    #[test]
    fn test_metric_sql_strings() {
        assert_eq!(VectorDistanceMetric::Cosine.as_sql(), "cosine");
        assert_eq!(VectorDistanceMetric::L2.as_sql(), "l2");
        assert_eq!(VectorDistanceMetric::InnerProduct.as_sql(), "inner");
    }

    #[test]
    fn test_index_kind_sql_strings() {
        assert_eq!(VectorIndexKind::Hnsw.as_sql(), "hnsw");
    }

    fn model_with_fks(name: &str, refs: &[&str]) -> ModelDiff {
        ModelDiff {
            name: name.to_string(),
            table_name: name.to_string(),
            fields: Vec::new(),
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: refs
                .iter()
                .enumerate()
                .map(|(i, target)| ForeignKeyDiff {
                    constraint_name: format!("{}_fk_{}", name, i),
                    columns: vec![format!("{}_id", target)],
                    referenced_table: (*target).to_string(),
                    referenced_columns: vec!["id".to_string()],
                    on_delete: None,
                    on_update: None,
                })
                .collect(),
        }
    }

    #[test]
    fn ordered_create_models_emits_referenced_tables_first() {
        // Mirrors the regression: tracks/playlists reference sync_sources,
        // but sync_sources was inserted last by HashMap iteration order.
        let mut diff = SchemaDiff::default();
        diff.create_models
            .push(model_with_fks("tracks", &["sync_sources"]));
        diff.create_models
            .push(model_with_fks("playlists", &["sync_sources"]));
        diff.create_models.push(model_with_fks("sync_sources", &[]));

        let ordered: Vec<&str> = diff
            .ordered_create_models()
            .iter()
            .map(|m| m.table_name.as_str())
            .collect();

        let pos = |name: &str| ordered.iter().position(|n| *n == name).unwrap();
        assert!(pos("sync_sources") < pos("tracks"));
        assert!(pos("sync_sources") < pos("playlists"));
        assert_eq!(ordered.len(), 3);
    }

    #[test]
    fn ordered_create_models_ignores_self_references() {
        let mut diff = SchemaDiff::default();
        diff.create_models.push(model_with_fks("nodes", &["nodes"]));
        let ordered = diff.ordered_create_models();
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].table_name, "nodes");
    }

    #[test]
    fn ordered_create_models_ignores_external_references() {
        // FK to a table not in this batch (already exists) should not block.
        let mut diff = SchemaDiff::default();
        diff.create_models
            .push(model_with_fks("orders", &["users"]));
        let ordered = diff.ordered_create_models();
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].table_name, "orders");
    }

    #[test]
    fn ordered_create_models_handles_cycles_without_dropping_models() {
        let mut diff = SchemaDiff::default();
        diff.create_models.push(model_with_fks("a", &["b"]));
        diff.create_models.push(model_with_fks("b", &["a"]));
        let ordered = diff.ordered_create_models();
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn ordered_create_models_handles_chain() {
        let mut diff = SchemaDiff::default();
        diff.create_models.push(model_with_fks("c", &["b"]));
        diff.create_models.push(model_with_fks("b", &["a"]));
        diff.create_models.push(model_with_fks("a", &[]));

        let ordered: Vec<&str> = diff
            .ordered_create_models()
            .iter()
            .map(|m| m.table_name.as_str())
            .collect();
        assert_eq!(ordered, vec!["a", "b", "c"]);
    }
}
