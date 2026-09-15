//! Schema diffing for generating migrations.

use std::collections::{HashMap, HashSet, VecDeque};

use prax_schema::Schema;
use prax_schema::ast::{Field, FieldType, GeneratedAttribute, IndexType, Model, VectorOps, View};

use crate::error::MigrateResult;
use crate::procedure::{
    ProcedureDefinition, ProcedureDiff, ProcedureDiffer, ProcedureLanguage, Volatility,
};

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
    /// Procedure changes.
    pub procedures: Option<ProcedureDiff>,
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
            && self.procedures.as_ref().is_none_or(|p| p.is_empty())
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

        if let Some(proc_diff) = &self.procedures {
            if !proc_diff.create.is_empty() {
                parts.push(format!("Create {} procedures", proc_diff.create.len()));
            }
            if !proc_diff.drop.is_empty() {
                parts.push(format!("Drop {} procedures", proc_diff.drop.len()));
            }
            if !proc_diff.alter.is_empty() {
                parts.push(format!("Alter {} procedures", proc_diff.alter.len()));
            }
            if !proc_diff.create_triggers.is_empty() {
                parts.push(format!(
                    "Create {} triggers",
                    proc_diff.create_triggers.len()
                ));
            }
            if !proc_diff.drop_triggers.is_empty() {
                parts.push(format!("Drop {} triggers", proc_diff.drop_triggers.len()));
            }
            if !proc_diff.alter_triggers.is_empty() {
                parts.push(format!("Alter {} triggers", proc_diff.alter_triggers.len()));
            }
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
    /// Default value expression, rendered in ANSI/Postgres form (see
    /// `render_default_sql_ansi`). Non-Postgres generators must remap
    /// dialect-specific functions — notably `gen_random_uuid()` → MySQL
    /// `(UUID())`, MSSQL `NEWID()`, SQLite `lower(hex(randomblob(16)))` or
    /// app-side generation with a warning.
    pub default: Option<String>,
    /// Whether this is a primary key.
    pub is_primary_key: bool,
    /// Whether this has auto increment.
    pub is_auto_increment: bool,
    /// Whether this is unique.
    pub is_unique: bool,
    /// Optional vector column metadata. Only used by SQLite backends; other
    /// generators ignore this field. Populated by the differ when a field
    /// declares a `Vector`/`HalfVector` type with `@dim(N)` (or a type-level
    /// dimension), plus optional `@vectorType`/`@metric`/`@index` attributes.
    pub vector: Option<VectorColumnInfo>,
    /// If this field is an enum type, the enum name; otherwise None.
    /// Dialects can choose how to render: Postgres uses `"name"` as the column
    /// type referencing a pre-created enum type; SQLite/MySQL/MSSQL/DuckDB use
    /// TEXT (optionally with a CHECK constraint of valid variants).
    pub enum_name: Option<String>,
    /// If this field is a generated/computed column, the `@generated` attribute
    /// payload. Generators use this to emit dialect-specific GENERATED AS syntax.
    pub generated: Option<GeneratedAttribute>,
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
    /// Old nullable. Always populated (both schemas are available to the
    /// differ) so generators whose ALTER syntax reissues the full column
    /// definition (MySQL `MODIFY COLUMN`, MSSQL `ALTER COLUMN`) can preserve
    /// nullability when only the type changed — on those engines omitting
    /// the NULL/NOT NULL clause silently relaxes the column to nullable.
    pub old_nullable: Option<bool>,
    /// New nullable (if changed).
    pub new_nullable: Option<bool>,
    /// Old default (if changed). Rendered in ANSI/Postgres form — see
    /// `render_default_sql_ansi` for the dialect-remapping contract.
    pub old_default: Option<String>,
    /// New default (if changed). `Some(old)` → `None` means the default was
    /// removed and requires `DROP DEFAULT`; generators that only inspect
    /// `new_default` do not currently emit that statement.
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
    ///
    /// LIMITATION: the Postgres and DuckDB generators currently ignore
    /// `remove_values` (enum value removal requires dropping and recreating
    /// the type plus rewriting dependent columns, which they do not emit),
    /// and `SchemaDiff` has no warnings channel to surface that gap. A
    /// non-empty `remove_values` therefore requires a hand-written migration
    /// to take effect on those dialects.
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

        // Models are keyed by their database table name (`@@map` or model
        // name), not the Prax model name: the table is the stable database
        // identity. This lets a `.prax` model `User @@map("users")` diff
        // cleanly against an introspected schema whose model is named `Users`
        // (PascalCase of the table) — same table, no spurious create/drop.
        let source_models: HashMap<&str, &Model> = self
            .source
            .as_ref()
            .map(|s| s.models.values().map(|m| (m.table_name(), m)).collect())
            .unwrap_or_default();

        let target_models: HashMap<&str, &Model> = self
            .target
            .models
            .values()
            .map(|m| (m.table_name(), m))
            .collect();

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
        //
        // The source model's foreign keys must resolve against the source
        // schema (its relation targets are named after PascalCased table
        // names), so pass it explicitly. `source_models` is only non-empty
        // when `self.source` is `Some`, so the fallback to target is never
        // taken in the alter path — it only keeps the call total.
        let source_schema = self.source.as_ref().unwrap_or(&self.target);
        for (name, target_model) in &target_models {
            if let Some(source_model) = source_models.get(name)
                && let Some(alter) =
                    diff_models(source_model, target_model, source_schema, &self.target)
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
                    // Emit the enum's database name (`@@map`), so a schema enum
                    // `TeamRole @@map("team_role")` creates the `team_role`
                    // Postgres type — matching how columns reference it and how
                    // introspection reads it back.
                    name: enum_def.database_name().to_string(),
                    values: enum_def
                        .variants
                        .iter()
                        .map(|v| v.name.to_string())
                        .collect(),
                });
            }
        }

        // Find enums to alter (variant set changes on enums present in both)
        for (name, target_enum) in &target_enums {
            if let Some(source_enum) = source_enums.get(name) {
                let source_values: Vec<&str> = source_enum
                    .variants
                    .iter()
                    .map(|v| v.name.as_str())
                    .collect();
                let target_values: Vec<&str> = target_enum
                    .variants
                    .iter()
                    .map(|v| v.name.as_str())
                    .collect();

                if source_values == target_values {
                    continue;
                }

                let source_set: HashSet<&str> = source_values.iter().copied().collect();
                let target_set: HashSet<&str> = target_values.iter().copied().collect();

                let add_values: Vec<String> = target_values
                    .iter()
                    .filter(|v| !source_set.contains(**v))
                    .map(|v| (*v).to_string())
                    .collect();
                let remove_values: Vec<String> = source_values
                    .iter()
                    .filter(|v| !target_set.contains(**v))
                    .map(|v| (*v).to_string())
                    .collect();

                // A pure reorder produces empty add/remove lists and needs no DDL.
                if !add_values.is_empty() || !remove_values.is_empty() {
                    result.alter_enums.push(EnumAlterDiff {
                        name: (*name).to_string(),
                        add_values,
                        remove_values,
                    });
                }
            }
        }

        // HashMap iteration order is nondeterministic; sort by enum name so
        // generated migration SQL (and its checksum) is stable run-to-run.
        result.alter_enums.sort_by(|a, b| a.name.cmp(&b.name));

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
                && let Some(view_diff) = view_to_diff(view, &self.target)
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
                let source_sql = self
                    .source
                    .as_ref()
                    .and_then(|s| resolve_view_sql(source_view, s));
                let target_sql = resolve_view_sql(target_view, &self.target);

                // Check if SQL or materialized status changed
                let sql_changed = source_sql != target_sql;
                let materialized_changed =
                    source_view.is_materialized() != target_view.is_materialized();

                if (sql_changed || materialized_changed)
                    && let Some(view_diff) = view_to_diff(target_view, &self.target)
                {
                    result.alter_views.push(view_diff);
                }
            }
        }

        // Diff procedures
        let source_procedures: Vec<ProcedureDefinition> = self
            .source
            .as_ref()
            .map(|s| {
                s.procedures
                    .values()
                    .map(ast_procedure_to_definition)
                    .collect()
            })
            .unwrap_or_default();

        let target_procedures: Vec<ProcedureDefinition> = self
            .target
            .procedures
            .values()
            .map(ast_procedure_to_definition)
            .collect();

        let proc_diff = ProcedureDiffer::diff(&source_procedures, &target_procedures);
        if !proc_diff.is_empty() {
            result.procedures = Some(proc_diff);
        }

        // Normalize enum type names to their database names (`@@map`).
        //
        // Field/enum diffs are built with the enum's *schema* name (e.g.
        // `RepoRole`), but the emitted SQL must reference the real Postgres
        // type — `repo_role` when the enum carries `@@map("repo_role")`.
        // Resolve once here so every consumer (CREATE TYPE, ALTER TYPE, DROP
        // TYPE, and every column's `enum_name`) uses the database name.
        {
            use std::collections::HashMap as Map;
            let enum_db: Map<String, String> = self
                .target
                .enums
                .values()
                .map(|e| (e.name().to_string(), e.database_name().to_string()))
                .collect();
            let resolve =
                |n: &str| -> String { enum_db.get(n).cloned().unwrap_or_else(|| n.to_string()) };
            for e in &mut result.alter_enums {
                e.name = resolve(&e.name);
            }
            for n in &mut result.drop_enums {
                *n = resolve(n);
            }
            // create_enums already store database_name(); leave them.
            let fix_field = |f: &mut FieldDiff| {
                if let Some(en) = &f.enum_name {
                    f.enum_name = Some(resolve(en));
                }
            };
            for m in &mut result.create_models {
                for f in &mut m.fields {
                    fix_field(f);
                }
            }
            for m in &mut result.alter_models {
                for f in &mut m.add_fields {
                    fix_field(f);
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

    let primary_key = primary_key_columns(model);

    let foreign_keys = extract_foreign_keys(model, schema);

    // Extract indexes from @@index/@@unique attributes; @@unique becomes a
    // unique constraint in CREATE TABLE rather than a separate index.
    let mut indexes = Vec::new();
    let mut unique_constraints = Vec::new();

    for index in extract_index_diffs(model) {
        if index.unique {
            unique_constraints.push(UniqueConstraint {
                name: Some(index.name),
                columns: index.columns,
            });
        } else {
            indexes.push(index);
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

/// Resolve a model's primary-key **column** names.
///
/// Prefers a model-level `@@id([a, b, …])` (composite key), falling back to
/// the field(s) carrying a field-level `@id`. Each referenced field is mapped
/// to its database column name via `@map` (so a PK on a mapped field emits the
/// real column). `@@id` argument order is preserved; field-level `@id` uses
/// the model's field declaration order.
pub(crate) fn primary_key_columns(model: &Model) -> Vec<String> {
    // Model-level @@id wins when present.
    if let Some(attr) = model.get_attribute("id")
        && let Some(first) = attr.first_arg()
    {
        let field_names: Vec<String> = match first {
            prax_schema::ast::AttributeValue::FieldRef(col) => vec![col.to_string()],
            prax_schema::ast::AttributeValue::FieldRefList(cols) => {
                cols.iter().map(|c| c.to_string()).collect()
            }
            _ => Vec::new(),
        };
        if !field_names.is_empty() {
            return field_names
                .iter()
                .map(|f| index_column_name(model, f))
                .collect();
        }
    }

    // Fall back to field-level @id, in declaration order.
    model
        .fields
        .values()
        .filter(|f| f.has_attribute("id"))
        .map(|f| {
            f.get_attribute("map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| f.name().to_string())
        })
        .collect()
}

/// Map an `@@index`/`@@unique` field reference to its column name,
/// respecting the field's `@map` attribute. Shared with the shadow-drift
/// index signatures so both generate identical fallback index names.
pub(crate) fn index_column_name(model: &Model, field_name: &str) -> String {
    model
        .fields
        .get(field_name)
        .and_then(|f| {
            f.get_attribute("map")
                .and_then(|a| a.first_arg())
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| field_name.to_string())
}

/// Extract `@@index`/`@@unique` model-level attributes as index diffs.
///
/// `@@unique` attributes are returned as `IndexDiff` with `unique: true`;
/// callers that need constraint form (e.g. CREATE TABLE) split on that flag.
fn extract_index_diffs(model: &Model) -> Vec<IndexDiff> {
    let mut indexes = Vec::new();

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
            .map(|field_name| index_column_name(model, field_name))
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

        indexes.push(IndexDiff {
            name: index_name,
            table_name: model.table_name().to_string(),
            columns: column_names,
            unique: attr_name == "unique",
            index_type: None,
            vector_ops: None,
            hnsw_m: None,
            hnsw_ef_construction: None,
            ivfflat_lists: None,
        });
    }

    indexes
}

/// Structural equality for two index definitions keyed by the same name.
fn index_def_eq(a: &IndexDiff, b: &IndexDiff) -> bool {
    a.table_name == b.table_name
        && a.columns == b.columns
        && a.unique == b.unique
        && a.index_type == b.index_type
        && a.vector_ops == b.vector_ops
        && a.hnsw_m == b.hnsw_m
        && a.hnsw_ef_construction == b.hnsw_ef_construction
        && a.ivfflat_lists == b.ivfflat_lists
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

        // FK columns must be the *database column* names, resolving each
        // field reference through `@map` — otherwise a `@map`'d field like
        // `organizationId @map("organization_id")` would emit
        // `FOREIGN KEY ("organizationId")`, referencing a column that does
        // not exist. `index_column_name` resolves a field ref against a
        // model's `@map`.
        let columns: Vec<String> = rel
            .fields
            .iter()
            .map(|f| index_column_name(model, f))
            .collect();
        // Referenced columns resolve against the *referenced* model's `@map`
        // when it is in the schema; otherwise fall back to the raw ref.
        let referenced_model = match &field.field_type {
            FieldType::Model(name) => schema.models.get(name.as_str()),
            _ => None,
        };
        let referenced_columns: Vec<String> = rel
            .references
            .iter()
            .map(|r| match referenced_model {
                Some(m) => index_column_name(m, r),
                None => r.to_string(),
            })
            .collect();

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
///
/// DIALECT CONTRACT: the differ is dialect-agnostic (`SchemaDiffer` never
/// learns the target backend), so function defaults render in their Postgres
/// form and each vendor generator MUST remap them before emitting DDL:
/// - `uuid()` → `gen_random_uuid()` here (Postgres/DuckDB-valid). MySQL:
///   `(UUID())`; MSSQL: `NEWID()`; SQLite: no built-in — use
///   `lower(hex(randomblob(16)))` or generate app-side and surface a warning.
/// - `now()` → `CURRENT_TIMESTAMP` is ANSI-valid on every supported dialect.
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
                // UUID generation - Postgres form; non-Postgres generators
                // must remap (see fn doc).
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

/// Render a field's `@default` attribute to its ANSI SQL form, if any.
fn field_default_sql(field: &Field) -> Option<String> {
    field
        .get_attribute("default")
        .and_then(|attr| attr.first_arg())
        .and_then(render_default_sql_ansi)
}

/// Convert a field to a diff.
fn field_to_diff(field: &Field) -> FieldDiff {
    // A `@db.*` native type overrides the scalar's default SQL type — e.g.
    // `String @db.Uuid` is a `uuid` column, not `TEXT`. Without this, every
    // `@db.Uuid` column diffs as TEXT-vs-uuid against the real database and
    // churns on every migration.
    let sql_type =
        native_type_to_sql(field).unwrap_or_else(|| field_type_to_sql(&field.field_type));
    let nullable = field.is_optional();
    let is_primary_key = field.has_attribute("id");
    let is_auto_increment = field.has_attribute("auto");
    let is_unique = field.has_attribute("unique");

    let default = field_default_sql(field);

    // Column name resolves `@map`; shared with the diff field-keying so an
    // added column is named identically to how it is matched.
    let column_name = field_column_name(field).to_string();

    // Extract enum name if this is an enum type
    let enum_name = match &field.field_type {
        FieldType::Enum(name) => Some(name.to_string()),
        _ => None,
    };

    let generated = field.generated();

    let vector = extract_vector_info(field);

    FieldDiff {
        name: field.name().to_string(),
        column_name,
        sql_type,
        nullable,
        default,
        is_primary_key,
        is_auto_increment,
        is_unique,
        vector,
        enum_name,
        generated,
    }
}

/// Extract vector column metadata from a `Vector`/`HalfVector` field.
///
/// Reads the scalar type plus the `@dim(N)`, `@vectorType(...)`,
/// `@metric(...)`, and `@index(...)` field attributes. The dimension comes
/// from `@dim` when present, falling back to the type parameter
/// (`Vector(1536)`). Returns `None` for non-vector fields and for vector
/// fields with no usable dimension. `SparseVector`/`Bit` have no
/// sqlite-vec equivalent and stay pgvector-only, so they yield `None`.
fn extract_vector_info(field: &Field) -> Option<VectorColumnInfo> {
    use prax_schema::ast::ScalarType;

    let FieldType::Scalar(scalar) = &field.field_type else {
        return None;
    };

    let (type_dim, default_element) = match scalar {
        ScalarType::Vector(dim) => (*dim, VectorElementType::Float4),
        ScalarType::HalfVector(dim) => (*dim, VectorElementType::Float2),
        _ => return None,
    };

    let dimensions = field
        .get_attribute("dim")
        .and_then(|attr| attr.first_arg())
        .and_then(|v| v.as_int())
        .and_then(|i| u32::try_from(i).ok())
        .or(type_dim)?;

    let element_type = field
        .get_attribute("vectorType")
        .and_then(|attr| attr.first_ident_arg())
        .map_or(default_element, parse_vector_element_type);

    let metric = field
        .get_attribute("metric")
        .and_then(|attr| attr.first_ident_arg())
        .map_or(VectorDistanceMetric::Cosine, parse_vector_metric);

    let index = field
        .get_attribute("index")
        .and_then(|attr| attr.first_ident_arg())
        .and_then(parse_vector_index_kind);

    Some(VectorColumnInfo {
        dimensions,
        element_type,
        metric,
        index,
    })
}

/// Parse a `@vectorType` value; unknown values fall back to float4 with a warning.
fn parse_vector_element_type(value: &str) -> VectorElementType {
    match value {
        "float2" => VectorElementType::Float2,
        "float4" => VectorElementType::Float4,
        "float8" => VectorElementType::Float8,
        "int1" => VectorElementType::Int1,
        "int2" => VectorElementType::Int2,
        "int4" => VectorElementType::Int4,
        other => {
            eprintln!(
                "Warning: invalid vector element type '{}' - using float4",
                other
            );
            VectorElementType::Float4
        }
    }
}

/// Parse a `@metric` value; unknown values fall back to cosine with a warning.
fn parse_vector_metric(value: &str) -> VectorDistanceMetric {
    match value {
        "cosine" => VectorDistanceMetric::Cosine,
        "l2" => VectorDistanceMetric::L2,
        "inner" => VectorDistanceMetric::InnerProduct,
        other => {
            eprintln!("Warning: invalid vector metric '{}' - using cosine", other);
            VectorDistanceMetric::Cosine
        }
    }
}

/// Parse a `@index` value into a vector index kind; unknown values warn and
/// yield no index.
fn parse_vector_index_kind(value: &str) -> Option<VectorIndexKind> {
    match value {
        "hnsw" => Some(VectorIndexKind::Hnsw),
        other => {
            eprintln!(
                "Warning: invalid vector index '{}' (expected: hnsw) - skipping index",
                other
            );
            None
        }
    }
}

/// Convert a field type to SQL.
/// Map a field's `@db.*` native-type attribute to its SQL type, when present.
///
/// A native type overrides the scalar default (e.g. `String @db.Uuid` is a
/// `uuid` column, not `TEXT`). The emitted strings match what the
/// introspection source produces for the same column, so an unchanged
/// `@db`-typed column diffs to nothing. Returns `None` when the field carries
/// no native type, so the caller falls back to the scalar mapping.
fn native_type_to_sql(field: &Field) -> Option<String> {
    // `@db.Uuid` / `@db.VarChar(255)` parse as an attribute whose *name* is
    // the dotted form `db.Uuid` (the grammar folds the namespace into the
    // name), so `extract_attributes` — which matches a bare `"db"` — never
    // captures them. Read the dotted attribute directly: find `db.<Type>` and
    // take `<Type>` as the native type name, with any parens as args.
    let attr = field
        .attributes
        .iter()
        .find(|a| a.name().starts_with("db."))?;
    let type_name = attr.name().strip_prefix("db.")?;
    let arg_i = |i: usize| -> Option<i64> {
        attr.args.get(i).and_then(|a| match &a.value {
            prax_schema::ast::AttributeValue::Int(n) => Some(*n),
            _ => None,
        })
    };
    let sql = match type_name {
        n if n.eq_ignore_ascii_case("Uuid") => "UUID".to_string(),
        n if n.eq_ignore_ascii_case("Text") => "TEXT".to_string(),
        n if n.eq_ignore_ascii_case("VarChar") || n.eq_ignore_ascii_case("VarChar2") => {
            match arg_i(0) {
                Some(len) => format!("VARCHAR({len})"),
                None => "VARCHAR".to_string(),
            }
        }
        n if n.eq_ignore_ascii_case("Char") => match arg_i(0) {
            Some(len) => format!("CHAR({len})"),
            None => "CHAR".to_string(),
        },
        n if n.eq_ignore_ascii_case("Boolean") || n.eq_ignore_ascii_case("Bool") => {
            "BOOLEAN".to_string()
        }
        n if n.eq_ignore_ascii_case("SmallInt") => "SMALLINT".to_string(),
        n if n.eq_ignore_ascii_case("Integer") || n.eq_ignore_ascii_case("Int") => {
            "INTEGER".to_string()
        }
        n if n.eq_ignore_ascii_case("BigInt") => "BIGINT".to_string(),
        n if n.eq_ignore_ascii_case("Real") => "REAL".to_string(),
        n if n.eq_ignore_ascii_case("DoublePrecision") => "DOUBLE PRECISION".to_string(),
        n if n.eq_ignore_ascii_case("Decimal") || n.eq_ignore_ascii_case("Numeric") => {
            match (arg_i(0), arg_i(1)) {
                (Some(p), Some(s)) => format!("DECIMAL({p}, {s})"),
                _ => "DECIMAL".to_string(),
            }
        }
        n if n.eq_ignore_ascii_case("Json") || n.eq_ignore_ascii_case("JsonB") => {
            "JSONB".to_string()
        }
        n if n.eq_ignore_ascii_case("Date") => "DATE".to_string(),
        n if n.eq_ignore_ascii_case("Time") => "TIME".to_string(),
        n if n.eq_ignore_ascii_case("Timestamp") => "TIMESTAMP".to_string(),
        n if n.eq_ignore_ascii_case("Timestamptz") => "TIMESTAMP WITH TIME ZONE".to_string(),
        n if n.eq_ignore_ascii_case("Bytea") => "BYTEA".to_string(),
        // Unknown native type: fall back to the scalar mapping.
        _ => return None,
    };
    Some(sql)
}

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

/// The database column a field maps to: its `@map("...")` argument when
/// present, otherwise the field name itself.
///
/// Field identity in a diff is the *column* name, not the Rust field name.
/// An introspected source schema names its fields after the real database
/// columns (snake_case), while a hand-written target may use a different
/// field name pinned to that column via `@map` (e.g. `familyId`
/// `@map("family_id")`). Keying the field comparison on the field name would
/// see `family_id` and `familyId` as unrelated — proposing a drop and an add
/// for a column that never changed. Keying on the mapped column name is what
/// makes a database that already matches its schema diff to empty.
///
/// Mirrors [`index_column_name`], which already resolves `@map` for
/// `@@index`/`@@unique` column references.
fn field_column_name(field: &Field) -> &str {
    field
        .get_attribute("map")
        .and_then(|a| a.first_arg())
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| field.name())
}

/// Diff two models and return alterations if any.
///
/// `source_schema`/`target_schema` are the schemas each model belongs to.
/// Foreign keys must be resolved against their *own* schema: a source model
/// synthesized from introspection names its relation target by the
/// PascalCased table name (`Users`), which only resolves in the source
/// schema — resolving it against the target (where the model is `User
/// @@map("users")`) would fail, mis-name the referenced table, and churn
/// every foreign key.
fn diff_models(
    source: &Model,
    target: &Model,
    source_schema: &Schema,
    target_schema: &Schema,
) -> Option<ModelAlterDiff> {
    // Fields are keyed by their mapped *column* name (respecting `@map`), not
    // the Rust field name, so a `@map`'d field and its introspected
    // column-named counterpart are recognized as the same column rather than a
    // drop+add pair. See [`field_column_name`].
    let source_fields: HashMap<&str, &Field> = source
        .fields
        .values()
        .filter(|f| !f.is_relation())
        .map(|f| (field_column_name(f), f))
        .collect();

    let target_fields: HashMap<&str, &Field> = target
        .fields
        .values()
        .filter(|f| !f.is_relation())
        .map(|f| (field_column_name(f), f))
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

    // Diff model-level indexes (@@index / @@unique, keyed by index name).
    // BTreeMap keeps iteration sorted by index name so add/drop ordering
    // (and therefore generated SQL and checksums) is deterministic.
    let source_indexes = extract_index_diffs(source);
    let target_indexes = extract_index_diffs(target);

    // Index identity is its shape — the ordered column list plus uniqueness —
    // NOT its name. An introspected source names an index by its real
    // database name (`bots_repository_idx`), while the target derives
    // `idx_<table>_<cols>`; keying on the name alone would drop the former and
    // add the latter for an index that already covers the same columns. Keying
    // on the shape reconciles them: same columns + uniqueness ⇒ same index, no
    // churn. A genuine definition change (e.g. uniqueness flip) still shows up
    // because it changes the signature.
    fn index_sig(i: &IndexDiff) -> (bool, Vec<String>) {
        (i.unique, i.columns.clone())
    }
    let source_by_sig: std::collections::BTreeMap<(bool, Vec<String>), &IndexDiff> =
        source_indexes.iter().map(|i| (index_sig(i), i)).collect();
    let target_by_sig: std::collections::BTreeMap<(bool, Vec<String>), &IndexDiff> =
        target_indexes.iter().map(|i| (index_sig(i), i)).collect();

    let mut add_indexes = Vec::new();
    let mut drop_indexes = Vec::new();

    for (sig, target_index) in &target_by_sig {
        match source_by_sig.get(sig) {
            // No index of this shape exists in the database — create it.
            None => add_indexes.push((*target_index).clone()),
            // An index of the same shape already exists (any name). Recreate
            // only when a non-shape property the differ tracks actually
            // differs; a pure name difference is left alone.
            Some(source_index) if !index_def_eq(source_index, target_index) => {
                drop_indexes.push(source_index.name.clone());
                add_indexes.push((*target_index).clone());
            }
            _ => {}
        }
    }
    for (sig, source_index) in &source_by_sig {
        if !target_by_sig.contains_key(sig) {
            drop_indexes.push(source_index.name.clone());
        }
    }

    // Diff foreign keys. Each side resolves against its own schema so the
    // introspected source's PascalCase relation targets (`Users`) resolve
    // there, while the target's (`User`) resolve in the target.
    let source_fks = extract_foreign_keys(source, source_schema);
    let target_fks = extract_foreign_keys(target, target_schema);

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
        && add_indexes.is_empty()
        && drop_indexes.is_empty()
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
            add_indexes,
            drop_indexes,
            add_foreign_keys,
            drop_foreign_keys,
        })
    }
}

/// Resolve the SQL defining a view: the inline `@@sql` attribute if present,
/// otherwise a top-level `@@sql` definition (`schema.raw_sql`) whose name
/// matches the view's database name (`@@map`) or model name.
fn resolve_view_sql<'a>(view: &'a View, schema: &'a Schema) -> Option<&'a str> {
    if let Some(sql) = view.sql_query() {
        return Some(sql);
    }
    schema
        .raw_sql
        .iter()
        .find(|r| r.name.as_str() == view.view_name() || r.name.as_str() == view.name())
        .map(|r| r.sql.as_str())
}

/// Convert a view to a diff for creation.
fn view_to_diff(view: &View, schema: &Schema) -> Option<ViewDiff> {
    // Views require a SQL definition (inline @@sql or top-level @@sql) to be migrated
    let sql_query = resolve_view_sql(view, schema)?.to_string();

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
    // Honor `@db.*` native types on both sides (e.g. `String @db.Uuid` is a
    // `uuid` column, not `TEXT`) so an unchanged native-typed column does not
    // churn. The introspected source expresses the type as a scalar
    // (`Scalar(Uuid)` → `UUID`); the hand-written target expresses it as
    // `String @db.Uuid` — both must resolve to the same SQL type here.
    let source_type =
        native_type_to_sql(source).unwrap_or_else(|| field_type_to_sql(&source.field_type));
    let target_type =
        native_type_to_sql(target).unwrap_or_else(|| field_type_to_sql(&target.field_type));

    let source_nullable = source.is_optional();
    let target_nullable = target.is_optional();

    let source_default = field_default_sql(source);
    let target_default = field_default_sql(target);

    let type_changed = source_type != target_type;
    let nullable_changed = source_nullable != target_nullable;
    let default_changed = source_default != target_default;

    if !type_changed && !nullable_changed && !default_changed {
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
        old_nullable: Some(source_nullable),
        new_nullable: if nullable_changed {
            Some(target_nullable)
        } else {
            None
        },
        old_default: if default_changed {
            source_default
        } else {
            None
        },
        new_default: if default_changed {
            target_default
        } else {
            None
        },
    })
}

/// Metadata describing a vector column.
///
/// Populated by the schema differ when a field is declared with the
/// `Vector`/`HalfVector` type and the `@dim(...)`, `@vectorType(...)`,
/// `@metric(...)`, and `@index(...)` attributes. Only consumed by the SQLite
/// generator; Postgres/MySQL/MSSQL/DuckDB generators treat fields with
/// `vector = Some(_)` as an error (reported by the schema differ).
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

/// Convert a schema AST [`Procedure`] to a [`ProcedureDefinition`] for diffing.
fn ast_procedure_to_definition(p: &prax_schema::ast::Procedure) -> ProcedureDefinition {
    ProcedureDefinition {
        name: p.name().to_string(),
        schema: None,
        is_function: p.is_function,
        parameters: Vec::new(),
        return_type: None,
        returns_set: false,
        return_columns: Vec::new(),
        language: ProcedureLanguage::Sql,
        body: p.body.clone(),
        volatility: Volatility::Volatile,
        security_definer: false,
        cost: None,
        rows: None,
        parallel: crate::procedure::ParallelSafety::Unsafe,
        or_replace: true,
        comment: None,
        checksum: None,
        version: None,
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
    fn test_composite_id_becomes_primary_key() {
        // A model-level @@id([a, b]) must produce a composite primary key in
        // the create-model diff. Prior to the fix, model_to_diff only looked
        // at field-level @id and silently produced an empty primary key for
        // @@id models (the motivating team_members shape).
        let schema = prax_schema::parse_schema(
            r#"
            model Membership {
                userId Int
                teamId Int

                @@id([userId, teamId])
            }
            "#,
        )
        .unwrap();
        let differ = SchemaDiffer::new(schema);
        let diff = differ.diff().unwrap();

        let model = diff
            .create_models
            .iter()
            .find(|m| m.name == "Membership")
            .expect("Membership created");
        assert_eq!(model.primary_key, vec!["userId", "teamId"]);
    }

    #[test]
    fn introspected_source_with_fk_does_not_churn() {
        // Regression: a source schema reverse-engineered from introspection
        // names each relation target by the PascalCased table name
        // (`refresh_tokens` FK → `users` → `FieldType::Model("Users")`). The
        // diff resolved *both* sides' foreign keys against the target schema,
        // where the model is `User` (not `Users`), so the source FK's
        // referenced table mis-resolved to the literal "Users" and every FK
        // (and the columns/tables around it) churned. Each side must resolve
        // against its own schema.
        use crate::introspect::{
            ColumnInfo as IC, ConstraintInfo as ICon, IntrospectionConfig, SchemaBuilder,
            TableInfo as IT,
        };
        let col = |name: &str| IC {
            name: name.to_string(),
            data_type: "text".to_string(),
            udt_name: "text".to_string(),
            character_maximum_length: None,
            numeric_precision: None,
            is_nullable: false,
            column_default: None,
            ordinal_position: 0,
            comment: None,
        };
        let table = |name: &str| IT {
            name: name.to_string(),
            schema: "public".to_string(),
            table_type: "BASE TABLE".to_string(),
            comment: None,
        };
        let pk = |t: &str| ICon {
            name: format!("{t}_pkey"),
            constraint_type: "PRIMARY KEY".to_string(),
            table_name: t.to_string(),
            columns: vec!["id".to_string()],
            referenced_table: None,
            referenced_columns: None,
            on_delete: None,
            on_update: None,
        };

        let source = SchemaBuilder::new(IntrospectionConfig::default())
            .with_tables(vec![table("users"), table("refresh_tokens")])
            .with_columns("users", vec![col("id")])
            .with_constraints("users", vec![pk("users")])
            .with_columns(
                "refresh_tokens",
                vec![col("id"), col("family_id"), col("user_id")],
            )
            .with_constraints(
                "refresh_tokens",
                vec![
                    pk("refresh_tokens"),
                    ICon {
                        name: "fk_refresh_tokens_user".to_string(),
                        constraint_type: "FOREIGN KEY".to_string(),
                        table_name: "refresh_tokens".to_string(),
                        columns: vec!["user_id".to_string()],
                        referenced_table: Some("users".to_string()),
                        referenced_columns: Some(vec!["id".to_string()]),
                        on_delete: Some("CASCADE".to_string()),
                        on_update: None,
                    },
                ],
            )
            .build()
            .unwrap()
            .schema;

        // Target: the same shape as a hand-written `.prax` — camelCase fields
        // and models with `@@map`, FK relation pinned to the real constraint
        // name so it matches the introspected one.
        let target = prax_schema::parse_schema(
            r#"
            model User {
                id            String        @id
                refreshTokens RefreshToken[]
                @@map("users")
            }
            model RefreshToken {
                id       String @id
                familyId String @map("family_id")
                userId   String @map("user_id")
                user     User   @relation(fields: [userId], references: [id], onDelete: Cascade, map: "fk_refresh_tokens_user")
                @@map("refresh_tokens")
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert!(
            diff.create_models.is_empty()
                && diff.drop_models.is_empty()
                && diff.alter_models.is_empty(),
            "introspected source with an FK must diff to empty against its \
             @map'd target, got create={:?} drop={:?} alter={:?}",
            diff.create_models
                .iter()
                .map(|m| &m.name)
                .collect::<Vec<_>>(),
            diff.drop_models,
            diff.alter_models,
        );
    }

    #[test]
    fn mapped_field_does_not_churn_against_introspected_column_name() {
        // Regression: an introspected source names its fields after the real
        // database columns (snake_case), while the target `.prax` pins a
        // camelCase field to that column via `@map`. Keying the field diff on
        // the field name saw `family_id` (source) and `familyId` (target) as
        // unrelated, proposing a spurious drop+add for an unchanged column.
        // Keying on the mapped column name must diff them to nothing.
        let source = prax_schema::parse_schema(
            r#"
            model RefreshToken {
                id        String @id
                family_id String

                @@map("refresh_tokens")
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::parse_schema(
            r#"
            model RefreshToken {
                id       String @id
                familyId String @map("family_id")

                @@map("refresh_tokens")
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert!(
            diff.alter_models.is_empty(),
            "a @map'd field matching an introspected column must not churn, got: {:?}",
            diff.alter_models
        );
    }

    #[test]
    fn test_composite_id_respects_field_map() {
        // @@id referencing fields that carry @map must resolve to the mapped
        // column names, so the PRIMARY KEY clause references real columns.
        let schema = prax_schema::parse_schema(
            r#"
            model Membership {
                userId Int @map("user_id")
                teamId Int @map("team_id")

                @@id([userId, teamId])
            }
            "#,
        )
        .unwrap();
        let differ = SchemaDiffer::new(schema);
        let diff = differ.diff().unwrap();
        let model = diff
            .create_models
            .iter()
            .find(|m| m.name == "Membership")
            .expect("Membership created");
        assert_eq!(model.primary_key, vec!["user_id", "team_id"]);
    }

    #[test]
    fn test_field_level_id_respects_map() {
        // A single field-level @id with @map must resolve to the mapped
        // column name in the primary key.
        let schema = prax_schema::parse_schema(
            r#"
            model User {
                userId Int @id @map("user_id")
            }
            "#,
        )
        .unwrap();
        let differ = SchemaDiffer::new(schema);
        let diff = differ.diff().unwrap();
        let model = diff
            .create_models
            .iter()
            .find(|m| m.name == "User")
            .expect("User created");
        assert_eq!(model.primary_key, vec!["user_id"]);
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
            generated: None,
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

    #[test]
    fn test_enum_added_variant_produces_alter_enums() {
        let source = prax_schema::validate_schema(
            r#"
            enum Status { active pending }
            model Task {
                id     Int    @id
                status Status
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            enum Status { active pending cancelled }
            model Task {
                id     Int    @id
                status Status
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert_eq!(diff.alter_enums.len(), 1);
        assert_eq!(diff.alter_enums[0].name, "Status");
        assert_eq!(
            diff.alter_enums[0].add_values,
            vec!["cancelled".to_string()]
        );
        assert!(diff.alter_enums[0].remove_values.is_empty());
        assert!(diff.create_enums.is_empty());
        assert!(diff.drop_enums.is_empty());
    }

    #[test]
    fn test_model_index_changes_produce_add_drop_indexes() {
        let source = prax_schema::validate_schema(
            r#"
            model Post {
                id        Int    @id
                title     String
                author_id Int
                slug      String

                @@map("posts")
                @@index([author_id])
                @@index([title])
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            model Post {
                id        Int    @id
                title     String
                author_id Int
                slug      String

                @@map("posts")
                @@index([author_id])
                @@index([slug])
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert_eq!(diff.alter_models.len(), 1);
        let alter = &diff.alter_models[0];
        assert_eq!(alter.add_indexes.len(), 1);
        assert_eq!(alter.add_indexes[0].name, "idx_posts_slug");
        assert_eq!(alter.add_indexes[0].columns, vec!["slug".to_string()]);
        assert!(!alter.add_indexes[0].unique);
        assert_eq!(alter.drop_indexes, vec!["idx_posts_title".to_string()]);
    }

    #[test]
    fn test_field_default_change_produces_old_new_defaults() {
        let source = prax_schema::validate_schema(
            r#"
            model Widget {
                id     Int @id
                rating Int @default(0)
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            model Widget {
                id     Int @id
                rating Int @default(5)
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert_eq!(diff.alter_models.len(), 1);
        let alter_fields = &diff.alter_models[0].alter_fields;
        assert_eq!(alter_fields.len(), 1);
        assert_eq!(alter_fields[0].name, "rating");
        assert_eq!(alter_fields[0].old_default, Some("0".to_string()));
        assert_eq!(alter_fields[0].new_default, Some("5".to_string()));
        assert!(alter_fields[0].old_type.is_none());
        // old_nullable is always populated so full-definition ALTER
        // generators (MySQL/MSSQL) can preserve nullability.
        assert_eq!(alter_fields[0].old_nullable, Some(false));
        assert!(alter_fields[0].new_nullable.is_none());
    }

    #[test]
    fn test_view_with_top_level_raw_sql_produces_create_view() {
        // Mirrors examples/schema.prax: the view's SQL lives in a top-level
        // @@sql definition named after the view's @@map name.
        let schema = prax_schema::validate_schema(
            r#"
            view PostStats {
                post_id       Int @unique
                comment_count Int

                @@map("post_stats_view")
            }

            @@sql("post_stats_view", """
                CREATE OR REPLACE VIEW post_stats_view AS
                SELECT
                    p.id as post_id,
                    COUNT(DISTINCT c.id) as comment_count
                FROM posts p
                LEFT JOIN comments c ON c.post_id = p.id
                GROUP BY p.id
            """)
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(schema).diff().unwrap();

        assert_eq!(diff.create_views.len(), 1);
        let view = &diff.create_views[0];
        assert_eq!(view.name, "PostStats");
        assert_eq!(view.view_name, "post_stats_view");
        assert!(
            view.sql_query
                .contains("CREATE OR REPLACE VIEW post_stats_view"),
            "actual: {}",
            view.sql_query
        );
        assert_eq!(view.fields.len(), 2);
    }

    #[test]
    fn test_vector_field_populates_vector_info() {
        let schema = prax_schema::validate_schema(
            r#"
            model Embedding {
                id        Int    @id
                embedding Vector @dim(1536)
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(schema).diff().unwrap();

        let model = &diff.create_models[0];
        let field = model.fields.iter().find(|f| f.name == "embedding").unwrap();
        let vector = field.vector.as_ref().expect("vector info populated");
        assert_eq!(vector.dimensions, 1536);
        assert_eq!(vector.element_type, VectorElementType::Float4);
        assert_eq!(vector.metric, VectorDistanceMetric::Cosine);
        assert!(vector.index.is_none());
    }

    #[test]
    fn test_alter_enums_and_changed_indexes_have_deterministic_sorted_order() {
        // Two altered enums + two same-name changed indexes: the emitted
        // order must be sorted by name, not HashMap iteration order, so
        // generated migration SQL and checksums are stable run-to-run.
        let source = prax_schema::validate_schema(
            r#"
            enum Zebra { a b }
            enum Alpha { x y }
            model Post {
                id    Int    @id
                title String
                slug  String
                z     Zebra
                a     Alpha

                @@index([title], map: "b_idx")
                @@index([slug], map: "a_idx")
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            enum Zebra { a b c }
            enum Alpha { x y z }
            model Post {
                id    Int    @id
                title String
                slug  String
                z     Zebra
                a     Alpha

                @@index([title, slug], map: "b_idx")
                @@index([slug, title], map: "a_idx")
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        let enum_names: Vec<&str> = diff.alter_enums.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(enum_names, vec!["Alpha", "Zebra"]);

        assert_eq!(diff.alter_models.len(), 1);
        let alter = &diff.alter_models[0];
        let added: Vec<&str> = alter.add_indexes.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(added, vec!["a_idx", "b_idx"]);
        assert_eq!(
            alter.drop_indexes,
            vec!["a_idx".to_string(), "b_idx".to_string()]
        );
    }

    #[test]
    fn test_unknown_vector_attribute_values_fall_back_to_defaults() {
        // Unknown @vectorType/@metric/@index values warn (eprintln) and fall
        // back: float4 element type, cosine metric, no index.
        let schema = prax_schema::validate_schema(
            r#"
            model Embedding {
                id        Int    @id
                embedding Vector @dim(8) @vectorType(bogus) @metric(bogus) @index(bogus)
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(schema).diff().unwrap();

        let model = &diff.create_models[0];
        let field = model.fields.iter().find(|f| f.name == "embedding").unwrap();
        let vector = field.vector.as_ref().expect("vector info populated");
        assert_eq!(vector.dimensions, 8);
        assert_eq!(vector.element_type, VectorElementType::Float4);
        assert_eq!(vector.metric, VectorDistanceMetric::Cosine);
        assert!(vector.index.is_none());
    }

    #[test]
    fn test_enum_removed_variant_produces_remove_values() {
        let source = prax_schema::validate_schema(
            r#"
            enum Status { active pending cancelled }
            model Task {
                id     Int    @id
                status Status
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            enum Status { active pending }
            model Task {
                id     Int    @id
                status Status
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert_eq!(diff.alter_enums.len(), 1);
        assert_eq!(diff.alter_enums[0].name, "Status");
        assert!(diff.alter_enums[0].add_values.is_empty());
        assert_eq!(
            diff.alter_enums[0].remove_values,
            vec!["cancelled".to_string()]
        );
    }

    #[test]
    fn test_enum_pure_reorder_produces_no_alter_enums() {
        // Same variant set in a different order: no DDL is required.
        let source = prax_schema::validate_schema(
            r#"
            enum Status { active pending cancelled }
            model Task {
                id     Int    @id
                status Status
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            enum Status { cancelled active pending }
            model Task {
                id     Int    @id
                status Status
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert!(diff.alter_enums.is_empty());
        assert!(diff.create_enums.is_empty());
        assert!(diff.drop_enums.is_empty());
    }

    #[test]
    fn test_same_name_index_with_changed_definition_is_recreated() {
        // Same index name but different columns: drop + re-add.
        let source = prax_schema::validate_schema(
            r#"
            model Post {
                id    Int    @id
                title String
                slug  String

                @@index([title], map: "post_search")
            }
            "#,
        )
        .unwrap();
        let target = prax_schema::validate_schema(
            r#"
            model Post {
                id    Int    @id
                title String
                slug  String

                @@index([title, slug], map: "post_search")
            }
            "#,
        )
        .unwrap();

        let diff = SchemaDiffer::new(target)
            .with_source(source)
            .diff()
            .unwrap();

        assert_eq!(diff.alter_models.len(), 1);
        let alter = &diff.alter_models[0];
        assert_eq!(alter.drop_indexes, vec!["post_search".to_string()]);
        assert_eq!(alter.add_indexes.len(), 1);
        assert_eq!(alter.add_indexes[0].name, "post_search");
        assert_eq!(
            alter.add_indexes[0].columns,
            vec!["title".to_string(), "slug".to_string()]
        );
    }
}
