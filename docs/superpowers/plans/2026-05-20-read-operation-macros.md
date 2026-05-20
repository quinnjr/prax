# Read-Operation Macros (Phase 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the Prisma-style declarative read DSL described in `docs/superpowers/specs/2026-05-18-typed-query-traits-design.md` §4-5. After this phase the user can write

```rust
prax::find_many!(client.user, {
    where: { email: { contains: "@x.com" }, age: { gte: 18 } },
    include: { posts: { where: { published: true }, take: 5 } },
    order_by: [{ created_at: desc }],
    take: 10,
});
```

and the proc-macro will (a) discover and parse the schema file at compile time, (b) parse the brace-block DSL into a typed AST, (c) validate every key/operator against the schema with "did you mean" diagnostics, and (d) lower to constructor calls on the phase-2 input types chained through the existing `with_*_input` builders.

Six read-side macros ship in this phase: `find_unique!`, `find_first!`, `find_many!`, `count!`, `delete!`, `delete_many!`. Write macros (`create!`, `update!`, `upsert!`, `update_many!`, `create_many!`, `aggregate!`, `group_by!`) are phase 4+ and out of scope here.

**Architecture:**

- New module tree: `prax-codegen/src/macros/` houses the entire macro pipeline. It is parallel to (not nested under) `generators/`.
  - `macros/mod.rs` — module root + shared error type
  - `macros/schema_resolve.rs` — schema-file discovery (env / walk-up) + cache + dependency tracking
  - `macros/dsl/` — DSL parser (typed AST + token-level parsing)
    - `dsl/mod.rs`, `dsl/ast.rs`, `dsl/parser.rs`, `dsl/value.rs`
  - `macros/lower/` — AST → `TokenStream` lowering per input shape
    - `lower/where_input.rs`, `lower/include_input.rs`, `lower/select_input.rs`, `lower/order_by_input.rs`, `lower/scalar_filter.rs`
  - `macros/validate.rs` — schema-aware validation + strsim "did you mean"
  - `macros/accessor.rs` — accessor expression parsing (`client.user` / `User on expr` / `expr(), for User`)
  - `macros/ops/` — per-operation entry points
    - `ops/find_unique.rs`, `ops/find_first.rs`, `ops/find_many.rs`, `ops/count.rs`, `ops/delete.rs`, `ops/delete_many.rs`
- Top-level `#[proc_macro] fn find_many` (etc.) wrappers live in `prax-codegen/src/lib.rs`, each delegating to `macros::ops::*`.
- Schema cache: `static SCHEMA_CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Schema>>>>` keyed by absolute schema path. First call parses; subsequent calls hit cache. Build dep tracking via `proc_macro::tracked_path::path` when available; fallback emits `const _: &[u8] = include_bytes!(...)` inside a hidden module so rustc's dep graph picks up changes.
- The `with_*_input` extension methods on every `Operation` type already exist (added in phase 2). This phase only emits calls — no runtime changes outside the proc-macro crate.
- Re-exports: `prax-orm/src/lib.rs` re-exports each new macro under the crate root (`prax::find_many!`).
- Feature-gated: existing `prax/inputs` feature is default-on. Gate the new macros behind `prax-codegen/inputs` too so the umbrella `--no-default-features` build still compiles.

**Tech Stack:** Rust 2024, `proc_macro2`, `quote`, `syn 2.0` (custom-parse the brace block via `Parse` impls on the DSL AST), `convert_case`, `prax-schema` AST, `strsim` (Jaro-Winkler for typo suggestions), `toml` (for `prax.toml` lookup), `trybuild` (compile-fail UI tests), `insta` (snapshot tests of lowered token streams).

---

## File Structure

### New files (in `prax-codegen/`)

- `src/macros/mod.rs`
- `src/macros/schema_resolve.rs`
- `src/macros/dsl/mod.rs`
- `src/macros/dsl/ast.rs`
- `src/macros/dsl/parser.rs`
- `src/macros/dsl/value.rs`
- `src/macros/lower/mod.rs`
- `src/macros/lower/where_input.rs`
- `src/macros/lower/include_input.rs`
- `src/macros/lower/select_input.rs`
- `src/macros/lower/order_by_input.rs`
- `src/macros/lower/scalar_filter.rs`
- `src/macros/validate.rs`
- `src/macros/accessor.rs`
- `src/macros/ops/mod.rs`
- `src/macros/ops/find_unique.rs`
- `src/macros/ops/find_first.rs`
- `src/macros/ops/find_many.rs`
- `src/macros/ops/count.rs`
- `src/macros/ops/delete.rs`
- `src/macros/ops/delete_many.rs`

### Test files

- `prax-codegen/tests/dsl_parser.rs` — unit tests for DSL AST round-trip on a hand-rolled token stream
- `prax-codegen/tests/lower_snapshots.rs` — `insta` snapshots of token-stream lowering for a fixture schema
- `prax-codegen/tests/ui/` — `trybuild` compile-fail fixtures (one `.rs` + `.stderr` per diagnostic):
  - `unknown_field.rs`
  - `unknown_field_typo.rs` (covers "did you mean" suggestion)
  - `wrong_operator.rs` (e.g. `contains` on Int)
  - `relation_op_on_scalar.rs`
  - `to_one_relation_op.rs` (using `some`/`every`/`none` on to-one)
  - `find_unique_non_unique_where.rs`
  - `select_and_include.rs` (xor violation)
  - `unknown_top_key.rs`
  - `cql_capability_gap.rs` (relation filter against a CQL engine)
- `prax-codegen/tests/trybuild_ui.rs` — driver invoking `trybuild::TestCases::new().compile_fail("tests/ui/*.rs")`
- `tests/read_macros_e2e.rs` (workspace-level `prax-orm` integration test) — full end-to-end exercising each of the six macros against the in-memory `prax-mongodb` test engine (or `prax-sqlite` in-memory if mongo isn't already wired). Tests should compile, build a query, and exec a smoke result.

### Modified files

- `prax-codegen/Cargo.toml`
  - `[dependencies]` add `strsim = "0.11"`, `toml = "0.8"`, `once_cell = "1"` (if not already present transitively — confirm before adding)
  - `[dev-dependencies]` add `insta = { workspace = true }` (already in tree from phase 2; reuse), `trybuild = { workspace = true }`
- `prax-codegen/src/lib.rs`
  - `mod macros;`
  - Six new `#[proc_macro] fn find_unique`/`find_first`/`find_many`/`count`/`delete`/`delete_many` entry points
- `src/lib.rs` (umbrella `prax-orm`)
  - Re-export: `pub use prax_codegen::{find_unique, find_first, find_many, count, delete, delete_many};`
  - Re-export at root (matching existing `pub use prax_codegen::Model;` pattern)
- `prax-codegen/src/schema_reader.rs` (existing)
  - Add `pub fn read_schema_for_macro(path: &Path) -> Result<Arc<Schema>, syn::Error>` helper that returns the cached `Arc<Schema>`. The existing `read_schema_with_config` path stays for `prax_schema!`.
- `CHANGELOG.md`
  - New `### Added` bullets under `[Unreleased]`.

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/read-operation-macros rev-parse --abbrev-ref HEAD`
Expected: `feature/read-operation-macros`.

- [ ] **Step 2: Confirm base point**

Run: `git -C /home/joseph/Projects/prax/.worktrees/read-operation-macros log --oneline -1`
Expected: starts with `04cf10f feat(codegen): typed input codegen + engine capabilities (phase 2)`.

- [ ] **Step 3: Workspace builds**

Run: `cargo check --workspace --all-features`
Expected: zero errors.

- [ ] **Step 4: Existing tests pass**

Run: `cargo test --workspace --lib --tests --no-fail-fast`
Expected: all pass.

- [ ] **Step 5: No commit — verification only.**

---

## Task 2: Scaffold `macros/` module + add dependencies

**Files:**
- Create: `prax-codegen/src/macros/mod.rs`
- Modify: `prax-codegen/Cargo.toml`
- Modify: `prax-codegen/src/lib.rs` — add `mod macros;` (private; entry points exported via the `#[proc_macro]` wrappers added later)

- [ ] **Step 1: Add deps to `prax-codegen/Cargo.toml`**

Under `[dependencies]` add (check whether each is already present transitively — if so, just reference workspace):
```toml
strsim = "0.11"
toml = "0.8"
```

Under `[dev-dependencies]` confirm both `insta` and `trybuild` are present. Phase 2 already added `trybuild`; reuse.

- [ ] **Step 2: Create `prax-codegen/src/macros/mod.rs`**

```rust
//! Schema-aware proc-macro pipeline for the read-operation DSL
//! (`find_unique!`, `find_first!`, `find_many!`, `count!`, `delete!`,
//! `delete_many!`). Phase 3 of the typed-query-traits work.
//!
//! Pipeline:
//!   parse TokenStream
//!     -> resolve schema (env var / walk-up prax.toml)
//!     -> resolve accessor expression and model
//!     -> parse DSL brace block into a typed AST
//!     -> validate AST against schema (unknown field, wrong op, ...)
//!     -> lower AST to TokenStream constructing layer-2 input structs
//!     -> emit chained `with_*_input(...)` calls on the operation

pub(crate) mod accessor;
pub(crate) mod dsl;
pub(crate) mod lower;
pub(crate) mod ops;
pub(crate) mod schema_resolve;
pub(crate) mod validate;
```

- [ ] **Step 3: Add `mod macros;` in `prax-codegen/src/lib.rs`**

Place after the existing `mod schema_reader;` declaration. No re-exports yet — the `#[proc_macro]` wrappers come in tasks 14–17.

- [ ] **Step 4: `cargo check -p prax-codegen`**

Expected: scaffold compiles (empty submodules are fine because nothing references them yet — but each must exist as an empty file for `pub(crate) mod` to resolve, so go ahead and `touch` each file listed under "New files" with a one-line doc comment).

- [ ] **Step 5: Commit**

```
chore(codegen): scaffold macros/ tree for phase-3 read DSL
```

---

## Task 3: Schema discovery resolver

**Files:**
- Create: `prax-codegen/src/macros/schema_resolve.rs`

- [ ] **Step 1: Implement `resolve_schema_path() -> Result<PathBuf, syn::Error>`**

Resolution order (per spec §5):
1. `std::env::var("PRAX_SCHEMA")` — if set, treat as path. Absolute paths used directly; relative paths resolved against `CARGO_MANIFEST_DIR`. Hard error if file doesn't exist.
2. Else walk up from `CARGO_MANIFEST_DIR` looking for `prax.toml`. For each ancestor including the manifest dir itself, check for `prax.toml`. On finding it, read it as a `toml::Value`, extract `generator.client.schema` (string), default to `"prax/schema.prax"`. Resolve relative to the directory containing the `prax.toml`. Hard error if the resolved file doesn't exist.
3. Else hard error: `"Could not find a 'prax.toml' in any ancestor of $CARGO_MANIFEST_DIR. Set PRAX_SCHEMA=path/to/schema.prax or run 'prax init'."`

All errors should be `syn::Error::new(proc_macro2::Span::call_site(), msg)` so the macro can convert them to `compile_error!`.

- [ ] **Step 2: Unit tests in the same file**

Use `#[cfg(test)]` + `tempfile` (already in workspace dev-deps; confirm). Cover:
  - PRAX_SCHEMA absolute happy path
  - PRAX_SCHEMA missing file → error
  - prax.toml walk-up two levels deep
  - default `prax/schema.prax` when key absent
  - explicit `generator.client.schema = "alt.prax"` override
  - no prax.toml anywhere → error

For each test, set the `CARGO_MANIFEST_DIR` env via `std::env::set_var` inside a serialized critical section (use `std::sync::Mutex<()>` static guard — env-mutation tests can't run in parallel).

- [ ] **Step 3: `cargo test -p prax-codegen schema_resolve`**

Expected: all pass.

- [ ] **Step 4: Commit**

```
feat(codegen): schema-path resolver for phase-3 macros
```

---

## Task 4: Schema cache + dependency tracking

**Files:**
- Modify: `prax-codegen/src/macros/schema_resolve.rs`

- [ ] **Step 1: Add cache**

```rust
use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use prax_schema::Schema;

static SCHEMA_CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Schema>>>> = OnceLock::new();

pub fn resolve_schema() -> Result<Arc<Schema>, syn::Error> {
    let path = resolve_schema_path()?;
    let cache = SCHEMA_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().map_err(|_| poison_error())?;
    if let Some(s) = guard.get(&path) {
        return Ok(Arc::clone(s));
    }
    let schema = prax_schema::parse_schema_file(&path)
        .map_err(|e| syn::Error::new(proc_macro2::Span::call_site(),
            format!("failed to parse schema at {}: {}", path.display(), e)))?;
    let arc = Arc::new(schema);
    guard.insert(path.clone(), Arc::clone(&arc));
    Ok(arc)
}
```

- [ ] **Step 2: Build-dep tracking emitter**

```rust
pub fn track_schema_dep(path: &std::path::Path) -> proc_macro2::TokenStream {
    // proc_macro::tracked_path is unstable; fallback emits include_bytes!
    // inside a hidden module so rustc's dep graph picks up the file.
    let abs = path.to_string_lossy().into_owned();
    quote::quote! {
        #[doc(hidden)]
        #[allow(dead_code)]
        const _PRAX_SCHEMA_DEP: &[u8] = include_bytes!(#abs);
    }
}
```

Each operation macro will include the result inside its expansion's hidden module.

- [ ] **Step 3: Unit tests for cache behavior**

Cover: cold parse, warm cache hit (same path → same `Arc` pointer via `Arc::ptr_eq`).

- [ ] **Step 4: `cargo test -p prax-codegen schema_resolve`**

Expected: pass.

- [ ] **Step 5: Commit**

```
feat(codegen): cached schema loader + dep tracking for proc-macros
```

---

## Task 5: DSL AST + brace-block parser foundation

**Files:**
- Create: `prax-codegen/src/macros/dsl/mod.rs`
- Create: `prax-codegen/src/macros/dsl/ast.rs`
- Create: `prax-codegen/src/macros/dsl/parser.rs`

- [ ] **Step 1: Define the AST in `dsl/ast.rs`**

```rust
use proc_macro2::Span;
use syn::Expr;

pub struct DslBlock {
    pub span: Span,
    pub fields: Vec<DslField>,
}

pub enum DslField {
    Pair { key: syn::Ident, value: DslValue, span: Span },
    Spread { expr: Expr, by_move: bool, span: Span },
    Conditional {
        cond: Expr,
        kind: CondKind,
        key: syn::Ident,
        value: DslValue,
        span: Span,
    },
}

pub enum CondKind { If, ElseIf, Else }

pub enum DslValue {
    Lit(syn::Lit),
    Path(syn::Path),
    Expr(Expr),               // wrapped in @(...) or surrounding parens
    Block(DslBlock),          // nested { ... }
    List(Vec<DslValue>),      // [ ... ]
    Bool(bool),
    BareIdent(syn::Ident),    // role: Admin
}
```

- [ ] **Step 2: `Parse` impls in `dsl/parser.rs`**

Implement `syn::parse::Parse for DslBlock` that consumes a `{ ... }` `syn::token::Brace`. Inside, loop:

- If lookahead matches `..` consume spread (with optional `move` keyword)
- If lookahead matches `#[` consume attribute, expect `if|else_if|else`, then `ident: value`
- Else parse `ident : value` plus trailing comma

Allow trailing comma. Use `syn::braced!` + `Punctuated<DslField, Token![,]>::parse_terminated`-style parsing.

`DslField::parse` dispatches on the leading token.

- [ ] **Step 3: Unit tests in `tests/dsl_parser.rs`**

Cover:
- Empty block `{}`
- Single `key: value`
- Multiple comma-separated keys, trailing comma OK
- Nested block `where: { equals: 5 }`
- List value `or: [{ a: 1 }, { b: 2 }]`
- Bare ident `role: Admin`
- Spread `..base`
- Spread-move `..move base`
- Conditional `#[if(cond)] take: 5`
- Escape `data: @(custom_expr())`
- Malformed input → useful error spans (assert on `parse2` error message containing line/col hints)

- [ ] **Step 4: `cargo test -p prax-codegen dsl_parser`**

- [ ] **Step 5: Commit**

```
feat(codegen): DSL AST + brace-block parser for phase-3 macros
```

---

## Task 6: DSL value parser primitives

**Files:**
- Create: `prax-codegen/src/macros/dsl/value.rs`

- [ ] **Step 1: `parse_value(input: ParseStream) -> syn::Result<DslValue>`**

Branching on lookahead:
- `Lit` → `DslValue::Lit`
- `true` / `false` keyword → `DslValue::Bool`
- `{` → recursive `DslBlock` → `DslValue::Block`
- `[` → `bracketed!` + `Punctuated<DslValue, Token![,]>::parse_terminated` → `DslValue::List`
- `@` followed by `(` → consume `@(...)` and parse inner as `syn::Expr` → `DslValue::Expr`
- Bare ident at end of stream or before `,`/`}` → `DslValue::BareIdent`
- `Path` (with `::` or multiple segments) → `DslValue::Path`
- Fallback: parse as generic `syn::Expr` → `DslValue::Expr`

- [ ] **Step 2: Disambiguation rules**

- A standalone single-segment ident is bare-ident **only if** the next token is `,`, `}`, `]`, or EOF. Otherwise treat as start of an `Expr` (e.g. `count()` or `foo.bar`).
- Paths with `::` separators always become `DslValue::Path`.

- [ ] **Step 3: Unit tests in `tests/dsl_parser.rs`** (extend existing file)

Confirm every branch. Cover edge cases: `role: Role::Admin` (Path), `role: Admin` (BareIdent), `take: 10` (Lit), `where: foo` (BareIdent treated as expression context — see Step 2), `where: foo.bar` (Expr).

- [ ] **Step 4: `cargo test -p prax-codegen dsl_parser`**

- [ ] **Step 5: Commit**

```
feat(codegen): value-parser primitives for DSL macros
```

---

## Task 7: WhereInput lowering

**Files:**
- Create: `prax-codegen/src/macros/lower/where_input.rs`
- Create: `prax-codegen/src/macros/lower/scalar_filter.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs`

- [ ] **Step 1: `LowerCtx` struct in `lower/mod.rs`**

```rust
pub struct LowerCtx<'a> {
    pub schema: &'a prax_schema::Schema,
    pub model: &'a prax_schema::Model,
    pub crate_root: proc_macro2::TokenStream, // ::prax or ::prax_orm — usually `::prax`
}
```

- [ ] **Step 2: `lower_where(block: &DslBlock, ctx: &LowerCtx) -> syn::Result<TokenStream>`** in `where_input.rs`

Produce a block expression that builds the per-model `WhereInput`. For each `DslField::Pair`:
- If key matches a scalar field name → call `lower_scalar_filter(field, value, ctx)` from `scalar_filter.rs`
- If key matches a relation name → call `lower_relation_filter(rel, value, ctx)`
- If key is `and` / `or` / `not` → recurse into a list of `WhereInput` lowerings
- Else → return validation error (handled in task 11 properly; here just bail with a span error)

Output skeleton:
```rust
{
    let mut __w = <#path_to_where_input>::default();
    __w.email = Some(/* scalar filter */);
    /* ... */
    __w
}
```

- [ ] **Step 3: `lower_scalar_filter` in `scalar_filter.rs`**

Given a scalar field (with its declared type) and a `DslValue`:
- If `DslValue::Lit` or `DslValue::BareIdent` (for enum) → emit `<Filter>::equals(value)` shorthand
- If `DslValue::Block(inner)` → for each pair inside, dispatch on the operator name (`gt`, `gte`, `lt`, `lte`, `equals`, `not`, `in_list`, `not_in`, `contains`, `starts_with`, `ends_with`, `mode`). Build the struct literal of the appropriate `*Filter` from `prax_query::inputs`.
- Operator → struct-field mapping is mechanical: snake_case key → struct field. `mode: insensitive` sets `mode: Some(QueryMode::Insensitive)`. `in_list: [a, b, c]` → `in_list: Some(vec![...])`.

- [ ] **Step 4: Relation filter lowering**

If the relation is to-many: build `ListRelationFilter { some, every, none }` from the nested block.
If the relation is to-one: build `RelationFilter { is, is_not, is_null }`. Reject `some`/`every`/`none` here (with a clear error).

- [ ] **Step 5: Logical `and` / `or` / `not`**

- `and: [block, block, ...]` → `and: Some(vec![<lower each>])`
- `or: [block, block, ...]` → same for `or`
- `not: block` → `not: Some(Box::new(<lower>))`

- [ ] **Step 6: Unit tests via insta snapshot in `tests/lower_snapshots.rs`**

Use a fixture schema (small `User { id, email, age, role: Role, posts: Post[] }`). For each of ~6 inputs (simple scalar, range, logical or, relation `some`, etc.), assert the lowered `TokenStream` via `insta::assert_snapshot!(pretty_format(tokens))`.

- [ ] **Step 7: `cargo test -p prax-codegen lower_snapshots`**

First run will write the snapshot files; commit them.

- [ ] **Step 8: Commit**

```
feat(codegen): lower DSL where blocks to typed WhereInput
```

---

## Task 8: IncludeInput / SelectInput lowering

**Files:**
- Create: `prax-codegen/src/macros/lower/include_input.rs`
- Create: `prax-codegen/src/macros/lower/select_input.rs`

- [ ] **Step 1: `lower_include(block, ctx)` in `include_input.rs`**

For each field:
- `relation: true` → `__i.relation = Some(<Relation>IncludeArgs::default());`
- `relation: { where: ..., include: ..., order_by: ..., take: ... }` → recurse into nested `<Relation>IncludeArgs` builder. The nested block lowers via the **target relation's** `LowerCtx` (look up the relation's target model in the schema and switch context).
- Unknown key → error (validation in task 11).

- [ ] **Step 2: `lower_select(block, ctx)` in `select_input.rs`**

For each field:
- Scalar field → `__s.email = Some(true);` (Select fields are typed booleans).
- Relation → either `true` shortcut or nested args block (similar to include).

- [ ] **Step 3: select/include xor**

Add a tiny helper in `lower/mod.rs` — `fn check_select_include_xor(has_select, has_include, top_key_span) -> syn::Result<()>` returning an error pointing at the second of the two when both are present.

- [ ] **Step 4: Snapshot tests in `lower_snapshots.rs`**

Cover nested include (`posts: { where: { published: true } }`) and select with mixed scalar + relation.

- [ ] **Step 5: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 6: Commit**

```
feat(codegen): lower DSL include / select blocks
```

---

## Task 9: OrderByInput + cursor/skip/take/distinct lowering

**Files:**
- Create: `prax-codegen/src/macros/lower/order_by_input.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs`

- [ ] **Step 1: `lower_order_by(value, ctx)` in `order_by_input.rs`**

`value` may be a single block (`{ created_at: desc }`) or a list (`[{ a: asc }, { b: desc }]`). Both lower to `Vec<<Model>OrderBy>`. Single-block form auto-wraps into a vec of length 1.

- [ ] **Step 2: `lower_cursor(block, ctx)` — `WhereUniqueInput` lowering**

Cursors use the unique-input enum (a one-of). Reuse the same logic as `where:` but call into a `WhereUniqueInput`-targeted lowering — strict: exactly one key, must be a `@unique` field or a `@@id` tuple key.

- [ ] **Step 3: Top-level scalar `skip` / `take` / `distinct`**

`skip: N` → `.skip(N)`, `take: N` → `.take(N)`, `distinct: [field, …]` → `.distinct(vec![…])`. Add these as direct chained calls in the operation's emit step (task 14+), not as `with_*_input`.

- [ ] **Step 4: Snapshot test**

`order_by: { created_at: desc }` and `order_by: [{ a: asc }, { b: desc }]`. Check both lower to the same `Vec<_>` shape.

- [ ] **Step 5: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 6: Commit**

```
feat(codegen): lower order_by / cursor / skip / take / distinct
```

---

## Task 10: Spread + conditional support

**Files:**
- Modify: `prax-codegen/src/macros/lower/where_input.rs`
- Modify: `prax-codegen/src/macros/lower/include_input.rs`
- Modify: `prax-codegen/src/macros/lower/select_input.rs`

- [ ] **Step 1: Spread `..expr`**

In the `let mut __w = <Type>::default();` skeleton, replace the default with `let mut __w = ::core::clone::Clone::clone(&(<expr>));` when the **first** field in iteration order is a `DslField::Spread`. Subsequent assignments overwrite per Rust struct-update semantics.

If a `Spread` appears in the middle, lower as: emit accumulated pairs first, then `__w = ::core::clone::Clone::clone(&(<spread_expr>)); /* then re-apply pairs after the spread */`. Actually simpler: process fields left-to-right, treating spread as an assignment to `__w` that overwrites everything. Document this in code with a `// Why:` comment.

- [ ] **Step 2: `..move expr`**

Same as spread but emit the bare `<expr>` (no clone). Mark with a comment that the caller is responsible for not using `expr` after this.

- [ ] **Step 3: `#[if(cond)]` / `#[else_if]` / `#[else]`**

Lower a `Conditional` field to:
```rust
if (cond) {
    __w.field = Some(/* lowered value */);
}
```

Else-if / else extend the same if-chain. The parser groups consecutive conditional fields by their position in the block, then lowers them as a single chained `if/else if/else`.

Implementation note: in the lowering pass, walk fields with a peeking iterator. When a `Conditional::If` is seen, collect contiguous `ElseIf`/`Else` siblings into one logical group.

- [ ] **Step 4: Snapshot tests**

`{ ..base, email: { equals: "x" } }`, `{ #[if(flag)] take: 5 }`, `{ #[if(a)] take: 5, #[else_if(b)] take: 10, #[else] take: 0 }`.

- [ ] **Step 5: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 6: Commit**

```
feat(codegen): spread + conditional DSL support
```

---

## Task 11: Schema validation + "did you mean" diagnostics

**Files:**
- Create: `prax-codegen/src/macros/validate.rs`

- [ ] **Step 1: `validate_block(block, model, schema, expectation) -> syn::Result<()>`**

`expectation` is an enum `Expectation::{WhereInput, WhereUniqueInput, IncludeInput, SelectInput, OrderByInput}`. For each pair, check:
- `WhereInput`: key must be (scalar field) OR (relation name) OR (`and`/`or`/`not`)
- `WhereUniqueInput`: key must be a unique-column or composite-key field; exactly one pair total
- `IncludeInput`: key must be a relation name
- `SelectInput`: key must be a scalar field OR relation name
- `OrderByInput`: key must be a scalar field OR a to-one relation (for nested order_by)

- [ ] **Step 2: "Did you mean" suggester**

```rust
fn suggest(key: &str, candidates: &[&str]) -> Option<String> {
    candidates.iter()
        .map(|c| (c, strsim::jaro_winkler(key, c)))
        .filter(|(_, s)| *s >= 0.85)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(c, _)| (*c).to_string())
}
```

On unknown-key error, format as:
> `unknown field 'emial' on model 'User'. did you mean 'email'?`

…or just `unknown field 'emial' on model 'User'` when no candidate is close enough.

- [ ] **Step 3: Operator-vs-type validation**

`lower_scalar_filter` already needs to know the field type to emit the right struct. Add a check up front: if the operator isn't valid for the scalar category, return a span-pointed error (e.g. `contains` is invalid for `Int`).

- [ ] **Step 4: Capability-style errors caught at lowering time**

- `some`/`every`/`none` on a to-one relation → "use `is` / `is_not` for to-one relation"
- `is`/`is_not` on a to-many relation → "use `some`/`every`/`none` for to-many"

(Backstop only; the trait-level `SupportsRelationFilter` gate from phase 1 catches the CQL engine case at link-time.)

- [ ] **Step 5: Wire the validator into each lowering entry point**

`lower_where` / `lower_include` / etc. call `validate_block` first and return its error before any token emission.

- [ ] **Step 6: Unit tests in `validate.rs`**

Cover each error case. Don't assert exact error strings here — that's the job of the trybuild UI tests (task 18).

- [ ] **Step 7: `cargo test -p prax-codegen validate`**

- [ ] **Step 8: Commit**

```
feat(codegen): schema-aware DSL validation + did-you-mean
```

---

## Task 12: Accessor resolution

**Files:**
- Create: `prax-codegen/src/macros/accessor.rs`

- [ ] **Step 1: `AccessorSpec` type + parser**

```rust
pub struct AccessorSpec {
    pub accessor_expr: syn::Expr,   // e.g. `client.user` or `(get_client().user())`
    pub model_name: String,         // PascalCase model name
    pub model_span: Span,
}
```

Parse the head of the macro input (everything before the first `,` that isn't inside parens/brackets/braces). Three forms:

1. `EXPR` followed by `,` then `{ ... }` (operation input). Infer model from last path-segment of EXPR, snake_case → match schema PascalCase. Example: `client.user` → "User"; `state.db.user` → "User".
2. `MODEL on EXPR` form: ident, kw `on`, expr. Take MODEL directly.
3. `EXPR, for MODEL,` form: arbitrary expr, then literal `for`, then model ident. Detect by scanning ahead for the `for` keyword position.

- [ ] **Step 2: `resolve_model_from_path(path: &syn::Path, schema: &Schema)`**

Snake_case the last segment, then find a model in `schema.models` whose snake_case name matches. Return error with span if not found.

- [ ] **Step 3: Unit tests**

Cover each call form, plus error cases (unknown model, ambiguous accessor).

- [ ] **Step 4: `cargo test -p prax-codegen accessor`**

- [ ] **Step 5: Commit**

```
feat(codegen): accessor-expression parser for read macros
```

---

## Task 13: `find_many!` macro end-to-end

**Files:**
- Create: `prax-codegen/src/macros/ops/find_many.rs`
- Modify: `prax-codegen/src/macros/ops/mod.rs` — add `pub(crate) mod find_many;` etc. stubs
- Modify: `prax-codegen/src/lib.rs` — add `#[proc_macro] fn find_many`

- [ ] **Step 1: `pub fn expand_find_many(input: TokenStream2) -> syn::Result<TokenStream2>`**

Flow:
1. Parse leading `AccessorSpec`.
2. `resolve_schema()` → `Arc<Schema>`.
3. Look up the model.
4. Parse the trailing `, { ... }` as `DslBlock`.
5. Build a `LowerCtx`.
6. For each top-level key in the DslBlock:
   - `where:` → `validate_block` + `lower_where` → `__where_input`
   - `include:` → ditto → `__include_input`
   - `select:` → ditto → `__select_input` (xor with include)
   - `order_by:` → `lower_order_by` → `__order_by`
   - `cursor:` → `lower_cursor` → `__cursor`
   - `skip` / `take` / `distinct` → bare scalars stashed for the chained calls
   - else → unknown-top-key error with did-you-mean against the allowed set
7. Emit:

```rust
{
    #_schema_dep_const
    let __op = <_ as ::prax::inputs::ModelAccessor<_>>::find_many(&(#accessor_expr));
    #(let __where = #where_block; let __op = __op.with_where_input(__where);)?
    #(let __include = #include_block; let __op = __op.with_include_input(__include);)?
    #(let __select = #select_block; let __op = __op.with_select_input(__select);)?
    #(let __order_by = #order_by_block; let __op = __op.with_order_by_input(__order_by);)?
    #(let __cursor = #cursor_block; let __op = __op.with_cursor_input(__cursor);)?
    #(let __op = __op.skip(#skip_expr);)?
    #(let __op = __op.take(#take_expr);)?
    #(let __op = __op.distinct(#distinct_expr);)?
    __op
}
```

`#_schema_dep_const` is `track_schema_dep(&schema_path)` from task 4 (emit one per macro expansion).

- [ ] **Step 2: `#[proc_macro] fn find_many` in `prax-codegen/src/lib.rs`**

```rust
#[proc_macro]
pub fn find_many(input: TokenStream) -> TokenStream {
    match macros::ops::find_many::expand_find_many(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
```

- [ ] **Step 3: Re-export from `prax-orm` umbrella**

In `src/lib.rs` of `prax-orm` add:
```rust
pub use prax_codegen::find_many;
```

- [ ] **Step 4: Smoke compile test**

Add `tests/read_macros_e2e.rs` with one test invoking `prax::find_many!(client.user, { where: { email: { equals: "x" } } });` against a fixture model. Test only needs to compile, not exec — for now.

- [ ] **Step 5: `cargo test -p prax-orm --test read_macros_e2e`**

- [ ] **Step 6: Commit**

```
feat(codegen): find_many! proc-macro end-to-end
```

---

## Task 14: `find_unique!` + `find_first!` macros

**Files:**
- Create: `prax-codegen/src/macros/ops/find_unique.rs`
- Create: `prax-codegen/src/macros/ops/find_first.rs`
- Modify: `prax-codegen/src/lib.rs`
- Modify: `src/lib.rs` (umbrella)
- Modify: `tests/read_macros_e2e.rs`

- [ ] **Step 1: Implement `expand_find_unique`**

Mostly the same as `find_many`, with these differences:
- `where:` lowers to `WhereUniqueInput` not `WhereInput`. Use the `WhereUniqueInput` enum variant constructor: the lowered block must have exactly one key, matched against a `@unique` column.
- Only `where`, `include`, `select` allowed at top level. Reject others with did-you-mean.

- [ ] **Step 2: Implement `expand_find_first`**

Allowed top-level keys: `where`, `order_by`, `cursor`, `skip`, `take`, `include`/`select`. No `distinct`.

- [ ] **Step 3: `#[proc_macro]` wrappers in `prax-codegen/src/lib.rs` + umbrella re-exports**

- [ ] **Step 4: Add e2e test cases for both**

- [ ] **Step 5: `cargo test -p prax-orm --test read_macros_e2e`**

- [ ] **Step 6: Commit**

```
feat(codegen): find_unique! + find_first! proc-macros
```

---

## Task 15: `count!` + `delete!` + `delete_many!` macros

**Files:**
- Create: `prax-codegen/src/macros/ops/count.rs`
- Create: `prax-codegen/src/macros/ops/delete.rs`
- Create: `prax-codegen/src/macros/ops/delete_many.rs`
- Modify: `prax-codegen/src/lib.rs`
- Modify: `src/lib.rs` (umbrella)
- Modify: `tests/read_macros_e2e.rs`

- [ ] **Step 1: `expand_count`**

Allowed top-level keys: `where`, `order_by`, `cursor`, `skip`, `take`, `select` (select is the `CountSelect` aggregate spec, not the row-shape select). For phase 3 we accept only `where` and the scalars; `select:` lowers to a placeholder `unimplemented!()` with a TODO comment, gated by a doc note that aggregate `_count` is phase 6.

Actually — to keep this clean, **reject `select:` on `count!` with a "phase 6" error** rather than emit `unimplemented!()`. Saves us debugging confusion.

Operation entry: `<Accessor>::count(&accessor)`.

- [ ] **Step 2: `expand_delete`**

Allowed top-level keys: `where` (unique), `include`/`select` (return-shape).

Operation entry: `<Accessor>::delete(&accessor)`.

- [ ] **Step 3: `expand_delete_many`**

Allowed top-level keys: `where` (non-unique allowed).

Operation entry: `<Accessor>::delete_many(&accessor)`.

- [ ] **Step 4: `#[proc_macro]` wrappers + umbrella re-exports**

- [ ] **Step 5: e2e test cases**

- [ ] **Step 6: `cargo test -p prax-orm --test read_macros_e2e`**

- [ ] **Step 7: Commit**

```
feat(codegen): count!/delete!/delete_many! proc-macros
```

---

## Task 16: trybuild UI tests for diagnostics

**Files:**
- Create: `prax-codegen/tests/ui/unknown_field.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/unknown_field_typo.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/wrong_operator.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/relation_op_on_scalar.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/to_one_relation_op.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/find_unique_non_unique_where.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/select_and_include.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/unknown_top_key.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/ui/cql_capability_gap.rs` (+ `.stderr`)
- Create: `prax-codegen/tests/trybuild_ui.rs`

- [ ] **Step 1: One trybuild driver**

```rust
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
```

- [ ] **Step 2: Author each `.rs` fixture**

Each fixture is a minimal `prax::find_many!(...)` (or whichever macro is relevant) that triggers exactly one error class. Each fixture needs a tiny accompanying schema or hand-rolled derive — share a single `tests/ui/_fixture_schema.rs` if convenient (trybuild crates may include unrelated files).

- [ ] **Step 3: Generate `.stderr` baselines**

Run: `TRYBUILD=overwrite cargo test -p prax-codegen --test trybuild_ui`. Inspect each generated `.stderr`. **Confirm the snapshot text matches what you'd want a user to see.** Adjust the diagnostic messages in the validator if needed and regenerate. Document the regen procedure in a short doc comment at the top of `trybuild_ui.rs`.

- [ ] **Step 4: `cargo test -p prax-codegen --test trybuild_ui`**

Expected: all pass against committed `.stderr` baselines.

- [ ] **Step 5: Commit**

```
test(codegen): trybuild UI tests for read-macro diagnostics
```

---

## Task 17: End-to-end smoke + docs + CHANGELOG

**Files:**
- Modify: `tests/read_macros_e2e.rs` — flesh out per-macro happy-path tests that actually exec against the in-process engine
- Modify: `CHANGELOG.md`
- Modify (optional): top-level `README.md` if a one-line "read macros" example fits the existing "Example" section

- [ ] **Step 1: Pick an executable test engine**

Use whichever in-process engine is already wired for derive_inputs_e2e (phase 2). If phase 2 used a mock engine, reuse it; if it used a real SQLite, follow suit. Don't add a new engine in this phase.

- [ ] **Step 2: One happy-path test per macro**

Six total: `find_unique!`, `find_first!`, `find_many!`, `count!`, `delete!`, `delete_many!`. Each test seeds a row, runs the macro through `.exec().await?`, asserts the expected return.

- [ ] **Step 3: One spread + one conditional test**

Confirm `{ ..base }` and `#[if(flag)] take: 5` do the right thing at runtime.

- [ ] **Step 4: Update CHANGELOG.md under `[Unreleased]`**

```
### Added
- Read-operation macros: `prax::find_unique!`, `prax::find_first!`, `prax::find_many!`,
  `prax::count!`, `prax::delete!`, `prax::delete_many!`. Schema-aware, with "did you mean"
  diagnostics for unknown fields.
- DSL grammar supports nested filters, logical `and`/`or`/`not`, `..spread`, `#[if(cond)]`
  conditionals, bare-ident enum resolution, and `@(expr)` Rust escapes.
- Schema-file discovery via `PRAX_SCHEMA` env var with `prax.toml` walk-up fallback.
```

- [ ] **Step 5: `cargo test --workspace --all-features`**

Expected: zero failures across the full suite.

- [ ] **Step 6: `cargo clippy --workspace --all-targets --all-features -- -D warnings`**

Fix any warnings. Pre-push hook will reject otherwise.

- [ ] **Step 7: `cargo fmt --all`**

- [ ] **Step 8: Commit**

```
feat(codegen): e2e tests + CHANGELOG for read-operation macros
```

---

## Task 18: Push + PR

**Files:**
- None.

- [ ] **Step 1: `git push -u origin feature/read-operation-macros`**

Pre-push hook runs the full test suite — expect ~2-3 minute run.

- [ ] **Step 2: Open PR**

```bash
gh pr create --base develop --head feature/read-operation-macros \
  --title "feat(codegen): read-operation macros + schema-aware DSL (phase 3)" \
  --body "..."
```

PR body should:
- Summarize scope (6 macros, schema discovery, DSL grammar, diagnostics)
- Note that write macros (create/update/upsert/etc.) remain in phase 4
- Link the spec: `docs/superpowers/specs/2026-05-18-typed-query-traits-design.md`
- Test plan checklist (lib tests, e2e tests, trybuild UI, manual sanity)

- [ ] **Step 3: Wait for CI**

If green, request squash-merge. If red, triage from the PR view, fix, push, repeat.

---

## Out of scope for phase 3 (deferred)

- `create!`, `update!`, `upsert!`, `create_many!`, `update_many!` — phase 4/5
- `aggregate!`, `group_by!` — phase 6
- Nested-write expansion inside `data:` — phase 5
- Standalone shape macros (`where!`, `include!`, `select!`) — phase 4
- Computed/virtual field codegen — phase 7
- Documentation-site Angular pages — phase 8
