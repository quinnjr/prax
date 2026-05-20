# Shape Macros (Phase 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the standalone shape macros described in spec §4 and the phase-4 row of §8. After this phase, users can build reusable filter / shape values that compose with the phase-3 read macros via spread.

```rust
// Reusable filter
let active_adult = prax::where!(User, {
    active: true,
    age: { gte: 18 },
});

// Plugged into find_many! via spread; later keys override
let results = prax::find_many!(client.user, {
    ..active_adult,
    email: { ends_with: "@x.com" },
    order_by: [{ created_at: desc }],
    take: 50,
}).exec().await?;
```

Five shape macros ship in this phase, each returning the corresponding phase-2 input struct **as a value** (no operation chaining):

| Macro | Returns |
|-------|---------|
| `prax::where!(Model, { ... })`     | `<Model>WhereInput` |
| `prax::include!(Model, { ... })`   | `<Model>Include` |
| `prax::select!(Model, { ... })`    | `<Model>Select` |
| `prax::order_by!(Model, ...)`      | `Vec<<Model>OrderBy>` (auto-wraps single block) |
| `prax::cursor!(Model, { ... })`    | `<Model>WhereUniqueInput` |

Phase 4 does **not** introduce any new lowering logic — the existing `pub fn lower_where`, `lower_include`, `lower_select`, `lower_order_by`, `lower_cursor` in `prax-codegen/src/macros/lower/` are reused as-is. Spread, conditional, bare-ident enum resolution, and validation are all inherited from phase 3.

**Architecture:**

- New entry-point module `prax-codegen/src/macros/ops/shape.rs` hosts the five `expand_*_shape` functions. Each:
  1. Resolves the schema (`resolve_schema()` from phase 3).
  2. Parses a leading model ident (`Model`) followed by `,` followed by a DSL value (block or list depending on macro).
  3. Looks up the model in the schema; errors with span on miss.
  4. Builds a `LowerCtx` and delegates to the existing `lower_*` function.
  5. Wraps the result with `track_schema_dep(&schema_path)` so rustc picks up schema changes.
- A small helper `parse_model_ident(s, schema) -> syn::Result<&Model>` lives next to `accessor.rs` (or in a new `shape_accessor.rs`).
- Top-level `#[proc_macro]` wrappers in `prax-codegen/src/lib.rs`, one per macro, each delegating to `macros::ops::shape::*`.
- Re-export from `prax-orm/src/lib.rs` matches the phase-3 pattern (`pub use prax_codegen::{r#where, include, select, order_by, cursor};`). Note `where` is a Rust keyword and must be exported as `r#where`.

**Tech Stack:** Rust 2024, `proc_macro2`, `quote`, `syn 2.0`, `prax-schema` AST, plus the existing phase-3 macro pipeline.

---

## File Structure

### New files

- `prax-codegen/src/macros/ops/shape.rs` — five `expand_*_shape` entry points
- `prax-codegen/src/macros/shape_accessor.rs` — `parse_model_ident` helper (or inline into shape.rs if it stays trivial — see Task 2)
- `prax-codegen/tests/ui/shape_macros/where_unknown_field.rs` + `.stderr`
- `prax-codegen/tests/ui/shape_macros/where_typo.rs` + `.stderr`
- `prax-codegen/tests/ui/shape_macros/order_by_wrong_dir.rs` + `.stderr` (e.g. `order_by!(User, { created_at: 5 })`)
- `tests/shape_macros_e2e.rs` — workspace-level e2e proving each macro produces a value of the expected type and that values compose with `find_many!` via spread

### Modified files

- `prax-codegen/src/macros/ops/mod.rs` — add `pub(crate) mod shape;`
- `prax-codegen/src/lib.rs` — five new `#[proc_macro]` entry points
- `src/lib.rs` (umbrella `prax-orm`) — five new re-exports
- `tests/trybuild_read_macros.rs` (existing) — extend to also glob `tests/ui/shape_macros/*.rs`, or add a sibling `tests/trybuild_shape_macros.rs` driver (Task 6 picks one)
- `CHANGELOG.md` — `[Unreleased]` section bullets
- `prax-codegen/src/macros/lower/mod.rs` (maybe) — only if a new helper is needed; expect no change

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/shape-macros rev-parse --abbrev-ref HEAD`
Expected: `feature/shape-macros`.

- [ ] **Step 2: Confirm base**

Run: `git -C /home/joseph/Projects/prax/.worktrees/shape-macros log --oneline -1`
Expected: starts with `df23431 test(codegen): commit trybuild stderr baselines for read macros`.

- [ ] **Step 3: Workspace builds**

Run: `cargo check --workspace --all-features`
Expected: zero errors.

- [ ] **Step 4: Existing tests pass**

Run: `cargo test -p prax-codegen --lib && cargo test -p prax-orm --tests`
Expected: all pass.

- [ ] **Step 5: No commit — verification only.**

---

## Task 2: `parse_model_ident` helper

**Files:**
- Create: `prax-codegen/src/macros/shape_accessor.rs`
- Modify: `prax-codegen/src/macros/mod.rs` — add `pub(crate) mod shape_accessor;`

- [ ] **Step 1: Implement the helper**

```rust
use proc_macro2::Span;
use prax_schema::{Model, Schema};
use syn::parse::ParseStream;

/// Parse a single model identifier and resolve it against the schema.
///
/// Used by the standalone shape macros (`where!`, `include!`, `select!`,
/// `order_by!`, `cursor!`). The accessor expression form used by the read
/// macros (`client.user`, `Model on expr`, …) is not relevant here —
/// shape macros operate on the typed input struct directly, with no
/// accessor.
pub fn parse_model_ident<'a>(
    input: ParseStream,
    schema: &'a Schema,
) -> syn::Result<(syn::Ident, &'a Model)> {
    let ident: syn::Ident = input.parse()?;
    let name = ident.to_string();
    let model = schema.models.iter().find_map(|(_, m)| {
        if m.name == name { Some(m) } else { None }
    });
    match model {
        Some(m) => Ok((ident, m)),
        None => Err(syn::Error::new(
            ident.span(),
            format!("unknown model `{name}` — not declared in schema"),
        )),
    }
}
```

The exact `Model` field name (`name`) and `Schema::models` shape should match phase-3 accessor parsing — reuse the same approach `accessor::resolve_model_from_path` uses for consistency. If that helper already does PascalCase lookup, prefer calling it.

- [ ] **Step 2: Unit tests**

In `prax-codegen/src/macros/shape_accessor.rs` under `#[cfg(test)]`:
- Happy path: `User` ident against a schema containing `User`.
- Unknown model: `Foo` against the same schema → error.

Use the same minimal in-line schema construction other unit tests in `macros/` use.

- [ ] **Step 3: `cargo test -p prax-codegen shape_accessor`**

- [ ] **Step 4: Commit**

```
feat(codegen): parse_model_ident helper for shape macros
```

---

## Task 3: `where!` macro

**Files:**
- Create: `prax-codegen/src/macros/ops/shape.rs`
- Modify: `prax-codegen/src/macros/ops/mod.rs` — `pub(crate) mod shape;`
- Modify: `prax-codegen/src/lib.rs` — add `#[proc_macro] fn r#where` (or `where_`; pick one — see Step 3)
- Modify: `src/lib.rs` (umbrella) — re-export

- [ ] **Step 1: Implement `expand_where_shape`**

```rust
pub fn expand_where_shape(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = crate::macros::schema_resolve::resolve_schema()?;
    let schema_path = crate::macros::schema_resolve::resolve_schema_path()?;
    let dep = crate::macros::schema_resolve::track_schema_dep(&schema_path);

    let parser = move |s: syn::parse::ParseStream<'_>| -> syn::Result<TokenStream> {
        let (_ident, model) =
            crate::macros::shape_accessor::parse_model_ident(s, &schema)?;
        let _: syn::Token![,] = s.parse()?;
        let block: crate::macros::dsl::DslBlock = s.parse()?;
        let ctx = crate::macros::lower::LowerCtx::new(&schema, model);
        crate::macros::lower::where_input::lower_where(&block, &ctx)
    };

    let body = syn::parse::Parser::parse2(parser, input)?;
    Ok(quote::quote! {
        {
            #dep
            #body
        }
    })
}
```

- [ ] **Step 2: Top-level `#[proc_macro]` wrapper in `prax-codegen/src/lib.rs`**

```rust
#[proc_macro]
pub fn r#where(input: TokenStream) -> TokenStream {
    match macros::ops::shape::expand_where_shape(input.into()) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
```

`r#where` is the raw-identifier form needed because `where` is a Rust keyword.

- [ ] **Step 3: Re-export from `prax-orm/src/lib.rs`**

```rust
pub use prax_codegen::r#where;
```

Users call it as `prax::where!(User, { ... })` — when used as a macro invocation, the raw-identifier prefix is not required at the call site. Confirm by writing the first e2e test in Step 5.

- [ ] **Step 4: Smoke-compile test in `tests/shape_macros_e2e.rs`**

Add a new test file with one test:
```rust
#[test]
fn where_macro_compiles() {
    let _w: ::prax_orm::inputs::user::UserWhereInput =
        ::prax_orm::r#where!(User, { id: { equals: 1 } });
}
```

Verify the macro can be invoked AND that the result has the expected concrete type. If the raw-identifier prefix `r#where` is required at the call site, also test the un-prefixed `prax::where!(User, ...)` form for ergonomics. If both work, prefer the unprefixed form in documentation.

- [ ] **Step 5: `cargo test -p prax-orm --test shape_macros_e2e`**

- [ ] **Step 6: Commit**

```
feat(codegen): where! shape macro
```

---

## Task 4: `include!` + `select!` macros

**Files:**
- Modify: `prax-codegen/src/macros/ops/shape.rs` — add `expand_include_shape`, `expand_select_shape`
- Modify: `prax-codegen/src/lib.rs` — add `#[proc_macro] fn include` (rename — see below) and `fn select`
- Modify: `src/lib.rs` (umbrella) — re-exports
- Modify: `tests/shape_macros_e2e.rs` — extend

**Naming note:** Rust has no built-in `include` or `select` keyword, so plain `pub fn include` and `pub fn select` are fine. However, `std::include!` exists as a macro — emitting a `prax::include!` should not clash because they're imported from different namespaces, but **do confirm** by writing an e2e test that imports `prax_orm::include` and uses it adjacent to a normal Rust file.

- [ ] **Step 1: Implement both functions**

Same pattern as `expand_where_shape` but delegate to `lower_include` and `lower_select` respectively. The `select!` macro is symmetric — no special handling.

- [ ] **Step 2: Top-level `#[proc_macro]` wrappers in `prax-codegen/src/lib.rs`**

- [ ] **Step 3: Umbrella re-exports**

- [ ] **Step 4: E2E tests in `shape_macros_e2e.rs`**

Add two tests proving the returned type matches `<Model>Include` / `<Model>Select`, similar to the `where!` test above.

- [ ] **Step 5: `cargo test -p prax-orm --test shape_macros_e2e`**

- [ ] **Step 6: Commit**

```
feat(codegen): include! + select! shape macros
```

---

## Task 5: `order_by!` + `cursor!` macros

**Files:**
- Modify: `prax-codegen/src/macros/ops/shape.rs`
- Modify: `prax-codegen/src/lib.rs`
- Modify: `src/lib.rs` (umbrella)
- Modify: `tests/shape_macros_e2e.rs`

- [ ] **Step 1: `expand_order_by_shape`**

Differs from the others: input can be a `{ ... }` block (single sort spec) OR a `[..., ...]` list (multiple). The existing `lower_order_by(value: &DslValue, ctx)` already handles both forms — parse the trailing input as a `DslValue` and pass through.

```rust
pub fn expand_order_by_shape(input: TokenStream) -> syn::Result<TokenStream> {
    let schema = ...;
    let parser = move |s: ParseStream| -> syn::Result<TokenStream> {
        let (_, model) = parse_model_ident(s, &schema)?;
        let _: Token![,] = s.parse()?;
        let value: DslValue = s.parse()?;
        let ctx = LowerCtx::new(&schema, model);
        lower_order_by(&value, &ctx)
    };
    ...
}
```

`DslValue: Parse` should already exist from phase 3 task 6. If not, add a `Parse` impl for `DslValue` that dispatches the same way the brace-block field parser already does.

- [ ] **Step 2: `expand_cursor_shape`**

Same pattern as `where!`, but delegate to `lower_cursor` (which targets `WhereUniqueInput`).

- [ ] **Step 3: `#[proc_macro]` wrappers + umbrella re-exports**

- [ ] **Step 4: E2E tests**

- Two `order_by!` tests: single block and list.
- One `cursor!` test against a `@unique` column.

- [ ] **Step 5: `cargo test -p prax-orm --test shape_macros_e2e`**

- [ ] **Step 6: Commit**

```
feat(codegen): order_by! + cursor! shape macros
```

---

## Task 6: trybuild UI tests for shape diagnostics

**Files:**
- Create: `tests/ui/shape_macros/where_unknown_field.rs` (+ `.stderr` — generate with `TRYBUILD=overwrite`)
- Create: `tests/ui/shape_macros/where_typo.rs` (+ `.stderr`)
- Create: `tests/ui/shape_macros/cursor_non_unique.rs` (+ `.stderr`)
- Create: `tests/ui/shape_macros/unknown_model.rs` (+ `.stderr`)
- Modify: `tests/trybuild_read_macros.rs` — change the glob to `tests/ui/**/*.rs` so it picks up both `read_macros/` and `shape_macros/`. Alternatively, create `tests/trybuild_shape_macros.rs` as a sibling driver — pick whichever requires less Cargo plumbing.

- [ ] **Step 1: Author each `.rs` fixture**

Each fixture must produce exactly one error class. The existing `unknown_field` fixture under `read_macros/` is a good shape reference — copy and adapt.

- [ ] **Step 2: Generate `.stderr` baselines**

Run: `TRYBUILD=overwrite cargo test -p prax-orm --features ui-tests --test trybuild_read_macros` (or the sibling driver name if you split). Inspect each generated `.stderr` to confirm the diagnostic text would help a real user. Improve the validator messages if the wording is poor and regenerate.

- [ ] **Step 3: `cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`**

Expected: green against the committed `.stderr` baselines.

- [ ] **Step 4: Commit**

```
test(codegen): trybuild UI fixtures for shape macros
```

---

## Task 7: End-to-end composition tests + CHANGELOG

**Files:**
- Modify: `tests/shape_macros_e2e.rs` — add composition tests
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Composition tests**

Prove the spread-composition story from this plan's intro works end to end:

```rust
#[test]
async fn shape_value_composes_with_find_many() {
    let base = prax::where!(User, { active: true });
    let _op = prax::find_many!(client.user, {
        ..base,
        age: { gte: 18 },
    });
    // assert against the engine's stashed Filter
}
```

Add at least one composition test per shape macro: `where!` + `find_many!`, `include!` + `find_unique!`, `select!` + `find_many!`, `order_by!` + `find_many!`, `cursor!` + `find_many!`. The engine can be in-process (whatever phase-3 e2e uses) — these are mostly compile-time assertions plus a trivial exec.

- [ ] **Step 2: CHANGELOG bullets under `[Unreleased]`**

```
### Added
- Shape macros: `prax::where!`, `prax::include!`, `prax::select!`,
  `prax::order_by!`, `prax::cursor!`. Each takes `(Model, { ... })`
  and returns the corresponding phase-2 typed input struct, enabling
  reusable filter / include / select / order values that compose with
  the read macros via `..spread`.
```

- [ ] **Step 3: Full verification sweep**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

All three must be green.

- [ ] **Step 4: Commit**

```
feat(codegen): e2e composition tests + CHANGELOG for shape macros
```

---

## Task 8: Push + open PR

**Files:**
- None.

- [ ] **Step 1: `git push -u origin feature/shape-macros`**

Pre-push hook runs the full test suite.

- [ ] **Step 2: Open PR**

```bash
gh pr create --base feature/read-operation-macros --head feature/shape-macros \
  --title "feat(codegen): shape macros — where!/include!/select!/order_by!/cursor! (phase 4)" \
  --body "..."
```

**Base branch is `feature/read-operation-macros`, not `develop`.** This PR is stacked on PR #102. After #102 merges to `develop`, retarget this PR to `develop` via the GitHub UI or `gh pr edit <num> --base develop`.

PR body should:
- Note the stacked-PR relationship to #102
- Summarize scope (5 shape macros + composition tests)
- Link the spec and plan
- Test plan checklist

- [ ] **Step 3: Wait for CI**

---

## Out of scope for phase 4 (deferred)

- Write macros (`create!`, `update!`, `upsert!`, `create_many!`, `update_many!`) — phase 5
- Aggregate macros (`aggregate!`, `group_by!`) — phase 6
- A standalone `data!` shape macro for write-input composition — phase 5 (paired with write macros)
- Computed/virtual field codegen — phase 5.5
- Documentation-site Angular pages — phase 7
