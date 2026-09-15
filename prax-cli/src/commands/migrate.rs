//! `prax migrate` commands - Database migration management.

use std::path::{Path, PathBuf};

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
    let source = resolve_source_schema(&config).await?;
    let migration_sql = generate_migration_sql(&schema, source, &config, args.allow_destructive)?;

    // 4. Generate migration
    output::step(4, total_steps, "Generating migration...");
    if migration_sql.trim().is_empty() {
        output::newline();
        success("No changes: the database already matches the schema. No migration created.");
        return Ok(());
    }
    let migration_path = create_migration(&migrations_dir, &migration_name, &migration_sql)?;

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
    let source = resolve_source_schema(&config).await?;
    let ddl_sql = generate_migration_sql(&schema, source, &config, args.allow_destructive)?;

    output::newline();
    if ddl_sql.trim().is_empty() {
        output::info("No changes: the database already matches the schema.");
    }

    output::newline();
    output::section("Generated DDL");
    output::code(&ddl_sql, "sql");

    if let Some(output_path) = args.output {
        std::fs::write(&output_path, &ddl_sql)?;
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
async fn resolve_source_schema(config: &Config) -> CliResult<Option<prax_schema::ast::Schema>> {
    // No URL configured/available → greenfield source.
    let Ok(database_url) = get_database_url(config) else {
        output::list_item("No DATABASE_URL configured; treating as a new database.");
        return Ok(None);
    };

    introspect_source_schema(config, &database_url).await
}

/// Introspect `database_url` and map the result to a diff-source schema.
///
/// Dispatches to the backend matching the configured provider (PostgreSQL,
/// MySQL, SQLite, MSSQL), each behind its cargo feature. A failure —
/// unreachable database, or a provider whose introspection feature was not
/// compiled in — is treated as "no source reachable" (with a warning) rather
/// than a hard error, so `migrate dev` still works offline for first-time
/// creation and the greenfield flow is preserved.
async fn introspect_source_schema(
    config: &Config,
    database_url: &str,
) -> CliResult<Option<prax_schema::ast::Schema>> {
    use crate::commands::introspect::{IntrospectionOptions, introspect_database};
    use crate::commands::schema_from_db::schema_from_database;

    let options = IntrospectionOptions::default();

    match introspect_database(&config.database.provider, database_url, &options).await {
        Ok(db_schema) => {
            let result = schema_from_database(&db_schema, Default::default())?;
            for warning in &result.warnings {
                output::warn(warning);
            }
            Ok(Some(result.schema))
        }
        Err(e) => {
            output::warn(&format!(
                "Could not introspect the database ({e}); treating as a new database. \
                 Generated SQL will be full-creation DDL."
            ));
            Ok(None)
        }
    }
}

/// Map a datasource provider string to a migration `SqlBackend`.
fn sql_backend_for_provider(provider: &str) -> CliResult<prax_migrate::SqlBackend> {
    use prax_migrate::SqlBackend;
    match provider.to_lowercase().as_str() {
        "postgresql" | "postgres" | "pg" => Ok(SqlBackend::Postgres),
        "mysql" | "mariadb" => Ok(SqlBackend::MySql),
        "sqlite" | "sqlite3" => Ok(SqlBackend::Sqlite),
        "mssql" | "sqlserver" | "sql_server" => Ok(SqlBackend::Mssql),
        "duckdb" => Ok(SqlBackend::DuckDb),
        other => Err(CliError::Config(format!(
            "Unsupported database provider for migration generation: '{}'",
            other
        ))),
    }
}

/// Diff the desired `schema` (target) against an optional introspected
/// `source`, render the resulting `SchemaDiff` through the provider's dialect
/// generator, and return the `up` SQL.
///
/// When `allow_destructive` is false (the default), drops (tables, columns,
/// enums, foreign keys, indexes, enum-value removals) are stripped from the
/// diff before generation so a stale schema never silently destroys data.
fn generate_migration_sql(
    schema: &prax_schema::ast::Schema,
    source: Option<prax_schema::ast::Schema>,
    config: &Config,
    allow_destructive: bool,
) -> CliResult<String> {
    use prax_migrate::{SchemaDiffer, SqlDialect};

    let backend = sql_backend_for_provider(&config.database.provider)?;

    let differ = SchemaDiffer::new(schema.clone());
    let differ = match source {
        Some(src) => differ.with_source(src),
        None => differ,
    };

    let mut diff = differ
        .diff()
        .map_err(|e| CliError::Migration(format!("Failed to diff schema against database: {e}")))?;

    if !allow_destructive {
        strip_destructive(&mut diff);
    }

    let migration = SqlDialect::for_backend(backend).generate_migration(&diff);

    let mut out = String::from("-- Migration generated by Prax\n");
    if !allow_destructive {
        out.push_str(
            "-- Additive-only: destructive statements (DROP) are omitted. Re-run with \
             --allow-destructive to include them.\n",
        );
    }
    for warning in &migration.warnings {
        out.push_str(&format!("-- WARNING: {}\n", warning));
    }
    out.push('\n');
    out.push_str(migration.up.trim_end());
    if !migration.up.trim_end().is_empty() {
        out.push('\n');
    }

    // A header-only result (no statements) counts as "no changes" to callers.
    if migration.up.trim().is_empty() {
        return Ok(String::new());
    }

    Ok(out)
}

/// Remove every destructive operation from a `SchemaDiff` in place, leaving
/// only additive/altering changes. Column *type/nullability/default* alters
/// are kept (they are not drops); dropped columns, tables, enums, enum values,
/// foreign keys, indexes, and extensions are removed.
fn strip_destructive(diff: &mut prax_migrate::SchemaDiff) {
    diff.drop_models.clear();
    diff.drop_enums.clear();
    diff.drop_views.clear();
    diff.drop_extensions.clear();
    diff.drop_indexes.clear();

    for alter in &mut diff.alter_models {
        alter.drop_fields.clear();
        alter.drop_indexes.clear();
        alter.drop_foreign_keys.clear();
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
        alter.remove_values.clear();
    }
    diff.alter_enums.retain(|a| !a.add_values.is_empty());
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
    }

    // -- generate_migration_sql: greenfield (no source) --------------------

    fn pg_config() -> Config {
        Config::default()
    }

    fn parse(schema: &str) -> prax_schema::ast::Schema {
        prax_schema::parse_schema(schema).expect("schema parses")
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
        let sql = generate_migration_sql(&schema, None, &pg_config(), false).unwrap();
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
        let sql = generate_migration_sql(&schema, Some(source), &pg_config(), false).unwrap();
        assert!(sql.trim().is_empty(), "expected no changes, got: {sql}");
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

        let sql = generate_migration_sql(&target, Some(source), &pg_config(), false).unwrap();

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

        let sql =
            generate_migration_sql(&target, Some(source.clone()), &pg_config(), false).unwrap();
        assert!(
            !sql.to_uppercase().contains("DROP"),
            "additive-only must not drop: {sql}"
        );

        // With --allow-destructive the drops appear.
        let sql_destructive =
            generate_migration_sql(&target, Some(source), &pg_config(), true).unwrap();
        assert!(
            sql_destructive.contains("DROP TABLE") && sql_destructive.contains("DROP COLUMN"),
            "destructive should drop: {sql_destructive}"
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

        let a = generate_migration_sql(&target, Some(source.clone()), &pg_config(), false).unwrap();
        let b = generate_migration_sql(&target, Some(source), &pg_config(), false).unwrap();
        assert_eq!(a, b, "same inputs must yield identical SQL");
        assert!(a.contains("ADD COLUMN \"nickname\""), "sql: {a}");
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
