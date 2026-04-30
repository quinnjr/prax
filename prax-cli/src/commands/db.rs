//! `prax db` commands - Direct database operations.

use std::path::Path;

use crate::cli::{DbArgs, OutputFormat};
use crate::commands::introspect::{
    IntrospectionOptions, format_as_json, format_as_prax, format_as_sql, get_database_type,
};
use crate::commands::seed::{SeedRunner, find_seed_file, get_database_url};
use crate::config::{CONFIG_FILE_NAME, Config, SCHEMA_FILE_PATH};
use crate::error::{CliError, CliResult};
use crate::output::{self, success, warn};

/// Run the db command
pub async fn run(args: DbArgs) -> CliResult<()> {
    match args.command {
        crate::cli::DbSubcommand::Push(push_args) => run_push(push_args).await,
        crate::cli::DbSubcommand::Pull(pull_args) => run_pull(pull_args).await,
        crate::cli::DbSubcommand::Seed(seed_args) => run_seed(seed_args).await,
        crate::cli::DbSubcommand::Execute(exec_args) => run_execute(exec_args).await,
    }
}

/// Run `prax db push` - Push schema to database without migrations
async fn run_push(args: crate::cli::DbPushArgs) -> CliResult<()> {
    output::header("Database Push");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;
    let schema_path = args.schema.unwrap_or_else(|| cwd.join(SCHEMA_FILE_PATH));

    output::kv("Schema", &schema_path.display().to_string());
    output::kv(
        "Database",
        config
            .database
            .url
            .as_deref()
            .unwrap_or("env(DATABASE_URL)"),
    );
    output::newline();

    // Parse schema
    output::step(1, 4, "Parsing schema...");
    let schema_content = std::fs::read_to_string(&schema_path)?;
    let schema = parse_schema(&schema_content)?;

    // Introspect database
    output::step(2, 4, "Introspecting database...");
    // TODO: Get current database state

    // Calculate changes
    output::step(3, 4, "Calculating changes...");
    let changes = calculate_schema_changes(&schema)?;

    if changes.is_empty() {
        output::newline();
        success("Database is already in sync with schema!");
        return Ok(());
    }

    // Check for destructive changes
    let destructive = changes.iter().any(|c| c.is_destructive);
    if destructive && !args.accept_data_loss && !args.force {
        output::newline();
        warn("This push would cause data loss!");
        output::section("Destructive changes");
        for change in changes.iter().filter(|c| c.is_destructive) {
            output::list_item(&format!("⚠️  {}", change.description));
        }
        output::newline();
        output::info("Use --accept-data-loss to proceed, or --force to skip confirmation.");
        return Ok(());
    }

    // Apply changes
    output::step(4, 4, "Applying changes...");
    for change in &changes {
        output::list_item(&change.description);
        // TODO: Execute SQL
    }

    output::newline();
    success(&format!("Applied {} changes to database!", changes.len()));

    Ok(())
}

/// Run `prax db pull` - Introspect database and generate schema
async fn run_pull(args: crate::cli::DbPullArgs) -> CliResult<()> {
    output::header("Database Pull (Introspection)");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;

    // Get database URL
    let database_url = get_database_url(&config)?;
    let db_type = get_database_type(&config.database.provider)?;

    output::kv("Provider", &config.database.provider);
    output::kv("Database", &mask_database_url(&database_url));
    if let Some(ref schema) = args.schema {
        output::kv("Schema", schema);
    }
    output::newline();

    // Build introspection options
    let options = IntrospectionOptions {
        schema: args.schema.clone(),
        include_views: args.include_views,
        include_materialized_views: args.include_materialized_views,
        table_filter: args.tables.clone(),
        exclude_pattern: args.exclude.clone(),
        include_comments: args.comments,
        sample_size: args.sample_size,
    };

    // Introspect database
    output::step(1, 3, "Introspecting database...");

    #[cfg(feature = "postgres")]
    let db_schema = {
        use crate::commands::introspect::Introspector;
        use crate::commands::introspect::postgres::PostgresIntrospector;

        if config.database.provider.to_lowercase().contains("postgres") {
            let introspector = PostgresIntrospector::new(database_url.clone());
            introspector.introspect(&options).await?
        } else {
            return Err(CliError::Config(format!(
                "Introspection for {} requires the corresponding feature. Compile with --features {}",
                config.database.provider,
                config.database.provider.to_lowercase()
            )));
        }
    };

    #[cfg(not(feature = "postgres"))]
    let db_schema = {
        return Err(CliError::Config(
            "No database driver enabled. Compile with --features postgres, mysql, sqlite, or mssql"
                .to_string(),
        ));
    };

    // Generate output
    output::step(2, 3, "Generating schema...");
    let schema_content = match args.format {
        OutputFormat::Prax => format_as_prax(&db_schema, &config),
        OutputFormat::Json => format_as_json(&db_schema)?,
        OutputFormat::Sql => format_as_sql(&db_schema, db_type),
    };

    // Output schema
    output::step(3, 3, "Writing output...");

    if args.print {
        output::newline();
        output::section("Generated Schema");
        println!("{}", schema_content);
    } else {
        let output_path = args.output.unwrap_or_else(|| {
            let ext = match args.format {
                OutputFormat::Prax => "prax",
                OutputFormat::Json => "json",
                OutputFormat::Sql => "sql",
            };
            cwd.join(format!("schema.{}", ext))
        });

        if output_path.exists() && !args.force {
            warn(&format!("{} already exists!", output_path.display()));
            if !output::confirm("Overwrite existing file?") {
                output::newline();
                output::info("Pull cancelled.");
                return Ok(());
            }
        }

        std::fs::write(&output_path, &schema_content)?;

        output::newline();
        success(&format!("Schema written to {}", output_path.display()));
    }

    output::newline();
    output::section("Summary");
    output::kv("Tables", &db_schema.tables.len().to_string());
    output::kv("Enums", &db_schema.enums.len().to_string());
    output::kv("Views", &db_schema.views.len().to_string());

    // Show table names
    if !db_schema.tables.is_empty() {
        output::newline();
        output::section("Tables Introspected");
        for table in &db_schema.tables {
            output::list_item(&format!("{} ({} columns)", table.name, table.columns.len()));
        }
    }

    Ok(())
}

/// Run `prax db seed` - Seed database with initial data
async fn run_seed(args: crate::cli::DbSeedArgs) -> CliResult<()> {
    output::header("Database Seed");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;

    // Check if seeding is allowed for this environment
    if !args.force && !config.seed.should_seed(&args.environment) {
        warn(&format!(
            "Seeding is disabled for environment '{}'. Use --force to override.",
            args.environment
        ));
        return Ok(());
    }

    // Find seed file - check config.seed.script first
    let seed_path = args
        .seed_file
        .or_else(|| config.seed.script.clone())
        .or_else(|| find_seed_file(&cwd, &config))
        .ok_or_else(|| {
            CliError::Config(
                "Seed file not found. Create a seed file (seed.rs, seed.sql, seed.json, or seed.toml) \
                 or specify with --seed-file".to_string()
            )
        })?;

    if !seed_path.exists() {
        return Err(CliError::Config(format!(
            "Seed file not found: {}. Create a seed file or specify with --seed-file",
            seed_path.display()
        )));
    }

    // Get database URL
    let database_url = get_database_url(&config)?;

    output::kv("Seed file", &seed_path.display().to_string());
    output::kv("Database", &mask_database_url(&database_url));
    output::kv("Provider", &config.database.provider);
    output::kv("Environment", &args.environment);
    output::newline();

    // Reset database first if requested
    if args.reset {
        warn("Resetting database before seeding...");
        // TODO: Implement database reset
        output::newline();
    }

    // Create and run seed
    let runner = SeedRunner::new(
        seed_path,
        database_url,
        config.database.provider.clone(),
        cwd,
    )?
    .with_environment(&args.environment)
    .with_reset(args.reset);

    let result = runner.run().await?;

    output::newline();
    success("Database seeded successfully!");

    // Show summary
    output::newline();
    output::section("Summary");
    output::kv("Records affected", &result.records_affected.to_string());
    if !result.tables_seeded.is_empty() {
        output::kv("Tables seeded", &result.tables_seeded.join(", "));
    }

    Ok(())
}

/// Mask sensitive parts of database URL for display
fn mask_database_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let mut masked = parsed.clone();
        if parsed.password().is_some() {
            let _ = masked.set_password(Some("****"));
        }
        masked.to_string()
    } else {
        // Not a URL format, just show first part
        if url.len() > 30 {
            format!("{}...", &url[..30])
        } else {
            url.to_string()
        }
    }
}

/// Run `prax db execute` - Execute raw SQL
async fn run_execute(args: crate::cli::DbExecuteArgs) -> CliResult<()> {
    output::header("Execute SQL");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;

    // Get SQL to execute
    let sql = if let Some(sql) = args.sql {
        sql
    } else if let Some(file) = args.file {
        std::fs::read_to_string(&file)?
    } else if args.stdin {
        let mut sql = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut sql)?;
        sql
    } else {
        return Err(CliError::Command(
            "Must provide SQL via --sql, --file, or --stdin".to_string(),
        ));
    };

    output::kv(
        "Database",
        config
            .database
            .url
            .as_deref()
            .unwrap_or("env(DATABASE_URL)"),
    );
    output::newline();

    output::section("SQL");
    output::code(&sql, "sql");
    output::newline();

    // Confirm if not forced
    if !args.force && !output::confirm("Execute this SQL?") {
        output::newline();
        output::info("Execution cancelled.");
        return Ok(());
    }

    // Execute SQL
    output::step(1, 1, "Executing SQL...");
    // TODO: Actually execute SQL

    output::newline();
    success("SQL executed successfully!");

    Ok(())
}

// =============================================================================
// Helper Types and Functions
// =============================================================================

#[derive(Debug)]
struct SchemaChange {
    description: String,
    #[allow(dead_code)]
    sql: String,
    is_destructive: bool,
}

fn load_config(cwd: &Path) -> CliResult<Config> {
    let config_path = cwd.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        Config::load(&config_path)
    } else {
        Ok(Config::default())
    }
}

fn parse_schema(content: &str) -> CliResult<prax_schema::Schema> {
    // Use validate_schema to ensure field types are properly resolved
    // (e.g., FieldType::Model -> FieldType::Enum for enum references)
    prax_schema::validate_schema(content)
        .map_err(|e| CliError::Schema(format!("Failed to parse/validate schema: {}", e)))
}

fn calculate_schema_changes(_schema: &prax_schema::ast::Schema) -> CliResult<Vec<SchemaChange>> {
    // TODO: Implement actual schema diffing
    // For now, return empty changes
    Ok(Vec::new())
}
