//! Example demonstrating DuckDB migration SQL generation.
//!
//! This example shows:
//! - Creating a DuckDB SQL generator
//! - Generating up.sql / down.sql from schema diffs
//! - Extension support (INSTALL / LOAD)
//! - Analytical types (DuckDB array columns)
//! - Enum types and views
//! - Warnings for data-loss operations
//!
//! Run with: cargo run --example duckdb_migration -p prax-migrate

use prax_migrate::{
    DuckDbSqlGenerator, EnumDiff, ExtensionDiff, FieldAlterDiff, FieldDiff, ForeignKeyDiff,
    IndexDiff, ModelAlterDiff, ModelDiff, SchemaDiff, ViewDiff,
};

fn main() {
    println!("DuckDB Migration Example");
    println!("========================\n");

    let generator = DuckDbSqlGenerator;

    example_basic_table(&generator);
    example_analytical_types(&generator);
    example_extensions(&generator);
    example_enums_and_views(&generator);
    example_warnings(&generator);
}

// ---------------------------------------------------------------------------
// Example 1: Basic table with IDENTITY primary key
// ---------------------------------------------------------------------------

fn example_basic_table(generator: &DuckDbSqlGenerator) {
    println!("Example 1: Basic Table with IDENTITY Primary Key");
    println!("-------------------------------------------------");

    let mut diff = SchemaDiff::default();
    diff.create_models.push(ModelDiff {
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
            FieldDiff {
                name: "created_at".to_string(),
                column_name: "created_at".to_string(),
                sql_type: "TIMESTAMPTZ".to_string(),
                nullable: false,
                default: Some("NOW()".to_string()),
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
    });

    diff.create_indexes.push(
        IndexDiff::new("idx_users_email_unique", "users", vec!["email".to_string()]).unique(),
    );

    let migration = generator.generate(&diff);

    println!("up.sql:\n{}\n", migration.up);
    println!("down.sql:\n{}\n", migration.down);
    println!();
}

// ---------------------------------------------------------------------------
// Example 2: Analytical types (DuckDB arrays)
// ---------------------------------------------------------------------------

fn example_analytical_types(generator: &DuckDbSqlGenerator) {
    println!("Example 2: Analytical Types (DuckDB Arrays)");
    println!("--------------------------------------------");

    let mut diff = SchemaDiff::default();
    diff.create_models.push(ModelDiff {
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
            // VARCHAR[] — DuckDB array of strings (analogous to List<String>)
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
            // DECIMAL(10,2)[] — DuckDB array of decimals (nullable)
            FieldDiff {
                name: "prices".to_string(),
                column_name: "prices".to_string(),
                sql_type: "DECIMAL(10,2)[]".to_string(),
                nullable: true,
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
    });

    let migration = generator.generate(&diff);

    println!("up.sql:\n{}\n", migration.up);
    println!("Note: VARCHAR[] and DECIMAL(10,2)[] are native DuckDB array types.");
    println!();
}

// ---------------------------------------------------------------------------
// Example 3: Extension installation
// ---------------------------------------------------------------------------

fn example_extensions(generator: &DuckDbSqlGenerator) {
    println!("Example 3: Extension Support");
    println!("-----------------------------");

    let mut diff = SchemaDiff::default();
    diff.create_extensions.push(ExtensionDiff {
        name: "parquet".to_string(),
        schema: None,
        version: None,
    });
    diff.create_extensions.push(ExtensionDiff {
        name: "json".to_string(),
        schema: None,
        version: None,
    });

    let migration = generator.generate(&diff);

    println!("up.sql:\n{}\n", migration.up);
    println!("Note: DuckDB cannot uninstall extensions; down.sql contains comments only.");
    println!("down.sql:\n{}\n", migration.down);
    println!();
}

// ---------------------------------------------------------------------------
// Example 4: Enums and views
// ---------------------------------------------------------------------------

fn example_enums_and_views(generator: &DuckDbSqlGenerator) {
    println!("Example 4: Enums and Views");
    println!("--------------------------");

    let mut diff = SchemaDiff::default();

    diff.create_enums.push(EnumDiff {
        name: "order_status".to_string(),
        values: vec![
            "pending".to_string(),
            "active".to_string(),
            "archived".to_string(),
        ],
    });

    diff.create_models.push(ModelDiff {
        name: "Order".to_string(),
        table_name: "orders".to_string(),
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
                name: "status".to_string(),
                column_name: "status".to_string(),
                sql_type: "order_status".to_string(), // references the enum above
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
    });

    // Regular (non-materialized) view
    diff.create_views.push(ViewDiff {
        name: "ActiveOrders".to_string(),
        view_name: "active_orders".to_string(),
        sql_query: "SELECT * FROM orders WHERE status = 'active'".to_string(),
        is_materialized: false,
        refresh_interval: None,
        fields: vec![],
    });

    // Materialized view — DuckDB does not support these; will be created as a
    // regular view with a warning
    diff.create_views.push(ViewDiff {
        name: "OrderSummary".to_string(),
        view_name: "order_summary".to_string(),
        sql_query: "SELECT status, COUNT(*) AS cnt FROM orders GROUP BY status".to_string(),
        is_materialized: true,
        refresh_interval: None,
        fields: vec![],
    });

    let migration = generator.generate(&diff);

    println!("up.sql:\n{}\n", migration.up);
    println!("down.sql:\n{}\n", migration.down);

    if !migration.warnings.is_empty() {
        println!("Warnings:");
        for w in &migration.warnings {
            println!("  ! {}", w);
        }
    }
    println!();
}

// ---------------------------------------------------------------------------
// Example 5: Data-loss warnings
// ---------------------------------------------------------------------------

fn example_warnings(generator: &DuckDbSqlGenerator) {
    println!("Example 5: Data-Loss Warnings");
    println!("------------------------------");

    let mut diff = SchemaDiff::default();

    // Dropping a whole table
    diff.drop_models.push("legacy_users".to_string());

    // Alter: drop a column and change a column type
    diff.alter_models.push(ModelAlterDiff {
        name: "Product".to_string(),
        table_name: "products".to_string(),
        add_fields: vec![],
        drop_fields: vec!["deprecated_field".to_string()],
        alter_fields: vec![FieldAlterDiff {
            name: "price".to_string(),
            column_name: "price".to_string(),
            old_type: Some("INTEGER".to_string()),
            new_type: Some("DECIMAL(10,2)".to_string()),
            old_nullable: None,
            new_nullable: None,
            old_default: None,
            new_default: None,
        }],
        add_indexes: vec![],
        drop_indexes: vec![],
        // Adding a foreign key triggers the "not enforced by default" warning
        add_foreign_keys: vec![ForeignKeyDiff {
            constraint_name: "fk_products_category".to_string(),
            columns: vec!["category_id".to_string()],
            referenced_table: "categories".to_string(),
            referenced_columns: vec!["id".to_string()],
            on_delete: Some("SET NULL".to_string()),
            on_update: None,
        }],
        drop_foreign_keys: vec![],
    });

    let migration = generator.generate(&diff);

    println!("up.sql:\n{}\n", migration.up);
    println!("Warnings ({}):", migration.warnings.len());
    for w in &migration.warnings {
        println!("  ! {}", w);
    }
    println!();
}
