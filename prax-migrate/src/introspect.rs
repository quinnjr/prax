//! Database introspection for reverse-engineering schemas.
//!
//! This module provides functionality to introspect an existing database
//! and generate a Prax schema from its structure.

use std::collections::HashMap;

use prax_schema::Schema;
use prax_schema::ast::{
    Attribute, AttributeArg, AttributeValue, Enum, EnumVariant, Field, FieldType, Ident, Model,
    ScalarType, Span, TypeModifier, View,
};

use crate::error::{MigrateResult, MigrationError};

/// Result of introspecting a database.
#[derive(Debug, Clone)]
pub struct IntrospectionResult {
    /// The generated schema.
    pub schema: Schema,
    /// Tables that were skipped.
    pub skipped_tables: Vec<SkippedTable>,
    /// Warnings generated during introspection.
    pub warnings: Vec<String>,
}

/// A table that was skipped during introspection.
#[derive(Debug, Clone)]
pub struct SkippedTable {
    /// Table name.
    pub name: String,
    /// Reason it was skipped.
    pub reason: String,
}

/// Configuration for introspection.
#[derive(Debug, Clone)]
pub struct IntrospectionConfig {
    /// Schema to introspect (default: "public").
    pub database_schema: String,
    /// Tables to include (empty = all).
    pub include_tables: Vec<String>,
    /// Tables to exclude.
    pub exclude_tables: Vec<String>,
    /// Whether to include views.
    pub include_views: bool,
    /// Whether to include enums.
    pub include_enums: bool,
}

impl Default for IntrospectionConfig {
    fn default() -> Self {
        Self {
            database_schema: "public".to_string(),
            include_tables: Vec::new(),
            exclude_tables: vec![
                "_prax_migrations".to_string(),
                "_prisma_migrations".to_string(),
                "schema_migrations".to_string(),
            ],
            include_views: true,
            include_enums: true,
        }
    }
}

impl IntrospectionConfig {
    /// Create a new introspection config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the database schema to introspect.
    pub fn database_schema(mut self, schema: impl Into<String>) -> Self {
        self.database_schema = schema.into();
        self
    }

    /// Include only these tables.
    pub fn include_tables(mut self, tables: Vec<String>) -> Self {
        self.include_tables = tables;
        self
    }

    /// Exclude these tables.
    pub fn exclude_tables(mut self, tables: Vec<String>) -> Self {
        self.exclude_tables = tables;
        self
    }

    /// Whether to include views.
    pub fn include_views(mut self, include: bool) -> Self {
        self.include_views = include;
        self
    }

    /// Whether to include enums.
    pub fn include_enums(mut self, include: bool) -> Self {
        self.include_enums = include;
        self
    }

    /// Check if a table should be included.
    pub fn should_include_table(&self, name: &str) -> bool {
        if self.exclude_tables.contains(&name.to_string()) {
            return false;
        }
        if self.include_tables.is_empty() {
            return true;
        }
        self.include_tables.contains(&name.to_string())
    }
}

/// Raw table information from the database.
#[derive(Debug, Clone)]
pub struct TableInfo {
    /// Table name.
    pub name: String,
    /// Table schema (e.g., "public").
    pub schema: String,
    /// Table type ("BASE TABLE" or "VIEW").
    pub table_type: String,
    /// Table comment.
    pub comment: Option<String>,
}

/// Raw column information from the database.
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// Data type (e.g., "integer", "character varying").
    pub data_type: String,
    /// Full UDT name (e.g., "int4", "varchar").
    pub udt_name: String,
    /// Character maximum length (for varchar, etc.).
    pub character_maximum_length: Option<i32>,
    /// Numeric precision.
    pub numeric_precision: Option<i32>,
    /// Whether the column is nullable.
    pub is_nullable: bool,
    /// Default value expression.
    pub column_default: Option<String>,
    /// Ordinal position.
    pub ordinal_position: i32,
    /// Column comment.
    pub comment: Option<String>,
}

/// Raw constraint information from the database.
#[derive(Debug, Clone)]
pub struct ConstraintInfo {
    /// Constraint name.
    pub name: String,
    /// Constraint type (PRIMARY KEY, UNIQUE, FOREIGN KEY, CHECK).
    pub constraint_type: String,
    /// Table name.
    pub table_name: String,
    /// Columns in the constraint.
    pub columns: Vec<String>,
    /// Referenced table (for foreign keys).
    pub referenced_table: Option<String>,
    /// Referenced columns (for foreign keys).
    pub referenced_columns: Option<Vec<String>>,
    /// On delete action (for foreign keys).
    pub on_delete: Option<String>,
    /// On update action (for foreign keys).
    pub on_update: Option<String>,
}

/// Raw enum information from the database.
#[derive(Debug, Clone)]
pub struct EnumInfo {
    /// Enum name.
    pub name: String,
    /// Enum values.
    pub values: Vec<String>,
    /// Schema the enum belongs to.
    pub schema: String,
}

/// Raw index information from the database.
#[derive(Debug, Clone)]
pub struct IndexInfo {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table_name: String,
    /// Columns in the index.
    pub columns: Vec<String>,
    /// Whether the index is unique.
    pub is_unique: bool,
    /// Whether this is a primary key index.
    pub is_primary: bool,
    /// Index method (btree, hash, etc.).
    pub index_method: String,
}

/// Trait for database introspection.
#[async_trait::async_trait]
pub trait Introspector: Send + Sync {
    /// Get all tables in the database.
    async fn get_tables(&self, config: &IntrospectionConfig) -> MigrateResult<Vec<TableInfo>>;

    /// Get columns for a table.
    async fn get_columns(&self, table: &str, schema: &str) -> MigrateResult<Vec<ColumnInfo>>;

    /// Get constraints for a table.
    async fn get_constraints(
        &self,
        table: &str,
        schema: &str,
    ) -> MigrateResult<Vec<ConstraintInfo>>;

    /// Get indexes for a table.
    async fn get_indexes(&self, table: &str, schema: &str) -> MigrateResult<Vec<IndexInfo>>;

    /// Get all enums in the database.
    async fn get_enums(&self, schema: &str) -> MigrateResult<Vec<EnumInfo>>;
}

/// Build a Prax schema from introspection data.
pub struct SchemaBuilder {
    config: IntrospectionConfig,
    tables: Vec<TableInfo>,
    columns: HashMap<String, Vec<ColumnInfo>>,
    constraints: HashMap<String, Vec<ConstraintInfo>>,
    indexes: HashMap<String, Vec<IndexInfo>>,
    enums: Vec<EnumInfo>,
}

impl SchemaBuilder {
    /// Create a new schema builder.
    pub fn new(config: IntrospectionConfig) -> Self {
        Self {
            config,
            tables: Vec::new(),
            columns: HashMap::new(),
            constraints: HashMap::new(),
            indexes: HashMap::new(),
            enums: Vec::new(),
        }
    }

    /// Add table information.
    pub fn with_tables(mut self, tables: Vec<TableInfo>) -> Self {
        self.tables = tables;
        self
    }

    /// Add column information for a table.
    pub fn with_columns(mut self, table: &str, columns: Vec<ColumnInfo>) -> Self {
        self.columns.insert(table.to_string(), columns);
        self
    }

    /// Add constraint information for a table.
    pub fn with_constraints(mut self, table: &str, constraints: Vec<ConstraintInfo>) -> Self {
        self.constraints.insert(table.to_string(), constraints);
        self
    }

    /// Add index information for a table.
    pub fn with_indexes(mut self, table: &str, indexes: Vec<IndexInfo>) -> Self {
        self.indexes.insert(table.to_string(), indexes);
        self
    }

    /// Add enum information.
    pub fn with_enums(mut self, enums: Vec<EnumInfo>) -> Self {
        self.enums = enums;
        self
    }

    /// Build the schema from the collected information.
    pub fn build(self) -> MigrateResult<IntrospectionResult> {
        let mut schema = Schema::new();
        let mut skipped_tables = Vec::new();
        let mut warnings = Vec::new();

        // Add enums first (they may be referenced by columns)
        if self.config.include_enums {
            for enum_info in &self.enums {
                let prax_enum = self.build_enum(enum_info);
                schema.add_enum(prax_enum);
            }
        }

        // Build models from tables
        for table in &self.tables {
            if !self.config.should_include_table(&table.name) {
                skipped_tables.push(SkippedTable {
                    name: table.name.clone(),
                    reason: "Excluded by configuration".to_string(),
                });
                continue;
            }

            // Skip views if not configured
            if table.table_type == "VIEW" && !self.config.include_views {
                continue;
            }

            // Views get the distinct View AST rather than being flattened
            // into ordinary models (see build_view).
            if table.table_type == "VIEW" {
                match self.build_view(table) {
                    Ok(view) => {
                        schema.add_view(view);
                    }
                    Err(e) => {
                        warnings.push(format!("Failed to build view for '{}': {}", table.name, e));
                        skipped_tables.push(SkippedTable {
                            name: table.name.clone(),
                            reason: e.to_string(),
                        });
                    }
                }
                continue;
            }

            match self.build_model(table) {
                Ok(model) => {
                    schema.add_model(model);
                }
                Err(e) => {
                    warnings.push(format!("Failed to build model for '{}': {}", table.name, e));
                    skipped_tables.push(SkippedTable {
                        name: table.name.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }

        Ok(IntrospectionResult {
            schema,
            skipped_tables,
            warnings,
        })
    }

    /// Build an enum from database enum info.
    fn build_enum(&self, info: &EnumInfo) -> Enum {
        let span = Span::new(0, 0);
        let name = Ident::new(to_pascal_case(&info.name), span);
        let mut prax_enum = Enum::new(name, span);

        for value in &info.values {
            prax_enum.add_variant(EnumVariant::new(Ident::new(value.clone(), span), span));
        }

        prax_enum
    }

    /// Build a model from table info.
    fn build_model(&self, table: &TableInfo) -> MigrateResult<Model> {
        let span = Span::new(0, 0);
        let name = Ident::new(to_pascal_case(&table.name), span);
        let mut model = Model::new(name, span);

        // Always emit @@map with the real table name. The AST `table_name()`
        // returns the model name when no `@@map` is present and does *not*
        // lowercase it, so a table `users` reverse-engineered to model `Users`
        // would otherwise report its table as `Users`. Pinning `@@map` makes
        // the database table name authoritative — essential for diffing an
        // introspected schema (keyed by table name) without spurious churn.
        model.attributes.push(Attribute::new(
            Ident::new("map", span),
            vec![AttributeArg::positional(
                AttributeValue::String(table.name.clone()),
                span,
            )],
            span,
        ));

        // Get columns for this table
        let columns = self.columns.get(&table.name).cloned().unwrap_or_default();

        // Get constraints for this table
        let constraints = self
            .constraints
            .get(&table.name)
            .cloned()
            .unwrap_or_default();

        // Find primary key columns
        let pk_columns: Vec<&str> = constraints
            .iter()
            .filter(|c| c.constraint_type == "PRIMARY KEY")
            .flat_map(|c| c.columns.iter().map(|s| s.as_str()))
            .collect();

        // Find unique columns
        let unique_columns: Vec<&str> = constraints
            .iter()
            .filter(|c| c.constraint_type == "UNIQUE")
            .filter(|c| c.columns.len() == 1)
            .flat_map(|c| c.columns.iter().map(|s| s.as_str()))
            .collect();

        // Build fields from columns
        for column in &columns {
            // A column whose SQL type can't be represented in the schema AST
            // (e.g. a Postgres `tsvector` search column or a pgvector
            // `vector` embedding that lives only in the database, not the
            // `.prax`) must not sink the whole table: skipping the column
            // keeps the table present in the diff source, so it is not
            // mistaken for a brand-new table. The column is simply absent from
            // the reverse-engineered model; since it is not in the target
            // schema either, the differ leaves it alone.
            match self.build_field(column, &pk_columns, &unique_columns) {
                Ok(field) => model.add_field(field),
                Err(e) => {
                    eprintln!(
                        "Warning: skipping column '{}.{}' — {} (kept out of the diff source; \
                         the table itself is still introspected)",
                        table.name, column.name, e
                    );
                }
            }
        }

        // Synthesize relation fields from foreign key constraints.
        for fk in constraints.iter().filter(|c| {
            c.constraint_type == "FOREIGN KEY"
                && c.referenced_table.is_some()
                && c.referenced_columns.is_some()
        }) {
            let field = Self::build_relation_field(fk, &columns, &model);
            model.add_field(field);
        }

        // Emit @@index / @@unique / @unique from the collected indexes.
        // Primary-key indexes are already represented by @id on the pk fields.
        let indexes = self.indexes.get(&table.name).cloned().unwrap_or_default();
        for index in &indexes {
            if index.is_primary {
                continue;
            }

            // Index attributes reference *field* names (diff.rs maps them
            // back to column names via @map); skip columns that produced no
            // field (e.g. expression indexes) and drop empty indexes.
            let field_names: Vec<String> = index
                .columns
                .iter()
                .map(|c| field_name_for_column(c))
                .filter(|f| model.get_field(f).is_some())
                .collect();
            if field_names.is_empty() {
                continue;
            }

            // Single-column unique indexes map to field-level @unique,
            // matching how single-column UNIQUE constraints are emitted.
            if index.is_unique && field_names.len() == 1 {
                if let Some(field) = model.fields.get_mut(field_names[0].as_str())
                    && !field.has_attribute("unique")
                {
                    field
                        .attributes
                        .push(Attribute::simple(Ident::new("unique", span), span));
                }
                continue;
            }

            let attr_name = if index.is_unique { "unique" } else { "index" };
            model.attributes.push(Attribute::new(
                Ident::new(attr_name, span),
                vec![
                    AttributeArg::positional(
                        AttributeValue::FieldRefList(
                            field_names.iter().map(|f| f.as_str().into()).collect(),
                        ),
                        span,
                    ),
                    // Preserve the database index name; diff.rs reads `map`
                    // as the custom index name when diffing.
                    AttributeArg::named(
                        Ident::new("map", span),
                        AttributeValue::String(index.name.clone()),
                        span,
                    ),
                ],
                span,
            ));
        }

        Ok(model)
    }

    /// Build a view from table info.
    ///
    /// The `Introspector` trait does not capture view definitions, so no
    /// `@@sql` body can be emitted. That is safe to round-trip: the
    /// migration diff engine skips views without `@@sql` (see
    /// `diff.rs::view_to_diff`), so introspected views are documented in the
    /// schema but never (incorrectly) recreated as tables.
    fn build_view(&self, table: &TableInfo) -> MigrateResult<View> {
        let span = Span::new(0, 0);
        let name = Ident::new(to_pascal_case(&table.name), span);
        let mut view = View::new(name, span);

        // Add @@map attribute if the view name differs from the AST name
        // (same rule as models).
        let view_name = to_pascal_case(&table.name);
        if table.name != view_name && table.name != to_snake_case(&view_name) {
            view.attributes.push(Attribute::new(
                Ident::new("map", span),
                vec![AttributeArg::positional(
                    AttributeValue::String(table.name.clone()),
                    span,
                )],
                span,
            ));
        }

        // Views carry no constraints, so pk/unique sets are empty.
        let columns = self.columns.get(&table.name).cloned().unwrap_or_default();
        for column in &columns {
            let field = self.build_field(column, &[], &[])?;
            view.add_field(field);
        }

        Ok(view)
    }

    /// Build a relation field from a foreign key constraint.
    ///
    /// Emits a model-typed field carrying
    /// `@relation(fields: [fk_cols], references: [ref_cols])` on the child
    /// (FK-holding) model — the exact shape `diff.rs::extract_foreign_keys`
    /// reads back when diffing. The back-relation field on the parent model
    /// is intentionally not synthesized; the schema validator does not
    /// require it.
    fn build_relation_field(fk: &ConstraintInfo, columns: &[ColumnInfo], model: &Model) -> Field {
        let span = Span::new(0, 0);
        let referenced_table = fk.referenced_table.as_deref().unwrap_or_default();
        let referenced_columns = fk.referenced_columns.as_deref().unwrap_or_default();

        // Relation field name: for a single-column FK like `author_id`, use
        // the column prefix (`author`); otherwise fall back to the
        // camelCased referenced model name. Collisions (e.g. multiple FKs to
        // the same table) get a numeric suffix.
        let base = if fk.columns.len() == 1 {
            fk.columns[0]
                .strip_suffix("_id")
                .filter(|s| !s.is_empty())
                .map(String::from)
        } else {
            None
        }
        .unwrap_or_else(|| to_camel_case(&to_pascal_case(referenced_table)));
        let base = if is_valid_field_identifier(&base) {
            base
        } else {
            field_name_for_column(&base)
        };

        let mut field_name = base.clone();
        let mut suffix = 2;
        while model.get_field(&field_name).is_some() {
            field_name = format!("{base}_{suffix}");
            suffix += 1;
        }

        // The relation is optional when any FK column is nullable.
        let modifier = if fk
            .columns
            .iter()
            .any(|col| columns.iter().any(|c| &c.name == col && c.is_nullable))
        {
            TypeModifier::Optional
        } else {
            TypeModifier::Required
        };

        let attributes = vec![Attribute::new(
            Ident::new("relation", span),
            vec![
                AttributeArg::named(
                    Ident::new("fields", span),
                    AttributeValue::FieldRefList(
                        fk.columns
                            .iter()
                            .map(|c| field_name_for_column(c).into())
                            .collect(),
                    ),
                    span,
                ),
                AttributeArg::named(
                    Ident::new("references", span),
                    AttributeValue::FieldRefList(
                        referenced_columns
                            .iter()
                            .map(|c| field_name_for_column(c).into())
                            .collect(),
                    ),
                    span,
                ),
                // Preserve the database constraint name so the diff engine
                // (which keys foreign keys by constraint name) matches this
                // FK against the same relation in a target schema instead of
                // proposing a drop/add. Without this, diff.rs auto-derives
                // `fk_<table>_<cols>`, which will not match a real DB name.
                AttributeArg::named(
                    Ident::new("map", span),
                    AttributeValue::String(fk.name.clone()),
                    span,
                ),
            ],
            span,
        )];

        Field::new(
            Ident::new(field_name, span),
            FieldType::Model(to_pascal_case(referenced_table).into()),
            modifier,
            attributes,
            span,
        )
    }

    /// Build a field from column info.
    fn build_field(
        &self,
        column: &ColumnInfo,
        pk_columns: &[&str],
        unique_columns: &[&str],
    ) -> MigrateResult<Field> {
        let span = Span::new(0, 0);
        // Derive the Prax field name; oddly-named columns are sanitized and
        // the original name preserved via @map below.
        let field_name = field_name_for_column(&column.name);
        let needs_map = field_name != column.name;
        let name = Ident::new(field_name, span);

        // Map SQL type to Prax type
        let field_type = self.sql_type_to_prax(&column.udt_name, &column.data_type)?;

        // Determine modifier
        let modifier = if column.is_nullable {
            TypeModifier::Optional
        } else {
            TypeModifier::Required
        };

        let mut attributes = Vec::new();

        // Add @id if this is a primary key
        if pk_columns.contains(&column.name.as_str()) {
            attributes.push(Attribute::simple(Ident::new("id", span), span));

            // Check for auto-increment
            if let Some(default) = &column.column_default
                && (default.contains("nextval") || default.contains("SERIAL"))
            {
                attributes.push(Attribute::simple(Ident::new("auto", span), span));
            }
        }

        // Add @unique if this is a unique column
        if unique_columns.contains(&column.name.as_str()) {
            attributes.push(Attribute::simple(Ident::new("unique", span), span));
        }

        // Add @default if there's a default value (skip auto-increment defaults)
        if let Some(default) = &column.column_default
            && !default.contains("nextval")
            && let Some(value) = parse_default_value(default)
        {
            attributes.push(Attribute::new(
                Ident::new("default", span),
                vec![AttributeArg::positional(value, span)],
                span,
            ));
        }

        // Add @map when the column name isn't usable as the Prax field name
        // (not a valid snake_case identifier), preserving the real name.
        if needs_map {
            attributes.push(Attribute::new(
                Ident::new("map", span),
                vec![AttributeArg::positional(
                    AttributeValue::String(column.name.clone()),
                    span,
                )],
                span,
            ));
        }

        Ok(Field::new(name, field_type, modifier, attributes, span))
    }

    /// Convert SQL type to Prax field type.
    fn sql_type_to_prax(&self, udt_name: &str, data_type: &str) -> MigrateResult<FieldType> {
        // Check if this is a known enum
        let enum_names: Vec<&str> = self.enums.iter().map(|e| e.name.as_str()).collect();
        if enum_names.contains(&udt_name) {
            return Ok(FieldType::Enum(to_pascal_case(udt_name).into()));
        }

        let scalar = match udt_name {
            "int2" | "int4" | "integer" | "smallint" => ScalarType::Int,
            "int8" | "bigint" => ScalarType::BigInt,
            "float4" | "float8" | "real" | "double precision" => ScalarType::Float,
            "numeric" | "decimal" | "money" => ScalarType::Decimal,
            "text" | "varchar" | "char" | "character varying" | "character" | "bpchar" => {
                ScalarType::String
            }
            "bool" | "boolean" => ScalarType::Boolean,
            "timestamp"
            | "timestamptz"
            | "timestamp with time zone"
            | "timestamp without time zone" => ScalarType::DateTime,
            "date" => ScalarType::Date,
            "time" | "timetz" | "time with time zone" | "time without time zone" => {
                ScalarType::Time
            }
            "json" | "jsonb" => ScalarType::Json,
            "bytea" => ScalarType::Bytes,
            "uuid" => ScalarType::Uuid,
            _ => {
                // Try to match by data_type as fallback
                match data_type {
                    "integer" | "smallint" => ScalarType::Int,
                    "bigint" => ScalarType::BigInt,
                    "real" | "double precision" => ScalarType::Float,
                    "numeric" => ScalarType::Decimal,
                    "character varying" | "character" | "text" => ScalarType::String,
                    "boolean" => ScalarType::Boolean,
                    "timestamp with time zone" | "timestamp without time zone" => {
                        ScalarType::DateTime
                    }
                    "date" => ScalarType::Date,
                    "time with time zone" | "time without time zone" => ScalarType::Time,
                    "json" | "jsonb" => ScalarType::Json,
                    "bytea" => ScalarType::Bytes,
                    "uuid" => ScalarType::Uuid,
                    "ARRAY" => {
                        // Arrays are complex - for now, treat as Json
                        ScalarType::Json
                    }
                    "USER-DEFINED" => {
                        // This might be an enum we haven't seen
                        return Err(MigrationError::InvalidMigration(format!(
                            "Unknown user-defined type: {}",
                            udt_name
                        )));
                    }
                    _ => {
                        return Err(MigrationError::InvalidMigration(format!(
                            "Unknown SQL type: {} ({})",
                            udt_name, data_type
                        )));
                    }
                }
            }
        };

        Ok(FieldType::Scalar(scalar))
    }
}

/// Parse a default value expression into an AttributeValue.
fn parse_default_value(default: &str) -> Option<AttributeValue> {
    let trimmed = default.trim();

    // Handle booleans
    if trimmed == "true" || trimmed == "TRUE" {
        return Some(AttributeValue::Boolean(true));
    }
    if trimmed == "false" || trimmed == "FALSE" {
        return Some(AttributeValue::Boolean(false));
    }

    // Handle NULL
    if trimmed.to_uppercase() == "NULL" {
        return None;
    }

    // Handle integers
    if let Ok(int) = trimmed.parse::<i64>() {
        return Some(AttributeValue::Int(int));
    }

    // Handle floats
    if let Ok(float) = trimmed.parse::<f64>() {
        return Some(AttributeValue::Float(float));
    }

    // Handle strings (enclosed in quotes)
    if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        return Some(AttributeValue::String(inner.to_string()));
    }

    // Handle PostgreSQL type casts (e.g., 'value'::type)
    if let Some(pos) = trimmed.find("::") {
        return parse_default_value(&trimmed[..pos]);
    }

    // Handle function calls (e.g., now(), uuid_generate_v4())
    if trimmed.ends_with("()") || trimmed.contains('(') {
        let func_name = if let Some(paren_pos) = trimmed.find('(') {
            &trimmed[..paren_pos]
        } else {
            &trimmed[..trimmed.len() - 2]
        };
        return Some(AttributeValue::Function(
            func_name.to_string().into(),
            vec![],
        ));
    }

    // Bare SQL keyword defaults (no parens) — `CURRENT_TIMESTAMP`, `now`,
    // `CURRENT_DATE`, `CURRENT_TIME`. These render unquoted, and the target
    // schema expresses the same thing as `@default(now())` →
    // `CURRENT_TIMESTAMP`. Normalizing to the matching function form here is
    // what stops an unchanged timestamp default from churning
    // (`'CURRENT_TIMESTAMP'` string vs `CURRENT_TIMESTAMP` keyword).
    match trimmed.to_ascii_uppercase().as_str() {
        "CURRENT_TIMESTAMP" | "NOW" => {
            return Some(AttributeValue::Function("now".into(), vec![]));
        }
        "CURRENT_DATE" => {
            return Some(AttributeValue::Function("current_date".into(), vec![]));
        }
        "CURRENT_TIME" => {
            return Some(AttributeValue::Function("current_time".into(), vec![]));
        }
        _ => {}
    }

    // Unknown default - return as string
    Some(AttributeValue::String(trimmed.to_string()))
}

/// Convert snake_case to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Convert PascalCase to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert PascalCase to camelCase (first character lowercased).
fn to_camel_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().chain(chars).collect(),
    }
}

/// Whether `name` is a valid snake_case Prax field identifier
/// (`[a-z][a-z0-9_]*`). The grammar requires an ASCII letter start, so a
/// leading digit or underscore is not parseable.
fn is_valid_field_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Derive the Prax field name for a database column.
///
/// Valid snake_case column names are used as-is. Anything else is
/// sanitized — camelCase boundaries split, lowercased, runs of invalid
/// characters collapsed to a single `_`, and a `col_` prefix when the
/// result would start with a digit — and the original column name is
/// preserved on the field via `@map`.
fn field_name_for_column(column_name: &str) -> String {
    if is_valid_field_identifier(column_name) {
        return column_name.to_string();
    }

    // Split camelCase boundaries, then lowercase.
    let mut snake = String::with_capacity(column_name.len() + 4);
    let mut prev_lower_or_digit = false;
    for ch in column_name.chars() {
        if ch.is_ascii_uppercase() && prev_lower_or_digit {
            snake.push('_');
        }
        snake.push(ch.to_ascii_lowercase());
        prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }

    // Collapse any remaining invalid characters to single underscores.
    let mut out = String::with_capacity(snake.len());
    for ch in snake.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }

    if out.is_empty() {
        return "column".to_string();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return format!("col_{out}");
    }
    out
}

/// SQL queries for PostgreSQL introspection.
pub mod postgres_queries {
    /// Query to get all tables and views.
    pub const TABLES: &str = r#"
        SELECT
            table_name,
            table_schema,
            table_type
        FROM information_schema.tables
        WHERE table_schema = $1
        ORDER BY table_name
    "#;

    /// Query to get columns for a table.
    pub const COLUMNS: &str = r#"
        SELECT
            column_name,
            data_type,
            udt_name,
            character_maximum_length,
            numeric_precision,
            is_nullable = 'YES' as is_nullable,
            column_default,
            ordinal_position
        FROM information_schema.columns
        WHERE table_schema = $1 AND table_name = $2
        ORDER BY ordinal_position
    "#;

    /// Query to get constraints.
    pub const CONSTRAINTS: &str = r#"
        SELECT
            tc.constraint_name,
            tc.constraint_type,
            tc.table_name,
            kcu.column_name,
            ccu.table_name AS referenced_table,
            ccu.column_name AS referenced_column,
            rc.delete_rule,
            rc.update_rule
        FROM information_schema.table_constraints tc
        LEFT JOIN information_schema.key_column_usage kcu
            ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
        LEFT JOIN information_schema.constraint_column_usage ccu
            ON tc.constraint_name = ccu.constraint_name
            AND tc.table_schema = ccu.table_schema
            AND tc.constraint_type = 'FOREIGN KEY'
        LEFT JOIN information_schema.referential_constraints rc
            ON tc.constraint_name = rc.constraint_name
            AND tc.table_schema = rc.constraint_schema
        WHERE tc.table_schema = $1 AND tc.table_name = $2
        ORDER BY tc.constraint_name, kcu.ordinal_position
    "#;

    /// Query to get indexes.
    pub const INDEXES: &str = r#"
        SELECT
            i.relname AS index_name,
            t.relname AS table_name,
            array_agg(a.attname ORDER BY array_position(ix.indkey, a.attnum)) AS columns,
            ix.indisunique AS is_unique,
            ix.indisprimary AS is_primary,
            am.amname AS index_method
        FROM pg_index ix
        JOIN pg_class i ON ix.indexrelid = i.oid
        JOIN pg_class t ON ix.indrelid = t.oid
        JOIN pg_namespace n ON t.relnamespace = n.oid
        JOIN pg_am am ON i.relam = am.oid
        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey)
        WHERE n.nspname = $1 AND t.relname = $2
        GROUP BY i.relname, t.relname, ix.indisunique, ix.indisprimary, am.amname
    "#;

    /// Query to get enums.
    pub const ENUMS: &str = r#"
        SELECT
            t.typname AS enum_name,
            n.nspname AS schema_name,
            array_agg(e.enumlabel ORDER BY e.enumsortorder) AS enum_values
        FROM pg_type t
        JOIN pg_namespace n ON t.typnamespace = n.oid
        JOIN pg_enum e ON t.oid = e.enumtypid
        WHERE n.nspname = $1
        GROUP BY t.typname, n.nspname
    "#;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("user"), "User");
        assert_eq!(to_pascal_case("user_profile"), "UserProfile");
        assert_eq!(
            to_pascal_case("user_profile_settings"),
            "UserProfileSettings"
        );
        assert_eq!(to_pascal_case("_user_"), "User");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("User"), "user");
        assert_eq!(to_snake_case("UserProfile"), "user_profile");
        assert_eq!(to_snake_case("HTTPResponse"), "h_t_t_p_response");
    }

    #[test]
    fn test_parse_default_value_boolean() {
        assert!(matches!(
            parse_default_value("true"),
            Some(AttributeValue::Boolean(true))
        ));
        assert!(matches!(
            parse_default_value("false"),
            Some(AttributeValue::Boolean(false))
        ));
    }

    #[test]
    fn test_parse_default_value_int() {
        assert!(matches!(
            parse_default_value("42"),
            Some(AttributeValue::Int(42))
        ));
        assert!(matches!(
            parse_default_value("-5"),
            Some(AttributeValue::Int(-5))
        ));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_parse_default_value_float() {
        if let Some(AttributeValue::Float(f)) = parse_default_value("3.14") {
            assert!((f - 3.14).abs() < 0.001);
        } else {
            panic!("Expected Float");
        }
    }

    #[test]
    fn test_parse_default_value_string() {
        if let Some(AttributeValue::String(s)) = parse_default_value("'hello'") {
            assert_eq!(s.as_str(), "hello");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_parse_default_value_function() {
        if let Some(AttributeValue::Function(name, args)) = parse_default_value("now()") {
            assert_eq!(name.as_str(), "now");
            assert!(args.is_empty());
        } else {
            panic!("Expected Function");
        }
    }

    #[test]
    fn test_parse_default_value_with_cast() {
        if let Some(AttributeValue::String(s)) = parse_default_value("'active'::status_type") {
            assert_eq!(s.as_str(), "active");
        } else {
            panic!("Expected String");
        }
    }

    #[test]
    fn test_config_should_include_table() {
        let config = IntrospectionConfig::default();
        assert!(config.should_include_table("users"));
        assert!(!config.should_include_table("_prax_migrations"));
    }

    #[test]
    fn test_config_include_specific_tables() {
        let config = IntrospectionConfig::new().include_tables(vec!["users".to_string()]);
        assert!(config.should_include_table("users"));
        assert!(!config.should_include_table("posts"));
    }

    #[test]
    fn test_sql_type_mapping() {
        let builder = SchemaBuilder::new(IntrospectionConfig::default());

        let ft = builder.sql_type_to_prax("int4", "integer").unwrap();
        assert!(matches!(ft, FieldType::Scalar(ScalarType::Int)));

        let ft = builder.sql_type_to_prax("text", "text").unwrap();
        assert!(matches!(ft, FieldType::Scalar(ScalarType::String)));

        let ft = builder.sql_type_to_prax("bool", "boolean").unwrap();
        assert!(matches!(ft, FieldType::Scalar(ScalarType::Boolean)));

        let ft = builder
            .sql_type_to_prax("timestamptz", "timestamp with time zone")
            .unwrap();
        assert!(matches!(ft, FieldType::Scalar(ScalarType::DateTime)));

        let ft = builder.sql_type_to_prax("uuid", "uuid").unwrap();
        assert!(matches!(ft, FieldType::Scalar(ScalarType::Uuid)));
    }

    fn table(name: &str, table_type: &str) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            schema: "public".to_string(),
            table_type: table_type.to_string(),
            comment: None,
        }
    }

    fn column(name: &str, udt_name: &str, data_type: &str, is_nullable: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            udt_name: udt_name.to_string(),
            character_maximum_length: None,
            numeric_precision: None,
            is_nullable,
            column_default: None,
            ordinal_position: 0,
            comment: None,
        }
    }

    fn primary_key(table_name: &str, columns: &[&str]) -> ConstraintInfo {
        ConstraintInfo {
            name: format!("{table_name}_pkey"),
            constraint_type: "PRIMARY KEY".to_string(),
            table_name: table_name.to_string(),
            columns: columns.iter().map(|c| c.to_string()).collect(),
            referenced_table: None,
            referenced_columns: None,
            on_delete: None,
            on_update: None,
        }
    }

    #[test]
    fn test_field_name_for_column_keeps_valid_names() {
        assert_eq!(field_name_for_column("id"), "id");
        assert_eq!(field_name_for_column("user_name"), "user_name");
        assert_eq!(field_name_for_column("created_at2"), "created_at2");
    }

    #[test]
    fn test_field_name_for_column_sanitizes_invalid_names() {
        assert_eq!(field_name_for_column("userName"), "user_name");
        assert_eq!(field_name_for_column("UserName"), "user_name");
        assert_eq!(field_name_for_column("user-name"), "user_name");
        assert_eq!(field_name_for_column("user name"), "user_name");
        assert_eq!(field_name_for_column("2fa_code"), "col_2fa_code");
        assert_eq!(field_name_for_column("ID"), "id");
        assert_eq!(field_name_for_column("_leading"), "leading");
    }

    #[test]
    fn test_build_model_emits_index_and_relation_attributes() {
        let builder = SchemaBuilder::new(IntrospectionConfig::default())
            .with_tables(vec![
                table("users", "BASE TABLE"),
                table("posts", "BASE TABLE"),
            ])
            .with_columns("users", vec![column("id", "int8", "bigint", false)])
            .with_columns(
                "posts",
                vec![
                    column("id", "int8", "bigint", false),
                    column("author_id", "int8", "bigint", false),
                    column("title", "text", "text", false),
                ],
            )
            .with_constraints("users", vec![primary_key("users", &["id"])])
            .with_constraints(
                "posts",
                vec![
                    primary_key("posts", &["id"]),
                    ConstraintInfo {
                        name: "posts_author_id_fkey".to_string(),
                        constraint_type: "FOREIGN KEY".to_string(),
                        table_name: "posts".to_string(),
                        columns: vec!["author_id".to_string()],
                        referenced_table: Some("users".to_string()),
                        referenced_columns: Some(vec!["id".to_string()]),
                        on_delete: None,
                        on_update: None,
                    },
                ],
            )
            .with_indexes(
                "posts",
                vec![
                    IndexInfo {
                        name: "posts_pkey".to_string(),
                        table_name: "posts".to_string(),
                        columns: vec!["id".to_string()],
                        is_unique: true,
                        is_primary: true,
                        index_method: "btree".to_string(),
                    },
                    IndexInfo {
                        name: "idx_posts_title".to_string(),
                        table_name: "posts".to_string(),
                        columns: vec!["title".to_string()],
                        is_unique: false,
                        is_primary: false,
                        index_method: "btree".to_string(),
                    },
                    IndexInfo {
                        name: "uq_posts_author_title".to_string(),
                        table_name: "posts".to_string(),
                        columns: vec!["author_id".to_string(), "title".to_string()],
                        is_unique: true,
                        is_primary: false,
                        index_method: "btree".to_string(),
                    },
                ],
            );

        let result = builder.build().unwrap();
        let post = result.schema.get_model("Posts").expect("Posts model built");

        // @@index([title], map: "idx_posts_title") is emitted; the primary
        // key index is not re-emitted.
        let index_attr = post.get_attribute("index").expect("@@index emitted");
        match index_attr.first_arg() {
            Some(AttributeValue::FieldRefList(cols)) => {
                assert_eq!(cols.as_slice(), ["title"]);
            }
            other => panic!("expected FieldRefList, got {other:?}"),
        }
        assert_eq!(
            index_attr.get_arg("map").and_then(|v| v.as_string()),
            Some("idx_posts_title")
        );

        // The multi-column unique index becomes @@unique([author_id, title]).
        let unique_attr = post.get_attribute("unique").expect("@@unique emitted");
        match unique_attr.first_arg() {
            Some(AttributeValue::FieldRefList(cols)) => {
                assert_eq!(cols.as_slice(), ["author_id", "title"]);
            }
            other => panic!("expected FieldRefList, got {other:?}"),
        }

        // The FK becomes a relation field — verified through the same
        // attribute extraction diff.rs uses when reading @relation back.
        let author = post.get_field("author").expect("relation field emitted");
        assert!(matches!(&author.field_type, FieldType::Model(m) if m.as_str() == "Users"));
        assert_eq!(author.modifier, TypeModifier::Required);
        let rel = author
            .extract_attributes()
            .relation
            .expect("@relation attribute present");
        assert_eq!(rel.fields, ["author_id"]);
        assert_eq!(rel.references, ["id"]);
    }

    #[test]
    fn test_build_model_marks_nullable_fk_relation_optional() {
        let builder = SchemaBuilder::new(IntrospectionConfig::default())
            .with_tables(vec![
                table("users", "BASE TABLE"),
                table("posts", "BASE TABLE"),
            ])
            .with_columns("users", vec![column("id", "int8", "bigint", false)])
            .with_columns(
                "posts",
                vec![
                    column("id", "int8", "bigint", false),
                    column("author_id", "int8", "bigint", true),
                ],
            )
            .with_constraints("users", vec![primary_key("users", &["id"])])
            .with_constraints(
                "posts",
                vec![
                    primary_key("posts", &["id"]),
                    ConstraintInfo {
                        name: "posts_author_id_fkey".to_string(),
                        constraint_type: "FOREIGN KEY".to_string(),
                        table_name: "posts".to_string(),
                        columns: vec!["author_id".to_string()],
                        referenced_table: Some("users".to_string()),
                        referenced_columns: Some(vec!["id".to_string()]),
                        on_delete: None,
                        on_update: None,
                    },
                ],
            );

        let result = builder.build().unwrap();
        let post = result.schema.get_model("Posts").expect("Posts model built");
        let author = post.get_field("author").expect("relation field emitted");
        assert_eq!(author.modifier, TypeModifier::Optional);
    }

    #[test]
    fn test_build_model_emits_map_for_invalid_column_names() {
        let builder = SchemaBuilder::new(IntrospectionConfig::default())
            .with_tables(vec![table("users", "BASE TABLE")])
            .with_columns(
                "users",
                vec![
                    column("id", "int8", "bigint", false),
                    column("userName", "text", "text", false),
                ],
            )
            .with_constraints("users", vec![primary_key("users", &["id"])]);

        let result = builder.build().unwrap();
        let user = result.schema.get_model("Users").expect("Users model built");

        let field = user.get_field("user_name").expect("sanitized field name");
        assert_eq!(
            field
                .get_attribute("map")
                .and_then(|a| a.first_string_arg()),
            Some("userName"),
            "@map preserves the original column name"
        );

        // Valid column names get no @map.
        let id = user.get_field("id").expect("id field");
        assert!(id.get_attribute("map").is_none());
    }

    #[test]
    fn test_build_emits_views_as_view_ast() {
        let builder = SchemaBuilder::new(IntrospectionConfig::default())
            .with_tables(vec![
                table("users", "BASE TABLE"),
                table("user_stats", "VIEW"),
            ])
            .with_columns("users", vec![column("id", "int8", "bigint", false)])
            .with_columns(
                "user_stats",
                vec![
                    column("user_id", "int8", "bigint", false),
                    column("post_count", "int8", "bigint", false),
                ],
            );

        let result = builder.build().unwrap();

        // Views land in schema.views, not schema.models — see build_view.
        assert!(result.schema.get_model("UserStats").is_none());
        let view = result
            .schema
            .views
            .get("UserStats")
            .expect("view emitted as View AST");
        assert_eq!(view.fields.len(), 2);
    }
}
