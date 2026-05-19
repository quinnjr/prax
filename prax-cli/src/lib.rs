//! Prax CLI - Command-line interface for the Prax ORM.
//!
//! This crate provides the CLI tool for managing Prax projects,
//! including schema validation, code generation, and migrations.

pub mod cli;
pub mod commands;
pub mod config;
pub mod error;
pub mod output;
pub mod schema_loader;
