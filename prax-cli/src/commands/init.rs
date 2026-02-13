//! `prax init` command - Initialize a new Prax project.

use std::path::Path;

use crate::cli::{DatabaseProvider, InitArgs};
use crate::config::{
    CONFIG_FILE_NAME, Config, MIGRATIONS_DIR, PRAX_DIR, SCHEMA_FILE_NAME, SCHEMA_FILE_PATH,
    SEEDS_DIR,
};
use crate::error::CliResult;
use crate::output::{self, confirm, input, select, success};

/// Run the init command
pub async fn run(args: InitArgs) -> CliResult<()> {
    output::header("Initialize Prax Project");

    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());

    // Check if already initialized
    let config_path = project_path.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        output::warn(&format!(
            "Project already initialized. {} exists.",
            CONFIG_FILE_NAME
        ));

        if !args.yes && !confirm("Reinitialize project?") {
            return Ok(());
        }
    }

    // Get database provider
    let provider = if args.yes {
        args.provider
    } else {
        let providers = ["PostgreSQL", "MySQL", "SQLite"];
        let selection = select("Select database provider:", &providers);
        match selection {
            Some(0) => DatabaseProvider::Postgresql,
            Some(1) => DatabaseProvider::Mysql,
            Some(2) => DatabaseProvider::Sqlite,
            _ => args.provider,
        }
    };

    // Get database URL
    let db_url = if args.yes {
        args.url
    } else {
        let default_url = match provider {
            DatabaseProvider::Postgresql => "postgresql://user:password@localhost:5432/mydb",
            DatabaseProvider::Mysql => "mysql://user:password@localhost:3306/mydb",
            DatabaseProvider::Sqlite => "file:./dev.db",
        };
        let prompt = format!("Database URL [{}] (or leave empty to use env)", default_url);
        let url = input(&prompt);
        if url.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
            None
        } else {
            url
        }
    };

    output::newline();
    output::step(1, 4, "Creating project structure...");

    // Create directories
    create_project_structure(&project_path)?;

    output::step(2, 4, "Creating configuration file...");

    // Create config
    let mut config = Config::default_for_provider(&provider.to_string());
    config.database.url = db_url.clone();
    config.save(&config_path)?;

    output::step(3, 4, "Creating schema file...");

    // Create schema file in prax/ directory
    let schema_path = project_path.join(SCHEMA_FILE_PATH);
    if !args.no_example {
        create_example_schema(&schema_path, provider)?;
    } else {
        create_minimal_schema(&schema_path, provider)?;
    }

    output::step(4, 4, "Creating .env file...");

    // Create .env file
    let env_path = project_path.join(".env");
    if !env_path.exists() {
        create_env_file(&env_path, provider, &db_url)?;
    }

    output::newline();
    success("Project initialized successfully!");
    output::newline();

    // Print next steps
    output::section("Next steps");
    output::list_item(&format!("Edit {} to define your schema", SCHEMA_FILE_PATH));
    output::list_item("Set your DATABASE_URL in .env");
    output::list_item("Run `prax generate` to generate Rust code");
    output::list_item("Run `prax migrate dev` to create your first migration");
    output::newline();

    // Show file structure
    output::section("Created files");
    output::kv(CONFIG_FILE_NAME, "Prax configuration (project root)");
    output::kv(&format!("{}/", PRAX_DIR), "Prax directory");
    output::kv(
        &format!("  {}", SCHEMA_FILE_NAME),
        "Database schema definition",
    );
    output::kv("  migrations/", "Migration files");
    output::kv("  seeds/", "Seed files");
    output::kv(".env", "Environment variables");

    Ok(())
}

/// Create the project directory structure
fn create_project_structure(path: &Path) -> CliResult<()> {
    // Create prax directory
    let prax_path = path.join(PRAX_DIR);
    std::fs::create_dir_all(&prax_path)?;

    // Create migrations directory inside prax/
    let migrations_path = path.join(MIGRATIONS_DIR);
    std::fs::create_dir_all(&migrations_path)?;

    // Create .gitkeep in migrations
    let gitkeep_path = migrations_path.join(".gitkeep");
    std::fs::write(gitkeep_path, "")?;

    // Create seeds directory inside prax/
    let seeds_path = path.join(SEEDS_DIR);
    std::fs::create_dir_all(&seeds_path)?;

    // Create .gitkeep in seeds
    let seeds_gitkeep_path = seeds_path.join(".gitkeep");
    std::fs::write(seeds_gitkeep_path, "")?;

    Ok(())
}

/// Create an example schema file
fn create_example_schema(path: &Path, provider: DatabaseProvider) -> CliResult<()> {
    let schema = match provider {
        DatabaseProvider::Postgresql => {
            r#"// Prax Schema File
// Learn more at https://prax.dev/docs/schema

// Database connection
datasource db {
    provider = "postgresql"
    url      = env("DATABASE_URL")
}

// Client generator
generator client {
    provider = "prax-client-rust"
    output   = "./src/generated"
}

// =============================================================================
// Example Models
// =============================================================================

/// A user in the system
model User {
    id        Int      @id @auto
    email     String   @unique
    name      String?
    password  String   @writeonly
    role      Role     @default(USER)
    posts     Post[]
    profile   Profile?
    createdAt DateTime @default(now()) @map("created_at")
    updatedAt DateTime @updatedAt @map("updated_at")

    @@map("users")
    @@index([email])
}

/// User profile with additional information
model Profile {
    id     Int     @id @auto
    bio    String?
    avatar String?
    user   User    @relation(fields: [userId], references: [id], onDelete: Cascade)
    userId Int     @unique @map("user_id")

    @@map("profiles")
}

/// A blog post
model Post {
    id        Int        @id @auto
    title     String
    content   String?
    published Boolean    @default(false)
    author    User       @relation(fields: [authorId], references: [id])
    authorId  Int        @map("author_id")
    tags      Tag[]
    createdAt DateTime   @default(now()) @map("created_at")
    updatedAt DateTime   @updatedAt @map("updated_at")

    @@map("posts")
    @@index([authorId])
    @@index([published])
}

/// Tags for posts
model Tag {
    id    Int    @id @auto
    name  String @unique
    posts Post[]

    @@map("tags")
}

/// User roles
enum Role {
    USER
    ADMIN
    MODERATOR
}
"#
        }
        DatabaseProvider::Mysql => {
            r#"// Prax Schema File
// Learn more at https://prax.dev/docs/schema

datasource db {
    provider = "mysql"
    url      = env("DATABASE_URL")
}

generator client {
    provider = "prax-client-rust"
    output   = "./src/generated"
}

/// A user in the system
model User {
    id        Int      @id @auto
    email     String   @unique @db.VarChar(255)
    name      String?  @db.VarChar(100)
    createdAt DateTime @default(now()) @map("created_at")
    updatedAt DateTime @updatedAt @map("updated_at")

    @@map("users")
}
"#
        }
        DatabaseProvider::Sqlite => {
            r#"// Prax Schema File
// Learn more at https://prax.dev/docs/schema

datasource db {
    provider = "sqlite"
    url      = env("DATABASE_URL")
}

generator client {
    provider = "prax-client-rust"
    output   = "./src/generated"
}

/// A user in the system
model User {
    id        Int      @id @auto
    email     String   @unique
    name      String?
    createdAt DateTime @default(now()) @map("created_at")
    updatedAt DateTime @updatedAt @map("updated_at")

    @@map("users")
}
"#
        }
    };

    std::fs::write(path, schema)?;
    Ok(())
}

/// Create a minimal schema file (without examples)
fn create_minimal_schema(path: &Path, provider: DatabaseProvider) -> CliResult<()> {
    let schema = format!(
        r#"// Prax Schema File
// Learn more at https://prax.dev/docs/schema

datasource db {{
    provider = "{}"
    url      = env("DATABASE_URL")
}}

generator client {{
    provider = "prax-client-rust"
    output   = "./src/generated"
}}

// Add your models here
"#,
        provider
    );

    std::fs::write(path, schema)?;
    Ok(())
}

/// Create .env file
fn create_env_file(path: &Path, provider: DatabaseProvider, url: &Option<String>) -> CliResult<()> {
    let default_url = match provider {
        DatabaseProvider::Postgresql => "postgresql://user:password@localhost:5432/mydb",
        DatabaseProvider::Mysql => "mysql://user:password@localhost:3306/mydb",
        DatabaseProvider::Sqlite => "file:./dev.db",
    };

    let url = url.as_deref().unwrap_or(default_url);

    let content = format!(
        r#"# Database connection URL
DATABASE_URL={}

# Shadow database for migrations (optional, PostgreSQL/MySQL only)
# SHADOW_DATABASE_URL=

# Direct database URL (bypasses connection pooling)
# DIRECT_URL=
"#,
        url
    );

    std::fs::write(path, content)?;
    Ok(())
}
