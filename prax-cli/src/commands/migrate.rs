//! `prax migrate` commands - Database migration management.

use std::path::{Path, PathBuf};

use prax_migrate::timescale::comment_safe;

use crate::cli::MigrateArgs;
use crate::commands::seed::{SeedRunner, find_seed_file, get_database_url};
use crate::config::{CONFIG_FILE_NAME, Config, MIGRATIONS_DIR, SCHEMA_FILE_PATH};
use crate::error::{CliError, CliResult};
use crate::output::{self, success, warn};

/// Run the migrate command
pub async fn run(args: MigrateArgs) -> CliResult<()> {
    match args.command {
        crate::cli::MigrateSubcommand::Dev(dev_args) => run_dev(dev_args).await,
        crate::cli::MigrateSubcommand::Deploy => run_deploy().await,
        crate::cli::MigrateSubcommand::Reset(reset_args) => run_reset(reset_args).await,
        crate::cli::MigrateSubcommand::Status => run_status().await,
        crate::cli::MigrateSubcommand::Resolve(resolve_args) => run_resolve(resolve_args).await,
        crate::cli::MigrateSubcommand::Diff(diff_args) => run_diff(diff_args).await,
        crate::cli::MigrateSubcommand::Rollback(rollback_args) => run_rollback(rollback_args).await,
        crate::cli::MigrateSubcommand::History(history_args) => run_history(history_args).await,
    }
}

/// Run `prax migrate dev` - development migration workflow
async fn run_dev(args: crate::cli::MigrateDevArgs) -> CliResult<()> {
    output::header("Migrate Dev");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;

    let migrations_dir = cwd.join(MIGRATIONS_DIR);

    let display_path = args
        .schema
        .as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| SCHEMA_FILE_PATH.to_string());
    output::kv("Schema", &display_path);
    output::kv("Migrations", &migrations_dir.display().to_string());
    output::newline();

    // Determine total steps (5 or 6 depending on seed)
    let total_steps = if args.skip_seed { 5 } else { 6 };

    // 1. Parse and validate schema
    output::step(1, total_steps, "Parsing schema...");
    let loaded = crate::schema_loader::load_schema(args.schema.as_deref())?;
    let schema = loaded.schema;

    // 2. Check for pending migrations
    output::step(2, total_steps, "Checking migration status...");
    let pending = check_pending_migrations(&migrations_dir)?;

    if !pending.is_empty() {
        output::list(&format!("{} pending migrations found:", pending.len()));
        for migration in &pending {
            output::list_item(&migration.display().to_string());
        }
        output::newline();
    }

    // 3. Diff schema against database
    output::step(3, total_steps, "Comparing schema to database...");
    let migration_name = args
        .name
        .unwrap_or_else(|| format!("migration_{}", chrono::Utc::now().format("%Y%m%d%H%M%S")));

    // Resolve the current database structure as the diff source (introspected),
    // or None when no database is reachable (greenfield → full-creation DDL).
    let source = resolve_source_schema(&config, &schema).await?;
    let generated = generate_migration_sql(&schema, source, &config, args.allow_destructive)?;

    // 4. Generate migration
    output::step(4, total_steps, "Generating migration...");
    let Some(migration_sql) = generated.statements_sql() else {
        output::newline();
        report_no_statements(&generated, "No migration created.");
        return Ok(());
    };
    print_notes(&generated);
    let migration_path = create_migration(&migrations_dir, &migration_name, migration_sql)?;

    // 5. Apply migration (if not --create-only)
    if !args.create_only {
        output::step(5, total_steps, "Applying migration...");
        apply_migration(&migration_path, &config).await?;
    } else {
        output::step(5, total_steps, "Skipping apply (--create-only)...");
    }

    // 6. Run seed (if not --skip-seed)
    if !args.skip_seed && !args.create_only {
        output::step(6, total_steps, "Running seed...");

        if let Some(seed_path) = find_seed_file(&cwd, &config) {
            let database_url = get_database_url(&config)?;
            let runner = SeedRunner::new(
                seed_path,
                database_url,
                config.database.provider.clone(),
                cwd.clone(),
            )?;

            match runner.run().await {
                Ok(result) => {
                    output::list_item(&format!("Seeded {} records", result.records_affected));
                }
                Err(e) => {
                    output::warn(&format!("Seed failed: {}. Continuing...", e));
                }
            }
        } else {
            output::list_item("No seed file found, skipping");
        }
    }

    output::newline();
    success(&format!("Migration '{}' created", migration_name));

    output::newline();
    output::section("Next steps");
    output::list_item("Review the generated migration SQL");
    output::list_item("Run `prax generate` to update your client");

    Ok(())
}

/// Run `prax migrate deploy` - production deployment
async fn run_deploy() -> CliResult<()> {
    output::header("Migrate Deploy");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;
    let migrations_dir = cwd.join(MIGRATIONS_DIR);

    output::kv("Migrations", &migrations_dir.display().to_string());
    output::newline();

    // Check for pending migrations
    output::step(1, 3, "Checking for pending migrations...");
    let pending = check_pending_migrations(&migrations_dir)?;

    if pending.is_empty() {
        output::newline();
        success("No pending migrations to apply.");
        return Ok(());
    }

    output::list(&format!("{} pending migrations:", pending.len()));
    for migration in &pending {
        output::list_item(&migration.file_name().unwrap().to_string_lossy());
    }
    output::newline();

    // Apply migrations
    output::step(2, 3, "Applying migrations...");
    for migration in &pending {
        output::list_item(&format!(
            "Applying {}",
            migration.file_name().unwrap().to_string_lossy()
        ));
        apply_migration(migration, &config).await?;
    }

    // Verify
    output::step(3, 3, "Verifying migrations...");

    output::newline();
    success(&format!(
        "Applied {} migrations successfully!",
        pending.len()
    ));

    Ok(())
}

/// Run `prax migrate reset` - reset database
async fn run_reset(args: crate::cli::MigrateResetArgs) -> CliResult<()> {
    output::header("Migrate Reset");

    let cwd = std::env::current_dir()?;
    let _config = load_config(&cwd)?;

    if !args.force {
        warn("This will delete all data in the database!");
        output::newline();
        if !output::confirm("Are you sure you want to reset the database?") {
            output::newline();
            output::info("Reset cancelled.");
            return Ok(());
        }
    }

    output::newline();

    // Honest failure: drop/create database and re-applying migrations require a
    // database executor that is not yet wired into the CLI. No changes are made.
    Err(CliError::Migration(
        "migrate reset is not yet implemented: dropping and recreating the database \
         requires a database executor that is not yet wired into the CLI. No changes \
         were made to the database."
            .to_string(),
    ))
}

/// Run `prax migrate status` - show migration status
async fn run_status() -> CliResult<()> {
    output::header("Migration Status");

    let cwd = std::env::current_dir()?;
    let _config = load_config(&cwd)?;
    let migrations_dir = cwd.join(MIGRATIONS_DIR);

    // List all migrations
    let mut migrations = Vec::new();
    if migrations_dir.exists() {
        for entry in std::fs::read_dir(&migrations_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                migrations.push(path);
            }
        }
    }
    migrations.sort();

    if migrations.is_empty() {
        output::info("No migrations found.");
        output::newline();
        output::section("Getting started");
        output::list_item("Run `prax migrate dev` to create your first migration");
        return Ok(());
    }

    output::section("Migrations");

    for (i, migration) in migrations.iter().enumerate() {
        let name = migration.file_name().unwrap().to_string_lossy();
        let applied = is_migration_applied(migration)?;

        let status = if applied {
            output::style_success("✓ Applied")
        } else {
            output::style_pending("○ Pending")
        };

        output::numbered_item(i + 1, &format!("{} - {}", name, status));
    }

    output::newline();

    let applied_count = migrations
        .iter()
        .filter(|m| is_migration_applied(m).unwrap_or(false))
        .count();
    let pending_count = migrations.len() - applied_count;

    output::kv("Total", &migrations.len().to_string());
    output::kv("Applied", &applied_count.to_string());
    output::kv("Pending", &pending_count.to_string());

    Ok(())
}

/// Run `prax migrate resolve` - resolve migration issues
async fn run_resolve(args: crate::cli::MigrateResolveArgs) -> CliResult<()> {
    output::header("Migrate Resolve");

    if !args.applied && !args.rolled_back {
        return Err(CliError::Command(
            "Must specify --applied or --rolled-back".to_string(),
        ));
    }

    // Honest failure: resolving requires writing to the migration history table,
    // which is not yet wired into the CLI. No changes are made.
    Err(CliError::Migration(format!(
        "migrate resolve is not yet implemented: marking migration '{}' as {} \
         requires updating the _prax_migrations history table, which is not yet \
         wired into the CLI. No changes were made.",
        args.migration,
        if args.applied {
            "applied"
        } else {
            "rolled back"
        }
    )))
}

/// Run `prax migrate diff` - generate schema DDL without applying
async fn run_diff(args: crate::cli::MigrateDiffArgs) -> CliResult<()> {
    output::header("Migrate Diff");

    let cwd = std::env::current_dir()?;
    let config = load_config(&cwd)?;

    // Diffing against a stored migration requires a migration snapshot store,
    // which is not wired into the CLI. The live-database source is supported
    // (that is the whole point of this command); a *specific past migration*
    // as the source is not.
    if let Some(from_migration) = &args.from_migration {
        return Err(CliError::Migration(format!(
            "--from-migration '{}' is not supported: diffing against a specific \
             migration requires a migration snapshot store, which is not yet \
             wired into the CLI. Omit --from-migration to diff against the live \
             database (or an empty schema when no database is reachable).",
            from_migration
        )));
    }

    // Parse the desired (target) schema.
    output::step(1, 2, "Parsing schema...");
    let loaded = crate::schema_loader::load_schema(args.schema.as_deref())?;
    let schema = loaded.schema;

    // Resolve the current database structure as the diff source (introspected),
    // or None when no database is reachable (greenfield → full-creation DDL).
    output::step(2, 2, "Comparing schema to database...");
    let source = resolve_source_schema(&config, &schema).await?;
    let generated = generate_migration_sql(&schema, source, &config, args.allow_destructive)?;
    let ddl_sql = generated.statements_sql().unwrap_or_default();

    output::newline();
    if !generated.has_statements {
        report_no_statements(&generated, "No DDL generated.");
    }

    output::newline();
    output::section("Generated DDL");
    output::code(ddl_sql, "sql");

    if let Some(output_path) = args.output {
        std::fs::write(&output_path, ddl_sql)?;
        output::newline();
        success(&format!("DDL written to {}", output_path.display()));
    }

    Ok(())
}

/// Run `prax migrate rollback` - rollback the last applied migration
async fn run_rollback(args: crate::cli::MigrateRollbackArgs) -> CliResult<()> {
    output::header("Migrate Rollback");

    output::newline();

    if let Some(to_migration) = &args.to {
        output::info(&format!("Rolling back to migration: {}", to_migration));
    } else {
        output::info("Rolling back last applied migration...");
    }

    if let Some(reason) = &args.reason {
        output::kv("Reason", reason);
    }

    if let Some(user) = &args.user {
        output::kv("User", user);
    }

    output::newline();

    // TODO: Implement actual rollback logic using event sourcing
    // The real implementation would:
    // 1. Load the event store
    // 2. Find the last applied migration (or specified migration)
    // 3. Append a RolledBack event
    // 4. Execute the down migration SQL
    // 5. Update migration state

    // Honest failure: rollback requires the prax-migrate event-sourcing engine,
    // which is not yet wired into the CLI. Exit non-zero — no changes are made.
    Err(CliError::Migration(
        "migrate rollback is not yet implemented: rolling back requires the \
         prax-migrate event-sourcing engine (event store and down-migration \
         execution), which is not yet wired into the CLI. No changes were made."
            .to_string(),
    ))
}

/// Run `prax migrate history` - view migration history
async fn run_history(args: crate::cli::MigrateHistoryArgs) -> CliResult<()> {
    output::header("Migration History");

    output::newline();

    if let Some(migration) = &args.migration {
        output::section(&format!("History for migration: {}", migration));
    } else {
        output::section("All migrations");
    }

    output::newline();

    // TODO: Implement actual history viewing using event sourcing
    // The real implementation would:
    // 1. Load the event store
    // 2. Query events for the specified migration (or all)
    // 3. Display events in chronological order
    // 4. Show event type, timestamp, and event-specific data

    // Honest failure: history requires reading the _prax_migrations event log,
    // which is not yet wired into the CLI. Exit non-zero.
    Err(CliError::Migration(
        "migrate history is not yet implemented: viewing history requires reading \
         the _prax_migrations event log, which is not yet wired into the CLI."
            .to_string(),
    ))
}

// =============================================================================
// Helper Functions
// =============================================================================

fn load_config(cwd: &Path) -> CliResult<Config> {
    let config_path = cwd.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        Config::load(&config_path)
    } else {
        Ok(Config::default())
    }
}

fn check_pending_migrations(migrations_dir: &Path) -> CliResult<Vec<PathBuf>> {
    let mut pending = Vec::new();

    if !migrations_dir.exists() {
        return Ok(pending);
    }

    for entry in std::fs::read_dir(migrations_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && !is_migration_applied(&path)? {
            pending.push(path);
        }
    }

    pending.sort();
    Ok(pending)
}

fn is_migration_applied(migration_path: &Path) -> CliResult<bool> {
    // Check for a marker file indicating the migration has been applied
    // In production, this would check the migration history table
    let marker = migration_path.join(".applied");
    Ok(marker.exists())
}

fn create_migration(migrations_dir: &Path, name: &str, sql: &str) -> CliResult<PathBuf> {
    // Create migration directory
    let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let migration_name = format!("{}_{}", timestamp, name);
    let migration_path = migrations_dir.join(&migration_name);

    std::fs::create_dir_all(&migration_path)?;

    // Write migration.sql
    let sql_path = migration_path.join("migration.sql");
    std::fs::write(&sql_path, sql)?;

    Ok(migration_path)
}

/// Resolve the current database structure as a diff *source* schema.
///
/// Resolution order per the incremental-migrations design:
/// 1. Introspect the database (default when a `DATABASE_URL` resolves and the
///    provider is supported) and map the result to a `prax_schema::Schema`.
/// 2. When no database URL is configured, or the database is unreachable, or
///    the provider does not support introspection yet, return `None` — the
///    differ then emits full-creation DDL, preserving the greenfield
///    `init` → first `migrate dev` flow.
///
/// The database itself (via introspection) — not the `_prax_migrations`
/// history — is the source of truth for structure, so a database migrated by
/// a foreign runner (empty/absent prax history) still diffs off its real
/// structure.
///
/// The TimescaleDB catalog is read only when `target` opts in
/// ([`prax_migrate::timescale::opts_into_timescale`]).
async fn resolve_source_schema(
    config: &Config,
    target: &prax_schema::ast::Schema,
) -> CliResult<Option<DiffSource>> {
    // No URL configured/available → greenfield source.
    let Ok(database_url) = get_database_url(config) else {
        output::list_item("No DATABASE_URL configured; treating as a new database.");
        return Ok(None);
    };

    introspect_source_schema(
        config,
        &database_url,
        prax_migrate::timescale::opts_into_timescale(target),
    )
    .await
}

/// The introspected diff source: the database structure plus, for Postgres,
/// the installed extensions and (when the target opts in) the TimescaleDB
/// catalog.
///
/// Public only so the live integration suite can drive
/// [`generate_migration_sql`]; not a stable API.
#[doc(hidden)]
pub struct DiffSource {
    /// Database structure mapped to a Prax schema.
    pub schema: prax_schema::ast::Schema,
    /// Postgres extension and TimescaleDB state (`None` for other providers).
    pub catalog: Option<crate::commands::introspect::PostgresCatalog>,
}

impl From<prax_schema::ast::Schema> for DiffSource {
    fn from(schema: prax_schema::ast::Schema) -> Self {
        Self {
            schema,
            catalog: None,
        }
    }
}

/// Introspect `database_url` and map the result to a diff-source schema.
///
/// Dispatches to the backend matching the configured provider (PostgreSQL,
/// MySQL, SQLite, MSSQL), each behind its cargo feature. For Postgres the
/// installed extensions are always read, the TimescaleDB catalog only when
/// `with_timescale` is set.
///
/// Error handling splits two cases so `migrate dev` is both safe and honest:
/// - **Unreachable database** ([`CliError::Unreachable`]) or a provider whose
///   introspection feature was not compiled in → treated as "no source"
///   (greenfield, with a warning), so first-time creation still works offline.
/// - **Reachable but the introspection query/permission failed**
///   ([`CliError::Database`] and anything else) → propagated as a hard error,
///   rather than silently emitting full-creation DDL against a populated
///   database.
async fn introspect_source_schema(
    config: &Config,
    database_url: &str,
    with_timescale: bool,
) -> CliResult<Option<DiffSource>> {
    use crate::commands::introspect::{
        IntrospectionOptions, introspect_database, introspect_postgres_catalog,
    };
    use crate::commands::schema_from_db::schema_from_database;

    // Scope introspection to a single schema/database where the provider needs
    // it. MySQL's `information_schema` spans every database on the server, so
    // without a `table_schema` filter the diff source would pull in unrelated
    // schemas; derive the database name from the connection URL.
    let mut options = IntrospectionOptions::default();
    if is_mysql(&config.database.provider) {
        options.schema = mysql_database_from_url(database_url);
    }

    match introspect_database(&config.database.provider, database_url, &options).await {
        Ok(db_schema) => {
            let result = schema_from_database(&db_schema, Default::default())?;
            for warning in &result.warnings {
                output::warn(warning);
            }
            let catalog = introspect_postgres_catalog(
                &config.database.provider,
                database_url,
                options.schema.as_deref(),
                with_timescale,
            )
            .await?;
            Ok(Some(DiffSource {
                schema: result.schema,
                catalog,
            }))
        }
        // Only "no database reachable", or a backend whose introspection
        // feature is not compiled in, falls back to greenfield. A reachable
        // database whose query/permission failed is a real error.
        Err(CliError::Unreachable(msg)) => {
            output::warn(&format!(
                "Database not reachable ({msg}); treating as a new database. \
                 Generated SQL will be full-creation DDL."
            ));
            Ok(None)
        }
        Err(CliError::FeatureUnavailable(msg)) => {
            output::list_item(&format!("{msg} Using an empty source (full-creation DDL)."));
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Whether the provider string denotes MySQL/MariaDB.
fn is_mysql(provider: &str) -> bool {
    matches!(provider.to_lowercase().as_str(), "mysql" | "mariadb")
}

/// Extract the database name from a MySQL connection URL
/// (`mysql://user:pass@host:port/DBNAME?params`). Returns `None` when no
/// path segment is present.
///
/// Credentials are stripped first (split at the last `@`) so a userinfo
/// component containing a `/` does not get mistaken for the path separator.
fn mysql_database_from_url(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    // Drop the `user:pass@` userinfo so its characters can't be read as the
    // path. The host/port/path remainder is everything after the last `@`.
    let host_and_path = match after_scheme.rsplit_once('@') {
        Some((_userinfo, rest)) => rest,
        None => after_scheme,
    };
    let after_authority = host_and_path.split_once('/')?.1;
    let db = after_authority
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_matches('/');
    if db.is_empty() {
        None
    } else {
        Some(db.to_string())
    }
}

/// Map a datasource provider string to a migration `SqlBackend`.
///
/// The provider table is kept in sync with
/// [`crate::commands::introspect::get_database_type`]: every provider that can
/// be a diff *source* (introspected) must also render *target* SQL here.
/// DuckDB is intentionally absent from both — it has no introspector yet — so
/// `migrate dev` against DuckDB fails cleanly rather than diffing against an
/// empty source.
fn sql_backend_for_provider(provider: &str) -> CliResult<prax_migrate::SqlBackend> {
    use prax_migrate::SqlBackend;
    match provider.to_lowercase().as_str() {
        "postgresql" | "postgres" | "pg" => Ok(SqlBackend::Postgres),
        "mysql" | "mariadb" => Ok(SqlBackend::MySql),
        "sqlite" | "sqlite3" => Ok(SqlBackend::Sqlite),
        "mssql" | "sqlserver" | "sql_server" => Ok(SqlBackend::Mssql),
        other => Err(CliError::Config(format!(
            "Unsupported database provider for migration generation: '{}'",
            other
        ))),
    }
}

/// The outcome of [`generate_migration_sql`].
///
/// Public only so the live integration suite can drive
/// [`generate_migration_sql`]; not a stable API.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct GeneratedMigration {
    /// Header, `-- WARNING:` lines, `-- SKIPPED` lines, then the `up` SQL.
    pub sql: String,
    /// Whether `up` contains any statement.
    pub has_statements: bool,
    /// Notes for changes skipped for lacking `--allow-destructive`.
    pub skipped: Vec<String>,
    /// Warnings raised while generating the migration.
    pub warnings: Vec<String>,
}

impl GeneratedMigration {
    /// The migration text to write or print, or `None` when `up` holds no
    /// statement (the text would only be comments).
    #[must_use]
    pub fn statements_sql(&self) -> Option<&str> {
        self.has_statements.then_some(self.sql.as_str())
    }
}

/// Print every warning and skipped change, then, when changes were skipped,
/// how many and how to include them. Database-sourced names in the notes are
/// kept on one line.
fn print_notes(generated: &GeneratedMigration) {
    for warning in &generated.warnings {
        output::warn(&comment_safe(warning));
    }
    for note in &generated.skipped {
        output::warn(&comment_safe(note));
    }
    if !generated.skipped.is_empty() {
        output::warn(&format!(
            "{} change(s) skipped — re-run with --allow-destructive to include them.",
            generated.skipped.len()
        ));
    }
}

/// Explain an empty migration: print any warnings and skipped changes, then
/// either the "No changes" message or the "does not match" message.
fn report_no_statements(generated: &GeneratedMigration, tail: &str) {
    print_notes(generated);
    if generated.skipped.is_empty() {
        success(&format!(
            "No changes: the database already matches the schema. {tail}"
        ));
    } else {
        output::warn(&format!("The database does not match the schema. {tail}"));
    }
}

/// Append one `-- {prefix}{line}` comment per entry of `lines`. Entries may
/// carry database-sourced names, so each is first made safe for a one-line
/// comment: a raw newline would otherwise end the comment and turn the rest
/// of the name into executable SQL.
fn push_comment_lines(out: &mut String, prefix: &str, lines: &[String]) {
    for line in lines {
        out.push_str("-- ");
        out.push_str(prefix);
        out.push_str(&comment_safe(line));
        out.push('\n');
    }
}

/// Diff the desired `schema` (target) against an optional introspected
/// `source`, render the resulting `SchemaDiff` through the provider's dialect
/// generator, and return a [`GeneratedMigration`]: the migration text (header,
/// warnings, skipped notes, then the `up` SQL), whether it has statements, and
/// the skipped notes and warnings for reporting.
///
/// When `allow_destructive` is false (the default), drops (tables, columns,
/// enums, foreign keys, indexes, enum-value removals) and gated TimescaleDB
/// changes are stripped from the diff before generation so a stale schema
/// never silently destroys data. The source's TimescaleDB catalog is passed to
/// the differ only when `schema` opts in
/// ([`prax_migrate::timescale::opts_into_timescale`]).
///
/// Public only so the live integration suite can drive the real CLI path;
/// not a stable API.
///
/// # Errors
///
/// Returns [`CliError::Config`] for a provider with no SQL backend, and
/// [`CliError::Migration`] when `schema` opts into TimescaleDB but the
/// installed extension is older than
/// [`prax_migrate::timescale::TIMESCALE_MIN_VERSION`], or when the diff fails.
#[doc(hidden)]
pub fn generate_migration_sql(
    schema: &prax_schema::ast::Schema,
    source: Option<DiffSource>,
    config: &Config,
    allow_destructive: bool,
) -> CliResult<GeneratedMigration> {
    use prax_migrate::{SchemaDiffer, SqlDialect};

    let backend = sql_backend_for_provider(&config.database.provider)?;

    // One opt-in decision drives both the version gate and the catalog
    // hand-off, so they can never disagree.
    let opted_in = prax_migrate::timescale::opts_into_timescale(schema);

    let differ = SchemaDiffer::new(schema.clone());
    let differ = match source {
        Some(src) => {
            let mut differ = differ.with_source(src.schema);
            if let Some(catalog) = src.catalog {
                if opted_in
                    && let Some(ts) = &catalog.timescale
                    && !prax_migrate::timescale::timescale_version_supported(&ts.version)
                {
                    let (major, minor) = prax_migrate::timescale::TIMESCALE_MIN_VERSION;
                    return Err(CliError::Migration(format!(
                        "TimescaleDB {} is installed, but the schema opts into TimescaleDB \
                         (`extensions = [timescaledb]` or `@@timescale.*`) and Prax requires \
                         TimescaleDB {major}.{minor} or newer. Upgrade the extension.",
                        ts.version
                    )));
                }
                differ = differ.with_installed_extensions(catalog.extensions);
                if opted_in && let Some(ts) = catalog.timescale {
                    differ = differ.with_timescale_catalog(ts);
                }
            }
            differ
        }
        None => differ,
    };

    let mut diff = differ
        .diff()
        .map_err(|e| CliError::Migration(format!("Failed to diff schema against database: {e}")))?;

    let skipped = if allow_destructive {
        Vec::new()
    } else {
        strip_destructive(&mut diff)
    };

    let migration = SqlDialect::for_backend(backend).generate_migration(&diff);

    let mut out = String::from("-- Migration generated by Prax\n");
    if !allow_destructive {
        out.push_str(
            "-- Additive-only: destructive statements (DROP) are omitted. Re-run with \
             --allow-destructive to include them.\n",
        );
    }
    push_comment_lines(&mut out, "WARNING: ", &migration.warnings);
    push_comment_lines(&mut out, "SKIPPED (needs --allow-destructive): ", &skipped);
    out.push('\n');
    out.push_str(migration.up.trim_end());
    if !migration.up.trim_end().is_empty() {
        out.push('\n');
    }

    Ok(GeneratedMigration {
        sql: out,
        has_statements: !migration.up.trim().is_empty(),
        skipped,
        warnings: migration.warnings.clone(),
    })
}

/// Remove every destructive operation from a `SchemaDiff` in place, leaving
/// only additive/altering changes. Column *type/nullability/default* alters
/// are kept (they are not drops); dropped columns, tables, enums, enum values,
/// foreign keys, indexes, extensions, procedures, and triggers are removed,
/// followed by gated TimescaleDB changes. Returns one note per removed item.
///
/// `alter_views` is intentionally **kept**: a view alter recreates the view
/// (drop + create), but views hold no data, so recreation is non-destructive
/// and safe under the additive-only default.
fn strip_destructive(diff: &mut prax_migrate::SchemaDiff) -> Vec<String> {
    let mut notes = Vec::new();

    for name in std::mem::take(&mut diff.drop_models) {
        notes.push(format!("Drop table \"{name}\""));
    }
    for name in std::mem::take(&mut diff.drop_enums) {
        notes.push(format!("Drop enum \"{name}\""));
    }
    for name in std::mem::take(&mut diff.drop_views) {
        notes.push(format!("Drop view \"{name}\""));
    }
    for name in std::mem::take(&mut diff.drop_extensions) {
        notes.push(format!("Drop extension \"{name}\""));
    }
    for index in std::mem::take(&mut diff.drop_indexes) {
        notes.push(format!("Drop index \"{}\"", index.name));
    }

    for alter in &mut diff.alter_models {
        for column in std::mem::take(&mut alter.drop_fields) {
            notes.push(format!(
                "Drop column \"{column}\" from table \"{}\"",
                alter.table_name
            ));
        }
        for index in std::mem::take(&mut alter.drop_indexes) {
            notes.push(format!("Drop index \"{index}\""));
        }
        for fk in std::mem::take(&mut alter.drop_foreign_keys) {
            notes.push(format!(
                "Drop foreign key \"{fk}\" on table \"{}\"",
                alter.table_name
            ));
        }
    }
    // An alter that now carries no changes would still be harmless (the
    // generator emits nothing for it), but drop the empties for a clean diff.
    diff.alter_models.retain(|a| {
        !a.add_fields.is_empty()
            || !a.alter_fields.is_empty()
            || !a.add_indexes.is_empty()
            || !a.add_foreign_keys.is_empty()
    });

    for alter in &mut diff.alter_enums {
        for value in std::mem::take(&mut alter.remove_values) {
            notes.push(format!(
                "Remove value \"{value}\" from enum \"{}\"",
                alter.name
            ));
        }
    }
    diff.alter_enums.retain(|a| !a.add_values.is_empty());

    // Procedure and trigger drops are destructive too. The differ carries them
    // on `SchemaDiff::procedures`; clear both channels and drop the whole
    // procedure diff if nothing additive/altering remains.
    if let Some(procs) = diff.procedures.as_mut() {
        for name in std::mem::take(&mut procs.drop) {
            notes.push(format!("Drop procedure \"{name}\""));
        }
        for name in std::mem::take(&mut procs.drop_triggers) {
            notes.push(format!("Drop trigger \"{name}\""));
        }
        if procs.is_empty() {
            diff.procedures = None;
        }
    }

    notes.extend(diff.strip_timescale_gated());
    notes
}

async fn apply_migration(migration_path: &Path, _config: &Config) -> CliResult<()> {
    let sql_path = migration_path.join("migration.sql");

    if !sql_path.exists() {
        return Err(CliError::Migration(format!(
            "Migration file not found: {}",
            sql_path.display()
        )));
    }

    // Honest failure: applying migrations requires a database executor (driver /
    // prax-migrate engine) that is not yet wired into the CLI. Do NOT write the
    // `.applied` marker or report success for work that was not performed.
    Err(CliError::Migration(format!(
        "Applying migration '{}' is not yet implemented: executing migration SQL \
         requires a database executor that is not yet wired into the CLI. The \
         migration SQL is at {}; apply it with an external tool for now.",
        migration_path.display(),
        sql_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- provider -> SqlBackend --------------------------------------------

    #[test]
    fn test_sql_backend_for_provider() {
        use prax_migrate::SqlBackend;
        assert_eq!(
            sql_backend_for_provider("postgresql").unwrap(),
            SqlBackend::Postgres
        );
        assert_eq!(
            sql_backend_for_provider("postgres").unwrap(),
            SqlBackend::Postgres
        );
        assert_eq!(
            sql_backend_for_provider("mysql").unwrap(),
            SqlBackend::MySql
        );
        assert_eq!(
            sql_backend_for_provider("sqlite").unwrap(),
            SqlBackend::Sqlite
        );
        assert!(sql_backend_for_provider("nonsense").is_err());
        // DuckDB is intentionally not a supported migrate backend (no
        // introspector) and must be rejected here, matching get_database_type.
        assert!(sql_backend_for_provider("duckdb").is_err());
    }

    #[test]
    fn test_mysql_database_from_url() {
        assert_eq!(
            mysql_database_from_url("mysql://u:p@host:3306/mydb"),
            Some("mydb".to_string())
        );
        assert_eq!(
            mysql_database_from_url("mysql://u:p@host:3306/mydb?ssl=true"),
            Some("mydb".to_string())
        );
        // No database path segment → None (falls back to unscoped/default).
        assert_eq!(mysql_database_from_url("mysql://u:p@host:3306"), None);
        assert_eq!(mysql_database_from_url("mysql://u:p@host:3306/"), None);
        // A `/` inside the credentials must not be mistaken for the path.
        assert_eq!(
            mysql_database_from_url("mysql://u:p/w@host:3306/mydb"),
            Some("mydb".to_string())
        );
    }

    #[tokio::test]
    async fn test_introspect_database_unsupported_provider_errors() {
        use crate::commands::introspect::{IntrospectionOptions, introspect_database};
        // A provider that has no SQL introspector (mongodb) must return a
        // Config error, not silently succeed.
        let opts = IntrospectionOptions::default();
        match introspect_database("mongodb", "mongodb://x/y", &opts).await {
            Err(CliError::Config(msg)) => {
                assert!(
                    msg.contains("Unsupported database provider"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected CliError::Config, got {other:?}"),
        }
    }

    // -- generate_migration_sql: greenfield (no source) --------------------

    fn pg_config() -> Config {
        Config::default()
    }

    fn parse(schema: &str) -> prax_schema::ast::Schema {
        prax_schema::parse_schema(schema).expect("schema parses")
    }

    /// The SQL with `--` comment lines (header, warnings, skipped notes) removed.
    fn statements_only(sql: &str) -> String {
        sql.lines()
            .filter(|l| !l.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    const USERS_V1: &str = r#"
        model User {
            id    Int    @id @auto
            email String @unique

            @@map("users")
        }
    "#;

    #[test]
    fn greenfield_generates_full_create_table() {
        // No source (None) => differ emits full creation DDL.
        let schema = parse(USERS_V1);
        let sql = generate_migration_sql(&schema, None, &pg_config(), false)
            .unwrap()
            .sql;
        assert!(sql.contains("CREATE TABLE \"users\""), "sql: {sql}");
        assert!(sql.contains("SERIAL"), "auto id -> SERIAL: {sql}");
        // Incremental generator never uses IF NOT EXISTS (the old bug).
        assert!(!sql.contains("IF NOT EXISTS"), "sql: {sql}");
    }

    #[test]
    fn empty_diff_against_identical_source_yields_no_sql() {
        // Source == target => empty diff => empty SQL (no spurious churn).
        let schema = parse(USERS_V1);
        let source = parse(USERS_V1);
        let generated =
            generate_migration_sql(&schema, Some(source.into()), &pg_config(), false).unwrap();
        assert!(
            !generated.has_statements,
            "expected no changes, got: {}",
            generated.sql
        );
    }

    #[test]
    fn incremental_diff_emits_only_the_delta() {
        // v2 adds a nullable column and a whole new table with an FK +
        // composite PK. The migration must ALTER the existing table and
        // CREATE the new one — and touch nothing that already exists.
        let source = parse(USERS_V1);
        let target = parse(
            r#"
            model User {
                id       Int     @id @auto
                email    String  @unique
                nickname String?

                @@map("users")
            }

            model Membership {
                userId Int  @map("user_id")
                teamId Int  @map("team_id")
                user   User @relation(fields: [userId], references: [id])

                @@id([userId, teamId])
                @@map("memberships")
            }
            "#,
        );

        let sql = generate_migration_sql(&target, Some(source.into()), &pg_config(), false)
            .unwrap()
            .sql;

        // Added column on the existing table.
        assert!(
            sql.contains("ALTER TABLE \"users\" ADD COLUMN \"nickname\""),
            "sql: {sql}"
        );
        // New table created with a composite primary key.
        assert!(sql.contains("CREATE TABLE \"memberships\""), "sql: {sql}");
        assert!(
            sql.contains("PRIMARY KEY (\"user_id\", \"team_id\")"),
            "composite PK: {sql}"
        );
        // FK constraint present.
        assert!(sql.contains("FOREIGN KEY"), "fk: {sql}");
        // Nothing recreates the pre-existing users table.
        assert!(!sql.contains("CREATE TABLE \"users\""), "sql: {sql}");
    }

    #[test]
    fn additive_only_strips_drops_by_default() {
        // Source has an extra table + extra column the target no longer
        // declares. Default (additive-only) must NOT emit any DROP.
        let source = parse(
            r#"
            model User {
                id       Int     @id @auto
                email    String  @unique
                obsolete String?

                @@map("users")
            }
            model Legacy {
                id Int @id @auto

                @@map("legacy")
            }
            "#,
        );
        let target = parse(USERS_V1);

        let sql = generate_migration_sql(&target, Some(source.clone().into()), &pg_config(), false)
            .unwrap()
            .sql;
        assert!(
            !statements_only(&sql).to_uppercase().contains("DROP"),
            "additive-only must not drop: {sql}"
        );

        // With --allow-destructive the drops appear.
        let sql_destructive =
            generate_migration_sql(&target, Some(source.into()), &pg_config(), true)
                .unwrap()
                .sql;
        assert!(
            sql_destructive.contains("DROP TABLE") && sql_destructive.contains("DROP COLUMN"),
            "destructive should drop: {sql_destructive}"
        );
    }

    #[test]
    fn destructive_emits_fk_and_index_drops_stripped_by_default() {
        // Source declares an FK and a named index that the target drops.
        // Additive-only must strip both DROP CONSTRAINT and DROP INDEX;
        // --allow-destructive must emit them.
        let source = parse(
            r#"
            model User {
                id Int @id @auto
                @@map("users")
            }
            model Post {
                id       Int @id @auto
                authorId Int @map("author_id")
                title    String
                author   User @relation(fields: [authorId], references: [id], map: "post_author_fk")
                @@index([title], map: "post_title_idx")
                @@map("posts")
            }
            "#,
        );
        // Target keeps the tables but drops the relation (FK) and the index.
        let target = parse(
            r#"
            model User {
                id Int @id @auto
                @@map("users")
            }
            model Post {
                id       Int @id @auto
                authorId Int @map("author_id")
                title    String
                @@map("posts")
            }
            "#,
        );

        let additive =
            generate_migration_sql(&target, Some(source.clone().into()), &pg_config(), false)
                .unwrap()
                .sql;
        assert!(
            !statements_only(&additive).to_uppercase().contains("DROP"),
            "additive-only must strip FK/index drops: {additive}"
        );

        let destructive = generate_migration_sql(&target, Some(source.into()), &pg_config(), true)
            .unwrap()
            .sql;
        assert!(
            destructive.contains("DROP CONSTRAINT") && destructive.contains("post_author_fk"),
            "destructive should drop the FK constraint: {destructive}"
        );
        assert!(
            destructive.contains("DROP INDEX") && destructive.contains("post_title_idx"),
            "destructive should drop the index: {destructive}"
        );
    }

    #[test]
    fn strip_destructive_clears_procedure_and_trigger_drops() {
        use prax_migrate::{ProcedureDiff, SchemaDiff};
        let mut diff = SchemaDiff {
            procedures: Some(ProcedureDiff {
                drop: vec!["old_proc".into()],
                drop_triggers: vec!["old_trigger".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        strip_destructive(&mut diff);
        // With only drops, the procedure diff is emptied and cleared entirely.
        assert!(
            diff.procedures.is_none(),
            "procedure drops must be stripped and the empty diff removed"
        );
    }

    #[test]
    fn diff_and_dev_share_identical_sql_for_same_inputs() {
        // `migrate diff` and `migrate dev --create-only` both route through
        // generate_migration_sql, so parity reduces to this helper being
        // deterministic for identical (target, source, config, flag) inputs.
        let source = parse(USERS_V1);
        let target = parse(
            r#"
            model User {
                id       Int     @id @auto
                email    String  @unique
                nickname String?

                @@map("users")
            }
            "#,
        );

        let a = generate_migration_sql(&target, Some(source.clone().into()), &pg_config(), false)
            .unwrap()
            .sql;
        let b = generate_migration_sql(&target, Some(source.into()), &pg_config(), false)
            .unwrap()
            .sql;
        assert_eq!(a, b, "same inputs must yield identical SQL");
        assert!(a.contains("ADD COLUMN \"nickname\""), "sql: {a}");
    }

    const TS_TARGET: &str = r#"
        datasource db {
            provider = "postgresql"
            url = "postgres://x"
            extensions = [timescaledb]
        }
        model Metric {
            time DateTime
            @@id([time])
            @@timescale.hypertable(time)
        }
    "#;

    fn ts_catalog(version: &str) -> crate::commands::introspect::PostgresCatalog {
        crate::commands::introspect::PostgresCatalog {
            extensions: vec!["timescaledb".to_string()],
            timescale: prax_migrate::TimescaleCatalog::from_rows(prax_migrate::CatalogRows {
                extensions: vec![prax_migrate::timescale::catalog::ExtensionRow {
                    name: "timescaledb".to_string(),
                    version: version.to_string(),
                }],
                ..Default::default()
            })
            .unwrap(),
        }
    }

    #[test]
    fn test_generate_rejects_old_timescale() {
        let target = parse(TS_TARGET);
        let source = DiffSource {
            schema: parse(""),
            catalog: Some(ts_catalog("2.17.2")),
        };
        let err = generate_migration_sql(&target, Some(source), &pg_config(), false).unwrap_err();
        let msg = err.to_string();
        let (major, minor) = prax_migrate::timescale::TIMESCALE_MIN_VERSION;
        assert!(
            msg.contains("2.17.2") && msg.contains(&format!("{major}.{minor}")),
            "msg: {msg}"
        );
        assert!(msg.contains("2.18"), "msg: {msg}");
    }

    #[test]
    fn test_generate_rejects_old_timescale_when_opted_in_by_extension_only() {
        // Listing `timescaledb` opts in as much as an `@@timescale.*`
        // attribute does, so the version gate applies too.
        let target = parse(
            r#"
            datasource db {
                provider = "postgresql"
                url = "postgres://x"
                extensions = [timescaledb]
            }
            model A { id Int @id }
            "#,
        );
        let source = DiffSource {
            schema: parse(""),
            catalog: Some(ts_catalog("2.17.2")),
        };
        let msg = generate_migration_sql(&target, Some(source), &pg_config(), false)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("2.17.2") && msg.contains("2.18"), "msg: {msg}");
    }

    #[test]
    fn test_generate_ignores_old_timescale_without_opt_in() {
        let target = parse(PLAIN_METRIC);
        let source = DiffSource {
            schema: parse(PLAIN_METRIC),
            catalog: Some(ts_catalog("2.17.2")),
        };
        let generated = generate_migration_sql(&target, Some(source), &pg_config(), false)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!generated.has_statements, "sql: {}", generated.sql);
    }

    #[test]
    fn test_statements_sql_only_when_there_are_statements() {
        let schema = parse(USERS_V1);
        let created = generate_migration_sql(&schema, None, &pg_config(), false).unwrap();
        assert_eq!(created.statements_sql(), Some(created.sql.as_str()));

        let unchanged =
            generate_migration_sql(&schema, Some(parse(USERS_V1).into()), &pg_config(), false)
                .unwrap();
        assert_eq!(unchanged.statements_sql(), None);
    }

    #[test]
    fn test_comment_lines_cannot_break_out_into_sql() {
        // A database-sourced name carrying a newline must stay inside the
        // comment rather than become an executable statement.
        let mut out = String::new();
        push_comment_lines(
            &mut out,
            "SKIPPED (needs --allow-destructive): ",
            &["Drop table \"x\nDROP TABLE users; --\"".to_string()],
        );
        push_comment_lines(&mut out, "WARNING: ", &["a\r\nb\u{2028}c".to_string()]);
        assert_eq!(out.lines().count(), 2, "out: {out:?}");
        assert!(out.lines().all(|l| l.starts_with("-- ")), "out: {out:?}");
        assert!(
            statements_only(&out).trim().is_empty(),
            "no statement may escape: {out:?}"
        );
    }

    /// A catalog holding a hand-made `Metric` hypertable on `time`.
    fn hypertable_catalog() -> crate::commands::introspect::PostgresCatalog {
        use prax_migrate::timescale::catalog::{DimensionRow, ExtensionRow, HypertableRow};
        crate::commands::introspect::PostgresCatalog {
            extensions: vec!["timescaledb".to_string()],
            timescale: prax_migrate::TimescaleCatalog::from_rows(prax_migrate::CatalogRows {
                extensions: vec![ExtensionRow {
                    name: "timescaledb".to_string(),
                    version: "2.18.2".to_string(),
                }],
                hypertables: vec![HypertableRow {
                    table: "Metric".to_string(),
                    columnstore_enabled: false,
                }],
                dimensions: vec![DimensionRow {
                    table: "Metric".to_string(),
                    column: "time".to_string(),
                    is_time: true,
                    interval: Some((0, 7, 0)),
                    partitions: None,
                }],
                ..Default::default()
            })
            .unwrap(),
        }
    }

    const PLAIN_METRIC: &str = r#"
        model Metric {
            time  DateTime
            value Float
            @@id([time])
        }
    "#;

    #[test]
    fn test_generate_ignores_timescale_state_without_opt_in() {
        // A hypertable created by hand and modeled as a plain table, in a
        // schema that never mentions TimescaleDB: nothing to do.
        let target = parse(PLAIN_METRIC);
        let source = DiffSource {
            schema: parse(PLAIN_METRIC),
            catalog: Some(hypertable_catalog()),
        };
        let generated = generate_migration_sql(&target, Some(source), &pg_config(), false)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!generated.has_statements, "sql: {}", generated.sql);
    }

    #[test]
    fn test_generate_opted_in_rejects_undeclared_hypertable() {
        let datasource = r#"
            datasource db {
                provider = "postgresql"
                url = "postgres://x"
                extensions = [timescaledb]
            }
        "#;
        let target = parse(&format!("{datasource}{PLAIN_METRIC}"));
        let source = DiffSource {
            schema: parse(PLAIN_METRIC),
            catalog: Some(hypertable_catalog()),
        };
        let msg = generate_migration_sql(&target, Some(source), &pg_config(), false)
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("its model does not declare @@timescale.hypertable"),
            "msg: {msg}"
        );
    }

    #[test]
    fn test_generate_passes_installed_extensions() {
        let target = parse(
            r#"
            datasource db {
                provider = "postgresql"
                url = "postgres://x"
                extensions = [timescaledb]
            }
            model A { id Int @id }
            "#,
        );
        let source = DiffSource {
            schema: parse(""),
            catalog: Some(crate::commands::introspect::PostgresCatalog {
                extensions: vec!["timescaledb".to_string()],
                timescale: None,
            }),
        };
        let sql = generate_migration_sql(&target, Some(source), &pg_config(), false)
            .unwrap()
            .sql;
        assert!(!sql.contains("CREATE EXTENSION"), "sql: {sql}");
    }

    #[test]
    fn strip_destructive_clears_all_drop_channels() {
        use prax_migrate::{EnumAlterDiff, ModelAlterDiff, SchemaDiff};
        let mut diff = SchemaDiff {
            drop_models: vec!["Legacy".into()],
            drop_enums: vec!["OldEnum".into()],
            drop_views: vec!["OldView".into()],
            drop_extensions: vec!["pgcrypto".into()],
            alter_enums: vec![EnumAlterDiff {
                name: "Status".into(),
                add_values: Vec::new(),
                remove_values: vec!["DEPRECATED".into()],
            }],
            alter_models: vec![ModelAlterDiff {
                name: "User".into(),
                table_name: "users".into(),
                add_fields: Vec::new(),
                drop_fields: vec!["obsolete".into()],
                alter_fields: Vec::new(),
                add_indexes: Vec::new(),
                drop_indexes: vec!["idx_old".into()],
                add_foreign_keys: Vec::new(),
                drop_foreign_keys: vec!["fk_old".into()],
            }],
            ..Default::default()
        };

        strip_destructive(&mut diff);

        assert!(diff.drop_models.is_empty());
        assert!(diff.drop_enums.is_empty());
        assert!(diff.drop_views.is_empty());
        assert!(diff.drop_extensions.is_empty());
        // The alter_model had only drops -> pruned entirely.
        assert!(diff.alter_models.is_empty());
        // The alter_enum had only removals -> pruned entirely.
        assert!(diff.alter_enums.is_empty());
    }

    const USERS_WITH_LEGACY: &str = r#"
        model User {
            id     Int     @id @auto
            email  String  @unique
            legacy String?

            @@map("users")
        }
    "#;

    #[test]
    fn test_skipped_changes_reported_when_nothing_else_changes() {
        let source = parse(USERS_WITH_LEGACY);
        let target = parse(USERS_V1);
        let generated =
            generate_migration_sql(&target, Some(source.into()), &pg_config(), false).unwrap();
        assert!(!generated.has_statements, "sql: {}", generated.sql);
        assert_eq!(
            generated.skipped,
            vec!["Drop column \"legacy\" from table \"users\"".to_string()]
        );
        assert!(
            generated.sql.contains(
                "-- SKIPPED (needs --allow-destructive): Drop column \"legacy\" from table \"users\""
            ),
            "sql: {}",
            generated.sql
        );
    }

    #[test]
    fn test_allow_destructive_emits_and_skips_nothing() {
        let source = parse(USERS_WITH_LEGACY);
        let target = parse(USERS_V1);
        let generated =
            generate_migration_sql(&target, Some(source.into()), &pg_config(), true).unwrap();
        assert!(generated.has_statements);
        assert!(generated.skipped.is_empty());
        assert!(!generated.sql.contains("SKIPPED"), "sql: {}", generated.sql);
    }

    #[test]
    fn test_skipped_timescale_change_reported() {
        use prax_migrate::timescale::catalog::HypertableState;
        let source_prax = r#"
            datasource db {
                provider = "postgresql"
                url = "postgres://x"
                extensions = [timescaledb]
            }
            model Metric {
                time DateTime
                @@id([time])
                @@timescale.hypertable(time, chunkInterval: "1 day")
            }
        "#;
        let target_prax = r#"
            datasource db {
                provider = "postgresql"
                url = "postgres://x"
                extensions = [timescaledb]
            }
            model Metric {
                time DateTime
                @@id([time])
                @@timescale.hypertable(time, chunkInterval: "1 day")
                @@timescale.retention(dropAfter: "90 days")
            }
        "#;
        let mut catalog = ts_catalog("2.18.0");
        if let Some(ts) = catalog.timescale.as_mut() {
            ts.hypertables.insert(
                "Metric".into(),
                HypertableState {
                    table: "Metric".into(),
                    time_column: "time".into(),
                    chunk_interval: prax_schema::ast::PgInterval::new(0, 1, 0),
                    space: None,
                    columnstore: None,
                    retention: None,
                },
            );
        }
        let source = DiffSource {
            schema: parse(source_prax),
            catalog: Some(catalog),
        };
        let generated =
            generate_migration_sql(&parse(target_prax), Some(source), &pg_config(), false).unwrap();
        assert!(!generated.has_statements, "sql: {}", generated.sql);
        assert!(
            generated
                .skipped
                .iter()
                .any(|n| n.contains("automatically drops chunks older than 90 days")),
            "skipped: {:?}",
            generated.skipped
        );
    }

    #[test]
    fn test_strip_destructive_notes_cover_every_category() {
        use prax_migrate::{EnumAlterDiff, IndexDiff, ModelAlterDiff, ProcedureDiff, SchemaDiff};
        let mut diff = SchemaDiff {
            drop_models: vec!["t".into()],
            drop_enums: vec!["e".into()],
            drop_views: vec!["v".into()],
            drop_extensions: vec!["x".into()],
            drop_indexes: vec![IndexDiff {
                name: "i".into(),
                table_name: "t".into(),
                columns: vec!["c".into()],
                unique: false,
                index_type: None,
                vector_ops: None,
                hnsw_m: None,
                hnsw_ef_construction: None,
                ivfflat_lists: None,
            }],
            alter_enums: vec![EnumAlterDiff {
                name: "e2".into(),
                add_values: Vec::new(),
                remove_values: vec!["val".into()],
            }],
            alter_models: vec![ModelAlterDiff {
                name: "M".into(),
                table_name: "t2".into(),
                add_fields: Vec::new(),
                drop_fields: vec!["c".into()],
                alter_fields: Vec::new(),
                add_indexes: Vec::new(),
                drop_indexes: vec!["i2".into()],
                add_foreign_keys: Vec::new(),
                drop_foreign_keys: vec!["fk".into()],
            }],
            procedures: Some(ProcedureDiff {
                drop: vec!["p".into()],
                drop_triggers: vec!["tr".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let notes = strip_destructive(&mut diff);
        assert_eq!(
            notes,
            vec![
                "Drop table \"t\"",
                "Drop enum \"e\"",
                "Drop view \"v\"",
                "Drop extension \"x\"",
                "Drop index \"i\"",
                "Drop column \"c\" from table \"t2\"",
                "Drop index \"i2\"",
                "Drop foreign key \"fk\" on table \"t2\"",
                "Remove value \"val\" from enum \"e2\"",
                "Drop procedure \"p\"",
                "Drop trigger \"tr\"",
            ]
        );
    }

    #[test]
    fn test_warnings_survive_an_empty_migration() {
        use prax_migrate::timescale::catalog::{AggregatePolicy, AggregateState, HypertableState};
        use prax_schema::ast::PgInterval;
        const DS: &str = "datasource db {\n  provider = \"postgresql\"\n  url = \"postgres://x\"\n  extensions = [timescaledb]\n}\n";
        const SQL: &str = "SELECT time_bucket('1 hour', \"time\") AS bucket, \"deviceId\", avg(\"value\") AS \"avgValue\"\nFROM \"Metric\"\nGROUP BY 1, 2";
        let target = parse(&format!(
            "{DS}\nmodel Metric {{\n  time DateTime\n  deviceId String\n  value Float\n\n  @@id([deviceId, time])\n  @@timescale.hypertable(time, chunkInterval: \"1 day\")\n}}\n\nview HourlyMetric {{\n  bucket DateTime\n  deviceId String\n  avgValue Float\n\n  @@timescale.continuous(source: Metric, scheduleInterval: \"1 hour\", startOffset: \"3 days\", endOffset: \"1 hour\")\n}}\n\n@@sql(\"HourlyMetric\", \"\"\"\n{SQL}\n\"\"\")\n"
        ));
        let source_schema = parse(&format!(
            "{DS}\nmodel Metric {{\n  time DateTime\n  deviceId String\n  value Float\n\n  @@id([deviceId, time])\n}}\n"
        ));
        let mut catalog = ts_catalog("2.18.0");
        if let Some(ts) = catalog.timescale.as_mut() {
            ts.hypertables.insert(
                "Metric".into(),
                HypertableState {
                    table: "Metric".into(),
                    time_column: "time".into(),
                    chunk_interval: PgInterval::new(0, 1, 0),
                    space: None,
                    columnstore: None,
                    retention: None,
                },
            );
            ts.aggregates.insert(
                "HourlyMetric".into(),
                AggregateState {
                    view: "HourlyMetric".into(),
                    source: "Metric".into(),
                    sql: None,
                    materialized_only: true,
                    policy: Some(AggregatePolicy {
                        start_offset: Some(PgInterval::new(0, 3, 0)),
                        end_offset: Some(PgInterval::new(0, 0, 3_600_000_000)),
                        schedule_interval: PgInterval::new(0, 0, 3_600_000_000),
                    }),
                },
            );
        }
        let source = DiffSource {
            schema: source_schema,
            catalog: Some(catalog),
        };
        let generated = generate_migration_sql(&target, Some(source), &pg_config(), false).unwrap();
        assert!(!generated.has_statements, "sql: {}", generated.sql);
        assert!(generated.skipped.is_empty());
        assert_eq!(generated.warnings.len(), 1, "{:?}", generated.warnings);
        assert!(
            generated.warnings[0].contains("not created by Prax"),
            "{:?}",
            generated.warnings
        );
    }

    // -- honest-error paths ---------------------------------------------------

    #[tokio::test]
    async fn test_apply_migration_fails_without_executor() {
        let dir = tempfile::tempdir().unwrap();
        let migration_path = dir.path().join("20240101000000_init");
        std::fs::create_dir_all(&migration_path).unwrap();
        std::fs::write(
            migration_path.join("migration.sql"),
            "CREATE TABLE t (id INT);",
        )
        .unwrap();

        let result = apply_migration(&migration_path, &Config::default()).await;

        match result {
            Err(CliError::Migration(msg)) => {
                assert!(
                    msg.contains("not yet implemented"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected CliError::Migration, got {other:?}"),
        }

        // The .applied marker must NOT be written for work that was not done.
        assert!(!migration_path.join(".applied").exists());
    }

    #[tokio::test]
    async fn test_apply_migration_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        match apply_migration(dir.path(), &Config::default()).await {
            Err(CliError::Migration(msg)) => assert!(msg.contains("not found")),
            other => panic!("expected CliError::Migration, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_reset_not_implemented() {
        let args = crate::cli::MigrateResetArgs {
            force: true,
            seed: false,
            skip_migrations: false,
        };

        match run_reset(args).await {
            Err(CliError::Migration(msg)) => {
                assert!(
                    msg.contains("not yet implemented"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected CliError::Migration, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_resolve_not_implemented() {
        let args = crate::cli::MigrateResolveArgs {
            migration: "20240101000000_init".to_string(),
            applied: true,
            rolled_back: false,
        };

        match run_resolve(args).await {
            Err(CliError::Migration(msg)) => {
                assert!(
                    msg.contains("not yet implemented"),
                    "unexpected message: {msg}"
                );
                assert!(msg.contains("20240101000000_init"));
            }
            other => panic!("expected CliError::Migration, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_resolve_requires_a_flag() {
        let args = crate::cli::MigrateResolveArgs {
            migration: "m".to_string(),
            applied: false,
            rolled_back: false,
        };

        match run_resolve(args).await {
            Err(CliError::Command(msg)) => assert!(msg.contains("--applied or --rolled-back")),
            other => panic!("expected CliError::Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_rollback_not_implemented() {
        let args = crate::cli::MigrateRollbackArgs {
            reason: None,
            user: None,
            to: None,
        };

        match run_rollback(args).await {
            Err(CliError::Migration(msg)) => {
                assert!(
                    msg.contains("not yet implemented"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected CliError::Migration, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_history_not_implemented() {
        let args = crate::cli::MigrateHistoryArgs { migration: None };

        match run_history(args).await {
            Err(CliError::Migration(msg)) => {
                assert!(
                    msg.contains("_prax_migrations"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected CliError::Migration, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_diff_from_migration_unsupported() {
        let args = crate::cli::MigrateDiffArgs {
            schema: None,
            output: None,
            from_migration: Some("20240101000000_init".to_string()),
            allow_destructive: false,
        };

        match run_diff(args).await {
            Err(CliError::Migration(msg)) => {
                assert!(
                    msg.contains("--from-migration"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected CliError::Migration, got {other:?}"),
        }
    }
}
