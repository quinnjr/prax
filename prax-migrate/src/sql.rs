//! SQL generation for migrations.

use crate::diff::{
    EnumAlterDiff, EnumDiff, ExtensionDiff, FieldAlterDiff, FieldDiff, ForeignKeyDiff, IndexDiff,
    ModelAlterDiff, ModelDiff, SchemaDiff, ViewDiff,
};

/// SQL generator for PostgreSQL.
pub struct PostgresSqlGenerator;

impl PostgresSqlGenerator {
    /// Generate SQL for a schema diff.
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // Create extensions first (they provide types used by tables)
        for ext in &diff.create_extensions {
            up.push(self.create_extension(ext));
            down.push(self.drop_extension(&ext.name));
        }

        // Drop extensions (in reverse order)
        for name in &diff.drop_extensions {
            up.push(self.drop_extension(name));
            // Can't easily recreate dropped extensions without knowing schema/version
        }

        // Create enums (they might be used in tables)
        for enum_diff in &diff.create_enums {
            up.push(self.create_enum(enum_diff));
            down.push(self.drop_enum(&enum_diff.name));
        }

        // Drop enums (in reverse order)
        for name in &diff.drop_enums {
            up.push(self.drop_enum(name));
            // Can't easily recreate dropped enums without knowing values
        }

        // Alter enums
        for alter in &diff.alter_enums {
            up.extend(self.alter_enum(alter));
            // Reversing enum alterations is complex
        }

        // Create models
        for model in &diff.create_models {
            up.push(self.create_table(model));
            down.push(self.drop_table(&model.table_name));
        }

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
            // Can't easily recreate dropped tables
        }

        // Alter models
        for alter in &diff.alter_models {
            // Warn about dropped columns
            for field_name in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field_name, alter.table_name
                ));
            }

            // Warn about column type changes
            for field in &alter.alter_fields {
                if let Some(_new_type) = &field.new_type {
                    if field.old_type.is_some() {
                        warnings.push(format!(
                            "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                            field.name, alter.table_name
                        ));
                    }
                }
            }

            up.extend(self.alter_table(alter));
            // Reverse alterations could be generated but complex
        }

        // Create indexes
        for index in &diff.create_indexes {
            up.push(self.create_index(index));
            down.push(self.drop_index(&index.name, &index.table_name));
        }

        // Drop indexes
        for index in &diff.drop_indexes {
            up.push(self.drop_index(&index.name, &index.table_name));
        }

        // Create views (after tables they depend on)
        for view in &diff.create_views {
            up.push(self.create_view(view));
            down.push(self.drop_view(&view.view_name, view.is_materialized));
        }

        // Drop views
        for name in &diff.drop_views {
            // We don't know if it was materialized, so try both
            up.push(self.drop_view(name, false));
        }

        // Alter views (drop and recreate)
        for view in &diff.alter_views {
            // Drop the old view first
            up.push(self.drop_view(&view.view_name, view.is_materialized));
            // Then create the new one
            up.push(self.create_view(view));
        }

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }

    /// Generate CREATE EXTENSION statement.
    fn create_extension(&self, ext: &ExtensionDiff) -> String {
        let mut sql = format!("CREATE EXTENSION IF NOT EXISTS \"{}\"", ext.name);
        if let Some(schema) = &ext.schema {
            sql.push_str(&format!(" SCHEMA \"{}\"", schema));
        }
        if let Some(version) = &ext.version {
            sql.push_str(&format!(" VERSION '{}'", version));
        }
        sql.push(';');
        sql
    }

    /// Generate DROP EXTENSION statement.
    fn drop_extension(&self, name: &str) -> String {
        format!("DROP EXTENSION IF EXISTS \"{}\" CASCADE;", name)
    }

    /// Generate CREATE TYPE for enum.
    fn create_enum(&self, enum_diff: &EnumDiff) -> String {
        let values: Vec<String> = enum_diff
            .values
            .iter()
            .map(|v| format!("'{}'", v))
            .collect();
        format!(
            "CREATE TYPE \"{}\" AS ENUM ({});",
            enum_diff.name,
            values.join(", ")
        )
    }

    /// Generate DROP TYPE.
    fn drop_enum(&self, name: &str) -> String {
        format!("DROP TYPE IF EXISTS \"{}\";", name)
    }

    /// Generate ALTER TYPE statements.
    fn alter_enum(&self, alter: &EnumAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        for value in &alter.add_values {
            stmts.push(format!(
                "ALTER TYPE \"{}\" ADD VALUE IF NOT EXISTS '{}';",
                alter.name, value
            ));
        }

        // Note: PostgreSQL doesn't support removing enum values directly
        // This would require recreating the type

        stmts
    }

    /// Generate CREATE TABLE statement.
    fn create_table(&self, model: &ModelDiff) -> String {
        let mut columns = Vec::new();

        for field in &model.fields {
            columns.push(self.column_definition(field));
        }

        // Add primary key constraint
        if !model.primary_key.is_empty() {
            let pk_cols: Vec<String> = model
                .primary_key
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect();
            columns.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        // Add unique constraints
        for uc in &model.unique_constraints {
            let cols: Vec<String> = uc.columns.iter().map(|c| format!("\"{}\"", c)).collect();
            let constraint = if let Some(name) = &uc.name {
                format!("CONSTRAINT \"{}\" UNIQUE ({})", name, cols.join(", "))
            } else {
                format!("UNIQUE ({})", cols.join(", "))
            };
            columns.push(constraint);
        }

        // Add foreign key constraints
        for fk in &model.foreign_keys {
            columns.push(self.foreign_key_constraint(fk));
        }

        format!(
            "CREATE TABLE \"{}\" (\n    {}\n);",
            model.table_name,
            columns.join(",\n    ")
        )
    }

    /// Generate a FOREIGN KEY constraint clause.
    fn foreign_key_constraint(&self, fk: &ForeignKeyDiff) -> String {
        let cols: Vec<String> = fk.columns.iter().map(|c| format!("\"{}\"", c)).collect();
        let ref_cols: Vec<String> = fk
            .referenced_columns
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect();

        let mut clause = format!(
            "CONSTRAINT \"{}\" FOREIGN KEY ({}) REFERENCES \"{}\" ({})",
            fk.constraint_name,
            cols.join(", "),
            fk.referenced_table,
            ref_cols.join(", ")
        );

        if let Some(action) = &fk.on_delete {
            clause.push_str(&format!(" ON DELETE {}", action));
        }
        if let Some(action) = &fk.on_update {
            clause.push_str(&format!(" ON UPDATE {}", action));
        }

        clause
    }

    /// Generate column definition.
    fn column_definition(&self, field: &FieldDiff) -> String {
        let mut parts = vec![format!("\"{}\"", field.column_name), field.sql_type.clone()];

        if field.is_auto_increment {
            // Replace type with SERIAL variants
            if field.sql_type == "INTEGER" {
                parts[1] = "SERIAL".to_string();
            } else if field.sql_type == "BIGINT" {
                parts[1] = "BIGSERIAL".to_string();
            }
        }

        if !field.nullable && !field.is_primary_key {
            parts.push("NOT NULL".to_string());
        }

        if field.is_unique && !field.is_primary_key {
            parts.push("UNIQUE".to_string());
        }

        if let Some(default) = &field.default {
            parts.push(format!("DEFAULT {}", default));
        }

        parts.join(" ")
    }

    /// Generate DROP TABLE statement.
    fn drop_table(&self, name: &str) -> String {
        format!("DROP TABLE IF EXISTS \"{}\" CASCADE;", name)
    }

    /// Generate ALTER TABLE statements.
    fn alter_table(&self, alter: &ModelAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        // Add columns
        for field in &alter.add_fields {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ADD COLUMN {};",
                alter.table_name,
                self.column_definition(field)
            ));
        }

        // Drop columns
        for name in &alter.drop_fields {
            stmts.push(format!(
                "ALTER TABLE \"{}\" DROP COLUMN IF EXISTS \"{}\";",
                alter.table_name, name
            ));
        }

        // Alter columns
        for field in &alter.alter_fields {
            stmts.extend(self.alter_column(&alter.table_name, field));
        }

        // Add indexes
        for index in &alter.add_indexes {
            stmts.push(self.create_index(index));
        }

        // Drop indexes
        for name in &alter.drop_indexes {
            stmts.push(format!("DROP INDEX IF EXISTS \"{}\";", name));
        }

        // Drop foreign keys
        for name in &alter.drop_foreign_keys {
            stmts.push(format!(
                "ALTER TABLE \"{}\" DROP CONSTRAINT IF EXISTS \"{}\";",
                alter.table_name, name
            ));
        }

        // Add foreign keys
        for fk in &alter.add_foreign_keys {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ADD {};",
                alter.table_name,
                self.foreign_key_constraint(fk)
            ));
        }

        stmts
    }

    /// Generate ALTER COLUMN statements.
    fn alter_column(&self, table: &str, field: &FieldAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        if let Some(new_type) = &field.new_type {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" TYPE {} USING \"{}\"::{};",
                table, field.column_name, new_type, field.column_name, new_type
            ));
        }

        if let Some(new_nullable) = field.new_nullable {
            if new_nullable {
                stmts.push(format!(
                    "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" DROP NOT NULL;",
                    table, field.column_name
                ));
            } else {
                stmts.push(format!(
                    "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" SET NOT NULL;",
                    table, field.column_name
                ));
            }
        }

        if let Some(new_default) = &field.new_default {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" SET DEFAULT {};",
                table, field.column_name, new_default
            ));
        }

        stmts
    }

    /// Generate CREATE INDEX statement.
    fn create_index(&self, index: &IndexDiff) -> String {
        let unique = if index.unique { "UNIQUE " } else { "" };

        // Handle vector indexes (HNSW, IVFFlat)
        if index.is_vector_index() {
            return self.create_vector_index(index);
        }

        // Standard index with optional type
        let using_clause = match &index.index_type {
            Some(idx_type) => format!(" USING {}", idx_type.as_sql()),
            None => String::new(),
        };

        let cols: Vec<String> = index.columns.iter().map(|c| format!("\"{}\"", c)).collect();
        format!(
            "CREATE {}INDEX \"{}\" ON \"{}\"{}({});",
            unique,
            index.name,
            index.table_name,
            using_clause,
            cols.join(", ")
        )
    }

    /// Generate CREATE INDEX for vector indexes (HNSW/IVFFlat).
    fn create_vector_index(&self, index: &IndexDiff) -> String {
        let index_type = index.index_type.as_ref().unwrap();
        let ops_class = index
            .vector_ops
            .as_ref()
            .map(|o| o.as_ops_class())
            .unwrap_or("vector_cosine_ops");

        // Build column expression with operator class
        let col_expr = if index.columns.len() == 1 {
            format!("\"{}\" {}", index.columns[0], ops_class)
        } else {
            // Multi-column vector index (rare but possible)
            index
                .columns
                .iter()
                .map(|c| format!("\"{}\" {}", c, ops_class))
                .collect::<Vec<_>>()
                .join(", ")
        };

        // Build WITH clause for index parameters
        let with_clause = match index_type {
            prax_schema::ast::IndexType::Hnsw => {
                let mut params = Vec::new();
                if let Some(m) = index.hnsw_m {
                    params.push(format!("m = {}", m));
                }
                if let Some(ef) = index.hnsw_ef_construction {
                    params.push(format!("ef_construction = {}", ef));
                }
                if params.is_empty() {
                    String::new()
                } else {
                    format!(" WITH ({})", params.join(", "))
                }
            }
            prax_schema::ast::IndexType::IvfFlat => {
                if let Some(lists) = index.ivfflat_lists {
                    format!(" WITH (lists = {})", lists)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };

        format!(
            "CREATE INDEX \"{}\" ON \"{}\" USING {} ({}){};",
            index.name,
            index.table_name,
            index_type.as_sql(),
            col_expr,
            with_clause
        )
    }

    /// Generate DROP INDEX statement.
    fn drop_index(&self, name: &str, _table: &str) -> String {
        format!("DROP INDEX IF EXISTS \"{}\";", name)
    }

    /// Generate CREATE VIEW statement.
    fn create_view(&self, view: &ViewDiff) -> String {
        if view.is_materialized {
            format!(
                "CREATE MATERIALIZED VIEW \"{}\" AS\n{};",
                view.view_name, view.sql_query
            )
        } else {
            format!(
                "CREATE OR REPLACE VIEW \"{}\" AS\n{};",
                view.view_name, view.sql_query
            )
        }
    }

    /// Generate DROP VIEW statement.
    fn drop_view(&self, name: &str, is_materialized: bool) -> String {
        if is_materialized {
            format!("DROP MATERIALIZED VIEW IF EXISTS \"{}\" CASCADE;", name)
        } else {
            format!("DROP VIEW IF EXISTS \"{}\" CASCADE;", name)
        }
    }

    /// Generate REFRESH MATERIALIZED VIEW statement.
    #[allow(dead_code)]
    fn refresh_materialized_view(&self, name: &str, concurrently: bool) -> String {
        if concurrently {
            format!("REFRESH MATERIALIZED VIEW CONCURRENTLY \"{}\";", name)
        } else {
            format!("REFRESH MATERIALIZED VIEW \"{}\";", name)
        }
    }
}

/// Generated SQL for a migration.
#[derive(Debug, Clone)]
pub struct MigrationSql {
    /// SQL to apply the migration.
    pub up: String,
    /// SQL to rollback the migration.
    pub down: String,
    /// Warnings about data loss or irreversible operations.
    pub warnings: Vec<String>,
}

impl MigrationSql {
    /// Check if the migration is empty.
    pub fn is_empty(&self) -> bool {
        self.up.trim().is_empty()
    }
}

/// SQL generator for MySQL.
pub struct MySqlGenerator;

impl MySqlGenerator {
    /// Generate SQL for a schema diff.
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // Create enums (MySQL uses ENUM type in column definitions)
        // Enums in MySQL are defined per-column, not as separate types

        // Create models
        for model in &diff.create_models {
            up.push(self.create_table(model));
            down.push(self.drop_table(&model.table_name));
        }

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models
        for alter in &diff.alter_models {
            // Warn about dropped columns
            for field_name in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field_name, alter.table_name
                ));
            }

            // Warn about column type changes
            for field in &alter.alter_fields {
                if let Some(_new_type) = &field.new_type {
                    if field.old_type.is_some() {
                        warnings.push(format!(
                            "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                            field.name, alter.table_name
                        ));
                    }
                }
            }

            up.extend(self.alter_table(alter));
        }

        // Create indexes
        for index in &diff.create_indexes {
            up.push(self.create_index(index));
            down.push(self.drop_index(&index.name, &index.table_name));
        }

        // Drop indexes
        for index in &diff.drop_indexes {
            up.push(self.drop_index(&index.name, &index.table_name));
        }

        // Create views (after tables they depend on)
        for view in &diff.create_views {
            up.push(self.create_view(view));
            down.push(self.drop_view(&view.view_name));
        }

        // Drop views
        for name in &diff.drop_views {
            up.push(self.drop_view(name));
        }

        // Alter views (drop and recreate)
        for view in &diff.alter_views {
            up.push(self.drop_view(&view.view_name));
            up.push(self.create_view(view));
        }

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }

    /// Generate CREATE TABLE statement.
    fn create_table(&self, model: &ModelDiff) -> String {
        let mut columns = Vec::new();

        for field in &model.fields {
            columns.push(self.column_definition(field));
        }

        // Add primary key constraint
        if !model.primary_key.is_empty() {
            let pk_cols: Vec<String> = model
                .primary_key
                .iter()
                .map(|c| format!("`{}`", c))
                .collect();
            columns.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        // Add unique constraints
        for uc in &model.unique_constraints {
            let cols: Vec<String> = uc.columns.iter().map(|c| format!("`{}`", c)).collect();
            let constraint = if let Some(name) = &uc.name {
                format!("CONSTRAINT `{}` UNIQUE ({})", name, cols.join(", "))
            } else {
                format!("UNIQUE ({})", cols.join(", "))
            };
            columns.push(constraint);
        }

        // Add foreign key constraints
        for fk in &model.foreign_keys {
            let cols: Vec<String> = fk.columns.iter().map(|c| format!("`{}`", c)).collect();
            let ref_cols: Vec<String> = fk
                .referenced_columns
                .iter()
                .map(|c| format!("`{}`", c))
                .collect();
            let mut clause = format!(
                "CONSTRAINT `{}` FOREIGN KEY ({}) REFERENCES `{}` ({})",
                fk.constraint_name,
                cols.join(", "),
                fk.referenced_table,
                ref_cols.join(", ")
            );
            if let Some(action) = &fk.on_delete {
                clause.push_str(&format!(" ON DELETE {}", action));
            }
            if let Some(action) = &fk.on_update {
                clause.push_str(&format!(" ON UPDATE {}", action));
            }
            columns.push(clause);
        }

        format!(
            "CREATE TABLE `{}` (\n    {}\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;",
            model.table_name,
            columns.join(",\n    ")
        )
    }

    /// Generate column definition for MySQL.
    fn column_definition(&self, field: &FieldDiff) -> String {
        let mut parts = vec![format!("`{}`", field.column_name)];

        // MySQL type mapping
        let sql_type = match field.sql_type.as_str() {
            "INTEGER" if field.is_auto_increment => "INT AUTO_INCREMENT".to_string(),
            "INTEGER" => "INT".to_string(),
            "BIGINT" if field.is_auto_increment => "BIGINT AUTO_INCREMENT".to_string(),
            "TEXT" => "VARCHAR(255)".to_string(), // Default length for VARCHAR
            "DOUBLE PRECISION" => "DOUBLE".to_string(),
            "TIMESTAMP WITH TIME ZONE" => "DATETIME".to_string(),
            "BOOLEAN" => "TINYINT(1)".to_string(),
            "BYTEA" => "BLOB".to_string(),
            "JSONB" | "JSON" => "JSON".to_string(),
            other => other.to_string(),
        };
        parts.push(sql_type);

        if !field.nullable && !field.is_primary_key {
            parts.push("NOT NULL".to_string());
        }

        if field.is_unique && !field.is_primary_key {
            parts.push("UNIQUE".to_string());
        }

        if let Some(default) = &field.default {
            parts.push(format!("DEFAULT {}", default));
        }

        parts.join(" ")
    }

    /// Generate DROP TABLE statement.
    fn drop_table(&self, name: &str) -> String {
        format!("DROP TABLE IF EXISTS `{}`;", name)
    }

    /// Generate ALTER TABLE statements.
    fn alter_table(&self, alter: &ModelAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        // Add columns
        for field in &alter.add_fields {
            stmts.push(format!(
                "ALTER TABLE `{}` ADD COLUMN {};",
                alter.table_name,
                self.column_definition(field)
            ));
        }

        // Drop columns
        for name in &alter.drop_fields {
            stmts.push(format!(
                "ALTER TABLE `{}` DROP COLUMN `{}`;",
                alter.table_name, name
            ));
        }

        // Alter columns
        for field in &alter.alter_fields {
            stmts.extend(self.alter_column(&alter.table_name, field));
        }

        // Drop foreign keys
        for name in &alter.drop_foreign_keys {
            stmts.push(format!(
                "ALTER TABLE `{}` DROP FOREIGN KEY `{}`;",
                alter.table_name, name
            ));
        }

        // Add foreign keys
        for fk in &alter.add_foreign_keys {
            let cols: Vec<String> = fk.columns.iter().map(|c| format!("`{}`", c)).collect();
            let ref_cols: Vec<String> = fk
                .referenced_columns
                .iter()
                .map(|c| format!("`{}`", c))
                .collect();
            let mut clause = format!(
                "ALTER TABLE `{}` ADD CONSTRAINT `{}` FOREIGN KEY ({}) REFERENCES `{}` ({})",
                alter.table_name,
                fk.constraint_name,
                cols.join(", "),
                fk.referenced_table,
                ref_cols.join(", ")
            );
            if let Some(action) = &fk.on_delete {
                clause.push_str(&format!(" ON DELETE {}", action));
            }
            if let Some(action) = &fk.on_update {
                clause.push_str(&format!(" ON UPDATE {}", action));
            }
            clause.push(';');
            stmts.push(clause);
        }

        stmts
    }

    /// Generate ALTER COLUMN statements.
    fn alter_column(&self, table: &str, field: &FieldAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        if let Some(new_type) = &field.new_type {
            stmts.push(format!(
                "ALTER TABLE `{}` MODIFY COLUMN `{}` {};",
                table, field.column_name, new_type
            ));
        }

        stmts
    }

    /// Generate CREATE INDEX statement.
    fn create_index(&self, index: &IndexDiff) -> String {
        let unique = if index.unique { "UNIQUE " } else { "" };

        // Handle FULLTEXT index for MySQL
        let index_type = match &index.index_type {
            Some(prax_schema::ast::IndexType::FullText) => "FULLTEXT ",
            _ => "",
        };

        let cols: Vec<String> = index.columns.iter().map(|c| format!("`{}`", c)).collect();
        format!(
            "CREATE {}{}INDEX `{}` ON `{}`({});",
            unique,
            index_type,
            index.name,
            index.table_name,
            cols.join(", ")
        )
    }

    /// Generate DROP INDEX statement.
    fn drop_index(&self, name: &str, table: &str) -> String {
        format!("DROP INDEX `{}` ON `{}`;", name, table)
    }

    /// Generate CREATE VIEW statement.
    fn create_view(&self, view: &ViewDiff) -> String {
        // MySQL doesn't support materialized views natively
        // but we can create a regular view
        format!(
            "CREATE OR REPLACE VIEW `{}` AS\n{};",
            view.view_name, view.sql_query
        )
    }

    /// Generate DROP VIEW statement.
    fn drop_view(&self, name: &str) -> String {
        format!("DROP VIEW IF EXISTS `{}`;", name)
    }
}

/// SQL generator for SQLite.
pub struct SqliteGenerator;

impl SqliteGenerator {
    /// Generate SQL for a schema diff.
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // Create models
        for model in &diff.create_models {
            up.push(self.create_table(model));
            if let Some(vt) = self.create_vector_virtual_table(model) {
                up.push(vt);
            }
        }

        // For down migrations, we need to drop virtual tables before main tables
        for model in &diff.create_models {
            if let Some(dvt) = self.drop_vector_virtual_table(model) {
                down.push(dvt);
            }
            down.push(self.drop_table(&model.table_name));
        }

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models
        for alter in &diff.alter_models {
            // Warn about dropped columns
            for field_name in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field_name, alter.table_name
                ));
            }

            // Warn about column type changes
            for field in &alter.alter_fields {
                if let Some(_new_type) = &field.new_type {
                    if field.old_type.is_some() {
                        warnings.push(format!(
                            "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                            field.name, alter.table_name
                        ));
                    }
                }
            }
        }

        // Create indexes
        for index in &diff.create_indexes {
            up.push(self.create_index(index));
            down.push(self.drop_index(&index.name));
        }

        // Drop indexes
        for index in &diff.drop_indexes {
            up.push(self.drop_index(&index.name));
        }

        // Create views (after tables they depend on)
        for view in &diff.create_views {
            up.push(self.create_view(view));
            down.push(self.drop_view(&view.view_name));
        }

        // Drop views
        for name in &diff.drop_views {
            up.push(self.drop_view(name));
        }

        // Alter views (drop and recreate)
        for view in &diff.alter_views {
            up.push(self.drop_view(&view.view_name));
            up.push(self.create_view(view));
        }

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }

    /// Generate CREATE TABLE statement.
    fn create_table(&self, model: &ModelDiff) -> String {
        let mut columns = Vec::new();

        for field in &model.fields {
            if field.vector.is_some() {
                continue; // Vector fields live in a companion virtual table.
            }
            columns.push(self.column_definition(field));
        }

        // SQLite handles primary key in column definition for INTEGER PRIMARY KEY
        let has_integer_pk = model
            .fields
            .iter()
            .any(|f| f.is_primary_key && f.sql_type == "INTEGER" && f.is_auto_increment);

        // Add primary key constraint only if not using INTEGER PRIMARY KEY
        if !model.primary_key.is_empty() && !has_integer_pk {
            let pk_cols: Vec<String> = model
                .primary_key
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect();
            columns.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        // Add unique constraints
        for uc in &model.unique_constraints {
            let cols: Vec<String> = uc.columns.iter().map(|c| format!("\"{}\"", c)).collect();
            let constraint = if let Some(name) = &uc.name {
                format!("CONSTRAINT \"{}\" UNIQUE ({})", name, cols.join(", "))
            } else {
                format!("UNIQUE ({})", cols.join(", "))
            };
            columns.push(constraint);
        }

        // Add foreign key constraints (SQLite supports inline FK in CREATE TABLE)
        for fk in &model.foreign_keys {
            let cols: Vec<String> = fk.columns.iter().map(|c| format!("\"{}\"", c)).collect();
            let ref_cols: Vec<String> = fk
                .referenced_columns
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect();
            let mut clause = format!(
                "CONSTRAINT \"{}\" FOREIGN KEY ({}) REFERENCES \"{}\" ({})",
                fk.constraint_name,
                cols.join(", "),
                fk.referenced_table,
                ref_cols.join(", ")
            );
            if let Some(action) = &fk.on_delete {
                clause.push_str(&format!(" ON DELETE {}", action));
            }
            if let Some(action) = &fk.on_update {
                clause.push_str(&format!(" ON UPDATE {}", action));
            }
            columns.push(clause);
        }

        format!(
            "CREATE TABLE \"{}\" (\n    {}\n);",
            model.table_name,
            columns.join(",\n    ")
        )
    }

    /// Generate column definition for SQLite.
    fn column_definition(&self, field: &FieldDiff) -> String {
        let mut parts = vec![format!("\"{}\"", field.column_name)];

        // SQLite type mapping
        let sql_type = match field.sql_type.as_str() {
            "INTEGER" if field.is_primary_key && field.is_auto_increment => {
                // INTEGER PRIMARY KEY is auto-increment in SQLite
                parts.push("INTEGER PRIMARY KEY".to_string());
                return parts.join(" ");
            }
            "BIGINT" => "INTEGER".to_string(),
            "DOUBLE PRECISION" => "REAL".to_string(),
            "TIMESTAMP WITH TIME ZONE" | "DATETIME" => "TEXT".to_string(), // SQLite stores dates as TEXT
            "BOOLEAN" => "INTEGER".to_string(),
            "BYTEA" | "BLOB" => "BLOB".to_string(),
            "JSONB" | "JSON" => "TEXT".to_string(), // SQLite stores JSON as TEXT
            other => other.to_string(),
        };
        parts.push(sql_type);

        if !field.nullable && !field.is_primary_key {
            parts.push("NOT NULL".to_string());
        }

        if field.is_unique && !field.is_primary_key {
            parts.push("UNIQUE".to_string());
        }

        if let Some(default) = &field.default {
            parts.push(format!("DEFAULT {}", default));
        }

        parts.join(" ")
    }

    /// Generate DROP TABLE statement.
    fn drop_table(&self, name: &str) -> String {
        format!("DROP TABLE IF EXISTS \"{}\";", name)
    }

    /// Strip a single trailing 's' for rowid column naming ("documents" -> "document").
    /// Irregular plurals require users to override the default outside migrations.
    ///
    /// NOTE: Keep in sync with the singularize fn in
    /// prax-sqlite/src/vector/search.rs — the search-time default rowid column
    /// name must match what the migration generator emits, or the default
    /// JOIN in VectorSearchBuilder will miss the real column.
    fn singularize(name: &str) -> String {
        if name.ends_with('s') && !name.ends_with("ss") {
            name[..name.len() - 1].to_string()
        } else {
            name.to_string()
        }
    }

    /// Quote a SQL identifier, doubling embedded double quotes.
    /// Mirrors prax-sqlite::vector::quote_ident (duplicated here to avoid
    /// a circular dependency between prax-migrate and prax-sqlite).
    fn quote_ident(name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    /// Escape a single-quoted SQL string literal by doubling embedded single
    /// quotes. Used for the string values inside `USING vector(...)` clauses.
    fn escape_sql_literal(s: &str) -> String {
        s.replace('\'', "''")
    }

    /// Generate CREATE VIRTUAL TABLE for every vector column on this model.
    /// Returns None if no fields are vector columns.
    fn create_vector_virtual_table(&self, model: &ModelDiff) -> Option<String> {
        let vector_fields: Vec<&FieldDiff> =
            model.fields.iter().filter(|f| f.vector.is_some()).collect();
        if vector_fields.is_empty() {
            return None;
        }

        let rowid = format!("{}_id", Self::singularize(&model.table_name));
        let vtable = format!("{}_vectors", model.table_name);

        let mut clauses = vec![format!(
            "rowid_column='{}'",
            Self::escape_sql_literal(&rowid)
        )];
        for f in vector_fields {
            let v = f.vector.as_ref().unwrap();
            let idx_part = match v.index {
                Some(k) => format!(" {}", k.as_sql()),
                None => String::new(),
            };
            clauses.push(format!(
                "{}='{}[{}] {}{}'",
                Self::quote_ident(&f.column_name),
                v.element_type.as_sql(),
                v.dimensions,
                v.metric.as_sql(),
                idx_part
            ));
        }

        Some(format!(
            "CREATE VIRTUAL TABLE {} USING vector(\n    {}\n);",
            Self::quote_ident(&vtable),
            clauses.join(",\n    ")
        ))
    }

    /// Generate DROP VIRTUAL TABLE for the vector companion of a model, if any.
    fn drop_vector_virtual_table(&self, model: &ModelDiff) -> Option<String> {
        if model.fields.iter().any(|f| f.vector.is_some()) {
            Some(format!(
                "DROP TABLE IF EXISTS {};",
                Self::quote_ident(&format!("{}_vectors", model.table_name))
            ))
        } else {
            None
        }
    }

    /// Generate CREATE INDEX statement.
    fn create_index(&self, index: &IndexDiff) -> String {
        let unique = if index.unique { "UNIQUE " } else { "" };

        let cols: Vec<String> = index.columns.iter().map(|c| format!("\"{}\"", c)).collect();
        format!(
            "CREATE {}INDEX \"{}\" ON \"{}\"({});",
            unique,
            index.name,
            index.table_name,
            cols.join(", ")
        )
    }

    /// Generate DROP INDEX statement.
    fn drop_index(&self, name: &str) -> String {
        format!("DROP INDEX IF EXISTS \"{}\";", name)
    }

    /// Generate CREATE VIEW statement.
    fn create_view(&self, view: &ViewDiff) -> String {
        // SQLite doesn't support materialized views
        // but we can create a regular view
        format!(
            "CREATE VIEW IF NOT EXISTS \"{}\" AS\n{};",
            view.view_name, view.sql_query
        )
    }

    /// Generate DROP VIEW statement.
    fn drop_view(&self, name: &str) -> String {
        format!("DROP VIEW IF EXISTS \"{}\";", name)
    }
}

/// SQL generator for Microsoft SQL Server.
pub struct MssqlGenerator;

impl MssqlGenerator {
    /// Generate SQL for a schema diff.
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // Create models
        for model in &diff.create_models {
            up.push(self.create_table(model));
            down.push(self.drop_table(&model.table_name));
        }

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models
        for alter in &diff.alter_models {
            // Warn about dropped columns
            for field_name in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field_name, alter.table_name
                ));
            }

            // Warn about column type changes
            for field in &alter.alter_fields {
                if let Some(_new_type) = &field.new_type {
                    if field.old_type.is_some() {
                        warnings.push(format!(
                            "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                            field.name, alter.table_name
                        ));
                    }
                }
            }

            up.extend(self.alter_table(alter));
        }

        // Create indexes
        for index in &diff.create_indexes {
            up.push(self.create_index(index));
            down.push(self.drop_index(&index.name, &index.table_name));
        }

        // Drop indexes
        for index in &diff.drop_indexes {
            up.push(self.drop_index(&index.name, &index.table_name));
        }

        // Create views (after tables they depend on)
        for view in &diff.create_views {
            up.push(self.create_view(view));
            down.push(self.drop_view(&view.view_name, view.is_materialized));
        }

        // Drop views
        for name in &diff.drop_views {
            up.push(self.drop_view(name, false));
        }

        // Alter views (drop and recreate)
        for view in &diff.alter_views {
            up.push(self.drop_view(&view.view_name, view.is_materialized));
            up.push(self.create_view(view));
        }

        MigrationSql {
            up: up.join("\n\nGO\n\n"),
            down: down.join("\n\nGO\n\n"),
            warnings,
        }
    }

    /// Generate CREATE TABLE statement.
    fn create_table(&self, model: &ModelDiff) -> String {
        let mut columns = Vec::new();

        for field in &model.fields {
            columns.push(self.column_definition(field));
        }

        // Add primary key constraint
        if !model.primary_key.is_empty() {
            let pk_cols: Vec<String> = model
                .primary_key
                .iter()
                .map(|c| format!("[{}]", c))
                .collect();
            columns.push(format!(
                "CONSTRAINT [PK_{}] PRIMARY KEY ({})",
                model.table_name,
                pk_cols.join(", ")
            ));
        }

        // Add unique constraints
        for uc in &model.unique_constraints {
            let cols: Vec<String> = uc.columns.iter().map(|c| format!("[{}]", c)).collect();
            let name = uc
                .name
                .clone()
                .unwrap_or_else(|| format!("UQ_{}_{}", model.table_name, uc.columns.join("_")));
            columns.push(format!(
                "CONSTRAINT [{}] UNIQUE ({})",
                name,
                cols.join(", ")
            ));
        }

        // Add foreign key constraints
        for fk in &model.foreign_keys {
            let cols: Vec<String> = fk.columns.iter().map(|c| format!("[{}]", c)).collect();
            let ref_cols: Vec<String> = fk
                .referenced_columns
                .iter()
                .map(|c| format!("[{}]", c))
                .collect();
            let mut clause = format!(
                "CONSTRAINT [{}] FOREIGN KEY ({}) REFERENCES [{}] ({})",
                fk.constraint_name,
                cols.join(", "),
                fk.referenced_table,
                ref_cols.join(", ")
            );
            if let Some(action) = &fk.on_delete {
                clause.push_str(&format!(" ON DELETE {}", action));
            }
            if let Some(action) = &fk.on_update {
                clause.push_str(&format!(" ON UPDATE {}", action));
            }
            columns.push(clause);
        }

        format!(
            "CREATE TABLE [{}] (\n    {}\n);",
            model.table_name,
            columns.join(",\n    ")
        )
    }

    /// Generate column definition for MSSQL.
    fn column_definition(&self, field: &FieldDiff) -> String {
        let mut parts = vec![format!("[{}]", field.column_name)];

        // MSSQL type mapping
        let sql_type = match field.sql_type.as_str() {
            "INTEGER" => "INT".to_string(),
            "BIGINT" => "BIGINT".to_string(),
            "TEXT" => "NVARCHAR(MAX)".to_string(),
            "DOUBLE PRECISION" => "FLOAT".to_string(),
            "TIMESTAMP WITH TIME ZONE" => "DATETIMEOFFSET".to_string(),
            "BOOLEAN" => "BIT".to_string(),
            "BYTEA" => "VARBINARY(MAX)".to_string(),
            "JSONB" | "JSON" => "NVARCHAR(MAX)".to_string(), // MSSQL 2016+ has JSON support
            "UUID" => "UNIQUEIDENTIFIER".to_string(),
            "DECIMAL" => "DECIMAL(18,2)".to_string(),
            other => other.to_string(),
        };
        parts.push(sql_type);

        if field.is_auto_increment {
            parts.push("IDENTITY(1,1)".to_string());
        }

        if !field.nullable && !field.is_primary_key {
            parts.push("NOT NULL".to_string());
        }

        if field.is_unique && !field.is_primary_key {
            // Unique constraint will be added at table level in MSSQL
        }

        if let Some(default) = &field.default {
            parts.push(format!("DEFAULT {}", default));
        }

        parts.join(" ")
    }

    /// Generate DROP TABLE statement.
    fn drop_table(&self, name: &str) -> String {
        format!("DROP TABLE IF EXISTS [{}];", name)
    }

    /// Generate ALTER TABLE statements.
    fn alter_table(&self, alter: &ModelAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        // Add columns
        for field in &alter.add_fields {
            stmts.push(format!(
                "ALTER TABLE [{}] ADD {};",
                alter.table_name,
                self.column_definition(field)
            ));
        }

        // Drop columns
        for name in &alter.drop_fields {
            stmts.push(format!(
                "ALTER TABLE [{}] DROP COLUMN [{}];",
                alter.table_name, name
            ));
        }

        // Alter columns
        for field in &alter.alter_fields {
            stmts.extend(self.alter_column(&alter.table_name, field));
        }

        // Drop foreign keys
        for name in &alter.drop_foreign_keys {
            stmts.push(format!(
                "ALTER TABLE [{}] DROP CONSTRAINT [{}];",
                alter.table_name, name
            ));
        }

        // Add foreign keys
        for fk in &alter.add_foreign_keys {
            let cols: Vec<String> = fk.columns.iter().map(|c| format!("[{}]", c)).collect();
            let ref_cols: Vec<String> = fk
                .referenced_columns
                .iter()
                .map(|c| format!("[{}]", c))
                .collect();
            let mut clause = format!(
                "ALTER TABLE [{}] ADD CONSTRAINT [{}] FOREIGN KEY ({}) REFERENCES [{}] ({})",
                alter.table_name,
                fk.constraint_name,
                cols.join(", "),
                fk.referenced_table,
                ref_cols.join(", ")
            );
            if let Some(action) = &fk.on_delete {
                clause.push_str(&format!(" ON DELETE {}", action));
            }
            if let Some(action) = &fk.on_update {
                clause.push_str(&format!(" ON UPDATE {}", action));
            }
            clause.push(';');
            stmts.push(clause);
        }

        stmts
    }

    /// Generate ALTER COLUMN statements.
    fn alter_column(&self, table: &str, field: &FieldAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        if let Some(new_type) = &field.new_type {
            stmts.push(format!(
                "ALTER TABLE [{}] ALTER COLUMN [{}] {};",
                table, field.column_name, new_type
            ));
        }

        stmts
    }

    /// Generate CREATE INDEX statement.
    fn create_index(&self, index: &IndexDiff) -> String {
        let unique = if index.unique { "UNIQUE " } else { "" };

        let cols: Vec<String> = index.columns.iter().map(|c| format!("[{}]", c)).collect();
        format!(
            "CREATE {}INDEX [{}] ON [{}]({});",
            unique,
            index.name,
            index.table_name,
            cols.join(", ")
        )
    }

    /// Generate DROP INDEX statement.
    fn drop_index(&self, name: &str, table: &str) -> String {
        format!("DROP INDEX [{}] ON [{}];", name, table)
    }

    /// Generate CREATE VIEW statement.
    ///
    /// MSSQL supports indexed views (similar to materialized views) with:
    /// - SCHEMABINDING option
    /// - Unique clustered index on the view
    fn create_view(&self, view: &ViewDiff) -> String {
        if view.is_materialized {
            // Create an indexed view (MSSQL's equivalent of materialized views)
            // Note: This requires additional setup like creating a clustered index
            format!(
                "CREATE VIEW [{}] WITH SCHEMABINDING AS\n{};\n\n-- Create unique clustered index for indexed view\n-- CREATE UNIQUE CLUSTERED INDEX [IX_{}_Clustered] ON [{}] ([id]);",
                view.view_name, view.sql_query, view.view_name, view.view_name
            )
        } else {
            format!(
                "CREATE OR ALTER VIEW [{}] AS\n{};",
                view.view_name, view.sql_query
            )
        }
    }

    /// Generate DROP VIEW statement.
    fn drop_view(&self, name: &str, _is_materialized: bool) -> String {
        // MSSQL uses the same syntax for regular and indexed views
        format!("DROP VIEW IF EXISTS [{}];", name)
    }

    /// Generate sp_refreshview for refreshing view metadata.
    #[allow(dead_code)]
    fn refresh_view(&self, name: &str) -> String {
        format!("EXEC sp_refreshview N'{}';", name)
    }
}

/// SQL generator for DuckDB.
pub struct DuckDbSqlGenerator;

impl DuckDbSqlGenerator {
    /// Generate SQL for a schema diff.
    pub fn generate(&self, diff: &SchemaDiff) -> MigrationSql {
        let mut up = Vec::new();
        let mut down = Vec::new();
        let mut warnings = Vec::new();

        // Install and load extensions first
        for ext in &diff.create_extensions {
            up.push(self.install_extension(&ext.name));
            down.push(format!(
                "-- Extension {} cannot be uninstalled (DuckDB limitation)",
                ext.name
            ));
        }

        // Drop extensions (best-effort comment)
        for name in &diff.drop_extensions {
            up.push(self.drop_extension(name));
        }

        // Create enums (they might be used in tables)
        for enum_diff in &diff.create_enums {
            up.push(self.create_enum(enum_diff));
            down.push(self.drop_enum(&enum_diff.name));
        }

        // Drop enums (in reverse order)
        for name in &diff.drop_enums {
            up.push(self.drop_enum(name));
            // Can't easily recreate dropped enums without knowing values
        }

        // Alter enums
        for alter in &diff.alter_enums {
            up.extend(self.alter_enum(alter));
            // Reversing enum alterations is complex
        }

        // Create models
        for model in &diff.create_models {
            up.push(self.create_table(model));
            down.push(self.drop_table(&model.table_name));
        }

        // Drop models
        for name in &diff.drop_models {
            up.push(self.drop_table(name));
            warnings.push(format!(
                "Dropping table '{}' - all data will be lost and cannot be recovered",
                name
            ));
        }

        // Alter models
        for alter in &diff.alter_models {
            // Warn about dropped columns
            for field_name in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field_name, alter.table_name
                ));
            }

            // Warn about column type changes
            for field in &alter.alter_fields {
                if field.new_type.is_some() && field.old_type.is_some() {
                    warnings.push(format!(
                        "Changing column '{}' type in table '{}' - reverse migration may fail if data is incompatible",
                        field.name, alter.table_name
                    ));
                }
            }

            up.extend(self.alter_table(alter));
        }

        // Warn about foreign keys being unenforced by default
        let has_foreign_keys = diff
            .alter_models
            .iter()
            .any(|a| !a.add_foreign_keys.is_empty());
        if has_foreign_keys {
            warnings.push(
                "Foreign keys defined but not enforced unless SET check_fk_violation = 'error'"
                    .to_string(),
            );
        }

        // Create indexes
        for index in &diff.create_indexes {
            up.push(self.create_index(index));
            down.push(self.drop_index(&index.name));
        }

        // Drop indexes
        for index in &diff.drop_indexes {
            up.push(self.drop_index(&index.name));
        }

        // Create views (after tables they depend on)
        for view in &diff.create_views {
            if view.is_materialized {
                warnings.push(format!(
                    "DuckDB does not support materialized views - generating regular view '{}' instead",
                    view.view_name
                ));
            }
            up.push(self.create_view(view));
            down.push(self.drop_view(&view.view_name));
        }

        // Drop views
        for name in &diff.drop_views {
            up.push(self.drop_view(name));
        }

        // Alter views (drop and recreate)
        for view in &diff.alter_views {
            up.push(self.drop_view(&view.view_name));
            up.push(self.create_view(view));
        }

        // Reverse the down statements so that teardown order is the inverse of
        // creation order: views first, then tables, then enums, then extensions.
        // This ensures referential dependencies are satisfied during rollback.
        down.reverse();

        MigrationSql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }

    /// Generate INSTALL and LOAD statements for an extension.
    fn install_extension(&self, name: &str) -> String {
        format!("INSTALL '{}';\nLOAD '{}';", name, name)
    }

    /// Generate a comment noting that an extension cannot be uninstalled.
    fn drop_extension(&self, name: &str) -> String {
        // DuckDB doesn't have UNINSTALL, extensions persist
        format!("-- Extension {} cannot be uninstalled", name)
    }

    /// Generate CREATE TYPE for enum.
    fn create_enum(&self, enum_diff: &EnumDiff) -> String {
        let values: Vec<String> = enum_diff
            .values
            .iter()
            .map(|v| format!("'{}'", v))
            .collect();
        format!(
            "CREATE TYPE \"{}\" AS ENUM ({});",
            enum_diff.name,
            values.join(", ")
        )
    }

    /// Generate DROP TYPE.
    fn drop_enum(&self, name: &str) -> String {
        format!("DROP TYPE IF EXISTS \"{}\";", name)
    }

    /// Generate ALTER TYPE statements.
    fn alter_enum(&self, alter: &EnumAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        for value in &alter.add_values {
            stmts.push(format!(
                "ALTER TYPE \"{}\" ADD VALUE '{}';",
                alter.name, value
            ));
        }

        // Note: DuckDB doesn't support removing enum values directly
        // This would require recreating the type

        stmts
    }

    /// Map SQL type from FieldDiff to DuckDB SQL type.
    fn map_field_type(&self, sql_type: &str) -> String {
        match sql_type {
            "INTEGER" => "INTEGER".to_string(),
            "BIGINT" => "BIGINT".to_string(),
            "VARCHAR" | "TEXT" => "VARCHAR".to_string(),
            "BOOLEAN" => "BOOLEAN".to_string(),
            "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" | "DATETIME" => {
                "TIMESTAMP WITH TIME ZONE".to_string()
            }
            "JSON" | "JSONB" => "JSON".to_string(),
            "DOUBLE PRECISION" => "DOUBLE".to_string(),
            "REAL" => "REAL".to_string(),
            "DECIMAL" => "DECIMAL".to_string(),
            "BLOB" | "BYTEA" => "BLOB".to_string(),
            "UUID" => "UUID".to_string(),
            other => other.to_string(),
        }
    }

    /// Generate CREATE TABLE statement.
    fn create_table(&self, model: &ModelDiff) -> String {
        let mut columns = Vec::new();

        for field in &model.fields {
            let mut col = format!("\"{}\"", field.column_name);
            col.push(' ');

            let field_type = self.map_field_type(&field.sql_type);
            col.push_str(&field_type);

            if field.is_primary_key && field.is_auto_increment {
                col.push_str(" GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY");
            } else if field.is_primary_key {
                col.push_str(" PRIMARY KEY");
            }

            if !field.nullable && !field.is_primary_key {
                col.push_str(" NOT NULL");
            }

            if field.is_unique && !field.is_primary_key {
                col.push_str(" UNIQUE");
            }

            if let Some(default) = &field.default {
                col.push_str(" DEFAULT ");
                col.push_str(default);
            }

            columns.push(col);
        }

        let has_inline_pk = model.fields.iter().any(|f| f.is_primary_key);
        if !has_inline_pk && !model.primary_key.is_empty() {
            let pk_cols: Vec<String> = model
                .primary_key
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect();
            columns.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        for uc in &model.unique_constraints {
            let cols: Vec<String> = uc.columns.iter().map(|c| format!("\"{}\"", c)).collect();
            let constraint = if let Some(name) = &uc.name {
                format!("CONSTRAINT \"{}\" UNIQUE ({})", name, cols.join(", "))
            } else {
                format!("UNIQUE ({})", cols.join(", "))
            };
            columns.push(constraint);
        }

        for fk in &model.foreign_keys {
            columns.push(self.foreign_key_constraint(fk));
        }

        format!(
            "CREATE TABLE \"{}\" (\n    {}\n);",
            model.table_name,
            columns.join(",\n    ")
        )
    }

    /// Generate DROP TABLE statement.
    fn drop_table(&self, name: &str) -> String {
        format!("DROP TABLE IF EXISTS \"{}\";", name)
    }

    /// Generate ALTER TABLE statements for model changes.
    fn alter_table(&self, alter: &ModelAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        // Add columns
        for field in &alter.add_fields {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ADD COLUMN {};",
                alter.table_name,
                self.column_definition(field)
            ));
        }

        // Drop columns
        for name in &alter.drop_fields {
            stmts.push(format!(
                "ALTER TABLE \"{}\" DROP COLUMN \"{}\";",
                alter.table_name, name
            ));
        }

        // Alter columns
        for field in &alter.alter_fields {
            stmts.extend(self.alter_column(&alter.table_name, field));
        }

        // Add indexes inline on the table
        for index in &alter.add_indexes {
            stmts.push(self.create_index(index));
        }

        // Drop indexes
        for name in &alter.drop_indexes {
            stmts.push(self.drop_index(name));
        }

        // Drop foreign keys
        for name in &alter.drop_foreign_keys {
            stmts.push(format!(
                "ALTER TABLE \"{}\" DROP CONSTRAINT \"{}\";",
                alter.table_name, name
            ));
        }

        // Add foreign keys
        for fk in &alter.add_foreign_keys {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ADD {};",
                alter.table_name,
                self.foreign_key_constraint(fk)
            ));
        }

        stmts
    }

    /// Generate a foreign key CONSTRAINT clause.
    fn foreign_key_constraint(&self, fk: &ForeignKeyDiff) -> String {
        let cols: Vec<String> = fk.columns.iter().map(|c| format!("\"{}\"", c)).collect();
        let ref_cols: Vec<String> = fk
            .referenced_columns
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect();

        let mut clause = format!(
            "CONSTRAINT \"{}\" FOREIGN KEY ({}) REFERENCES \"{}\" ({})",
            fk.constraint_name,
            cols.join(", "),
            fk.referenced_table,
            ref_cols.join(", ")
        );

        if let Some(action) = &fk.on_delete {
            clause.push_str(&format!(" ON DELETE {}", action));
        }
        if let Some(action) = &fk.on_update {
            clause.push_str(&format!(" ON UPDATE {}", action));
        }

        clause
    }

    /// Generate column definition string for CREATE/ALTER TABLE.
    fn column_definition(&self, field: &FieldDiff) -> String {
        let mut parts = vec![
            format!("\"{}\"", field.column_name),
            self.map_field_type(&field.sql_type),
        ];

        if !field.nullable && !field.is_primary_key {
            parts.push("NOT NULL".to_string());
        }

        if field.is_unique && !field.is_primary_key {
            parts.push("UNIQUE".to_string());
        }

        if let Some(default) = &field.default {
            parts.push(format!("DEFAULT {}", default));
        }

        parts.join(" ")
    }

    /// Generate ALTER COLUMN statements.
    fn alter_column(&self, table: &str, field: &FieldAlterDiff) -> Vec<String> {
        let mut stmts = Vec::new();

        if let Some(new_type) = &field.new_type {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" TYPE {};",
                table, field.column_name, new_type
            ));
        }

        if let Some(new_nullable) = field.new_nullable {
            if new_nullable {
                stmts.push(format!(
                    "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" DROP NOT NULL;",
                    table, field.column_name
                ));
            } else {
                stmts.push(format!(
                    "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" SET NOT NULL;",
                    table, field.column_name
                ));
            }
        }

        if let Some(new_default) = &field.new_default {
            stmts.push(format!(
                "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" SET DEFAULT {};",
                table, field.column_name, new_default
            ));
        }

        stmts
    }

    /// Generate CREATE INDEX statement.
    fn create_index(&self, index: &IndexDiff) -> String {
        let unique = if index.unique { "UNIQUE " } else { "" };
        let cols: Vec<String> = index.columns.iter().map(|c| format!("\"{}\"", c)).collect();

        format!(
            "CREATE {}INDEX \"{}\" ON \"{}\"({});",
            unique,
            index.name,
            index.table_name,
            cols.join(", ")
        )
    }

    /// Generate DROP INDEX statement.
    fn drop_index(&self, name: &str) -> String {
        format!("DROP INDEX IF EXISTS \"{}\";", name)
    }

    /// Generate CREATE VIEW statement.
    /// DuckDB does not support materialized views; always emits a regular view.
    fn create_view(&self, view: &ViewDiff) -> String {
        format!("CREATE VIEW \"{}\" AS\n{};", view.view_name, view.sql_query)
    }

    /// Generate DROP VIEW statement.
    fn drop_view(&self, name: &str) -> String {
        format!("DROP VIEW IF EXISTS \"{}\";", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_enum() {
        let generator = PostgresSqlGenerator;
        let enum_diff = EnumDiff {
            name: "Status".to_string(),
            values: vec!["PENDING".to_string(), "ACTIVE".to_string()],
        };

        let sql = generator.create_enum(&enum_diff);
        assert!(sql.contains("CREATE TYPE"));
        assert!(sql.contains("Status"));
        assert!(sql.contains("PENDING"));
        assert!(sql.contains("ACTIVE"));
    }

    #[test]
    fn test_create_table() {
        let generator = PostgresSqlGenerator;
        let model = ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![
                FieldDiff {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    sql_type: "INTEGER".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: true,
                    is_auto_increment: true,
                    is_unique: false,
                    vector: None,
                },
                FieldDiff {
                    name: "email".to_string(),
                    column_name: "email".to_string(),
                    sql_type: "TEXT".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_unique: true,
                    vector: None,
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };

        let sql = generator.create_table(&model);
        assert!(sql.contains("CREATE TABLE"));
        assert!(sql.contains("users"));
        assert!(sql.contains("SERIAL"));
        assert!(sql.contains("email"));
        assert!(sql.contains("UNIQUE"));
        assert!(sql.contains("PRIMARY KEY"));
    }

    #[test]
    fn test_create_index() {
        let generator = PostgresSqlGenerator;
        let index = IndexDiff::new("idx_users_email", "users", vec!["email".to_string()]).unique();

        let sql = generator.create_index(&index);
        assert!(sql.contains("CREATE UNIQUE INDEX"));
        assert!(sql.contains("idx_users_email"));
        assert!(sql.contains("users"));
    }

    #[test]
    fn test_create_hnsw_index() {
        use prax_schema::ast::{IndexType, VectorOps};

        let generator = PostgresSqlGenerator;
        let index = IndexDiff::new("idx_embedding", "documents", vec!["embedding".to_string()])
            .with_type(IndexType::Hnsw)
            .with_vector_ops(VectorOps::Cosine)
            .with_hnsw_m(16)
            .with_hnsw_ef_construction(64);

        let sql = generator.create_index(&index);
        assert!(sql.contains("CREATE INDEX"));
        assert!(sql.contains("USING hnsw"));
        assert!(sql.contains("vector_cosine_ops"));
        assert!(sql.contains("m = 16"));
        assert!(sql.contains("ef_construction = 64"));
    }

    #[test]
    fn test_create_ivfflat_index() {
        use prax_schema::ast::{IndexType, VectorOps};

        let generator = PostgresSqlGenerator;
        let index = IndexDiff::new(
            "idx_embedding_l2",
            "documents",
            vec!["embedding".to_string()],
        )
        .with_type(IndexType::IvfFlat)
        .with_vector_ops(VectorOps::L2)
        .with_ivfflat_lists(100);

        let sql = generator.create_index(&index);
        assert!(sql.contains("CREATE INDEX"));
        assert!(sql.contains("USING ivfflat"));
        assert!(sql.contains("vector_l2_ops"));
        assert!(sql.contains("lists = 100"));
    }

    #[test]
    fn test_create_gin_index() {
        use prax_schema::ast::IndexType;

        let generator = PostgresSqlGenerator;
        let index =
            IndexDiff::new("idx_tags", "posts", vec!["tags".to_string()]).with_type(IndexType::Gin);

        let sql = generator.create_index(&index);
        assert!(sql.contains("CREATE INDEX"));
        assert!(sql.contains("USING GIN"));
        assert!(sql.contains("idx_tags"));
    }

    #[test]
    fn test_alter_table_add_column() {
        let generator = PostgresSqlGenerator;
        let alter = ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: vec![FieldDiff {
                name: "age".to_string(),
                column_name: "age".to_string(),
                sql_type: "INTEGER".to_string(),
                nullable: true,
                default: None,
                is_primary_key: false,
                is_auto_increment: false,
                is_unique: false,
                vector: None,
            }],
            drop_fields: Vec::new(),
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        };

        let stmts = generator.alter_table(&alter);
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].contains("ADD COLUMN"));
        assert!(stmts[0].contains("age"));
    }

    #[test]
    fn test_create_view() {
        let generator = PostgresSqlGenerator;
        let view = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) as post_count FROM users GROUP BY id".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert!(sql.contains("CREATE OR REPLACE VIEW"));
        assert!(sql.contains("user_stats"));
        assert!(sql.contains("SELECT id"));
        assert!(sql.contains("post_count"));
    }

    #[test]
    fn test_create_materialized_view() {
        let generator = PostgresSqlGenerator;
        let view = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) as post_count FROM users GROUP BY id".to_string(),
            is_materialized: true,
            refresh_interval: Some("1h".to_string()),
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert!(sql.contains("CREATE MATERIALIZED VIEW"));
        assert!(sql.contains("user_stats"));
        assert!(!sql.contains("OR REPLACE")); // Materialized views don't support OR REPLACE
    }

    #[test]
    fn test_drop_view() {
        let generator = PostgresSqlGenerator;

        let sql = generator.drop_view("user_stats", false);
        assert!(sql.contains("DROP VIEW"));
        assert!(sql.contains("user_stats"));
        assert!(sql.contains("CASCADE"));

        let sql_mat = generator.drop_view("user_stats", true);
        assert!(sql_mat.contains("DROP MATERIALIZED VIEW"));
        assert!(sql_mat.contains("user_stats"));
    }

    #[test]
    fn test_refresh_materialized_view() {
        let generator = PostgresSqlGenerator;

        let sql = generator.refresh_materialized_view("user_stats", false);
        assert!(sql.contains("REFRESH MATERIALIZED VIEW"));
        assert!(sql.contains("user_stats"));
        assert!(!sql.contains("CONCURRENTLY"));

        let sql_concurrent = generator.refresh_materialized_view("user_stats", true);
        assert!(sql_concurrent.contains("CONCURRENTLY"));
    }

    #[test]
    fn test_generate_with_views() {
        use crate::diff::SchemaDiff;

        let generator = PostgresSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "ActiveUsers".to_string(),
            view_name: "active_users".to_string(),
            sql_query: "SELECT * FROM users WHERE active = true".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        });

        let sql = generator.generate(&diff);
        assert!(!sql.is_empty());
        assert!(sql.up.contains("CREATE OR REPLACE VIEW"));
        assert!(sql.up.contains("active_users"));
        assert!(sql.down.contains("DROP VIEW"));
    }

    // ==================== MySQL Generator Tests ====================

    #[test]
    fn test_mysql_create_view() {
        let generator = MySqlGenerator;
        let view = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) as post_count FROM users GROUP BY id".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert!(sql.contains("CREATE OR REPLACE VIEW"));
        assert!(sql.contains("`user_stats`"));
        assert!(sql.contains("SELECT id"));
    }

    #[test]
    fn test_mysql_drop_view() {
        let generator = MySqlGenerator;
        let sql = generator.drop_view("user_stats");
        assert!(sql.contains("DROP VIEW IF EXISTS"));
        assert!(sql.contains("`user_stats`"));
    }

    #[test]
    fn test_mysql_generate_with_views() {
        use crate::diff::SchemaDiff;

        let generator = MySqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "ActiveUsers".to_string(),
            view_name: "active_users".to_string(),
            sql_query: "SELECT * FROM users WHERE active = 1".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        });

        let sql = generator.generate(&diff);
        assert!(!sql.is_empty());
        assert!(sql.up.contains("CREATE OR REPLACE VIEW"));
        assert!(sql.up.contains("`active_users`"));
        assert!(sql.down.contains("DROP VIEW"));
    }

    #[test]
    fn test_mysql_create_table() {
        let generator = MySqlGenerator;
        let model = ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![FieldDiff {
                name: "id".to_string(),
                column_name: "id".to_string(),
                sql_type: "INTEGER".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
                is_auto_increment: true,
                is_unique: false,
                vector: None,
            }],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };

        let sql = generator.create_table(&model);
        assert!(sql.contains("CREATE TABLE `users`"));
        assert!(sql.contains("AUTO_INCREMENT"));
        assert!(sql.contains("ENGINE=InnoDB"));
    }

    // ==================== SQLite Generator Tests ====================

    #[test]
    fn test_sqlite_create_view() {
        let generator = SqliteGenerator;
        let view = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) as post_count FROM users GROUP BY id".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert!(sql.contains("CREATE VIEW IF NOT EXISTS"));
        assert!(sql.contains("\"user_stats\""));
        assert!(sql.contains("SELECT id"));
    }

    #[test]
    fn test_sqlite_drop_view() {
        let generator = SqliteGenerator;
        let sql = generator.drop_view("user_stats");
        assert!(sql.contains("DROP VIEW IF EXISTS"));
        assert!(sql.contains("\"user_stats\""));
    }

    #[test]
    fn test_sqlite_generate_with_views() {
        use crate::diff::SchemaDiff;

        let generator = SqliteGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "ActiveUsers".to_string(),
            view_name: "active_users".to_string(),
            sql_query: "SELECT * FROM users WHERE active = 1".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        });

        let sql = generator.generate(&diff);
        assert!(!sql.is_empty());
        assert!(sql.up.contains("CREATE VIEW IF NOT EXISTS"));
        assert!(sql.up.contains("\"active_users\""));
        assert!(sql.down.contains("DROP VIEW"));
    }

    #[test]
    fn test_sqlite_create_table_with_autoincrement() {
        let generator = SqliteGenerator;
        let model = ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![FieldDiff {
                name: "id".to_string(),
                column_name: "id".to_string(),
                sql_type: "INTEGER".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
                is_auto_increment: true,
                is_unique: false,
                vector: None,
            }],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };

        let sql = generator.create_table(&model);
        assert!(sql.contains("CREATE TABLE \"users\""));
        assert!(sql.contains("INTEGER PRIMARY KEY"));
    }

    #[test]
    fn test_sqlite_drop_table_generates_warning() {
        let generator = SqliteGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_models.push("users".to_string());

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[0].to_lowercase().contains("data"));
    }

    #[test]
    fn test_sqlite_drop_column_generates_warning() {
        let generator = SqliteGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["email".to_string(), "phone".to_string()],
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 2);
        assert!(sql.warnings[0].contains("email"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[1].contains("phone"));
        assert!(sql.warnings[1].contains("users"));
    }

    #[test]
    fn test_sqlite_alter_column_type_generates_warning() {
        let generator = SqliteGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: vec![
                FieldAlterDiff {
                    name: "age".to_string(),
                    column_name: "age".to_string(),
                    old_type: Some("INTEGER".to_string()),
                    new_type: Some("TEXT".to_string()),
                    old_nullable: None,
                    new_nullable: None,
                    old_default: None,
                    new_default: None,
                },
                FieldAlterDiff {
                    name: "email".to_string(),
                    column_name: "email".to_string(),
                    old_type: None,
                    new_type: None,
                    old_nullable: Some(true),
                    new_nullable: Some(false),
                    old_default: None,
                    new_default: None,
                },
            ],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should only warn about the type change, not nullable change
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("age"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].contains("reverse migration"));
        assert!(sql.warnings[0].contains("incompatible"));
    }

    #[test]
    fn test_sqlite_multiple_warnings() {
        let generator = SqliteGenerator;
        let mut diff = SchemaDiff::default();

        // Drop a table
        diff.drop_models.push("old_table".to_string());

        // Alter a table with drop column and type change
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["deprecated_field".to_string()],
            alter_fields: vec![FieldAlterDiff {
                name: "status".to_string(),
                column_name: "status".to_string(),
                old_type: Some("INTEGER".to_string()),
                new_type: Some("TEXT".to_string()),
                old_nullable: None,
                new_nullable: None,
                old_default: None,
                new_default: None,
            }],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should have 3 warnings: 1 drop table, 1 drop column, 1 type change
        assert_eq!(sql.warnings.len(), 3);

        // Find each warning type
        let drop_table_warning = sql.warnings.iter().find(|w| w.contains("old_table"));
        let drop_column_warning = sql.warnings.iter().find(|w| w.contains("deprecated_field"));
        let type_change_warning = sql
            .warnings
            .iter()
            .find(|w| w.contains("reverse migration"));

        assert!(drop_table_warning.is_some());
        assert!(drop_column_warning.is_some());
        assert!(type_change_warning.is_some());
    }

    #[test]
    fn test_sqlite_generator_emits_virtual_table_for_single_vector_field() {
        use crate::diff::{
            FieldDiff, ModelDiff, SchemaDiff, VectorColumnInfo, VectorDistanceMetric,
            VectorElementType, VectorIndexKind,
        };

        let mut diff = SchemaDiff::default();
        diff.create_models.push(ModelDiff {
            name: "Document".to_string(),
            table_name: "documents".to_string(),
            fields: vec![
                FieldDiff {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    sql_type: "INTEGER".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: true,
                    is_auto_increment: true,
                    is_unique: false,
                    vector: None,
                },
                FieldDiff {
                    name: "embedding".to_string(),
                    column_name: "embedding".to_string(),
                    sql_type: "BLOB".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_unique: false,
                    vector: Some(VectorColumnInfo {
                        dimensions: 1536,
                        element_type: VectorElementType::Float4,
                        metric: VectorDistanceMetric::Cosine,
                        index: Some(VectorIndexKind::Hnsw),
                    }),
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        });

        let sql = SqliteGenerator.generate(&diff);

        // Main table should NOT contain the vector column.
        assert!(sql.up.contains("CREATE TABLE \"documents\""));
        assert!(!sql.up.contains("\"embedding\" BLOB"));

        // A companion virtual table should appear.
        assert!(
            sql.up
                .contains("CREATE VIRTUAL TABLE \"documents_vectors\" USING vector")
        );
        assert!(sql.up.contains("rowid_column='document_id'"));
        assert!(sql.up.contains("\"embedding\"='float4[1536] cosine hnsw'"));

        // Down migration drops the virtual table before the main table.
        let vt_pos = sql
            .down
            .find("DROP TABLE IF EXISTS \"documents_vectors\"")
            .unwrap();
        let mt_pos = sql.down.find("DROP TABLE IF EXISTS \"documents\"").unwrap();
        assert!(vt_pos < mt_pos);
    }

    #[test]
    fn test_sqlite_generator_combines_multiple_vector_columns_into_one_virtual_table() {
        use crate::diff::{
            FieldDiff, ModelDiff, SchemaDiff, VectorColumnInfo, VectorDistanceMetric,
            VectorElementType, VectorIndexKind,
        };

        let mut diff = SchemaDiff::default();
        diff.create_models.push(ModelDiff {
            name: "Document".to_string(),
            table_name: "documents".to_string(),
            fields: vec![
                FieldDiff {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    sql_type: "INTEGER".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: true,
                    is_auto_increment: true,
                    is_unique: false,
                    vector: None,
                },
                FieldDiff {
                    name: "embedding".to_string(),
                    column_name: "embedding".to_string(),
                    sql_type: "BLOB".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_unique: false,
                    vector: Some(VectorColumnInfo {
                        dimensions: 1536,
                        element_type: VectorElementType::Float4,
                        metric: VectorDistanceMetric::Cosine,
                        index: Some(VectorIndexKind::Hnsw),
                    }),
                },
                FieldDiff {
                    name: "summary_vec".to_string(),
                    column_name: "summary_vec".to_string(),
                    sql_type: "BLOB".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_unique: false,
                    vector: Some(VectorColumnInfo {
                        dimensions: 384,
                        element_type: VectorElementType::Float4,
                        metric: VectorDistanceMetric::Cosine,
                        index: Some(VectorIndexKind::Hnsw),
                    }),
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        });

        let sql = SqliteGenerator.generate(&diff);

        // Exactly one virtual table with both columns listed.
        let count = sql
            .up
            .matches("CREATE VIRTUAL TABLE \"documents_vectors\"")
            .count();
        assert_eq!(count, 1);
        assert!(sql.up.contains("\"embedding\"='float4[1536] cosine hnsw'"));
        assert!(sql.up.contains("\"summary_vec\"='float4[384] cosine hnsw'"));
    }

    #[test]
    fn test_sqlite_generator_without_vector_fields_produces_no_virtual_table() {
        use crate::diff::{FieldDiff, ModelDiff, SchemaDiff};

        let mut diff = SchemaDiff::default();
        diff.create_models.push(ModelDiff {
            name: "Document".to_string(),
            table_name: "documents".to_string(),
            fields: vec![FieldDiff {
                name: "id".to_string(),
                column_name: "id".to_string(),
                sql_type: "INTEGER".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
                is_auto_increment: true,
                is_unique: false,
                vector: None,
            }],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        });

        let sql = SqliteGenerator.generate(&diff);
        assert!(!sql.up.contains("USING vector"));
    }

    // ==================== MSSQL Generator Tests ====================

    #[test]
    fn test_mssql_create_view() {
        let generator = MssqlGenerator;
        let view = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) as post_count FROM users GROUP BY id".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert!(sql.contains("CREATE OR ALTER VIEW"));
        assert!(sql.contains("[user_stats]"));
        assert!(sql.contains("SELECT id"));
    }

    #[test]
    fn test_mssql_create_indexed_view() {
        let generator = MssqlGenerator;
        let view = ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT id, COUNT(*) as post_count FROM users GROUP BY id".to_string(),
            is_materialized: true,
            refresh_interval: None,
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert!(sql.contains("WITH SCHEMABINDING"));
        assert!(sql.contains("[user_stats]"));
        // Should include comment about clustered index
        assert!(sql.contains("CLUSTERED INDEX"));
    }

    #[test]
    fn test_mssql_drop_view() {
        let generator = MssqlGenerator;
        let sql = generator.drop_view("user_stats", false);
        assert!(sql.contains("DROP VIEW IF EXISTS"));
        assert!(sql.contains("[user_stats]"));
    }

    #[test]
    fn test_mssql_generate_with_views() {
        use crate::diff::SchemaDiff;

        let generator = MssqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "ActiveUsers".to_string(),
            view_name: "active_users".to_string(),
            sql_query: "SELECT * FROM users WHERE active = 1".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        });

        let sql = generator.generate(&diff);
        assert!(!sql.is_empty());
        assert!(sql.up.contains("CREATE OR ALTER VIEW"));
        assert!(sql.up.contains("[active_users]"));
        assert!(sql.down.contains("DROP VIEW"));
    }

    #[test]
    fn test_mssql_create_table() {
        let generator = MssqlGenerator;
        let model = ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![FieldDiff {
                name: "id".to_string(),
                column_name: "id".to_string(),
                sql_type: "INTEGER".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
                is_auto_increment: true,
                is_unique: false,
                vector: None,
            }],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };

        let sql = generator.create_table(&model);
        assert!(sql.contains("CREATE TABLE [users]"));
        assert!(sql.contains("IDENTITY(1,1)"));
        assert!(sql.contains("[PK_users]"));
    }

    #[test]
    fn test_migration_sql_with_warnings() {
        let sql = MigrationSql {
            up: "CREATE TABLE users (id INT);".to_string(),
            down: "DROP TABLE users;".to_string(),
            warnings: vec!["Dropping table 'users' - all data will be lost".to_string()],
        };

        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("data will be lost"));
    }

    #[test]
    fn test_mssql_drop_table_generates_warning() {
        let generator = MssqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_models.push("users".to_string());

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[0].to_lowercase().contains("data"));
    }

    #[test]
    fn test_mssql_drop_column_generates_warning() {
        let generator = MssqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["email".to_string(), "phone".to_string()],
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 2);
        assert!(sql.warnings[0].contains("email"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[1].contains("phone"));
        assert!(sql.warnings[1].contains("users"));
    }

    #[test]
    fn test_mssql_alter_column_type_generates_warning() {
        let generator = MssqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: vec![
                FieldAlterDiff {
                    name: "age".to_string(),
                    column_name: "age".to_string(),
                    old_type: Some("INTEGER".to_string()),
                    new_type: Some("TEXT".to_string()),
                    old_nullable: None,
                    new_nullable: None,
                    old_default: None,
                    new_default: None,
                },
                FieldAlterDiff {
                    name: "email".to_string(),
                    column_name: "email".to_string(),
                    old_type: None,
                    new_type: None,
                    old_nullable: Some(true),
                    new_nullable: Some(false),
                    old_default: None,
                    new_default: None,
                },
            ],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should only warn about the type change, not nullable change
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("age"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].contains("reverse migration"));
        assert!(sql.warnings[0].contains("incompatible"));
    }

    #[test]
    fn test_mssql_multiple_warnings() {
        let generator = MssqlGenerator;
        let mut diff = SchemaDiff::default();

        // Drop a table
        diff.drop_models.push("old_table".to_string());

        // Alter a table with drop column and type change
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["deprecated_field".to_string()],
            alter_fields: vec![FieldAlterDiff {
                name: "status".to_string(),
                column_name: "status".to_string(),
                old_type: Some("INTEGER".to_string()),
                new_type: Some("TEXT".to_string()),
                old_nullable: None,
                new_nullable: None,
                old_default: None,
                new_default: None,
            }],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should have 3 warnings: 1 drop table, 1 drop column, 1 type change
        assert_eq!(sql.warnings.len(), 3);

        // Find each warning type
        let drop_table_warning = sql.warnings.iter().find(|w| w.contains("old_table"));
        let drop_column_warning = sql.warnings.iter().find(|w| w.contains("deprecated_field"));
        let type_change_warning = sql
            .warnings
            .iter()
            .find(|w| w.contains("reverse migration"));

        assert!(drop_table_warning.is_some());
        assert!(drop_column_warning.is_some());
        assert!(type_change_warning.is_some());
    }

    #[test]
    fn test_migration_sql_no_warnings() {
        let sql = MigrationSql {
            up: "CREATE INDEX idx_email ON users(email);".to_string(),
            down: "DROP INDEX idx_email;".to_string(),
            warnings: Vec::new(),
        };

        assert!(sql.warnings.is_empty());
    }

    #[test]
    fn test_postgres_drop_table_generates_warning() {
        let generator = PostgresSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_models.push("users".to_string());

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[0].to_lowercase().contains("data"));
    }

    #[test]
    fn test_postgres_drop_column_generates_warning() {
        let generator = PostgresSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["email".to_string(), "phone".to_string()],
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 2);
        assert!(sql.warnings[0].contains("email"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[1].contains("phone"));
        assert!(sql.warnings[1].contains("users"));
    }

    #[test]
    fn test_postgres_alter_column_type_generates_warning() {
        let generator = PostgresSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: vec![
                FieldAlterDiff {
                    name: "age".to_string(),
                    column_name: "age".to_string(),
                    old_type: Some("INTEGER".to_string()),
                    new_type: Some("TEXT".to_string()),
                    old_nullable: None,
                    new_nullable: None,
                    old_default: None,
                    new_default: None,
                },
                FieldAlterDiff {
                    name: "email".to_string(),
                    column_name: "email".to_string(),
                    old_type: None,
                    new_type: None,
                    old_nullable: Some(true),
                    new_nullable: Some(false),
                    old_default: None,
                    new_default: None,
                },
            ],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should only warn about the type change, not nullable change
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("age"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].contains("reverse migration"));
        assert!(sql.warnings[0].contains("incompatible"));
    }

    #[test]
    fn test_postgres_multiple_warnings() {
        let generator = PostgresSqlGenerator;
        let mut diff = SchemaDiff::default();

        // Drop a table
        diff.drop_models.push("old_table".to_string());

        // Alter a table with drop column and type change
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["deprecated_field".to_string()],
            alter_fields: vec![FieldAlterDiff {
                name: "status".to_string(),
                column_name: "status".to_string(),
                old_type: Some("INTEGER".to_string()),
                new_type: Some("TEXT".to_string()),
                old_nullable: None,
                new_nullable: None,
                old_default: None,
                new_default: None,
            }],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should have 3 warnings: 1 drop table, 1 drop column, 1 type change
        assert_eq!(sql.warnings.len(), 3);

        // Find each warning type
        let drop_table_warning = sql.warnings.iter().find(|w| w.contains("old_table"));
        let drop_column_warning = sql.warnings.iter().find(|w| w.contains("deprecated_field"));
        let type_change_warning = sql
            .warnings
            .iter()
            .find(|w| w.contains("reverse migration"));

        assert!(drop_table_warning.is_some());
        assert!(drop_column_warning.is_some());
        assert!(type_change_warning.is_some());
    }

    #[test]
    fn test_mysql_drop_table_generates_warning() {
        let generator = MySqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_models.push("users".to_string());

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[0].to_lowercase().contains("data"));
    }

    #[test]
    fn test_mysql_drop_column_generates_warning() {
        let generator = MySqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["email".to_string(), "phone".to_string()],
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        assert_eq!(sql.warnings.len(), 2);
        assert!(sql.warnings[0].contains("email"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].to_lowercase().contains("drop"));
        assert!(sql.warnings[1].contains("phone"));
        assert!(sql.warnings[1].contains("users"));
    }

    #[test]
    fn test_mysql_alter_column_type_generates_warning() {
        let generator = MySqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: vec![
                FieldAlterDiff {
                    name: "age".to_string(),
                    column_name: "age".to_string(),
                    old_type: Some("INTEGER".to_string()),
                    new_type: Some("TEXT".to_string()),
                    old_nullable: None,
                    new_nullable: None,
                    old_default: None,
                    new_default: None,
                },
                FieldAlterDiff {
                    name: "email".to_string(),
                    column_name: "email".to_string(),
                    old_type: None,
                    new_type: None,
                    old_nullable: Some(true),
                    new_nullable: Some(false),
                    old_default: None,
                    new_default: None,
                },
            ],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should only warn about the type change, not nullable change
        assert_eq!(sql.warnings.len(), 1);
        assert!(sql.warnings[0].contains("age"));
        assert!(sql.warnings[0].contains("users"));
        assert!(sql.warnings[0].contains("reverse migration"));
        assert!(sql.warnings[0].contains("incompatible"));
    }

    #[test]
    fn test_mysql_multiple_warnings() {
        let generator = MySqlGenerator;
        let mut diff = SchemaDiff::default();

        // Drop a table
        diff.drop_models.push("old_table".to_string());

        // Alter a table with drop column and type change
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["deprecated_field".to_string()],
            alter_fields: vec![FieldAlterDiff {
                name: "status".to_string(),
                column_name: "status".to_string(),
                old_type: Some("INTEGER".to_string()),
                new_type: Some("TEXT".to_string()),
                old_nullable: None,
                new_nullable: None,
                old_default: None,
                new_default: None,
            }],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let sql = generator.generate(&diff);
        // Should have 3 warnings: 1 drop table, 1 drop column, 1 type change
        assert_eq!(sql.warnings.len(), 3);

        // Find each warning type
        let drop_table_warning = sql.warnings.iter().find(|w| w.contains("old_table"));
        let drop_column_warning = sql.warnings.iter().find(|w| w.contains("deprecated_field"));
        let type_change_warning = sql
            .warnings
            .iter()
            .find(|w| w.contains("reverse migration"));

        assert!(drop_table_warning.is_some());
        assert!(drop_column_warning.is_some());
        assert!(type_change_warning.is_some());
    }

    #[test]
    fn test_duckdb_generator_exists() {
        let generator = DuckDbSqlGenerator;
        let diff = SchemaDiff::default();
        let result = generator.generate(&diff);
        assert!(result.is_empty());
        assert!(result.warnings.is_empty());
    }
}

#[cfg(test)]
mod duckdb_tests {
    use super::*;

    #[test]
    fn test_duckdb_install_extension_generates_sql() {
        let generator = DuckDbSqlGenerator;
        let sql = generator.install_extension("parquet");
        assert_eq!(sql, "INSTALL 'parquet';\nLOAD 'parquet';");
    }

    #[test]
    fn test_duckdb_generate_with_extensions() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_extensions.push(ExtensionDiff {
            name: "parquet".to_string(),
            schema: None,
            version: None,
        });

        let migration = generator.generate(&diff);
        assert!(!migration.up.is_empty());
        assert!(migration.up.contains("INSTALL 'parquet'"));
        assert!(migration.up.contains("LOAD 'parquet'"));

        // Verify down migration comment added
        assert!(!migration.down.is_empty());
        assert!(migration.down.contains("cannot be uninstalled"));
    }

    #[test]
    fn test_duckdb_drop_extension_generates_comment() {
        let generator = DuckDbSqlGenerator;
        let comment = generator.drop_extension("parquet");
        assert!(comment.starts_with("-- Extension"));
        assert!(comment.contains("parquet"));
        assert!(comment.contains("cannot be uninstalled"));
    }

    #[test]
    fn test_duckdb_generate_with_drop_extensions() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_extensions.push("parquet".to_string());

        let migration = generator.generate(&diff);
        assert!(!migration.up.is_empty());
        assert!(migration.up.starts_with("-- Extension"));
        assert!(migration.up.contains("parquet"));
        assert!(migration.up.contains("cannot be uninstalled"));
    }

    #[test]
    fn test_duckdb_create_enum() {
        let generator = DuckDbSqlGenerator;
        let enum_diff = EnumDiff {
            name: "order_status".to_string(),
            values: vec![
                "pending".to_string(),
                "active".to_string(),
                "archived".to_string(),
            ],
        };

        let sql = generator.create_enum(&enum_diff);
        assert!(sql.contains("CREATE TYPE"));
        assert!(sql.contains("\"order_status\""));
        assert!(sql.contains("ENUM"));
        assert!(sql.contains("'pending'"));
        assert!(sql.contains("'active'"));
        assert!(sql.contains("'archived'"));
    }

    #[test]
    fn test_duckdb_drop_enum() {
        let generator = DuckDbSqlGenerator;
        let sql = generator.drop_enum("order_status");
        assert!(sql.contains("DROP TYPE"));
        assert!(sql.contains("\"order_status\""));
    }

    #[test]
    fn test_duckdb_generate_with_enums() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_enums.push(EnumDiff {
            name: "status".to_string(),
            values: vec!["pending".to_string(), "active".to_string()],
        });

        let migration = generator.generate(&diff);
        assert!(!migration.up.is_empty());
        assert!(migration.up.contains("CREATE TYPE"));
        assert!(migration.up.contains("\"status\""));
        assert!(migration.up.contains("'pending'"));
        assert!(migration.up.contains("'active'"));

        // Check down migration
        assert!(migration.down.contains("DROP TYPE"));
        assert!(migration.down.contains("\"status\""));
    }

    #[test]
    fn test_duckdb_alter_enum() {
        let generator = DuckDbSqlGenerator;
        let alter = EnumAlterDiff {
            name: "order_status".to_string(),
            add_values: vec!["cancelled".to_string(), "refunded".to_string()],
            remove_values: vec![],
        };

        let stmts = generator.alter_enum(&alter);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("ALTER TYPE"));
        assert!(stmts[0].contains("\"order_status\""));
        assert!(stmts[0].contains("ADD VALUE"));
        assert!(stmts[0].contains("'cancelled'"));
        assert!(!stmts[0].contains("IF NOT EXISTS"));
        assert!(stmts[1].contains("'refunded'"));
    }

    #[test]
    fn test_duckdb_generate_with_alter_enums() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_enums.push(EnumAlterDiff {
            name: "status".to_string(),
            add_values: vec!["new_status".to_string()],
            remove_values: vec![],
        });

        let migration = generator.generate(&diff);
        assert!(!migration.up.is_empty());
        assert!(migration.up.contains("ALTER TYPE"));
        assert!(migration.up.contains("\"status\""));
        assert!(migration.up.contains("ADD VALUE"));
        assert!(migration.up.contains("'new_status'"));
    }

    #[test]
    fn test_duckdb_generate_with_drop_enums() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_enums.push("old_status".to_string());

        let migration = generator.generate(&diff);
        assert!(!migration.up.is_empty());
        assert!(migration.up.contains("DROP TYPE"));
        assert!(migration.up.contains("old_status"));
    }

    #[test]
    fn test_duckdb_create_table_with_identity_primary_key() {
        let generator = DuckDbSqlGenerator;
        let model = ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![
                FieldDiff {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    sql_type: "BIGINT".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: true,
                    is_auto_increment: true,
                    is_unique: false,
                    vector: None,
                },
                FieldDiff {
                    name: "email".to_string(),
                    column_name: "email".to_string(),
                    sql_type: "VARCHAR".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_unique: false,
                    vector: None,
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };

        let sql = generator.create_table(&model);
        assert!(sql.contains("CREATE TABLE \"users\""));
        assert!(sql.contains("\"id\" BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY"));
        assert!(sql.contains("\"email\" VARCHAR NOT NULL"));
    }

    #[test]
    fn test_duckdb_create_table_with_list_type() {
        let generator = DuckDbSqlGenerator;
        let model = ModelDiff {
            name: "Product".to_string(),
            table_name: "products".to_string(),
            fields: vec![
                FieldDiff {
                    name: "id".to_string(),
                    column_name: "id".to_string(),
                    sql_type: "BIGINT".to_string(),
                    nullable: false,
                    default: None,
                    is_primary_key: true,
                    is_auto_increment: true,
                    is_unique: false,
                    vector: None,
                },
                FieldDiff {
                    name: "tags".to_string(),
                    column_name: "tags".to_string(),
                    sql_type: "VARCHAR[]".to_string(),
                    nullable: false,
                    default: Some("[]".to_string()),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_unique: false,
                    vector: None,
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };

        let sql = generator.create_table(&model);
        assert!(sql.contains("CREATE TABLE \"products\""));
        assert!(sql.contains("\"tags\" VARCHAR[] NOT NULL DEFAULT []"));
    }

    #[test]
    fn test_duckdb_drop_table_generates_warning() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.drop_models.push("users".to_string());

        let migration = generator.generate(&diff);
        assert_eq!(migration.warnings.len(), 1);
        assert!(migration.warnings[0].contains("Dropping table 'users'"));
        assert!(migration.warnings[0].contains("all data will be lost"));
    }

    #[test]
    fn test_duckdb_alter_table_add_column() {
        let generator = DuckDbSqlGenerator;
        let alter = ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: vec![FieldDiff {
                name: "age".to_string(),
                column_name: "age".to_string(),
                sql_type: "INTEGER".to_string(),
                nullable: true,
                default: None,
                is_primary_key: false,
                is_auto_increment: false,
                is_unique: false,
                vector: None,
            }],
            drop_fields: Vec::new(),
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        };

        let statements = generator.alter_table(&alter);
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("ALTER TABLE \"users\" ADD COLUMN \"age\" INTEGER"));
    }

    #[test]
    fn test_duckdb_drop_column_generates_warning() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: vec!["old_field".to_string()],
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let migration = generator.generate(&diff);
        assert_eq!(migration.warnings.len(), 1);
        assert!(migration.warnings[0].contains("Dropping column 'old_field'"));
        assert!(migration.warnings[0].contains("users"));
        assert!(migration.warnings[0].contains("data in this column will be lost"));
    }

    #[test]
    fn test_duckdb_type_change_generates_warning() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: vec![FieldAlterDiff {
                name: "age".to_string(),
                column_name: "age".to_string(),
                old_type: Some("INTEGER".to_string()),
                new_type: Some("VARCHAR".to_string()),
                old_nullable: None,
                new_nullable: None,
                old_default: None,
                new_default: None,
            }],
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: Vec::new(),
            drop_foreign_keys: Vec::new(),
        });

        let migration = generator.generate(&diff);
        assert_eq!(migration.warnings.len(), 1);
        assert!(migration.warnings[0].contains("Changing column 'age'"));
        assert!(migration.warnings[0].contains("users"));
        assert!(migration.warnings[0].contains("reverse migration may fail"));
    }

    #[test]
    fn test_duckdb_create_index() {
        let generator = DuckDbSqlGenerator;
        let index = IndexDiff::new("idx_users_email", "users", vec!["email".to_string()]);

        let sql = generator.create_index(&index);
        assert_eq!(
            sql,
            "CREATE INDEX \"idx_users_email\" ON \"users\"(\"email\");"
        );
    }

    #[test]
    fn test_duckdb_create_unique_index() {
        let generator = DuckDbSqlGenerator;
        let index =
            IndexDiff::new("idx_users_email_unique", "users", vec!["email".to_string()]).unique();

        let sql = generator.create_index(&index);
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX \"idx_users_email_unique\" ON \"users\"(\"email\");"
        );
    }

    #[test]
    fn test_duckdb_create_index_in_generate() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_indexes.push(IndexDiff::new(
            "idx_posts_user",
            "posts",
            vec!["user_id".to_string()],
        ));

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("CREATE INDEX \"idx_posts_user\" ON \"posts\"(\"user_id\")")
        );
        assert!(
            migration
                .down
                .contains("DROP INDEX IF EXISTS \"idx_posts_user\"")
        );
    }

    #[test]
    fn test_duckdb_create_view() {
        let generator = DuckDbSqlGenerator;
        let view = ViewDiff {
            name: "ActiveUsers".to_string(),
            view_name: "active_users".to_string(),
            sql_query: "SELECT * FROM users WHERE active = true".to_string(),
            is_materialized: false,
            refresh_interval: None,
            fields: vec![],
        };

        let sql = generator.create_view(&view);
        assert_eq!(
            sql,
            "CREATE VIEW \"active_users\" AS\nSELECT * FROM users WHERE active = true;"
        );
    }

    #[test]
    fn test_duckdb_materialized_view_generates_warning() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.create_views.push(ViewDiff {
            name: "UserStats".to_string(),
            view_name: "user_stats".to_string(),
            sql_query: "SELECT COUNT(*) FROM users".to_string(),
            is_materialized: true,
            refresh_interval: None,
            fields: vec![],
        });

        let migration = generator.generate(&diff);
        let has_mat_warning = migration
            .warnings
            .iter()
            .any(|w| w.contains("materialized views") && w.contains("user_stats"));
        assert!(
            has_mat_warning,
            "Expected materialized view warning, got: {:?}",
            migration.warnings
        );
        // The view SQL should still be generated as a regular view
        assert!(migration.up.contains("CREATE VIEW \"user_stats\" AS"));
    }

    #[test]
    fn test_duckdb_foreign_key_generates_warning() {
        let generator = DuckDbSqlGenerator;
        let mut diff = SchemaDiff::default();
        diff.alter_models.push(ModelAlterDiff {
            name: "Post".to_string(),
            table_name: "posts".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: vec![ForeignKeyDiff {
                constraint_name: "fk_posts_user".to_string(),
                columns: vec!["user_id".to_string()],
                referenced_table: "users".to_string(),
                referenced_columns: vec!["id".to_string()],
                on_delete: None,
                on_update: None,
            }],
            drop_foreign_keys: Vec::new(),
        });

        let migration = generator.generate(&diff);
        let has_fk_warning = migration
            .warnings
            .iter()
            .any(|w| w.contains("Foreign keys") && w.contains("not enforced"));
        assert!(
            has_fk_warning,
            "Should warn about foreign key enforcement, got: {:?}",
            migration.warnings
        );
    }

    #[test]
    fn test_duckdb_foreign_key_sql_generated() {
        let generator = DuckDbSqlGenerator;
        let alter = ModelAlterDiff {
            name: "Post".to_string(),
            table_name: "posts".to_string(),
            add_fields: Vec::new(),
            drop_fields: Vec::new(),
            alter_fields: Vec::new(),
            add_indexes: Vec::new(),
            drop_indexes: Vec::new(),
            add_foreign_keys: vec![ForeignKeyDiff {
                constraint_name: "fk_posts_user".to_string(),
                columns: vec!["user_id".to_string()],
                referenced_table: "users".to_string(),
                referenced_columns: vec!["id".to_string()],
                on_delete: Some("CASCADE".to_string()),
                on_update: None,
            }],
            drop_foreign_keys: Vec::new(),
        };

        let stmts = generator.alter_table(&alter);
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].contains("ALTER TABLE \"posts\" ADD"));
        assert!(stmts[0].contains("CONSTRAINT \"fk_posts_user\""));
        assert!(stmts[0].contains("FOREIGN KEY (\"user_id\")"));
        assert!(stmts[0].contains("REFERENCES \"users\" (\"id\")"));
        assert!(stmts[0].contains("ON DELETE CASCADE"));
    }
}
