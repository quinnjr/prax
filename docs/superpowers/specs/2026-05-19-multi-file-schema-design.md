# Multi-file schema support — design spec

**Status:** Draft
**Date:** 2026-05-19
**Branch:** `feature/multi-file-schema`
**Driver:** allow Prax to load multiple `.prax` files from a directory and merge them into one cohesive schema (Prisma-style), and have `prax import --from prisma` mirror Prisma's multi-file layouts into the equivalent Prax layout.

## Motivation

Today every Prax project has exactly one `schema.prax` file. As schemas grow this becomes unwieldy — large schemas in single files have poor diff hygiene, painful merge conflicts, and force one team's domain to live alongside another's. Prisma solved this with the `prismaSchemaFolder` preview (now GA): point the schema config at a directory, every `*.prisma` file in it merges into one logical schema.

We adopt the same model, with one deliberate divergence: discovery is **recursive** so users can organize files into subdirectories (`models/`, `enums/`, `policies/`) without flag gymnastics.

## Non-goals

- No new schema-language constructs (no `include` / `import` directive).
- No partial loading or lazy parsing.
- No change to generated client code shape — multi-file in, single cohesive `src/generated/` out.
- No change to migration history or event log — the merged schema is the desired state, exactly as today.

## User-visible behavior

### Activation: auto-detect from `[schema].path`

`prax.toml`'s `[schema].path` already exists; today it's expected to be a file. After this work:

- If `[schema].path` points to an **existing file**, load that file (today's behavior, unchanged).
- If `[schema].path` points to an **existing directory**, load every `*.prax` in it recursively, merged into one schema.

No new config key. `prax init` still scaffolds `schema.prax` by default. A new `--multi-file` flag on `prax init` (or `--layout=directory`) scaffolds the directory layout instead:

```
prax/
  schema/
    datasource.prax     # datasource + generator only
    models/
      user.prax
      post.prax
```

and points `prax.toml`'s `[schema].path` at `prax/schema`.

### Discovery rules

Inside a schema directory:

- Recursive descent (subdirectories are walked).
- Match: `*.prax`, sorted lexicographically by **relative path** for determinism.
- Skip: hidden files and directories (leading `.`), symlinks, and any `target/` directory.
- Empty directory → `SchemaError::EmptySchemaDirectory { path }` rather than silently producing nothing.

### Conflict policy (hard-error with both locations)

Duplicate definitions across files are an error, **never** silently overridden. The following items must be unique across the entire directory:

| Item | Uniqueness key |
|---|---|
| `model` | name |
| `enum` | name |
| `type` (composite) | name |
| `view` | name |
| `serverGroup` | name |
| `policy` | name |
| `generator` | name (multiple distinct generators allowed) |
| `datasource` | **exactly one** across the whole directory |
| raw SQL (`@@sql(name = "…")`) | name |

Each duplicate produces a `SchemaError::DuplicateAcrossFiles { kind, name, first, second }` where both `SourceLoc`s point at the originating file + span, so the rendered miette diagnostic shows both locations under one error header. `try_merge` collects **all** conflicts before returning so users see every duplicate in one run.

### Backward compatibility

The single-file path is preserved end-to-end:
- `parse_schema(&str)` and `parse_schema_file(path)` keep their current signatures and semantics.
- Existing `schema.prax` projects continue to work with no `prax.toml` change.
- Generated output for a single-file project is byte-for-byte identical to today.

A regression snapshot test in `prax-schema` asserts this invariant.

## Architecture

### Module layout in `prax-schema`

```
prax-schema/src/
├── ast/                 (small additive change: see §AST provenance)
├── parser/              unchanged; still takes &str
├── loader/              NEW
│   ├── mod.rs           pub fn load(path) -> SchemaResult<LoadedSchema>
│   ├── discovery.rs     recursive walk, returns Vec<SourceFile>
│   ├── source.rs        SourceId, SourceMap, SourceFile
│   └── merge.rs         try_merge with collision detection
├── error.rs             additions: file-aware variants
├── validator.rs         unchanged signature; sees the merged Schema
└── lib.rs               re-exports loader::{load, LoadedSchema, SourceMap}
```

### Public surface added to `prax_schema`

```rust
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SourceId(u32);

pub struct SourceFile {
    pub path: PathBuf,
    pub content: String,
}

pub struct SourceMap {
    files: Vec<SourceFile>,           // index = SourceId.0 as usize
}

impl SourceMap {
    pub fn get(&self, id: SourceId) -> Option<&SourceFile>;
    pub fn iter(&self) -> impl Iterator<Item = (SourceId, &SourceFile)>;
    pub fn len(&self) -> usize;
}

pub struct LoadedSchema {
    pub schema: Schema,
    pub sources: SourceMap,
}

pub struct LoadError {
    pub error: SchemaError,
    pub sources: SourceMap,            // partial map built up to the point of failure
}

pub fn load(path: impl AsRef<Path>) -> Result<LoadedSchema, LoadError>;
```

`SourceId(u32)` is cheap to copy and embed in errors. The map lives on `LoadedSchema` on the success path and on `LoadError` on the failure path — either way the renderer can resolve `SourceId` → file content. Errors themselves carry only `SourceId`s, not inline buffers.

## Load flow

```
load(path):
  if path is a file → load_single(path)
  else if path is a dir → load_directory(path)
  else → Err(SchemaError::IoError { ... not found ... })

load_single(path):
  read file → assign SourceId(0) → parse_schema(content)
  stamp_source(&mut schema, SourceId(0))
  validate(&schema)
  LoadedSchema { schema, sources: { 0 => path } }

load_directory(root):
  files = discover(root)
  if files.is_empty() → Err(EmptySchemaDirectory { path: root })

  sources = SourceMap::new()
  per_file_schemas = []

  for sid, file in files.enumerate():
      sources.insert(sid, file)            # always insert first, so even a parse
                                           # failure carries content for rendering
      match parse_schema(&file.content):
          Ok(s)  → per_file_schemas.push((sid, s))
          Err(e) → return Err(LoadError {
              error: SchemaError::ParseInFile { source: sid, inner: Box::new(e) },
              sources,
          })

  for (sid, schema_i) in &mut per_file_schemas:
      stamp_source(schema_i, *sid)

  merged = Schema::new()
  all_conflicts: Vec<MergeConflict> = []
  for (_, schema_i) in per_file_schemas:
      if let Err(conflicts) = merged.try_merge(schema_i):
          all_conflicts.extend(conflicts)
  if !all_conflicts.is_empty():
      return Err(LoadError {
          error: SchemaError::from_conflicts(all_conflicts),
          sources,
      })

  if let Err(e) = validate(&merged):       # existing validator, sees merged AST
      return Err(LoadError { error: e, sources });
  Ok(LoadedSchema { schema: merged, sources })
```

Key decisions:

- **Sort order:** lexicographic on the path relative to the root, so merge order is stable across machines.
- **Per-file syntax errors are fail-fast** (we stop at the first malformed file), but **merge conflicts are batched** (all duplicates reported in one error). Mixed behavior is intentional: there's no useful partial schema if file 3 of 10 is unparseable, but there is value in seeing all duplicate-model errors at once.
- **Validation runs once on the merged schema** — relation resolution, unknown-type errors, missing `@id`, etc. all work naturally across files because by validation time everything's in one `Schema`.

### Discovery details

- Walker: the `walkdir` crate (already a transitive dep across the workspace; if absent from `prax-schema/Cargo.toml`, add it directly — it's small).
- Skip predicate, applied during the walk:
  - Hidden entries (filename starts with `.`).
  - Symlinks (do not follow).
  - Any directory literally named `target` (avoids accidentally scanning build output if a user points the schema dir at the repo root by mistake).
- Files matched: extension `.prax` (case-sensitive). Non-`.prax` files in the directory are ignored.
- Sort: collect all matched files, then sort by `path.strip_prefix(root)` lexicographically (UTF-8 byte order).

## AST provenance

Every top-level item gains an additive field:

```rust
pub source_id: Option<SourceId>,
```

Applied to: `Model`, `Enum`, `CompositeType`, `View`, `ServerGroup`, `Policy`, `Generator`, `Datasource`, `RawSql`. `Relation` is populated post-validation from already-stamped models and inherits provenance via `from_model`'s `source_id` (no separate field needed).

- Field is `Option<SourceId>` so the default constructor doesn't need to know about source IDs.
- `parse_schema(&str)` (single-buffer) leaves the field `None` — callers that don't use the loader see no behavior change.
- The loader's `stamp_source(&mut Schema, SourceId)` helper sets it on every item after parsing each file, before merging.

This is the only AST change. It's purely additive and serde-compatible (skips `None` on serialize).

## Merge semantics (`try_merge`)

The current `Schema::merge` does `IndexMap::extend` — silent overwrite. It is deprecated for one release in favor of:

```rust
impl Schema {
    pub fn try_merge(&mut self, other: Schema) -> Result<(), Vec<MergeConflict>>;
}

pub enum MergeConflict {
    DuplicateModel       { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateEnum        { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateType        { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateView        { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateServerGroup { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicatePolicy      { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateGenerator   { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateRawSql      { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    MultipleDatasource   {                existing: SourceLoc, incoming: SourceLoc },
}

pub struct SourceLoc {
    pub source: SourceId,
    pub span: Span,
}
```

Per-item rules:

| Container | Behavior |
|---|---|
| `models`, `enums`, `types`, `views`, `server_groups`, `generators` (`IndexMap<name, _>`) | Hard error on duplicate name. Otherwise insert. |
| `policies` (`Vec<Policy>`) | Hard error on duplicate `name()`. Otherwise concatenate. |
| `raw_sql` (`Vec<RawSql>`) | Hard error on duplicate `name`. Otherwise concatenate. |
| `datasource: Option<Datasource>` | Hard error if both schemas have one. Otherwise take whichever is `Some`. |
| `relations` | Empty pre-validation; nothing to merge. |

`try_merge` collects all conflicts without short-circuiting so the user sees every error in one pass.

The existing `Schema::merge` stays for one release with `#[deprecated]` pointing at `try_merge`, then is removed.

## Diagnostics

### New error variants

`SchemaError` gains:

```rust
SchemaError::ParseInFile {
    source: SourceId,
    #[source] inner: Box<SchemaError>,    // wraps the inner SyntaxError
},

SchemaError::DuplicateAcrossFiles {
    kind: &'static str,        // "model", "enum", "datasource", …
    name: String,              // empty for datasource
    first: SourceLoc,
    second: SourceLoc,
},

SchemaError::EmptySchemaDirectory { path: PathBuf },
```

### Rendering with a `SourceMap`

A small newtype pairs an error with its source map for miette rendering:

```rust
pub struct Report<'a> {
    err: &'a SchemaError,
    sources: &'a SourceMap,
}

impl miette::Diagnostic for Report<'_> {
    fn source_code(&self) -> Option<&dyn miette::SourceCode> { /* picks the right file */ }
    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> { /* two labels for cross-file conflicts */ }
}

impl SchemaError {
    pub fn report<'a>(&'a self, sources: &'a SourceMap) -> Report<'a> {
        Report { err: self, sources }
    }
}
```

For `DuplicateAcrossFiles`, the rendered miette diagnostic has two labeled spans (one per file) under a single error header. For `ParseInFile`, it unwraps to the inner `SyntaxError` and points its `source_code` at the right file's content from the map.

### Single-file path keeps existing errors

`parse_schema(&str)` still emits `SyntaxError { src, span, message }` directly — single-file callers, tests, and doctests are unchanged. The loader catches that error, wraps it in `ParseInFile { source, inner }`, and the report layer renders either form correctly.

### Loader / caller API for error handling

```rust
match prax_schema::load(&schema_path) {
    Ok(LoadedSchema { schema, sources }) => /* use schema; sources for any later rendering */,
    Err(LoadError { error, sources }) => {
        eprintln!("{:?}", miette::Report::new(error.report(&sources)));
        std::process::exit(1);
    }
}
```

For the CLI specifically: `CliError::Schema` grows to a struct variant `Schema { error: SchemaError, sources: Option<SourceMap> }` (Option because some `SchemaError`s — e.g., the `IoError` raised when the path itself doesn't exist — are produced before any file is read and have no source map). The CLI's miette setup picks `error.report(&sources)` when `Some`, falls back to the bare error otherwise.

## Integration with consumers

### `prax-cli` — five subcommands

Each command today has its own local `parse_schema(content: &str)` helper plus a `std::fs::read_to_string + parse` block. Both go away and are replaced by a single shared helper in `prax-cli/src/schema_loader.rs`:

```rust
pub fn load_schema(args_path: Option<&Path>, config: &Config) -> CliResult<LoadedSchema> {
    let path = args_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&config.schema.path));
    if !path.exists() {
        return Err(CliError::Config(format!(
            "Schema not found: {}", path.display()
        )));
    }
    prax_schema::load(&path).map_err(|LoadError { error, sources }| CliError::Schema {
        error,
        sources: Some(sources),
    })
}
```

Per-command treatment:

| Command | Behavior change |
|---|---|
| `generate` | Use `loaded.schema` for codegen. Generated output unchanged. |
| `migrate` | Use `loaded.schema` as desired state. Diffing unaffected. |
| `validate` | `load()` already validates. Print success / error report. |
| `db` (push/pull/seed) | Same as `migrate`. |
| `format` | **Per-file**: when path is a directory, walk each `*.prax` and reformat it independently, rewriting in place. Skips merge — formatting is purely syntactic. |

`format` is the **only** command that does not go through `load()`; it stays at the syntactic layer and uses `parse_schema(&str)` per file. A comment in the implementation explains why.

#### Error rendering refactor

`CliError::Schema` becomes a struct variant carrying both the error and the partial source map (which is `Some(_)` whenever the error came out of `load()`, `None` when raised earlier):

```rust
pub enum CliError {
    // …
    Schema {
        error: prax_schema::SchemaError,
        sources: Option<prax_schema::SourceMap>,
    },
}
```

The `Display` / miette impl renders via `error.report(sources)` when `sources.is_some()` and falls back to the bare error otherwise.

### `prax-codegen` — `schema_reader.rs`

`read_and_parse_schema(path: &str) -> Result<Schema, SchemaReadError>` becomes a thin wrapper around `prax_schema::load`, returning `LoadedSchema.schema`. It accepts either a file or a directory path transparently — the proc-macro path now also supports `prax::client!(path = "./prax/schema")` for multi-file projects without any macro-level changes.

`SchemaReadError` either:

- (Option 1, minimal) keeps its current shape and prints `file: line` from each `SourceLoc` directly without rendering snippets, *or*
- (Option 2, fuller) adds a `SourceMap` field and renders miette diagnostics in compile-time errors.

The plan picks Option 1 first and notes Option 2 as a follow-up — proc-macro span fidelity isn't critical for the multi-file rollout.

### `prax-import` — multi-file Prisma import

Today `prax_import::import_prisma_schema_file(path)` handles one `.prisma` file. We extend the importer to handle directories produced by Prisma's `prismaSchemaFolder` layout, emitting a mirrored directory of `.prax` files.

#### Input/output detection (no new flags)

- `--input` is a **file** → today's behavior, unchanged. `--output` defaults to `schema.prax`, must be a file path.
- `--input` is a **directory** → multi-file mode. `--output` defaults to `./prax/schema`, must be a directory path. If it exists and is non-empty, `--force` is required to overwrite.

Same auto-detect rule the loader uses — one mental model.

#### Discovery

Recursively walk `--input` for `*.prisma`, sorted lexicographically by relative path. Skip:

- Hidden files and directories.
- Symlinks.
- `node_modules/`.

(We deliberately do not reuse the loader's `target/` skip — the importer's skip set is Prisma-ecosystem-specific.)

#### Conversion pipeline (parse → merge → split-emit)

```
Phase 1: parse + merge
  for each prisma file:
    parse_prisma_schema(content) → PrismaSchema   (per-file, no validation)
    stamp each PrismaModel/PrismaEnum/PrismaDatasource/PrismaGenerator with a PrismaSourceId
  merge per-file PrismaSchemas → one PrismaSchema with full provenance
  cross-file rules:
    - hard error on duplicate model/enum/type names across files
    - hard error if more than one datasource block
    - multiple generators allowed if distinctly named

Phase 2: convert + emit
  convert_prisma_to_prax on the merged PrismaSchema → one prax Schema
  relation resolution runs once on the merged Schema (this is what makes
  cross-file relations like `Post.author: User` work cleanly)
  bucket each prax item by its PrismaSourceId
  for each PrismaSourceId:
    output_path = <output_dir>/<relative_path with .prax extension>
    emit only the items in this bucket via format_schema()
    create parent dirs as needed; write file
```

The key insight: **merge first, then split on emit**. Per-file conversion would fail on cross-file relation references (`Post` references `User` defined in `users.prisma`). Merging gives the converter the whole picture; bucketing by `PrismaSourceId` at emit time preserves the user's file organization.

#### Mirroring rules

- `prisma/schema.prisma` → `<output>/schema.prax`
- `prisma/models/user.prisma` → `<output>/models/user.prax`
- Datasource block stays in whichever output `.prax` mirrors the input `.prisma` that contained it.
- `.prisma` files containing only comments / no declarations: no `.prax` emitted (logged in CLI output).
- Empty `--input` directory: error.

#### Result

A Prisma project organized as:

```
prisma/
  schema.prisma          # datasource + generator
  models/
    user.prisma
    post.prisma
  enums/
    role.prisma
```

becomes, after `prax import --from prisma --input ./prisma --output ./prax/schema`:

```
prax/schema/
  schema.prax            # datasource + generator
  models/
    user.prax
    post.prax
  enums/
    role.prax
```

…which `prax load` then picks up directly because `[schema].path = "prax/schema"` is now a directory.

#### Importer provenance is parallel, not unified

`PrismaSourceId` in `prax-import` and `SourceId` in `prax-schema` are deliberately parallel types, not a single shared one. They operate on different ASTs (`PrismaSchema` vs `Schema`) and live in different crates. Unifying them would couple `prax-import` to `prax-schema`'s loader internals for no real gain.

### What does **not** change

- `prax-schema`'s `parse_schema(&str)` / `parse_schema_file(path)` public API.
- `Schema` field shapes (only additive `source_id: Option<SourceId>` fields on top-level items).
- `prax-query`, `prax-migrate`, all database engine crates — they consume `Schema`, agnostic to where it came from.
- Generated client code shape — multi-file in, one cohesive output out.
- Migration history / event log.

## Testing strategy

### `prax-schema` unit tests (`prax-schema/src/loader/` and `prax-schema/tests/`)

**Discovery:**
- Flat dir, three files: all picked up, sorted lexicographically.
- Nested dirs: recursive descent finds files in `models/`, `enums/`.
- `.git/`, `target/`, `.worktrees/`, hidden files: skipped.
- Non-`.prax` files (`README.md`): ignored.
- Empty directory → `EmptySchemaDirectory` error.
- Single file (path is a file): single-file path still works.

**Merge happy path:**
- `User` in `users.prax`, `Post` in `posts.prax` with `User` relation across files → relation resolves.
- `datasource` in one file, `generator` in another, models in a third → all present in merged schema.
- `source_id` stamped correctly on each item.

**Merge conflicts** (each produces `SchemaError::DuplicateAcrossFiles` with both `SourceLoc`s):
- duplicate model name across files
- duplicate enum / type / view / serverGroup / policy / generator / raw_sql
- two datasource blocks → `MultipleDatasource`
- multiple conflicts in one load → all reported in one error batch

**Parse errors:**
- Syntax error in one of many files → `ParseInFile` wraps it with the right `SourceId`.
- File content retrievable through `SourceMap` for rendering.

**Validation post-merge:**
- Unknown type referenced across files: validator reports it.

**Backward-compat regression snapshot:**
- `load("examples/prax/schema.prax")` produces a `Schema` identical (modulo `source_id = Some(0)`) to today's `parse_schema_file("examples/prax/schema.prax")`.

### Fixtures (`prax-schema/tests/fixtures/multi_file/`)

```
multi_file/
  happy_path/          # models split across 3 files, all valid
  duplicate_models/    # User in two files
  nested/              # subdirs with models in them
  empty/               # empty directory (with .gitkeep so git preserves it)
  two_datasources/     # datasource block in two files
  cross_file_relation/ # Post in one file, User in another
```

### CLI integration tests (`prax-cli/tests/`)

- `prax generate --schema ./prax/schema` (directory) emits byte-identical output to `prax generate --schema ./schema.prax` for an equivalent concatenated layout.
- `prax validate ./prax/schema` with a known duplicate model prints both file paths and line numbers.
- `prax format ./prax/schema` reformats each file in place; total file count preserved.

### Codegen tests (`prax-codegen/tests/`)

- One trybuild case where the proc-macro receives a directory path and successfully generates a client.
- Existing single-file trybuild cases stay as-is.

### Importer tests (`prax-import/tests/`)

- Single-file `.prisma` → single-file `.prax` (regression).
- Multi-file `.prisma` directory → mirrored `.prax` directory; file count and relative paths match.
- Cross-file relation in Prisma survives the round-trip and resolves cleanly when the output is then loaded via `prax_schema::load`.
- `datasource` / `generator` stay in their original (mirrored) files.
- A Prisma file with only comments → no `.prax` emitted.
- `--input` directory, `--output` non-empty directory without `--force` → error.

### Out of scope for tests

- Live database round-trips for multi-file (covered by existing engine tests; they consume `Schema`, not files).
- Performance benchmarks for very large directories. Realistic schemas have <100 files; this won't show up in profiles.

## Migration & rollout

- Single PR to `develop` containing all changes (loader, AST provenance, CLI integration, importer multi-file).
- No CHANGELOG-visible breakage — all changes additive on the public API (single-file path preserved; `Schema::merge` deprecated but kept for one release).
- A short docs note in the repo README + `examples/` showing the new directory layout.
- `prax init --multi-file` lands in the same PR so users discovering multi-file have a one-command path.

## Open questions deferred to the plan

- Should `walkdir` be added directly to `prax-schema/Cargo.toml` or pulled in via a small bespoke walker (~30 LoC)? Plan decides based on existing dep tree.
- Proc-macro error-rendering richness (option 1 vs option 2 above) — start with option 1; option 2 is a follow-up.
- Whether the `--multi-file` flag on `prax init` is `--multi-file` or `--layout=directory`. Plan picks one.
