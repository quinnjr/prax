# Multi-file Schema Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `prax.toml`'s `[schema].path` point at a directory of `*.prax` files (recursively walked, hard-error on duplicates, one cohesive output), and extend `prax import --from prisma` to mirror Prisma's multi-file layouts into the equivalent Prax tree.

**Architecture:** New `prax_schema::loader` module that walks a directory, parses each file independently, merges with collision detection, and validates the merged AST. Every top-level AST item gains an additive `source_id: Option<SourceId>` so diagnostics can point at originating files. CLI/codegen/importer route through a single `prax_schema::load(path)` entry point that auto-detects file vs. directory.

**Tech Stack:** Rust (workspace edition), `pest` (existing parser), `walkdir` (new dep in `prax-schema`), `miette` (existing diagnostics), `indexmap` (existing).

**Spec:** `docs/superpowers/specs/2026-05-19-multi-file-schema-design.md` — read this first for full context.

**Phasing:** Phase 1 (prax-schema foundation) → Phase 2 (CLI integration) → Phase 3 (codegen) → Phase 4 (Prisma importer). Each phase produces working, testable software. Phase 4 depends on Phase 1 being merged conceptually but its code lives in `prax-import` and is independent.

---

## Phase 1 — `prax-schema` foundation

### Task 1: Add `walkdir` dependency to `prax-schema`

**Files:**
- Modify: `prax-schema/Cargo.toml`
- Modify: `Cargo.toml` (root, if `walkdir` isn't already a workspace dep)

- [ ] **Step 1: Check if walkdir is already a workspace dep**

Run: `grep -n "walkdir" Cargo.toml prax-schema/Cargo.toml`
Expected: probably not present (the cli uses `globset`/`ignore` indirectly but `walkdir` isn't direct).

- [ ] **Step 2: Add to workspace dependencies**

In root `Cargo.toml` under `[workspace.dependencies]`, add (alphabetical order):

```toml
walkdir = "2.5"
```

- [ ] **Step 3: Add to prax-schema/Cargo.toml**

Under `[dependencies]`:

```toml
walkdir = { workspace = true }
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p prax-schema`
Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml prax-schema/Cargo.toml
git commit -m "build(schema): add walkdir for recursive schema directory discovery"
```

---

### Task 2: Define `SourceId`, `SourceFile`, `SourceMap`

**Files:**
- Create: `prax-schema/src/loader/mod.rs`
- Create: `prax-schema/src/loader/source.rs`
- Modify: `prax-schema/src/lib.rs:242` (add `pub mod loader;`)

- [ ] **Step 1: Write the failing test**

Create `prax-schema/src/loader/source.rs`:

```rust
//! Source provenance tracking for multi-file schemas.

use std::path::{Path, PathBuf};

/// Opaque, dense identifier for a source file in a [`SourceMap`].
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct SourceId(pub u32);

/// A single source file (path + content) loaded into the schema.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub content: String,
}

/// Map of [`SourceId`] → [`SourceFile`].
///
/// Built incrementally during loading. Empty by default.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new source file and return its [`SourceId`].
    pub fn insert(&mut self, path: impl Into<PathBuf>, content: impl Into<String>) -> SourceId {
        let id = SourceId(self.files.len() as u32);
        self.files.push(SourceFile {
            path: path.into(),
            content: content.into(),
        });
        id
    }

    pub fn get(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (SourceId, &SourceFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| (SourceId(i as u32), f))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Convenience: path for a given id.
    pub fn path_of(&self, id: SourceId) -> Option<&Path> {
        self.get(id).map(|f| f.path.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_assigns_monotonic_ids() {
        let mut map = SourceMap::new();
        let a = map.insert("a.prax", "model A {}");
        let b = map.insert("b.prax", "model B {}");
        assert_eq!(a, SourceId(0));
        assert_eq!(b, SourceId(1));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn get_returns_inserted_file() {
        let mut map = SourceMap::new();
        let id = map.insert("/tmp/x.prax", "content");
        let f = map.get(id).unwrap();
        assert_eq!(f.path.to_str().unwrap(), "/tmp/x.prax");
        assert_eq!(f.content, "content");
    }

    #[test]
    fn get_unknown_id_returns_none() {
        let map = SourceMap::new();
        assert!(map.get(SourceId(42)).is_none());
    }
}
```

Create `prax-schema/src/loader/mod.rs`:

```rust
//! Multi-file schema loader.
//!
//! See `docs/superpowers/specs/2026-05-19-multi-file-schema-design.md`.

pub mod source;

pub use source::{SourceFile, SourceId, SourceMap};
```

Wire into `prax-schema/src/lib.rs` — add after `pub mod parser;`:

```rust
pub mod loader;
```

And re-export under the existing block:

```rust
pub use loader::{SourceFile, SourceId, SourceMap};
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p prax-schema loader::source::tests`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add prax-schema/src/loader prax-schema/src/lib.rs
git commit -m "feat(schema): add SourceId/SourceMap for multi-file provenance"
```

---

### Task 3: Add `source_id` field to top-level AST items

**Files:**
- Modify: `prax-schema/src/ast/model.rs`
- Modify: `prax-schema/src/ast/types.rs` (CompositeType)
- Modify: `prax-schema/src/ast/schema.rs` (View, RawSql)
- Modify: `prax-schema/src/ast/datasource.rs`
- Modify: `prax-schema/src/ast/generator.rs`
- Modify: `prax-schema/src/ast/server_group.rs`
- Modify: `prax-schema/src/ast/policy.rs`
- Modify: `prax-schema/src/ast/mod.rs` (Enum)

- [ ] **Step 1: Add field to Model**

Locate `pub struct Model` in `prax-schema/src/ast/model.rs`. Add after the existing fields:

```rust
    /// Source file this model was parsed from (None for single-file path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<crate::loader::SourceId>,
```

The struct must already derive `Serialize, Deserialize` — add `serde::{Serialize, Deserialize}` to imports if not already there.

- [ ] **Step 2: Repeat for Enum, CompositeType, View, ServerGroup, Policy, Generator, Datasource, RawSql**

Add the identical field declaration to each. Locations:
- `Enum` — likely `prax-schema/src/ast/mod.rs` or its own file; grep `pub struct Enum`
- `CompositeType` — `prax-schema/src/ast/types.rs`
- `View` — `prax-schema/src/ast/schema.rs` (search `pub struct View`)
- `ServerGroup` — `prax-schema/src/ast/server_group.rs`
- `Policy` — `prax-schema/src/ast/policy.rs`
- `Generator` — `prax-schema/src/ast/generator.rs`
- `Datasource` — `prax-schema/src/ast/datasource.rs`
- `RawSql` — `prax-schema/src/ast/schema.rs`

Run: `grep -rn "pub struct \(Model\|Enum\|CompositeType\|View\|ServerGroup\|Policy\|Generator\|Datasource\|RawSql\)\b" prax-schema/src/ast/`
Use the results to find exact file:line for each.

- [ ] **Step 3: Compile check**

Run: `cargo build -p prax-schema`
Expected: clean compile. Any `Default` derives will still work because `Option::default() == None`.

- [ ] **Step 4: Verify serde compatibility**

Existing tests deserialize schemas. Verify they still pass:

Run: `cargo test -p prax-schema`
Expected: all existing tests pass (the new field defaults to None and is skipped on serialize).

- [ ] **Step 5: Commit**

```bash
git add prax-schema/src/ast/
git commit -m "feat(schema): add optional source_id to top-level AST items"
```

---

### Task 4: Add `stamp_source` helper

**Files:**
- Modify: `prax-schema/src/loader/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to `prax-schema/src/loader/mod.rs`:

```rust
use crate::ast::Schema;

/// Stamp every top-level item in `schema` with `source`.
///
/// Called by the loader right after parsing a per-file [`Schema`], before merging.
pub(crate) fn stamp_source(schema: &mut Schema, source: SourceId) {
    for m in schema.models.values_mut() {
        m.source_id = Some(source);
    }
    for e in schema.enums.values_mut() {
        e.source_id = Some(source);
    }
    for t in schema.types.values_mut() {
        t.source_id = Some(source);
    }
    for v in schema.views.values_mut() {
        v.source_id = Some(source);
    }
    for sg in schema.server_groups.values_mut() {
        sg.source_id = Some(source);
    }
    for p in &mut schema.policies {
        p.source_id = Some(source);
    }
    for g in schema.generators.values_mut() {
        g.source_id = Some(source);
    }
    if let Some(ds) = &mut schema.datasource {
        ds.source_id = Some(source);
    }
    for r in &mut schema.raw_sql {
        r.source_id = Some(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_schema;

    #[test]
    fn stamp_marks_all_items() {
        let mut schema = parse_schema(
            r#"
            datasource db { provider = "postgresql" url = "x" }
            generator client { provider = "prax-client" }
            enum Role { User Admin }
            model User { id Int @id @auto role Role }
            "#,
        )
        .unwrap();
        stamp_source(&mut schema, SourceId(7));
        assert_eq!(schema.models["User"].source_id, Some(SourceId(7)));
        assert_eq!(schema.enums["Role"].source_id, Some(SourceId(7)));
        assert_eq!(schema.datasource.unwrap().source_id, Some(SourceId(7)));
        assert_eq!(schema.generators["client"].source_id, Some(SourceId(7)));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p prax-schema loader::tests::stamp_marks_all_items`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add prax-schema/src/loader/mod.rs
git commit -m "feat(schema): add stamp_source helper for loader provenance"
```

---

### Task 5: Add new `SchemaError` variants and `SourceLoc`

**Files:**
- Modify: `prax-schema/src/error.rs`
- Modify: `prax-schema/src/loader/source.rs` (add `SourceLoc`)

- [ ] **Step 1: Add SourceLoc to source.rs**

In `prax-schema/src/loader/source.rs`, append:

```rust
use crate::ast::Span;

/// A (source file id, span) pair used in cross-file diagnostics.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SourceLoc {
    pub source: SourceId,
    pub span: Span,
}

impl SourceLoc {
    pub fn new(source: SourceId, span: Span) -> Self {
        Self { source, span }
    }
}
```

Re-export from `loader/mod.rs`:

```rust
pub use source::{SourceFile, SourceId, SourceLoc, SourceMap};
```

And from `lib.rs`:

```rust
pub use loader::{SourceFile, SourceId, SourceLoc, SourceMap};
```

- [ ] **Step 2: Add error variants**

In `prax-schema/src/error.rs`, append new variants to the `SchemaError` enum (after the existing variants, before any closing brace):

```rust
    /// Parse failure in a specific file within a multi-file schema directory.
    #[error("parse error in {source:?}")]
    #[diagnostic(code(prax::schema::parse_in_file))]
    ParseInFile {
        source: crate::loader::SourceId,
        #[source]
        inner: Box<SchemaError>,
    },

    /// Same-named item declared in two source files.
    #[error("duplicate {kind} `{name}` declared in two files")]
    #[diagnostic(code(prax::schema::duplicate_across_files))]
    DuplicateAcrossFiles {
        kind: &'static str,
        name: String,
        first: crate::loader::SourceLoc,
        second: crate::loader::SourceLoc,
    },

    /// More than one `datasource` block across source files.
    #[error("multiple datasource blocks declared (exactly one allowed across all files)")]
    #[diagnostic(code(prax::schema::multiple_datasource))]
    MultipleDatasource {
        first: crate::loader::SourceLoc,
        second: crate::loader::SourceLoc,
    },

    /// Schema directory contained no `*.prax` files.
    #[error("schema directory `{path}` contains no .prax files")]
    #[diagnostic(code(prax::schema::empty_directory))]
    EmptySchemaDirectory { path: std::path::PathBuf },
```

- [ ] **Step 3: Compile check**

Run: `cargo build -p prax-schema`
Expected: clean compile (note: miette `#[diagnostic]` and thiserror `#[error]` macros pick up the new variants automatically).

- [ ] **Step 4: Add a small test**

Append to `prax-schema/src/error.rs` (or its test mod) — verifies the new variants format correctly:

```rust
#[cfg(test)]
mod multi_file_error_tests {
    use super::*;
    use crate::ast::Span;
    use crate::loader::{SourceId, SourceLoc};

    #[test]
    fn duplicate_across_files_displays_name_and_kind() {
        let err = SchemaError::DuplicateAcrossFiles {
            kind: "model",
            name: "User".to_string(),
            first: SourceLoc::new(SourceId(0), Span::new(0, 10)),
            second: SourceLoc::new(SourceId(1), Span::new(0, 10)),
        };
        let msg = format!("{err}");
        assert!(msg.contains("duplicate model"));
        assert!(msg.contains("User"));
    }

    #[test]
    fn empty_directory_displays_path() {
        let err = SchemaError::EmptySchemaDirectory {
            path: std::path::PathBuf::from("/tmp/empty"),
        };
        assert!(format!("{err}").contains("/tmp/empty"));
    }
}
```

Run: `cargo test -p prax-schema multi_file_error_tests`
Expected: 2 pass.

- [ ] **Step 5: Commit**

```bash
git add prax-schema/src/error.rs prax-schema/src/loader/source.rs prax-schema/src/loader/mod.rs prax-schema/src/lib.rs
git commit -m "feat(schema): add ParseInFile/DuplicateAcrossFiles/EmptyDir error variants"
```

---

### Task 6: Add `MergeConflict` and `Schema::try_merge`

**Files:**
- Create: `prax-schema/src/loader/merge.rs`
- Modify: `prax-schema/src/loader/mod.rs` (export merge)
- Modify: `prax-schema/src/ast/schema.rs` (deprecate `merge`, add `try_merge`)

- [ ] **Step 1: Define MergeConflict in merge.rs**

Create `prax-schema/src/loader/merge.rs`:

```rust
//! Schema merging with cross-file collision detection.

use smol_str::SmolStr;

use super::source::SourceLoc;

/// A single conflict found while merging two [`Schema`]s.
///
/// Collected without short-circuiting so the loader can report every duplicate
/// in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeConflict {
    DuplicateModel { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateEnum { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateType { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateView { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateServerGroup { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicatePolicy { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateGenerator { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    DuplicateRawSql { name: SmolStr, existing: SourceLoc, incoming: SourceLoc },
    MultipleDatasource { existing: SourceLoc, incoming: SourceLoc },
}
```

Re-export from `loader/mod.rs`:

```rust
pub mod merge;
pub use merge::MergeConflict;
```

- [ ] **Step 2: Add try_merge to Schema**

In `prax-schema/src/ast/schema.rs`, mark the existing `merge` as deprecated and add `try_merge`. Find the existing `pub fn merge`:

```rust
    /// Merge another schema into this one.
    pub fn merge(&mut self, other: Schema) {
        self.models.extend(other.models);
        // ...existing body...
    }
```

Replace with (keep existing `merge` but deprecate it):

```rust
    /// Merge another schema into this one (deprecated: use [`Schema::try_merge`]).
    ///
    /// Silently overwrites duplicates. Kept for backward compatibility; will be
    /// removed in a future release.
    #[deprecated(since = "0.9.8", note = "use try_merge for collision-aware merging")]
    pub fn merge(&mut self, other: Schema) {
        self.models.extend(other.models);
        self.enums.extend(other.enums);
        self.types.extend(other.types);
        self.views.extend(other.views);
        self.server_groups.extend(other.server_groups);
        self.policies.extend(other.policies);
        self.raw_sql.extend(other.raw_sql);
    }

    /// Merge `other` into `self`, returning every collision found rather than
    /// silently overwriting.
    ///
    /// Items whose `source_id` is `None` are treated as locationless (used in
    /// tests that don't go through the loader); their span is reported as
    /// `Span::new(0, 0)` and source as `SourceId(u32::MAX)`. In production
    /// usage the loader stamps every item via `stamp_source` first.
    pub fn try_merge(&mut self, other: Schema) -> Result<(), Vec<crate::loader::MergeConflict>> {
        use crate::loader::{MergeConflict, SourceId, SourceLoc};
        use crate::ast::Span;

        fn loc<T: HasProvenance>(item: &T) -> SourceLoc {
            SourceLoc::new(
                item.source_id().unwrap_or(SourceId(u32::MAX)),
                item.span(),
            )
        }

        let mut conflicts = Vec::new();

        // Models
        for (name, m) in other.models {
            if let Some(existing) = self.models.get(&name) {
                conflicts.push(MergeConflict::DuplicateModel {
                    name: name.clone(),
                    existing: loc(existing),
                    incoming: loc(&m),
                });
            } else {
                self.models.insert(name, m);
            }
        }

        // Enums
        for (name, e) in other.enums {
            if let Some(existing) = self.enums.get(&name) {
                conflicts.push(MergeConflict::DuplicateEnum {
                    name: name.clone(),
                    existing: loc(existing),
                    incoming: loc(&e),
                });
            } else {
                self.enums.insert(name, e);
            }
        }

        // Composite types
        for (name, t) in other.types {
            if let Some(existing) = self.types.get(&name) {
                conflicts.push(MergeConflict::DuplicateType {
                    name: name.clone(),
                    existing: loc(existing),
                    incoming: loc(&t),
                });
            } else {
                self.types.insert(name, t);
            }
        }

        // Views
        for (name, v) in other.views {
            if let Some(existing) = self.views.get(&name) {
                conflicts.push(MergeConflict::DuplicateView {
                    name: name.clone(),
                    existing: loc(existing),
                    incoming: loc(&v),
                });
            } else {
                self.views.insert(name, v);
            }
        }

        // Server groups
        for (name, sg) in other.server_groups {
            if let Some(existing) = self.server_groups.get(&name) {
                conflicts.push(MergeConflict::DuplicateServerGroup {
                    name: name.clone(),
                    existing: loc(existing),
                    incoming: loc(&sg),
                });
            } else {
                self.server_groups.insert(name, sg);
            }
        }

        // Generators
        for (name, g) in other.generators {
            if let Some(existing) = self.generators.get(&name) {
                conflicts.push(MergeConflict::DuplicateGenerator {
                    name: name.clone(),
                    existing: loc(existing),
                    incoming: loc(&g),
                });
            } else {
                self.generators.insert(name, g);
            }
        }

        // Policies: Vec, key is policy.name()
        for p in other.policies {
            if let Some(existing) = self.policies.iter().find(|x| x.name() == p.name()) {
                conflicts.push(MergeConflict::DuplicatePolicy {
                    name: SmolStr::new(p.name()),
                    existing: loc(existing),
                    incoming: loc(&p),
                });
            } else {
                self.policies.push(p);
            }
        }

        // Raw SQL: Vec, key is raw.name
        for r in other.raw_sql {
            if let Some(existing) = self.raw_sql.iter().find(|x| x.name == r.name) {
                conflicts.push(MergeConflict::DuplicateRawSql {
                    name: r.name.clone(),
                    existing: loc(existing),
                    incoming: loc(&r),
                });
            } else {
                self.raw_sql.push(r);
            }
        }

        // Datasource: at most one across all files
        match (&self.datasource, other.datasource) {
            (Some(existing), Some(incoming)) => {
                conflicts.push(MergeConflict::MultipleDatasource {
                    existing: loc(existing),
                    incoming: loc(&incoming),
                });
            }
            (None, Some(incoming)) => self.datasource = Some(incoming),
            (_, None) => {}
        }

        if conflicts.is_empty() {
            Ok(())
        } else {
            Err(conflicts)
        }
    }
}

/// Internal helper trait used by [`Schema::try_merge`] to extract location info.
trait HasProvenance {
    fn source_id(&self) -> Option<crate::loader::SourceId>;
    fn span(&self) -> crate::ast::Span;
}

macro_rules! impl_has_provenance {
    ($t:ty) => {
        impl HasProvenance for $t {
            fn source_id(&self) -> Option<crate::loader::SourceId> {
                self.source_id
            }
            fn span(&self) -> crate::ast::Span {
                self.span
            }
        }
    };
    ($t:ty, span_method) => {
        impl HasProvenance for $t {
            fn source_id(&self) -> Option<crate::loader::SourceId> {
                self.source_id
            }
            fn span(&self) -> crate::ast::Span {
                self.span()
            }
        }
    };
}

impl_has_provenance!(crate::ast::Model);
impl_has_provenance!(crate::ast::Enum);
impl_has_provenance!(crate::ast::CompositeType);
impl_has_provenance!(crate::ast::View);
impl_has_provenance!(crate::ast::ServerGroup);
impl_has_provenance!(crate::ast::Generator);
impl_has_provenance!(crate::ast::Datasource);

// Policy and RawSql may not have a `span: Span` field; if not, use Span::default().
impl HasProvenance for crate::ast::Policy {
    fn source_id(&self) -> Option<crate::loader::SourceId> {
        self.source_id
    }
    fn span(&self) -> crate::ast::Span {
        self.span()
    }
}

impl HasProvenance for crate::ast::RawSql {
    fn source_id(&self) -> Option<crate::loader::SourceId> {
        self.source_id
    }
    fn span(&self) -> crate::ast::Span {
        crate::ast::Span::new(0, 0)
    }
}
```

> **Implementation note:** if any of the AST structs use a `span()` accessor instead of a `span: Span` field, adjust the `impl_has_provenance!` macro arm or write the impl by hand. Check with: `grep -n "pub span\|fn span(" prax-schema/src/ast/{model,enum,types,policy}.rs`.

- [ ] **Step 3: Write the failing test**

Append to `prax-schema/src/loader/merge.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::ast::Schema;
    use crate::loader::{SourceId, stamp_source};
    use crate::parser::parse_schema;

    use super::MergeConflict;

    fn stamped(input: &str, sid: u32) -> Schema {
        let mut s = parse_schema(input).unwrap();
        stamp_source(&mut s, SourceId(sid));
        s
    }

    #[test]
    fn merge_distinct_models_succeeds() {
        let mut a = stamped("model A { id Int @id @auto }", 0);
        let b = stamped("model B { id Int @id @auto }", 1);
        assert!(a.try_merge(b).is_ok());
        assert!(a.get_model("A").is_some());
        assert!(a.get_model("B").is_some());
        assert_eq!(a.get_model("B").unwrap().source_id, Some(SourceId(1)));
    }

    #[test]
    fn merge_duplicate_models_reports_both_locations() {
        let mut a = stamped("model User { id Int @id @auto }", 0);
        let b = stamped("model User { id Int @id @auto }", 1);
        let err = a.try_merge(b).unwrap_err();
        assert_eq!(err.len(), 1);
        match &err[0] {
            MergeConflict::DuplicateModel { name, existing, incoming } => {
                assert_eq!(name.as_str(), "User");
                assert_eq!(existing.source, SourceId(0));
                assert_eq!(incoming.source, SourceId(1));
            }
            other => panic!("unexpected conflict: {other:?}"),
        }
    }

    #[test]
    fn merge_collects_all_conflicts_without_short_circuit() {
        let mut a = stamped(
            "model A { id Int @id @auto } model B { id Int @id @auto }",
            0,
        );
        let b = stamped(
            "model A { id Int @id @auto } model B { id Int @id @auto }",
            1,
        );
        let err = a.try_merge(b).unwrap_err();
        assert_eq!(err.len(), 2);
    }

    #[test]
    fn merge_two_datasources_errors() {
        let mut a = stamped(
            r#"datasource db { provider = "postgresql" url = "x" }"#,
            0,
        );
        let b = stamped(
            r#"datasource db { provider = "postgresql" url = "y" }"#,
            1,
        );
        let err = a.try_merge(b).unwrap_err();
        assert!(matches!(err[0], MergeConflict::MultipleDatasource { .. }));
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p prax-schema loader::merge`
Expected: 4 pass.

(If the `HasProvenance` macro arms don't match an AST struct's actual shape, the build will fail with a clear error. Fix by writing the impl by hand using whatever `span()` accessor or field exists.)

- [ ] **Step 5: Commit**

```bash
git add prax-schema/src/loader/merge.rs prax-schema/src/loader/mod.rs prax-schema/src/ast/schema.rs
git commit -m "feat(schema): add try_merge with cross-file collision detection"
```

---

### Task 7: Discovery — recursive `*.prax` walker

**Files:**
- Create: `prax-schema/src/loader/discovery.rs`
- Modify: `prax-schema/src/loader/mod.rs` (export discovery)

- [ ] **Step 1: Define the walker**

Create `prax-schema/src/loader/discovery.rs`:

```rust
//! Recursive `*.prax` discovery for multi-file schema directories.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{SchemaError, SchemaResult};

/// A discovered `*.prax` file with its absolute and relative paths.
#[derive(Debug, Clone)]
pub struct Discovered {
    /// Absolute path on disk.
    pub absolute: PathBuf,
    /// Path relative to the discovery root (used for sort order + emit mirroring).
    pub relative: PathBuf,
}

/// Recursively find all `*.prax` files under `root`, sorted lexicographically
/// by the relative path.
///
/// Skipped:
/// - Hidden entries (filename starts with `.`)
/// - Symlinks (not followed)
/// - Any directory named `target`
pub fn discover(root: impl AsRef<Path>) -> SchemaResult<Vec<Discovered>> {
    let root = root.as_ref();
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut out = Vec::new();
    for entry in WalkDir::new(&canonical_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e))
    {
        let entry = entry.map_err(|e| SchemaError::IoError {
            path: e.path().map(|p| p.display().to_string()).unwrap_or_default(),
            source: e.into(),
        })?;

        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("prax") {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(&canonical_root)
            .unwrap_or(entry.path())
            .to_path_buf();

        out.push(Discovered {
            absolute: entry.path().to_path_buf(),
            relative,
        });
    }

    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

fn is_skipped(entry: &walkdir::DirEntry) -> bool {
    // Always allow the root itself.
    if entry.depth() == 0 {
        return false;
    }
    if let Some(name) = entry.file_name().to_str() {
        if name.starts_with('.') {
            return true;
        }
        if entry.file_type().is_dir() && name == "target" {
            return true;
        }
    }
    if entry.file_type().is_symlink() {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, content: &str) {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    #[test]
    fn flat_directory_returns_sorted_prax_files() {
        let dir = tempdir().unwrap();
        write(dir.path(), "b.prax", "// b");
        write(dir.path(), "a.prax", "// a");
        write(dir.path(), "c.prax", "// c");

        let found = discover(dir.path()).unwrap();
        let names: Vec<_> = found.iter().map(|d| d.relative.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a.prax", "b.prax", "c.prax"]);
    }

    #[test]
    fn recursive_descent_finds_nested_files() {
        let dir = tempdir().unwrap();
        write(dir.path(), "schema.prax", "// root");
        write(dir.path(), "models/user.prax", "model U {}");
        write(dir.path(), "models/post.prax", "model P {}");
        write(dir.path(), "enums/role.prax", "enum R {}");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 4);
    }

    #[test]
    fn hidden_dirs_are_skipped() {
        let dir = tempdir().unwrap();
        write(dir.path(), "ok.prax", "// ok");
        write(dir.path(), ".git/HEAD", "// not prax");
        write(dir.path(), ".cache/bad.prax", "// skipped");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].relative.to_str().unwrap(), "ok.prax");
    }

    #[test]
    fn target_directory_is_skipped() {
        let dir = tempdir().unwrap();
        write(dir.path(), "ok.prax", "// ok");
        write(dir.path(), "target/build.prax", "// skipped");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn non_prax_files_ignored() {
        let dir = tempdir().unwrap();
        write(dir.path(), "ok.prax", "// ok");
        write(dir.path(), "README.md", "# readme");
        write(dir.path(), "schema.prisma", "// wrong ext");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
    }
}
```

- [ ] **Step 2: Add tempfile as dev-dependency**

In `prax-schema/Cargo.toml` under `[dev-dependencies]`:

```toml
tempfile = { workspace = true }
```

(Verify `tempfile` is already a workspace dep in root `Cargo.toml`; if not, add `tempfile = "3"` to `[workspace.dependencies]`.)

- [ ] **Step 3: Wire into loader/mod.rs**

```rust
pub mod discovery;
pub use discovery::Discovered;
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p prax-schema loader::discovery`
Expected: 5 pass.

- [ ] **Step 5: Commit**

```bash
git add prax-schema/src/loader prax-schema/Cargo.toml Cargo.toml
git commit -m "feat(schema): add recursive .prax file discovery"
```

---

### Task 8: `LoadedSchema`, `LoadError`, and the `load` entry point

**Files:**
- Modify: `prax-schema/src/loader/mod.rs`

- [ ] **Step 1: Add LoadedSchema and LoadError**

Append to `prax-schema/src/loader/mod.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::error::SchemaError;
use crate::ast::Schema;
use crate::parser::parse_schema;
use crate::validator::validate_schema;

/// A successfully loaded multi-file (or single-file) schema, paired with the
/// source map needed for downstream diagnostics rendering.
#[derive(Debug, Clone)]
pub struct LoadedSchema {
    pub schema: Schema,
    pub sources: SourceMap,
}

/// Error returned by [`load`], carrying the partial source map built up to the
/// point of failure so the renderer can resolve [`SourceId`]s back to file
/// content.
#[derive(Debug)]
pub struct LoadError {
    pub error: SchemaError,
    pub sources: SourceMap,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Load a schema from a file or directory.
///
/// - If `path` is a file: parse the single file.
/// - If `path` is a directory: recursively find `*.prax`, parse each, merge
///   with collision detection, then validate the merged AST.
pub fn load(path: impl AsRef<Path>) -> Result<LoadedSchema, LoadError> {
    let path = path.as_ref();
    let meta = std::fs::metadata(path).map_err(|e| LoadError {
        error: SchemaError::IoError {
            path: path.display().to_string(),
            source: e,
        },
        sources: SourceMap::new(),
    })?;

    if meta.is_file() {
        load_single(path)
    } else if meta.is_dir() {
        load_directory(path)
    } else {
        Err(LoadError {
            error: SchemaError::ConfigError {
                message: format!(
                    "schema path `{}` is neither a file nor a directory",
                    path.display()
                ),
            },
            sources: SourceMap::new(),
        })
    }
}

fn load_single(path: &Path) -> Result<LoadedSchema, LoadError> {
    let mut sources = SourceMap::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return Err(LoadError {
                error: SchemaError::IoError {
                    path: path.display().to_string(),
                    source: e,
                },
                sources,
            });
        }
    };
    let sid = sources.insert(path.to_path_buf(), content.clone());

    let mut schema = match parse_schema(&content) {
        Ok(s) => s,
        Err(e) => {
            return Err(LoadError { error: e, sources });
        }
    };
    stamp_source(&mut schema, sid);

    // Re-validate by re-parsing through validate_schema for relation resolution.
    // (validate_schema reads &str — we already have the AST; the existing
    // validate path uses content. We replicate by running the validator
    // directly on the merged schema.)
    if let Err(e) = crate::validator::Validator::new().validate(&mut schema) {
        return Err(LoadError { error: e, sources });
    }

    Ok(LoadedSchema { schema, sources })
}

fn load_directory(root: &Path) -> Result<LoadedSchema, LoadError> {
    let mut sources = SourceMap::new();

    let files = match discovery::discover(root) {
        Ok(v) => v,
        Err(e) => return Err(LoadError { error: e, sources }),
    };

    if files.is_empty() {
        return Err(LoadError {
            error: SchemaError::EmptySchemaDirectory {
                path: root.to_path_buf(),
            },
            sources,
        });
    }

    // Phase 1: read + parse each file (fail-fast on syntax error)
    let mut per_file: Vec<(SourceId, Schema)> = Vec::with_capacity(files.len());
    for f in files {
        let content = match std::fs::read_to_string(&f.absolute) {
            Ok(c) => c,
            Err(e) => {
                return Err(LoadError {
                    error: SchemaError::IoError {
                        path: f.absolute.display().to_string(),
                        source: e,
                    },
                    sources,
                });
            }
        };
        let sid = sources.insert(f.absolute.clone(), content.clone());

        let mut schema_i = match parse_schema(&content) {
            Ok(s) => s,
            Err(inner) => {
                return Err(LoadError {
                    error: SchemaError::ParseInFile {
                        source: sid,
                        inner: Box::new(inner),
                    },
                    sources,
                });
            }
        };
        stamp_source(&mut schema_i, sid);
        per_file.push((sid, schema_i));
    }

    // Phase 2: merge with conflict collection
    let mut merged = Schema::new();
    let mut all_conflicts: Vec<MergeConflict> = Vec::new();
    for (_, schema_i) in per_file {
        if let Err(conflicts) = merged.try_merge(schema_i) {
            all_conflicts.extend(conflicts);
        }
    }

    if !all_conflicts.is_empty() {
        return Err(LoadError {
            error: from_conflicts(all_conflicts),
            sources,
        });
    }

    // Phase 3: validate merged schema
    if let Err(e) = crate::validator::Validator::new().validate(&mut merged) {
        return Err(LoadError { error: e, sources });
    }

    Ok(LoadedSchema { schema: merged, sources })
}

/// Bundle a batch of [`MergeConflict`]s into a single [`SchemaError`].
///
/// If exactly one conflict, returns it directly. Otherwise wraps in
/// `ValidationFailed` to display all at once.
fn from_conflicts(conflicts: Vec<MergeConflict>) -> SchemaError {
    let mut errors: Vec<SchemaError> = conflicts.into_iter().map(conflict_to_error).collect();
    if errors.len() == 1 {
        errors.remove(0)
    } else {
        SchemaError::ValidationFailed {
            count: errors.len(),
            errors,
        }
    }
}

fn conflict_to_error(c: MergeConflict) -> SchemaError {
    match c {
        MergeConflict::DuplicateModel { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "model",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicateEnum { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "enum",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicateType { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "type",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicateView { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "view",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicateServerGroup { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "serverGroup",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicatePolicy { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "policy",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicateGenerator { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "generator",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::DuplicateRawSql { name, existing, incoming } => {
            SchemaError::DuplicateAcrossFiles {
                kind: "rawSql",
                name: name.to_string(),
                first: existing,
                second: incoming,
            }
        }
        MergeConflict::MultipleDatasource { existing, incoming } => {
            SchemaError::MultipleDatasource {
                first: existing,
                second: incoming,
            }
        }
    }
}
```

> **If `Validator::new().validate(&mut schema)` doesn't match the existing API:** The validator today is invoked via `validate_schema(content: &str) -> SchemaResult<Schema>` which re-parses. Look at `prax-schema/src/validator.rs` to find the actual public entry. If only the string-form exists, add a sibling `pub fn validate_ast(schema: &mut Schema) -> SchemaResult<()>` that runs the existing validation logic on an already-parsed schema, and call it here.

- [ ] **Step 2: Re-export load/LoadedSchema/LoadError**

In `prax-schema/src/loader/mod.rs` (existing re-exports section):

```rust
pub use merge::MergeConflict;
pub use source::{SourceFile, SourceId, SourceLoc, SourceMap};
```

…stays. Then `load`, `LoadedSchema`, `LoadError` are already public from the same file.

In `prax-schema/src/lib.rs`, append to existing re-exports:

```rust
pub use loader::{LoadedSchema, LoadError, MergeConflict, load};
```

- [ ] **Step 3: Compile**

Run: `cargo build -p prax-schema`
Expected: clean. May need the `validate_ast` shim mentioned above.

- [ ] **Step 4: Commit**

```bash
git add prax-schema/src/loader/mod.rs prax-schema/src/lib.rs prax-schema/src/validator.rs
git commit -m "feat(schema): add load() entry point for single-file or directory schemas"
```

---

### Task 9: Loader integration test — happy path

**Files:**
- Create: `prax-schema/tests/multi_file_loader.rs`
- Create: `prax-schema/tests/fixtures/multi_file/happy_path/datasource.prax`
- Create: `prax-schema/tests/fixtures/multi_file/happy_path/models/user.prax`
- Create: `prax-schema/tests/fixtures/multi_file/happy_path/models/post.prax`

- [ ] **Step 1: Create fixture files**

`prax-schema/tests/fixtures/multi_file/happy_path/datasource.prax`:

```
datasource db {
    provider = "postgresql"
    url = "postgres://localhost/test"
}

generator client {
    provider = "prax-client"
}
```

`prax-schema/tests/fixtures/multi_file/happy_path/models/user.prax`:

```
model User {
    id    Int    @id @auto
    email String @unique
    posts Post[]
}
```

`prax-schema/tests/fixtures/multi_file/happy_path/models/post.prax`:

```
model Post {
    id        Int  @id @auto
    title     String
    author_id Int
    author    User @relation(fields: [author_id], references: [id])
}
```

- [ ] **Step 2: Write the failing test**

`prax-schema/tests/multi_file_loader.rs`:

```rust
use prax_schema::load;

#[test]
fn loads_directory_and_resolves_cross_file_relations() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_file/happy_path");
    let loaded = load(&path).expect("load should succeed");

    assert!(loaded.schema.get_model("User").is_some());
    assert!(loaded.schema.get_model("Post").is_some());
    assert!(loaded.schema.datasource.is_some());

    // Source map covers all three files.
    assert_eq!(loaded.sources.len(), 3);

    // Cross-file relation resolved.
    let post = loaded.schema.get_model("Post").unwrap();
    let author = post.fields.get("author").unwrap();
    assert!(author.is_relation());
}

#[test]
fn loads_single_file_unchanged() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/prax/schema.prax");
    let loaded = load(&path).expect("single-file load should work");
    assert!(loaded.sources.len() == 1);
    assert!(loaded.schema.models.len() >= 1);
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-schema --test multi_file_loader`
Expected: both pass.

- [ ] **Step 4: Commit**

```bash
git add prax-schema/tests
git commit -m "test(schema): add multi-file loader happy-path fixtures + tests"
```

---

### Task 10: Loader integration test — error paths

**Files:**
- Modify: `prax-schema/tests/multi_file_loader.rs`
- Create: `prax-schema/tests/fixtures/multi_file/duplicate_models/a.prax`
- Create: `prax-schema/tests/fixtures/multi_file/duplicate_models/b.prax`
- Create: `prax-schema/tests/fixtures/multi_file/two_datasources/a.prax`
- Create: `prax-schema/tests/fixtures/multi_file/two_datasources/b.prax`
- Create: `prax-schema/tests/fixtures/multi_file/empty/.gitkeep`

- [ ] **Step 1: Create fixture files**

`duplicate_models/a.prax`:

```
model User { id Int @id @auto email String }
```

`duplicate_models/b.prax`:

```
model User { id Int @id @auto name String }
```

`two_datasources/a.prax`:

```
datasource db { provider = "postgresql" url = "x" }
```

`two_datasources/b.prax`:

```
datasource db { provider = "postgresql" url = "y" }
model X { id Int @id @auto }
```

`empty/.gitkeep`: empty file (so git preserves the directory).

- [ ] **Step 2: Append tests**

To `prax-schema/tests/multi_file_loader.rs`:

```rust
use prax_schema::SchemaError;

#[test]
fn duplicate_model_across_files_errors() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_file/duplicate_models");
    let err = load(&path).unwrap_err();
    let msg = format!("{}", err.error);
    assert!(msg.contains("duplicate model"), "got: {msg}");
    assert!(msg.contains("User"), "got: {msg}");
    // SourceMap has both files even on error.
    assert_eq!(err.sources.len(), 2);
}

#[test]
fn two_datasources_errors() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_file/two_datasources");
    let err = load(&path).unwrap_err();
    match err.error {
        SchemaError::MultipleDatasource { .. } => {}
        SchemaError::ValidationFailed { errors, .. } => {
            assert!(errors.iter().any(|e| matches!(e, SchemaError::MultipleDatasource { .. })));
        }
        other => panic!("expected MultipleDatasource, got {other:?}"),
    }
}

#[test]
fn empty_directory_errors() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_file/empty");
    let err = load(&path).unwrap_err();
    assert!(matches!(err.error, SchemaError::EmptySchemaDirectory { .. }));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-schema --test multi_file_loader`
Expected: 5 pass total.

- [ ] **Step 4: Commit**

```bash
git add prax-schema/tests
git commit -m "test(schema): add multi-file loader error-path fixtures + tests"
```

---

## Phase 2 — `prax-cli` integration

### Task 11: `schema_loader.rs` helper + `CliError::Schema` refactor

**Files:**
- Create: `prax-cli/src/schema_loader.rs`
- Modify: `prax-cli/src/lib.rs` or `prax-cli/src/main.rs` (`pub mod schema_loader;`)
- Modify: `prax-cli/src/error.rs`

- [ ] **Step 1: Refactor CliError::Schema**

Find the existing `CliError::Schema` variant in `prax-cli/src/error.rs`. Today it's likely a tuple variant `Schema(String)`. Replace with:

```rust
    Schema {
        error: prax_schema::SchemaError,
        sources: Option<prax_schema::SourceMap>,
    },
```

Update its `Display` and miette impls to render via `error.report(&sources)` when `sources.is_some()` (the `report` helper lands in Task 12). For now, just print the underlying error:

```rust
    CliError::Schema { error, .. } => write!(f, "{error}"),
```

Update every existing call site that constructs `CliError::Schema(string)` — they'll exist in each command's local `parse_schema` helper. Grep:

```bash
grep -rn "CliError::Schema" prax-cli/src/
```

For now, convert each to:

```rust
CliError::Schema {
    error: prax_schema::SchemaError::ConfigError { message: <the old string> },
    sources: None,
}
```

…knowing Task 12+ replaces those callsites entirely.

- [ ] **Step 2: Add the loader helper**

Create `prax-cli/src/schema_loader.rs`:

```rust
//! Shared schema-loading helper used by every CLI command.

use std::path::{Path, PathBuf};

use prax_schema::{LoadedSchema, LoadError, load};

use crate::config::{Config, SCHEMA_FILE_PATH};
use crate::error::{CliError, CliResult};

/// Resolve the schema path (from `--schema` flag or config), then load it.
///
/// Returns `LoadedSchema` whose `sources` is non-empty whenever a file was
/// successfully read at least partially.
pub fn load_schema(
    args_path: Option<&Path>,
    config: &Config,
) -> CliResult<LoadedSchema> {
    let path: PathBuf = args_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&config.schema.path));
    if !path.exists() {
        return Err(CliError::Config(format!(
            "Schema not found: {}",
            path.display()
        )));
    }
    load(&path).map_err(|LoadError { error, sources }| CliError::Schema {
        error,
        sources: Some(sources),
    })
}

/// Variant that uses the default schema path from config when no override.
pub fn load_schema_default(config: &Config) -> CliResult<LoadedSchema> {
    load_schema(None, config)
}
```

- [ ] **Step 3: Wire into lib/main**

In `prax-cli/src/lib.rs` (or `main.rs`, wherever modules are declared):

```rust
pub mod schema_loader;
```

- [ ] **Step 4: Compile**

Run: `cargo build -p prax-orm-cli`
Expected: clean build, possibly with warnings about unused old `parse_schema` helpers in each command (those go away next).

- [ ] **Step 5: Commit**

```bash
git add prax-cli/src/schema_loader.rs prax-cli/src/error.rs prax-cli/src/lib.rs prax-cli/src/main.rs
git commit -m "feat(cli): add schema_loader helper for multi-file aware loading"
```

---

### Task 12: Add `SchemaError::report()` helper for miette rendering

**Files:**
- Modify: `prax-schema/src/error.rs`

- [ ] **Step 1: Add the Report newtype**

Append to `prax-schema/src/error.rs`:

```rust
use crate::loader::{SourceId, SourceMap};

/// Pairs a [`SchemaError`] with a [`SourceMap`] so miette can render
/// file-aware diagnostics. Produced by [`SchemaError::report`].
pub struct Report<'a> {
    pub(crate) err: &'a SchemaError,
    pub(crate) sources: &'a SourceMap,
}

impl SchemaError {
    /// Pair this error with a source map for miette rendering.
    pub fn report<'a>(&'a self, sources: &'a SourceMap) -> Report<'a> {
        Report { err: self, sources }
    }
}

impl std::fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.err.fmt(f)?;
        match self.err {
            SchemaError::DuplicateAcrossFiles { kind, name, first, second } => {
                if let (Some(a), Some(b)) = (self.sources.path_of(first.source), self.sources.path_of(second.source)) {
                    write!(f, "\n  first:  {} (span {}..{})", a.display(), first.span.start, first.span.end)?;
                    write!(f, "\n  second: {} (span {}..{})", b.display(), second.span.start, second.span.end)?;
                }
            }
            SchemaError::MultipleDatasource { first, second } => {
                let _ = name_unused();
                if let (Some(a), Some(b)) = (self.sources.path_of(first.source), self.sources.path_of(second.source)) {
                    write!(f, "\n  first:  {}", a.display())?;
                    write!(f, "\n  second: {}", b.display())?;
                }
            }
            SchemaError::ParseInFile { source, inner } => {
                if let Some(p) = self.sources.path_of(*source) {
                    write!(f, "\n  in file: {}", p.display())?;
                }
                write!(f, "\n  detail: {inner}")?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl std::fmt::Debug for Report<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Report<'_> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.err)
    }
}

// Helper to keep the compiler happy in unused-binding match arms.
fn name_unused() {}
```

> **Note:** A proper `miette::Diagnostic` impl with `source_code()` + labeled spans is a follow-up. For now, this `Display`-based renderer prints clear file paths for every multi-file error, which is the user-facing requirement. Migration to full miette spans goes in a later PR.

- [ ] **Step 2: Wire CliError::Schema Display**

In `prax-cli/src/error.rs`, update the `Schema { error, sources }` Display arm:

```rust
CliError::Schema { error, sources } => match sources {
    Some(map) => write!(f, "{}", error.report(map)),
    None => write!(f, "{error}"),
},
```

- [ ] **Step 3: Test**

Add to `prax-schema/src/error.rs` test module:

```rust
#[test]
fn report_prints_file_paths_for_duplicate() {
    use crate::loader::{SourceId, SourceLoc, SourceMap};
    use crate::ast::Span;

    let mut sources = SourceMap::new();
    sources.insert("/tmp/a.prax", "x");
    sources.insert("/tmp/b.prax", "y");

    let err = SchemaError::DuplicateAcrossFiles {
        kind: "model",
        name: "User".into(),
        first: SourceLoc::new(SourceId(0), Span::new(0, 5)),
        second: SourceLoc::new(SourceId(1), Span::new(0, 5)),
    };
    let rendered = format!("{}", err.report(&sources));
    assert!(rendered.contains("/tmp/a.prax"));
    assert!(rendered.contains("/tmp/b.prax"));
}
```

Run: `cargo test -p prax-schema report_prints_file_paths_for_duplicate`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add prax-schema/src/error.rs prax-cli/src/error.rs
git commit -m "feat(schema): add SchemaError::report for source-aware rendering"
```

---

### Task 13: Wire `prax-cli/src/commands/generate.rs` to the loader

**Files:**
- Modify: `prax-cli/src/commands/generate.rs`

- [ ] **Step 1: Replace the local schema-loading block**

In `prax-cli/src/commands/generate.rs`, find the existing schema-loading code (around lines 25–56). It currently:

```rust
let schema_path = args.schema.clone().unwrap_or_else(|| cwd.join(SCHEMA_FILE_PATH));
if !schema_path.exists() { /* err */ }
let schema_content = std::fs::read_to_string(&schema_path)?;
let schema = parse_schema(&schema_content)?;
```

Replace with:

```rust
use crate::schema_loader::load_schema;

// ...

let loaded = load_schema(args.schema.as_deref(), &config)?;
let schema = &loaded.schema;
```

Update `generate_code(schema, ...)` call to pass `&loaded.schema`. Remove the local `fn parse_schema(content: &str) -> CliResult<Schema>` helper at the bottom of the file (lines ~91–97) and the no-op `fn validate_schema`.

- [ ] **Step 2: Output a schema path log line that works for both modes**

Where `output::kv("Schema", &schema_path.display().to_string())` is called, replace with:

```rust
let display_path = args.schema.as_deref()
    .map(|p| p.display().to_string())
    .unwrap_or_else(|| config.schema.path.clone());
output::kv("Schema", &display_path);
if loaded.sources.len() > 1 {
    output::kv("Source files", &loaded.sources.len().to_string());
}
```

- [ ] **Step 3: Build and run existing tests**

Run: `cargo build -p prax-orm-cli && cargo test -p prax-orm-cli`
Expected: existing CLI tests pass (they use single-file schemas, which still work).

- [ ] **Step 4: Commit**

```bash
git add prax-cli/src/commands/generate.rs
git commit -m "refactor(cli): route generate through prax_schema::load"
```

---

### Task 14: Wire migrate, validate, db through the loader

**Files:**
- Modify: `prax-cli/src/commands/migrate.rs`
- Modify: `prax-cli/src/commands/validate.rs`
- Modify: `prax-cli/src/commands/db.rs`

Apply the same pattern as Task 13 to each command:

- [ ] **Step 1: migrate.rs — both call sites**

`prax-cli/src/commands/migrate.rs` has two schema reads (line ~48 and line ~328). Replace each:

```rust
let loaded = crate::schema_loader::load_schema(args.schema.as_deref(), &config)?;
let schema = &loaded.schema;
```

Delete the bottom local `fn parse_schema(content: &str) -> CliResult<Schema>` (around line 442).

- [ ] **Step 2: validate.rs**

`prax-cli/src/commands/validate.rs:28`: same replacement. Delete local helper at line ~109.

- [ ] **Step 3: db.rs**

`prax-cli/src/commands/db.rs:46`: same replacement. Delete local helper at line ~373.

- [ ] **Step 4: Compile + test**

Run: `cargo build -p prax-orm-cli && cargo test -p prax-orm-cli`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add prax-cli/src/commands
git commit -m "refactor(cli): route migrate/validate/db through prax_schema::load"
```

---

### Task 15: `prax format` — per-file directory handling

**Files:**
- Modify: `prax-cli/src/commands/format.rs`

- [ ] **Step 1: Replace single-file body with branching logic**

`prax-cli/src/commands/format.rs` today reads one file, parses it, reformats, writes it. Refactor:

```rust
use std::path::Path;
use prax_schema::parse_schema;
use prax_schema::loader::discovery::discover;

pub async fn run(args: FormatArgs) -> CliResult<()> {
    let path = resolve_path(&args);   // existing logic for resolving --schema
    if path.is_file() {
        format_one(&path)?;
    } else if path.is_dir() {
        let files = discover(&path).map_err(|e| CliError::Schema { error: e, sources: None })?;
        if files.is_empty() {
            return Err(CliError::Config(format!(
                "No .prax files found in {}",
                path.display()
            )));
        }
        for f in files {
            format_one(&f.absolute)?;
        }
    } else {
        return Err(CliError::Config(format!(
            "Schema path not found: {}",
            path.display()
        )));
    }
    Ok(())
}

fn format_one(path: &Path) -> CliResult<()> {
    let content = std::fs::read_to_string(path)?;
    let schema = parse_schema(&content).map_err(|e| CliError::Schema {
        error: e,
        sources: None,
    })?;
    let formatted = format_schema_text(&schema);   // existing renderer
    std::fs::write(path, formatted)?;
    output::success(&format!("Formatted {}", path.display()));
    Ok(())
}
```

> **Why per-file (not merged):** Formatting is purely syntactic. Merging would require a way to re-split the formatted output back into the source files, which is not how `format_schema_text` works. Per-file is correct, simpler, and preserves the user's organization. Add a `//` comment in the code explaining this.

- [ ] **Step 2: Re-export discovery::discover from prax-schema**

The format command needs `prax_schema::loader::discovery::discover` to be public. In `prax-schema/src/loader/mod.rs`:

```rust
pub mod discovery;
```

is already there from Task 7. Verify `pub fn discover` in `discovery.rs` — should already be `pub`. The `crate::loader::discovery::discover` path should already work from outside the crate as `prax_schema::loader::discovery::discover` (verify with `cargo doc -p prax-schema --open`).

- [ ] **Step 3: Build + run existing format tests**

Run: `cargo build -p prax-orm-cli`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add prax-cli/src/commands/format.rs
git commit -m "feat(cli): format walks directory schemas per-file"
```

---

### Task 16: `prax init --multi-file` flag

**Files:**
- Modify: `prax-cli/src/cli.rs` (or wherever `InitArgs` lives)
- Modify: `prax-cli/src/commands/init.rs`

- [ ] **Step 1: Add flag to InitArgs**

Find `InitArgs` (likely in `prax-cli/src/cli.rs`). Add:

```rust
    /// Scaffold a multi-file schema directory (./prax/schema/) instead of a single schema.prax.
    #[arg(long)]
    pub multi_file: bool,
```

- [ ] **Step 2: Branch in init::run**

In `prax-cli/src/commands/init.rs`, find the schema-writing block. Today it writes one file `schema.prax`. Add:

```rust
if args.multi_file {
    let root = cwd.join("prax/schema");
    std::fs::create_dir_all(root.join("models"))?;

    std::fs::write(
        root.join("datasource.prax"),
        DATASOURCE_TEMPLATE,
    )?;
    std::fs::write(
        root.join("models/example.prax"),
        EXAMPLE_MODEL_TEMPLATE,
    )?;

    // Update prax.toml's [schema].path to point at the directory.
    let config_path = cwd.join(CONFIG_FILE_NAME);
    let mut config = Config::default();
    config.schema.path = "prax/schema".to_string();
    let toml = toml::to_string(&config).unwrap();
    std::fs::write(&config_path, toml)?;

    output::success("Scaffolded multi-file schema at ./prax/schema/");
} else {
    // ... existing single-file path ...
}
```

With templates split out:

```rust
const DATASOURCE_TEMPLATE: &str = r#"datasource db {
    provider = "postgresql"
    url = env("DATABASE_URL")
}

generator client {
    provider = "prax-client"
    output   = "./src/generated"
}
"#;

const EXAMPLE_MODEL_TEMPLATE: &str = r#"/// Example model — replace me.
model Example {
    id    Int    @id @auto
    name  String
}
"#;
```

- [ ] **Step 3: Add CLI integration test**

In `prax-cli/tests/cli_tests.rs` add:

```rust
#[test]
fn init_multi_file_creates_directory_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_prax"))
        .arg("init")
        .arg("--multi-file")
        .current_dir(tmp.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(tmp.path().join("prax/schema/datasource.prax").exists());
    assert!(tmp.path().join("prax/schema/models/example.prax").exists());
    assert!(tmp.path().join("prax.toml").exists());
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-orm-cli --test cli_tests init_multi_file_creates_directory_layout`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add prax-cli/src
git commit -m "feat(cli): add --multi-file flag to prax init"
```

---

### Task 17: CLI integration tests for multi-file generate / validate

**Files:**
- Modify: `prax-cli/tests/cli_tests.rs`
- Create: `prax-cli/tests/fixtures/multi_file_basic/schema/datasource.prax`
- Create: `prax-cli/tests/fixtures/multi_file_basic/schema/models/user.prax`
- Create: `prax-cli/tests/fixtures/multi_file_basic/schema/models/post.prax`
- Create: `prax-cli/tests/fixtures/multi_file_basic/prax.toml`

- [ ] **Step 1: Create fixture**

`prax-cli/tests/fixtures/multi_file_basic/prax.toml`:

```toml
[database]
provider = "postgresql"
url = "postgres://localhost/test"

[schema]
path = "schema"

[generator.client]
output = "./generated"
```

`schema/datasource.prax`:

```
datasource db {
    provider = "postgresql"
    url = "postgres://localhost/test"
}

generator client {
    provider = "prax-client"
}
```

`schema/models/user.prax`:

```
model User {
    id    Int    @id @auto
    email String @unique
    posts Post[]
}
```

`schema/models/post.prax`:

```
model Post {
    id        Int    @id @auto
    title     String
    author_id Int
    author    User   @relation(fields: [author_id], references: [id])
}
```

- [ ] **Step 2: Add tests**

In `prax-cli/tests/cli_tests.rs`:

```rust
#[test]
fn validate_works_on_directory_schema() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_file_basic");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_prax"))
        .arg("validate")
        .current_dir(&fixture)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn generate_works_on_directory_schema() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_file_basic");
    // Generate into a temp output dir so we don't pollute the fixture.
    let out = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_prax"))
        .arg("generate")
        .arg("--output")
        .arg(out.path())
        .current_dir(&fixture)
        .status()
        .unwrap();
    assert!(status.success());
    // At minimum, the client module should exist.
    assert!(out.path().join("mod.rs").exists() || out.path().join("lib.rs").exists());
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-orm-cli --test cli_tests validate_works_on_directory_schema generate_works_on_directory_schema`
Expected: both pass.

- [ ] **Step 4: Commit**

```bash
git add prax-cli/tests
git commit -m "test(cli): add directory-schema integration tests for validate + generate"
```

---

## Phase 3 — `prax-codegen` integration

### Task 18: Route `schema_reader::read_and_parse_schema` through `load`

**Files:**
- Modify: `prax-codegen/src/schema_reader.rs`

- [ ] **Step 1: Update the function**

In `prax-codegen/src/schema_reader.rs:18`:

```rust
pub fn read_and_parse_schema(path: &str) -> Result<Schema, SchemaReadError> {
    let loaded = prax_schema::load(path).map_err(|e| {
        // SchemaReadError today wraps a string — keep that shape, format the
        // loader error with its source map for path-aware messages.
        SchemaReadError::ParseError(format!("{}", e.error.report(&e.sources)))
    })?;
    Ok(loaded.schema)
}
```

- [ ] **Step 2: Existing tests cover single-file path**

Run: `cargo test -p prax-codegen`
Expected: existing tests still pass (single-file path is preserved).

- [ ] **Step 3: Add a multi-file test**

In `prax-codegen/src/schema_reader.rs` test module, add:

```rust
#[test]
fn reads_directory_schema() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ds.prax"),
        r#"datasource db { provider = "postgresql" url = "x" }"#,
    ).unwrap();
    std::fs::write(
        dir.path().join("user.prax"),
        "model User { id Int @id @auto }",
    ).unwrap();
    let schema = read_and_parse_schema(dir.path().to_str().unwrap()).unwrap();
    assert!(schema.get_model("User").is_some());
    assert!(schema.datasource.is_some());
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p prax-codegen schema_reader`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add prax-codegen/src/schema_reader.rs
git commit -m "refactor(codegen): route schema reader through prax_schema::load"
```

---

## Phase 4 — Prisma multi-file import

### Task 19: Add `PrismaSourceId` provenance to `PrismaSchema`

**Files:**
- Modify: `prax-import/src/prisma/types.rs`

- [ ] **Step 1: Add type**

In `prax-import/src/prisma/types.rs` at the top:

```rust
/// Opaque identifier for a `.prisma` source file when importing multi-file
/// Prisma schemas. Parallel to (but deliberately distinct from)
/// `prax_schema::SourceId`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct PrismaSourceId(pub u32);
```

Add an `Option<PrismaSourceId>` field to `PrismaModel`, `PrismaEnum`, `PrismaDatasource`, and `PrismaGenerator` (or whatever they're called — grep `pub struct Prisma` in the file). Same shape:

```rust
    pub source_id: Option<PrismaSourceId>,
```

Default for these structs already exists; the new `Option` field defaults to `None`. Single-file existing path needs no changes.

- [ ] **Step 2: Compile**

Run: `cargo build -p prax-import`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add prax-import/src/prisma/types.rs
git commit -m "feat(import): add PrismaSourceId provenance to Prisma AST"
```

---

### Task 20: Multi-file Prisma parse + merge

**Files:**
- Create: `prax-import/src/prisma/multi_file.rs`
- Modify: `prax-import/src/prisma/mod.rs`

- [ ] **Step 1: Define the multi-file loader**

Create `prax-import/src/prisma/multi_file.rs`:

```rust
//! Multi-file Prisma schema discovery, parse, and merge.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::parser::parse_prisma_schema;
use super::types::{PrismaSchema, PrismaSourceId};
use crate::error::{ImportError, ImportResult};

#[derive(Debug, Clone)]
pub struct PrismaFile {
    pub absolute: PathBuf,
    pub relative: PathBuf,
}

#[derive(Debug, Default)]
pub struct PrismaSourceMap {
    files: Vec<PrismaFile>,
}

impl PrismaSourceMap {
    pub fn get(&self, id: PrismaSourceId) -> Option<&PrismaFile> {
        self.files.get(id.0 as usize)
    }
    pub fn iter(&self) -> impl Iterator<Item = (PrismaSourceId, &PrismaFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| (PrismaSourceId(i as u32), f))
    }
    pub fn len(&self) -> usize { self.files.len() }
    pub fn is_empty(&self) -> bool { self.files.is_empty() }
}

/// Discover all `*.prisma` files under `root`, sorted by relative path.
pub fn discover_prisma(root: &Path) -> ImportResult<Vec<PrismaFile>> {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    for entry in WalkDir::new(&canonical)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e))
    {
        let entry = entry.map_err(|e| ImportError::Io {
            message: format!("{e}"),
        })?;
        if !entry.file_type().is_file() { continue; }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("prisma") { continue; }
        let relative = entry
            .path()
            .strip_prefix(&canonical)
            .unwrap_or(entry.path())
            .to_path_buf();
        out.push(PrismaFile {
            absolute: entry.path().to_path_buf(),
            relative,
        });
    }
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

fn is_skipped(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 { return false; }
    if let Some(name) = entry.file_name().to_str() {
        if name.starts_with('.') { return true; }
        if entry.file_type().is_dir() && name == "node_modules" { return true; }
    }
    if entry.file_type().is_symlink() { return true; }
    false
}

/// Parse every `.prisma` file under `root`, stamp source provenance,
/// and merge into one `PrismaSchema`.
pub fn parse_and_merge_directory(root: &Path) -> ImportResult<(PrismaSchema, PrismaSourceMap)> {
    let files = discover_prisma(root)?;
    if files.is_empty() {
        return Err(ImportError::Io {
            message: format!("no .prisma files found under {}", root.display()),
        });
    }

    let mut sources = PrismaSourceMap { files: files.clone() };
    let mut merged = PrismaSchema::default();

    for (idx, f) in files.iter().enumerate() {
        let sid = PrismaSourceId(idx as u32);
        let content = std::fs::read_to_string(&f.absolute).map_err(|e| ImportError::Io {
            message: format!("read {}: {e}", f.absolute.display()),
        })?;
        let mut per_file = parse_prisma_schema(&content)?;
        stamp_prisma(&mut per_file, sid);
        try_merge_prisma(&mut merged, per_file, sid)?;
    }

    Ok((merged, sources))
}

fn stamp_prisma(s: &mut PrismaSchema, sid: PrismaSourceId) {
    for m in &mut s.models { m.source_id = Some(sid); }
    for e in &mut s.enums { e.source_id = Some(sid); }
    if let Some(ds) = &mut s.datasource { ds.source_id = Some(sid); }
    for g in &mut s.generators { g.source_id = Some(sid); }
}

fn try_merge_prisma(
    into: &mut PrismaSchema,
    other: PrismaSchema,
    incoming_sid: PrismaSourceId,
) -> ImportResult<()> {
    for m in other.models {
        if let Some(existing) = into.models.iter().find(|x| x.name == m.name) {
            return Err(ImportError::DuplicateModel {
                name: m.name.clone(),
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.models.push(m);
    }
    for e in other.enums {
        if let Some(existing) = into.enums.iter().find(|x| x.name == e.name) {
            return Err(ImportError::DuplicateEnum {
                name: e.name.clone(),
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.enums.push(e);
    }
    if let Some(incoming) = other.datasource {
        if let Some(existing) = &into.datasource {
            return Err(ImportError::MultipleDatasource {
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.datasource = Some(incoming);
    }
    for g in other.generators {
        if let Some(existing) = into.generators.iter().find(|x| x.name == g.name) {
            return Err(ImportError::DuplicateGenerator {
                name: g.name.clone(),
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.generators.push(g);
    }
    Ok(())
}
```

> **Note on `ImportError`:** the new variants (`DuplicateModel`, `DuplicateEnum`, `MultipleDatasource`, `DuplicateGenerator`, `Io`) need to exist on the type. Add them in `prax-import/src/error.rs` if they don't already.

- [ ] **Step 2: Add the new ImportError variants**

In `prax-import/src/error.rs`, add to the `ImportError` enum:

```rust
    #[error("duplicate prisma model `{name}` across files")]
    DuplicateModel { name: String, first: crate::prisma::types::PrismaSourceId, second: crate::prisma::types::PrismaSourceId },

    #[error("duplicate prisma enum `{name}` across files")]
    DuplicateEnum { name: String, first: crate::prisma::types::PrismaSourceId, second: crate::prisma::types::PrismaSourceId },

    #[error("multiple datasource blocks across prisma files")]
    MultipleDatasource { first: crate::prisma::types::PrismaSourceId, second: crate::prisma::types::PrismaSourceId },

    #[error("duplicate prisma generator `{name}` across files")]
    DuplicateGenerator { name: String, first: crate::prisma::types::PrismaSourceId, second: crate::prisma::types::PrismaSourceId },

    #[error("i/o error: {message}")]
    Io { message: String },
```

- [ ] **Step 3: Wire into the prisma module**

In `prax-import/src/prisma/mod.rs`:

```rust
pub mod multi_file;
pub use multi_file::{discover_prisma, parse_and_merge_directory, PrismaFile, PrismaSourceMap};
```

Add `walkdir = { workspace = true }` to `prax-import/Cargo.toml`.

- [ ] **Step 4: Test**

Create `prax-import/tests/multi_file_prisma.rs`:

```rust
use prax_import::prisma::parse_and_merge_directory;

#[test]
fn parses_and_merges_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.prisma"),
        r#"
        datasource db { provider = "postgresql" url = env("X") }
        model A { id Int @id @default(autoincrement()) }
        "#,
    ).unwrap();
    std::fs::write(
        dir.path().join("b.prisma"),
        "model B { id Int @id @default(autoincrement()) }",
    ).unwrap();

    let (merged, sources) = parse_and_merge_directory(dir.path()).unwrap();
    assert_eq!(merged.models.len(), 2);
    assert!(merged.datasource.is_some());
    assert_eq!(sources.len(), 2);
}

#[test]
fn duplicate_models_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.prisma"),
        "model X { id Int @id @default(autoincrement()) }",
    ).unwrap();
    std::fs::write(
        dir.path().join("b.prisma"),
        "model X { id Int @id @default(autoincrement()) }",
    ).unwrap();

    let err = parse_and_merge_directory(dir.path()).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("duplicate") && msg.contains("X"));
}
```

Run: `cargo test -p prax-import --test multi_file_prisma`
Expected: 2 pass.

- [ ] **Step 5: Commit**

```bash
git add prax-import prax-import/Cargo.toml
git commit -m "feat(import): add multi-file Prisma parse + merge"
```

---

### Task 21: Multi-file Prisma → Prax conversion + emission

**Files:**
- Modify: `prax-import/src/prisma/parser.rs` (or a new `convert.rs`)
- Modify: `prax-import/src/lib.rs` (re-exports)

- [ ] **Step 1: Add the conversion + emit function**

In `prax-import/src/prisma/parser.rs` (or a new sibling module), add:

```rust
use std::path::{Path, PathBuf};
use super::multi_file::{parse_and_merge_directory, PrismaFile, PrismaSourceMap};
use super::types::{PrismaSchema, PrismaSourceId};

/// Convert a directory of `.prisma` files to a directory of `.prax` files,
/// mirroring the input layout.
///
/// `format_text` is the function that turns a per-file `prax Schema` back into
/// `.prax` source text (the CLI's existing `format_schema`).
pub fn import_prisma_directory<F>(
    input: &Path,
    output: &Path,
    format_text: F,
    force: bool,
) -> ImportResult<usize>
where
    F: Fn(&crate::Schema) -> String,
{
    let (merged_prisma, sources) = parse_and_merge_directory(input)?;
    let merged_prax = convert_prisma_to_prax(merged_prisma.clone())?;

    // Bucket prax items by their PrismaSourceId (carried through via the
    // converter — see Step 2).
    let mut buckets: std::collections::HashMap<PrismaSourceId, crate::Schema> = Default::default();
    bucket_prax_items(&merged_prax, &mut buckets);

    // Pre-flight: check output dir
    if output.exists() {
        let is_empty = output.read_dir().map(|mut d| d.next().is_none()).unwrap_or(true);
        if !is_empty && !force {
            return Err(ImportError::Io {
                message: format!(
                    "output directory {} is not empty (pass --force to overwrite)",
                    output.display()
                ),
            });
        }
    }
    std::fs::create_dir_all(output).map_err(|e| ImportError::Io {
        message: format!("create {}: {e}", output.display()),
    })?;

    let mut emitted = 0usize;
    for (sid, prax_for_file) in &buckets {
        // Empty bucket = source had only comments, no declarations. Skip.
        if prax_for_file.models.is_empty()
            && prax_for_file.enums.is_empty()
            && prax_for_file.datasource.is_none()
            && prax_for_file.generators.is_empty()
        {
            continue;
        }

        let file = sources.get(*sid).expect("source map covers all stamped ids");
        let mut out_path = output.join(&file.relative);
        out_path.set_extension("prax");

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ImportError::Io {
                message: format!("create {}: {e}", parent.display()),
            })?;
        }

        let text = format_text(prax_for_file);
        std::fs::write(&out_path, text).map_err(|e| ImportError::Io {
            message: format!("write {}: {e}", out_path.display()),
        })?;
        emitted += 1;
    }
    Ok(emitted)
}

fn bucket_prax_items(
    merged: &crate::Schema,
    out: &mut std::collections::HashMap<PrismaSourceId, crate::Schema>,
) {
    // Models and enums in the merged prax::Schema have `source_id: Option<prax_schema::SourceId>`,
    // NOT PrismaSourceId. The converter (Step 2) must translate Prisma source IDs
    // through to prax source IDs in a 1:1 mapping (PrismaSourceId(n) → SourceId(n)).
    use prax_schema::SourceId as PraxSourceId;
    for (name, m) in &merged.models {
        let sid = PrismaSourceId(m.source_id.unwrap_or(PraxSourceId(0)).0);
        out.entry(sid).or_insert_with(crate::Schema::new).add_model(m.clone());
    }
    for (name, e) in &merged.enums {
        let sid = PrismaSourceId(e.source_id.unwrap_or(PraxSourceId(0)).0);
        out.entry(sid).or_insert_with(crate::Schema::new).add_enum(e.clone());
    }
    if let Some(ds) = &merged.datasource {
        let sid = PrismaSourceId(ds.source_id.unwrap_or(PraxSourceId(0)).0);
        out.entry(sid).or_insert_with(crate::Schema::new).set_datasource(ds.clone());
    }
    for (name, g) in &merged.generators {
        let sid = PrismaSourceId(g.source_id.unwrap_or(PraxSourceId(0)).0);
        out.entry(sid).or_insert_with(crate::Schema::new).add_generator(g.clone());
    }
}
```

- [ ] **Step 2: Wire source IDs through `convert_prisma_to_prax`**

`convert_prisma_to_prax` (in `prax-import/src/prisma/parser.rs:524`) takes a `PrismaSchema`, returns a prax `Schema`. We need each output item to carry its originating `PrismaSourceId` (translated to `prax_schema::SourceId(same_number)`).

In `convert_model`, after constructing the prax `Model`, add:

```rust
    if let Some(pid) = prisma_model.source_id {
        model.source_id = Some(prax_schema::SourceId(pid.0));
    }
```

Same for `convert_enum`, datasource, generators.

- [ ] **Step 3: Test the full multi-file round-trip**

Append to `prax-import/tests/multi_file_prisma.rs`:

```rust
use prax_import::prisma::import_prisma_directory;
use prax_import::format_schema;   // re-exported from prax-import (or wherever the renderer lives)

#[test]
fn directory_round_trip_mirrors_layout() {
    let input = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(input.path().join("models")).unwrap();
    std::fs::write(
        input.path().join("schema.prisma"),
        r#"
        datasource db { provider = "postgresql" url = env("DB") }
        generator client { provider = "prisma-client-js" }
        "#,
    ).unwrap();
    std::fs::write(
        input.path().join("models/user.prisma"),
        "model User { id Int @id @default(autoincrement()) email String @unique }",
    ).unwrap();
    std::fs::write(
        input.path().join("models/post.prisma"),
        "model Post { id Int @id @default(autoincrement()) author_id Int author User @relation(fields: [author_id], references: [id]) }",
    ).unwrap();

    let count = import_prisma_directory(input.path(), output.path(), format_schema, false).unwrap();
    assert_eq!(count, 3);
    assert!(output.path().join("schema.prax").exists());
    assert!(output.path().join("models/user.prax").exists());
    assert!(output.path().join("models/post.prax").exists());

    // The output should now load cleanly with prax_schema::load.
    let loaded = prax_schema::load(output.path()).expect("output loads cleanly");
    assert!(loaded.schema.get_model("User").is_some());
    assert!(loaded.schema.get_model("Post").is_some());
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p prax-import --test multi_file_prisma`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add prax-import/src
git commit -m "feat(import): emit multi-file Prax mirroring Prisma directory layouts"
```

---

### Task 22: CLI `prax import` auto-detects directory inputs

**Files:**
- Modify: `prax-cli/src/commands/import.rs`

- [ ] **Step 1: Branch on input type**

In `prax-cli/src/commands/import.rs:run`, after the input-exists check, add:

```rust
let input_is_dir = args.input.is_dir();
if input_is_dir && args.from != ImportSource::Prisma {
    return Err(CliError::Config(
        "Directory inputs are only supported for --from prisma".to_string(),
    ));
}

if input_is_dir {
    // Multi-file mode.
    let output_dir = args.output.clone().unwrap_or_else(|| std::path::PathBuf::from("./prax/schema"));
    let count = prax_import::prisma::import_prisma_directory(
        &args.input,
        &output_dir,
        format_schema,
        args.force,
    )
    .map_err(|e| CliError::Config(format!("Import failed: {e}")))?;
    output::success(&format!("✓ Imported {count} files into {}", output_dir.display()));
    return Ok(());
}

// Single-file path falls through to existing code.
```

- [ ] **Step 2: CLI integration test**

In `prax-cli/tests/cli_tests.rs`:

```rust
#[test]
fn import_prisma_directory_creates_mirrored_prax_directory() {
    let input = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(input.path().join("models")).unwrap();
    std::fs::write(
        input.path().join("schema.prisma"),
        r#"datasource db { provider = "postgresql" url = env("X") } generator client { provider = "prisma-client-js" }"#,
    ).unwrap();
    std::fs::write(
        input.path().join("models/u.prisma"),
        "model U { id Int @id @default(autoincrement()) }",
    ).unwrap();
    let output = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_prax"))
        .arg("import")
        .arg("--from").arg("prisma")
        .arg("--input").arg(input.path())
        .arg("--output").arg(output.path())
        .arg("--force")
        .status()
        .unwrap();
    assert!(status.success());
    assert!(output.path().join("schema.prax").exists());
    assert!(output.path().join("models/u.prax").exists());
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p prax-orm-cli --test cli_tests import_prisma_directory_creates_mirrored_prax_directory`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add prax-cli/src/commands/import.rs prax-cli/tests/cli_tests.rs
git commit -m "feat(cli): import auto-detects Prisma directory inputs"
```

---

## Wrap-up

### Task 23: Update example, README, CHANGELOG

**Files:**
- Modify: `examples/prax/` (add a multi-file variant)
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add multi-file example**

Create `examples/prax/multi_file_schema/`:

```
multi_file_schema/
  datasource.prax        # datasource + generator
  models/
    user.prax
    post.prax
  enums/
    role.prax
```

Content mirrors the existing `examples/prax/schema.prax` split across files. Aim for ~20-30 lines per file.

- [ ] **Step 2: README note**

Add a short section to `README.md` (under existing schema docs):

```markdown
## Multi-file schemas

Point `[schema].path` in `prax.toml` at a directory instead of a file:

```toml
[schema]
path = "prax/schema"
```

Prax recursively loads every `*.prax` file in that directory and merges them
into one schema. Duplicate model/enum/type names across files are hard errors
with both file locations reported. Run `prax init --multi-file` to scaffold
the directory layout.
```

- [ ] **Step 3: CHANGELOG entry**

Prepend to `CHANGELOG.md`:

```markdown
## [Unreleased]

### Added
- Multi-file schema support: `[schema].path` may now be a directory; `*.prax`
  files are loaded recursively, sorted lexicographically by relative path, and
  merged with hard-error collision detection (`SchemaError::DuplicateAcrossFiles`).
- `prax_schema::load(path)` returns `LoadedSchema { schema, sources }` and a
  partial source map on the error path via `LoadError`.
- `prax init --multi-file` scaffolds `./prax/schema/` directory layout.
- `prax import --from prisma --input <dir>` mirrors Prisma `prismaSchemaFolder`
  layouts into a Prax directory of `.prax` files.

### Deprecated
- `Schema::merge` (silent overwrite) — use `Schema::try_merge` for collision-aware merging.
```

- [ ] **Step 4: Commit**

```bash
git add examples README.md CHANGELOG.md
git commit -m "docs(schema): document multi-file schemas + Prisma directory import"
```

---

### Task 24: Full workspace check + final commit

- [ ] **Step 1: Lint, test, format**

Run in order:
```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Expected: all pass.

- [ ] **Step 2: Open PR (optional, user-driven)**

If the user is ready to merge, run:

```bash
gh pr create --base develop --title "feat(schema): multi-file schemas + Prisma directory import" \
  --body-file docs/superpowers/specs/2026-05-19-multi-file-schema-design.md
```

(Or use the gh PR command in the standard PR-creation flow.)

---

## Self-Review Notes

- All spec requirements have at least one task: discovery (Task 7), AST provenance (Tasks 3-4), merge semantics (Task 6), diagnostics (Tasks 5, 12), load entry point (Task 8), CLI integration (Tasks 11, 13-17), codegen (Task 18), importer (Tasks 19-22), backward-compat regression (covered by existing tests + Task 9 single-file case).
- `LoadError` always carries the partial `SourceMap` — spec requirement honored end-to-end.
- `format` is intentionally NOT routed through `load()` — per-file path, with comment in code.
- No "TBD" / "TODO" in tasks. Two implementation notes flagged inline (Validator entry-point and HasProvenance macro arms) tell the engineer how to resolve them based on what they find in the codebase.

## Open follow-ups not in this plan

- Full miette `Diagnostic` impl with `source_code()` + labeled spans (current `Report` uses Display rendering). Tracked as a docs note in the spec under "Open questions deferred to the plan" — pick this up after the multi-file PR lands.
- `prax-codegen`'s `SchemaReadError` carrying a `SourceMap` for richer proc-macro errors (currently strings).
