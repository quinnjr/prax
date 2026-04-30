# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**Prax ORM** — a type-safe, async-first, Prisma-inspired ORM for Rust. Organized as a Cargo workspace of ~20 focused sub-crates that publish together to crates.io under a single workspace version (currently 0.7.3).

The top-level crate is `prax-orm` (`src/lib.rs` + root `Cargo.toml`); it re-exports the ecosystem. Sub-crates live at `prax-<name>/` and are all members of the workspace.

## Workspace layout

- **Core**: `prax-schema` (schema DSL parser + AST), `prax-query` (query builder), `prax-codegen` (proc-macros), `prax-migrate` (migration engine)
- **Database engines**: `prax-postgres`, `prax-mysql`, `prax-sqlite`, `prax-mssql`, `prax-mongodb`, `prax-duckdb`, `prax-scylladb`, `prax-cassandra`, `prax-sqlx`, `prax-pgvector`
- **Integration**: `prax-armature`, `prax-axum`, `prax-actix`, `prax-import` (Prisma/Diesel/SeaORM importers), `prax-typegen`
- **Tooling**: `prax-cli` (installs as `prax` binary; published as `prax-orm-cli`)

All internal crates use `version.workspace = true` and are referenced via `workspace = true` in their own `Cargo.toml` — the single source of truth is `[workspace.package].version` and `[workspace.dependencies]` in the root `Cargo.toml`.

## Architecture notes that span multiple files

- **Migration system uses event sourcing** (see `prax-migrate/src/event.rs`, `event_store.rs`, `state.rs`). All operations are appended as immutable events (`Applied`, `RolledBack`, `Failed`, `Resolved`) to a `_prax_migrations` table; current state is derived by replay. Rollbacks produce a new `RolledBack` event rather than mutating prior records.
- **Migration dialect abstraction** — `prax-migrate/src/dialect.rs` defines `MigrationDialect` with `SqlDialect` and `CqlDialect` impls. `SqlDialect` routes to the SQL generators (Postgres/MySQL/SQLite/MSSQL/DuckDB share one generator family in `prax-migrate/src/sql.rs`). `CqlDialect` routes to `prax-migrate/src/cql/generator.rs` for ScyllaDB/Cassandra (both share the same CQL dialect). Event-log table differs per dialect (`_prax_migrations` vs `_prax_cql_migrations`).
- **SQL generators are vendor-specific per dialect** but structurally similar — `PostgresSqlGenerator`, `MySqlGenerator`, `SqliteGenerator`, `MssqlGenerator`, `DuckDbSqlGenerator` all live in `prax-migrate/src/sql.rs` and share the same `SchemaDiff` → `MigrationSql` shape. Avoid cross-generator refactors without a plan — duplication between them is deliberate since each dialect has subtle differences.
- **Schema diff is SQL-centric**; CQL has its own `CqlSchemaDiff` with partition/clustering keys, UDTs, keyspaces, materialized views.
- **prax-cassandra and prax-scylladb are separate crates** despite sharing the CQL protocol — they use different Rust drivers (`cdrs-tokio` vs `scylla`) and expose different types. Don't try to unify them.

## Common commands

Build, lint, test:

```bash
cargo build --workspace
cargo test --workspace
cargo test -p prax-migrate              # single crate
cargo test -p prax-migrate --test duckdb_migration    # single test file
cargo test -p prax-migrate test_duckdb_generate_list_type  # single test
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all
```

Run a single doc example or binary example:

```bash
cargo run --example cql_migration -p prax-migrate
cargo run --example duckdb_migration -p prax-migrate
```

Integration tests gated behind features (don't run in default `cargo test`):

```bash
cargo test -p prax-cassandra --features cassandra-live    # requires live Cassandra at 127.0.0.1:9042
```

CLI:

```bash
cargo run --bin prax -- --help
cargo test -p prax-orm-cli --test cli_tests    # CLI integration tests
```

## Git hooks are installed and enforced

Hooks live in `.cargo-husky/hooks/`. Referenced in root `Cargo.toml` under `[package.metadata.husky.hooks]`, installed automatically via the `cargo-husky` dev-dependency on first build.

- **pre-commit**: runs `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings`
- **commit-msg**: enforces `<type>(<scope>): <description>` with **scope REQUIRED**. Valid types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`. Valid scopes include crate names (`query`, `postgres`, `mysql`, `sqlite`, `mssql`, `mongodb`, `duckdb`, `scylladb`, `cassandra`, `schema`, `codegen`, `migrate`, `cli`, `sqlx`, `armature`, `axum`, `actix`, `import`, `pgvector`, `typegen`) and special scopes (`deps`, `ci`, `docs`, `release`, `security`, `repo`, `workspace`).
- **pre-push**: runs the full test suite

**Never bypass hooks** with `--no-verify`, `-n`, `HUSKY=0`, or any equivalent. If a hook fails, fix the underlying issue — see `.cursor/rules/no-skip-hooks.mdc` for the specific remedies.

## Branching model (git-flow)

- **`main`** — production/release; never direct-commit; merged from `release/*` or `hotfix/*`.
- **`develop`** — integration branch; base for all `feature/*` and `bugfix/*`.
- **`feature/<scope>-<description>`**, **`bugfix/...`**, **`release/<semver>`**, **`hotfix/...`**.
- Feature work uses worktrees under `.worktrees/` (gitignored). Use `git worktree add .worktrees/<branch> -b feature/<branch>` rather than switching branches in the main checkout.

PRs target `develop`; `main` receives only release/hotfix merges. `feature/*` → `develop` uses squash merges; `release/*`/`hotfix/*` → `main` use merge commits.

## Release and publish

Scripts live in `scripts/`:

- **`scripts/release.sh <version> [--no-push]`** — bumps workspace version in `Cargo.toml` (workspace.package and all internal workspace.dependencies), updates CHANGELOG, runs checks. Example: `./scripts/release.sh 0.7.4 --no-push`.
- **`scripts/publish.sh [--dry-run | --allow-dirty | --version VER]`** — publishes all crates to crates.io in dependency order, waiting for the index between tiers, skipping already-published versions.

Publish order (enforced by the script): Tier 1 `prax-schema`, `prax-query` → Tier 2 `prax-codegen`, `prax-migrate`, database engines, `prax-sqlx` → Tier 3 `prax-armature`, `prax-axum`, `prax-actix`, `prax-orm-cli`, `prax-import`, `prax-pgvector`, `prax-typegen`, `prax-cassandra` → Tier 4 `prax-orm`.

The CLI test assertions hardcode the current version (`prax-cli/tests/cli_tests.rs`) — bump them when changing the workspace version.

## Feature-dev workflow conventions used here

Plans and specs live under `docs/superpowers/`:

- **`docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md`** — design specs produced via the brainstorming skill
- **`docs/superpowers/plans/YYYY-MM-DD-<topic>.md`** — implementation plans produced via writing-plans, with checkbox tasks

Completed plans from recent work (event-sourced migrations, DuckDB migrations, ScyllaDB/CQL dialect, prax-cassandra) are committed under this path and serve as reference for how similar work gets scoped. When picking up a plan to execute, prefer the subagent-driven-development flow with a worktree per branch.

## SQL safety

See `.cursor/rules/sql-safety.mdc`. Never concatenate user input into SQL; always use parameterized queries (`Filter::to_sql` / `SqlBuilder::push_param`). Identifier names (tables, columns) can't be parameterized — whitelist them via enums or constant lists.

## External URLs and projects

- crates.io: https://crates.io/crates/prax-orm (and per-crate pages)
- docs.rs: https://docs.rs/prax-orm
- Repo: https://github.com/quinnjr/prax (note: README still references the old `pegasusheavy/prax-orm` URL in places — treat `quinnjr/prax` as canonical for this environment)
