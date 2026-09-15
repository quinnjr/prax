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

    let display_path = args
        .schema
        .as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| SCHEMA_FILE_PATH.to_string());
    output::kv("Schema", &display_path);
    output::kv(
        "Database",
        config
            .database
            .url
            .as_deref()
            .unwrap_or("env(DATABASE_URL)"),
    );
    output::newline();

    // Parse schema (still validates the schema file before failing)
    output::step(1, 1, "Parsing schema...");
    crate::schema_loader::load_schema(args.schema.as_deref())?;

    // Pushing requires diffing the parsed schema against the introspected
    // database state and executing the resulting SQL. The CLI has no
    // introspected database state to feed prax-migrate's differ here, so
    // fail honestly instead of claiming the database is in sync.
    Err(CliError::Command(
        "`prax db push` is not yet implemented: computing schema changes requires database \
         introspection and diffing, which are not available yet. Use \
         `prax migrate dev --create-only` to generate migration SQL, then apply it with an \
         external tool (psql, mysql, sqlite3)."
            .to_string(),
    ))
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

    let db_schema = crate::commands::introspect::introspect_database(
        &config.database.provider,
        &database_url,
        &options,
    )
    .await?;

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

    // The SQL shell-out helpers in seed.rs (execute_postgres_sql /
    // execute_mysql_sql / execute_sqlite_sql) are private to that module, so
    // there is no execution path available here. Fail honestly instead of
    // printing a success message for SQL that never ran.
    Err(CliError::Command(
        "`prax db execute` is not yet implemented: the CLI has no SQL execution path wired \
         up yet. Run this SQL with your database's native client (psql, mysql, sqlite3)."
            .to_string(),
    ))
}

// =============================================================================
// Helper Types and Functions
// =============================================================================

fn load_config(cwd: &Path) -> CliResult<Config> {
    let config_path = cwd.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        Config::load(&config_path)
    } else {
        Ok(Config::default())
    }
}
