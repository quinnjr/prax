//! CLI argument definitions using clap.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Prax CLI - A modern ORM for Rust
#[derive(Parser, Debug)]
#[command(name = "prax")]
#[command(author = "Pegasus Heavy Industries LLC")]
#[command(version)]
#[command(about = "Prax CLI - A modern ORM for Rust", long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Command,
}

/// Available CLI commands
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize a new Prax project
    Init(InitArgs),

    /// Generate Rust client code from schema
    Generate(GenerateArgs),

    /// Schema validation and formatting
    Validate(ValidateArgs),

    /// Format schema file
    Format(FormatArgs),

    /// Database migration commands
    Migrate(MigrateArgs),

    /// Direct database operations
    Db(DbArgs),

    /// Import schema from Prisma or Diesel
    Import(ImportArgs),

    /// Display version information
    Version,
}

// =============================================================================
// Init Command
// =============================================================================

/// Arguments for the `init` command
#[derive(Args, Debug)]
pub struct InitArgs {
    /// Path to initialize the project (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Database provider to use
    #[arg(short, long, default_value = "postgresql")]
    pub provider: DatabaseProvider,

    /// Database connection URL
    #[arg(short, long)]
    pub url: Option<String>,

    /// Skip generating example schema
    #[arg(long)]
    pub no_example: bool,

    /// Accept all defaults without prompting
    #[arg(short, long)]
    pub yes: bool,
}

/// Supported database providers
#[derive(ValueEnum, Debug, Clone, Copy, Default)]
pub enum DatabaseProvider {
    #[default]
    Postgresql,
    Mysql,
    Sqlite,
}

impl std::fmt::Display for DatabaseProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseProvider::Postgresql => write!(f, "postgresql"),
            DatabaseProvider::Mysql => write!(f, "mysql"),
            DatabaseProvider::Sqlite => write!(f, "sqlite"),
        }
    }
}

// =============================================================================
// Generate Command
// =============================================================================

/// Arguments for the `generate` command
#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// Path to schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Output directory for generated code
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Features to generate (e.g., serde, graphql)
    #[arg(short, long, value_delimiter = ',')]
    pub features: Vec<String>,

    /// Watch for schema changes and regenerate
    #[arg(short, long)]
    pub watch: bool,
}

// =============================================================================
// Validate Command
// =============================================================================

/// Arguments for the `validate` command
#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Path to schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,
}

// =============================================================================
// Format Command
// =============================================================================

/// Arguments for the `format` command
#[derive(Args, Debug)]
pub struct FormatArgs {
    /// Path to schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Check formatting without writing changes
    #[arg(short, long)]
    pub check: bool,
}

// =============================================================================
// Migrate Command
// =============================================================================

/// Arguments for the `migrate` command
#[derive(Args, Debug)]
pub struct MigrateArgs {
    #[command(subcommand)]
    pub command: MigrateSubcommand,
}

/// Migrate subcommands
#[derive(Subcommand, Debug)]
pub enum MigrateSubcommand {
    /// Create and apply migrations during development
    Dev(MigrateDevArgs),

    /// Deploy pending migrations to production
    Deploy,

    /// Reset database and re-apply all migrations
    Reset(MigrateResetArgs),

    /// Show migration status
    Status,

    /// Resolve migration issues
    Resolve(MigrateResolveArgs),

    /// Generate migration diff without applying
    Diff(MigrateDiffArgs),

    /// Rollback the last applied migration
    Rollback(MigrateRollbackArgs),

    /// View migration history
    History(MigrateHistoryArgs),
}

/// Arguments for `migrate dev`
#[derive(Args, Debug)]
pub struct MigrateDevArgs {
    /// Name for the migration
    #[arg(short, long)]
    pub name: Option<String>,

    /// Create migration without applying
    #[arg(long)]
    pub create_only: bool,

    /// Skip seed after migration
    #[arg(long)]
    pub skip_seed: bool,

    /// Path to schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,
}

/// Arguments for `migrate reset`
#[derive(Args, Debug)]
pub struct MigrateResetArgs {
    /// Skip confirmation prompt
    #[arg(short, long)]
    pub force: bool,

    /// Run seed after reset
    #[arg(long)]
    pub seed: bool,

    /// Skip applying migrations (just reset)
    #[arg(long)]
    pub skip_migrations: bool,
}

/// Arguments for `migrate resolve`
#[derive(Args, Debug)]
pub struct MigrateResolveArgs {
    /// Name of the migration to resolve
    pub migration: String,

    /// Mark migration as applied
    #[arg(long)]
    pub applied: bool,

    /// Mark migration as rolled back
    #[arg(long)]
    pub rolled_back: bool,
}

/// Arguments for `migrate diff`
#[derive(Args, Debug)]
pub struct MigrateDiffArgs {
    /// Path to schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Output path for generated SQL
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Compare against a specific migration
    #[arg(long)]
    pub from_migration: Option<String>,
}

/// Arguments for `migrate rollback`
#[derive(Args, Debug)]
pub struct MigrateRollbackArgs {
    /// Reason for rollback
    #[arg(long)]
    pub reason: Option<String>,

    /// User performing the rollback
    #[arg(long)]
    pub user: Option<String>,

    /// Rollback to a specific migration
    #[arg(long)]
    pub to: Option<String>,
}

/// Arguments for `migrate history`
#[derive(Args, Debug)]
pub struct MigrateHistoryArgs {
    /// Show history for a specific migration
    #[arg(long)]
    pub migration: Option<String>,
}

// =============================================================================
// Db Command
// =============================================================================

/// Arguments for the `db` command
#[derive(Args, Debug)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbSubcommand,
}

/// Db subcommands
#[derive(Subcommand, Debug)]
pub enum DbSubcommand {
    /// Push schema to database without migrations
    Push(DbPushArgs),

    /// Introspect database and generate schema
    Pull(DbPullArgs),

    /// Seed database with initial data
    Seed(DbSeedArgs),

    /// Execute raw SQL
    Execute(DbExecuteArgs),
}

/// Arguments for `db push`
#[derive(Args, Debug)]
pub struct DbPushArgs {
    /// Path to schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Accept data loss from destructive changes
    #[arg(long)]
    pub accept_data_loss: bool,

    /// Skip confirmation prompts
    #[arg(short, long)]
    pub force: bool,

    /// Reset database before push
    #[arg(long)]
    pub reset: bool,
}

/// Arguments for `db pull`
#[derive(Args, Debug)]
pub struct DbPullArgs {
    /// Output path for generated schema
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Overwrite existing schema without prompting
    #[arg(short, long)]
    pub force: bool,

    /// Include views in introspection
    #[arg(long)]
    pub include_views: bool,

    /// Include materialized views in introspection
    #[arg(long)]
    pub include_materialized_views: bool,

    /// Schema/namespace to introspect (default: public for PostgreSQL, dbo for MSSQL)
    #[arg(long)]
    pub schema: Option<String>,

    /// Filter tables by pattern (glob-style, e.g., "user*")
    #[arg(long)]
    pub tables: Option<String>,

    /// Exclude tables by pattern (glob-style, e.g., "_prisma*")
    #[arg(long)]
    pub exclude: Option<String>,

    /// Print schema to stdout instead of writing to file
    #[arg(long)]
    pub print: bool,

    /// Output format
    #[arg(long, default_value = "prax")]
    pub format: OutputFormat,

    /// Number of documents to sample for MongoDB schema inference
    #[arg(long, default_value = "100")]
    pub sample_size: usize,

    /// Include column comments in schema
    #[arg(long)]
    pub comments: bool,
}

/// Output format for schema introspection
#[derive(ValueEnum, Debug, Clone, Copy, Default)]
pub enum OutputFormat {
    /// Prax schema format (.prax)
    #[default]
    Prax,
    /// JSON format
    Json,
    /// SQL DDL format
    Sql,
}

/// Arguments for `db seed`
#[derive(Args, Debug)]
pub struct DbSeedArgs {
    /// Path to seed file
    #[arg(short, long)]
    pub seed_file: Option<PathBuf>,

    /// Reset database before seeding
    #[arg(long)]
    pub reset: bool,

    /// Environment to run seed for (development, staging, production)
    #[arg(short, long, default_value = "development")]
    pub environment: String,

    /// Force seeding even if environment config says not to
    #[arg(short, long)]
    pub force: bool,
}

/// Arguments for `db execute`
#[derive(Args, Debug)]
pub struct DbExecuteArgs {
    /// SQL to execute
    #[arg(short, long)]
    pub sql: Option<String>,

    /// Path to SQL file
    #[arg(short, long)]
    pub file: Option<PathBuf>,

    /// Read SQL from stdin
    #[arg(long)]
    pub stdin: bool,

    /// Skip confirmation prompt
    #[arg(short = 'y', long)]
    pub force: bool,
}

// =============================================================================
// Import Command
// =============================================================================

/// Arguments for the `import` command
#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Source ORM to import from
    #[arg(long, value_enum)]
    pub from: ImportSource,

    /// Input schema file path
    #[arg(short, long)]
    pub input: PathBuf,

    /// Output Prax schema file path
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Database provider for the imported schema
    #[arg(short = 'P', long)]
    pub provider: Option<DatabaseProvider>,

    /// Database connection URL for the imported schema
    #[arg(short, long)]
    pub url: Option<String>,

    /// Print to stdout instead of writing to file
    #[arg(long)]
    pub print: bool,

    /// Overwrite existing output file without prompting
    #[arg(short, long)]
    pub force: bool,
}

/// Source ORM for import
#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum ImportSource {
    /// Prisma schema (.prisma files)
    Prisma,
    /// Diesel schema (schema.rs files with table! macros)
    Diesel,
    /// SeaORM entity (entity files with DeriveEntityModel)
    SeaOrm,
}
