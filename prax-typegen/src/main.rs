//! CLI for prax-typegen.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use prax_schema::validate_schema;
use prax_typegen::{Typegen, resolve_output_dir};

#[derive(Parser)]
#[command(
    name = "prax-typegen",
    about = "Generate TypeScript interfaces and Zod schemas from Prax schema files"
)]
struct Cli {
    /// Path to the .prax schema file.
    #[arg(short, long, default_value = "schema.prax")]
    schema: PathBuf,

    /// Output directory (overrides the generator block's output).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Generator name to look for in the schema (default: "typescript").
    #[arg(short, long, default_value = "typescript")]
    generator: String,

    /// Generate only TypeScript interfaces (skip Zod schemas).
    #[arg(long)]
    interfaces_only: bool,

    /// Generate only Zod schemas (skip TypeScript interfaces).
    #[arg(long)]
    zod_only: bool,

    /// Run even if the generator block is disabled or missing.
    #[arg(long)]
    force: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let input = match std::fs::read_to_string(&cli.schema) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read {}: {e}", cli.schema.display());
            return ExitCode::FAILURE;
        }
    };

    let schema = match validate_schema(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: schema validation failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    if !cli.force {
        if let Some(generator) = schema.get_generator(&cli.generator) {
            if !generator.is_enabled() {
                eprintln!(
                    "generator '{}' is disabled (set the env var or use --force)",
                    cli.generator
                );
                return ExitCode::SUCCESS;
            }
        }
    }

    let out_dir = cli.output.unwrap_or_else(|| {
        PathBuf::from(resolve_output_dir(&schema, &cli.generator, "./generated"))
    });

    let typegen = if cli.interfaces_only {
        Typegen::interfaces_only()
    } else if cli.zod_only {
        Typegen::zod_only()
    } else {
        Typegen::new()
    };

    match typegen.write_to_dir(&schema, &out_dir) {
        Ok(files) => {
            for f in &files {
                println!("  wrote {f}");
            }
            println!(
                "prax-typegen: generated {} file(s) in {}",
                files.len(),
                out_dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
