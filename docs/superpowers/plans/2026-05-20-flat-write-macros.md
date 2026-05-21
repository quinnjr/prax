# Flat Write Macros (Phase 5a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the five write macros described in spec §4 and §6 for flat (non-nested) row writes. After this phase, users can write:

```rust
let user = prax::create!(client.user, {
    data: {
        email: "alice@example.com",
        name: "Alice",
        age: 30,
    },
    select: { id: true, email: true },
}).exec().await?;

let updated = prax::update!(client.user, {
    where: { id: 1 },
    data: {
        name: "Renamed",
        age: { increment: 1 },     // FieldUpdate atomic operator
        last_seen: { set: now() }, // explicit `set` for scalar
    },
    select: { id: true, age: true, last_seen: true },
}).exec().await?;
```

Five macros ship in this phase, each constraining the top-level keys per spec §4:

| Macro | Top-level keys |
|-------|---------------|
| `create!`       | `data`, `include` xor `select` |
| `create_many!`  | `data` (list), `skip_duplicates` |
| `update!`       | `where` (unique), `data`, `include` xor `select` |
| `update_many!`  | `where`, `data` |
| `upsert!`       | `where` (unique), `create`, `update`, `include` xor `select` |

**Phase 5a is scalar-only.** Nested-write relation operators (`create`, `connect`, `disconnect`, `set`, `update`, `update_many`, `upsert`, `delete`, `delete_many`, `connect_or_create` inside `data:`) plus the `NestedWritePlan` IR / executor and `connect_or_create` paths are deferred to phase 5b. Phase 5a's `data:` parser rejects relation keys with a clear "phase 5b" diagnostic.

**Architecture:**

- **Runtime (`prax-query`)**:
  - Add `with_create_input<I: CreateInput<Model = M>>(self, input: I) -> Self` to `CreateOperation` and `UpsertOperation`.
  - Add `with_update_input<I: UpdateInput<Model = M>>(self, input: I) -> Self` to `UpdateOperation`, `UpdateManyOperation`, and `UpsertOperation`.
  - Add `with_create_inputs(self, inputs: impl IntoIterator<Item = I>) -> Self` to `CreateManyOperation`.
  - Add `create_many()` and `update_many()` accessor methods to the `ModelAccessor` trait. Default impls return a generic `CreateManyOperation`/`UpdateManyOperation` constructed from `&self.engine()` — same pattern as the existing `create()` / `update()`.
- **Codegen (`prax-codegen`)**:
  - Extend the phase-2 input emitter to emit `impl prax::inputs::CreateInput for <Model>CreateInput` and `impl prax::inputs::UpdateInput for <Model>UpdateInput`. `into_ir()` lowers the struct to the existing `<Model as CreateData>::Data` shape (or the equivalent existing runtime payload — verify by reading `prax-query/src/traits.rs::CreateData::Data` before implementing).
  - New `prax-codegen/src/macros/lower/data_input.rs` — lowers the DSL `data:` block to either `<Model>CreateInput` (create path) or `<Model>UpdateInput` (update path). Reuses the existing phase-3 DSL parser and validator. Scalar fields lower to the typed input struct field assignments; on the update path, nested `{ increment: N }` / `{ decrement: N }` / `{ set: V }` / `{ unset: true }` operator blocks lower to the matching `*FieldUpdate` wrapper from `prax-query/src/inputs/scalar_update.rs`.
  - Five new entry points in `prax-codegen/src/macros/ops/`:
    - `create.rs::expand_create`
    - `create_many.rs::expand_create_many`
    - `update.rs::expand_update`
    - `update_many.rs::expand_update_many`
    - `upsert.rs::expand_upsert`
  - Each follows the same shape as the phase-3 read ops: parse accessor → parse `{ ... }` block → match top-level keys against an allow-list → lower each → emit `with_*_input` chain.
- **Umbrella (`prax-orm`)**: re-export the five new proc-macros.

**Tech Stack:** Rust 2024, `proc_macro2`, `quote`, `syn 2.0`, plus the phase-3 macro pipeline (`schema_resolve`, `dsl`, `lower`, `validate`, `accessor`). No new external dependencies.

---

## File Structure

### New files

- `prax-codegen/src/macros/lower/data_input.rs` — DSL `data:` lowering for both create and update paths
- `prax-codegen/src/macros/ops/create.rs`
- `prax-codegen/src/macros/ops/create_many.rs`
- `prax-codegen/src/macros/ops/update.rs`
- `prax-codegen/src/macros/ops/update_many.rs`
- `prax-codegen/src/macros/ops/upsert.rs`
- `tests/write_macros_e2e.rs` — workspace-level integration tests against the in-process engine used in `read_macros_e2e.rs` and `shape_macros_e2e.rs`
- `tests/ui/write_macros/data_unknown_field.rs` + `.stderr`
- `tests/ui/write_macros/data_nested_relation_phase_5b.rs` + `.stderr` (using a relation key inside `data:` — should emit "phase 5b" diagnostic)
- `tests/ui/write_macros/upsert_missing_create.rs` + `.stderr`
- `tests/ui/write_macros/update_many_with_unique.rs` + `.stderr` (using a unique-only where clause where a permissive where was expected — Should still compile actually; reconsider this fixture during Task 11)

### Modified files

- `prax-query/src/operations/create.rs` — add `with_create_input` on `CreateOperation`; add `with_create_inputs` on `CreateManyOperation`
- `prax-query/src/operations/update.rs` — add `with_update_input` on `UpdateOperation` and `UpdateManyOperation`
- `prax-query/src/operations/upsert.rs` — add `with_create_input` + `with_update_input` on `UpsertOperation`
- `prax-query/src/traits.rs` — add `create_many()` and `update_many()` accessor methods on `ModelAccessor`
- `prax-codegen/src/generators/inputs/create_input.rs` — emit `impl CreateInput for <Model>CreateInput`
- `prax-codegen/src/generators/inputs/update_input.rs` — emit `impl UpdateInput for <Model>UpdateInput`
- `prax-codegen/src/macros/ops/mod.rs` — add module declarations for the five new ops
- `prax-codegen/src/macros/lower/mod.rs` — add `pub mod data_input;`
- `prax-codegen/src/lib.rs` — five new `#[proc_macro]` entry points
- `src/lib.rs` (umbrella `prax-orm`) — five re-exports
- `tests/trybuild_read_macros.rs` — extend glob to include `tests/ui/write_macros/*.rs` (or add a sibling driver)
- `CHANGELOG.md` — `[Unreleased]` bullets

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/write-macros rev-parse --abbrev-ref HEAD`
Expected: `feature/write-macros`.

- [ ] **Step 2: Confirm base**

Run: `git -C /home/joseph/Projects/prax/.worktrees/write-macros log --oneline -1`
Expected: starts with `d638c97 feat(codegen): shape macros (phase 4) (#103)`.

- [ ] **Step 3: Workspace builds**

Run: `cargo check --workspace --all-features`
Expected: zero errors.

- [ ] **Step 4: Existing tests pass**

Run: `cargo test -p prax-codegen --lib && cargo test -p prax-orm --tests`
Expected: all pass.

- [ ] **Step 5: No commit — verification only.**

---

## Task 2: Runtime — `with_create_input` / `with_update_input` on operation types

**Files:**
- Modify: `prax-query/src/operations/create.rs`
- Modify: `prax-query/src/operations/update.rs`
- Modify: `prax-query/src/operations/upsert.rs`

- [ ] **Step 1: Read `prax-query/src/traits.rs::CreateData`** to understand the existing runtime payload shape that `CreateInput::Data` lowers to. The phase-1 trait comment says "The associated `Data` type is the existing `<Model as CreateData>::Data`" — verify and reuse.

- [ ] **Step 2: Add `with_create_input` to `CreateOperation`**

```rust
impl<E: QueryEngine, M: Model> CreateOperation<E, M> {
    pub fn with_create_input<I: crate::inputs::CreateInput<Model = M>>(
        mut self,
        input: I,
    ) -> Self {
        let data: <M as crate::traits::CreateData>::Data = input.into_ir();
        // Apply `data` to self — match the existing builder API. If
        // CreateData::Data is a Vec<(column, value)> tuple list, loop
        // and call self.set(col, val). If it's a richer struct, call
        // its existing accessor.
        // ...
        self
    }
}
```

The exact lowering depends on `CreateData::Data`. If `Data = Vec<(String, FilterValue)>`, loop and call the existing `.set(column, value)`. If it's a richer struct, look for an existing setter or extend `set` to take an iterator. **Read first, design second.**

- [ ] **Step 3: Add `with_create_inputs` to `CreateManyOperation`**

```rust
impl<E: QueryEngine, M: Model> CreateManyOperation<E, M> {
    pub fn with_create_inputs<I, T>(mut self, inputs: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: crate::inputs::CreateInput<Model = M>,
    {
        for input in inputs {
            let data = input.into_ir();
            // Append `data` to self.rows (or whichever field
            // CreateManyOperation uses to accumulate batch rows)
        }
        self
    }
}
```

- [ ] **Step 4: Add `with_update_input` to `UpdateOperation`, `UpdateManyOperation`**

Same pattern, using `<M as UpdateData>::Data` (verify the trait name in `prax-query/src/traits.rs`). On the update path, `Data` likely includes both atomic-operator updates (`increment`/`decrement`/etc.) and plain `set` semantics.

- [ ] **Step 5: Add `with_create_input` + `with_update_input` to `UpsertOperation`**

Same pattern.

- [ ] **Step 6: Unit tests in each modified file**

For each new method: construct a typed input (use a hand-rolled `MockCreateInput` / `MockUpdateInput` impl since the codegen-emitted types don't exist yet), pass to the operation, assert the resulting SQL string matches what the equivalent `.set(col, val)` chain produces.

- [ ] **Step 7: `cargo test -p prax-query --lib`**

Expected: all pass.

- [ ] **Step 8: Commit**

```
feat(query): with_create_input / with_update_input on write operations
```

---

## Task 3: Runtime — `create_many()` / `update_many()` on `ModelAccessor`

**Files:**
- Modify: `prax-query/src/traits.rs`

- [ ] **Step 1: Add the two methods**

```rust
pub trait ModelAccessor<E: QueryEngine>: Send + Sync {
    type Model: Model;
    fn engine(&self) -> &E;

    // ... existing methods ...

    fn create_many(&self) -> crate::operations::CreateManyOperation<E, Self::Model> {
        crate::operations::CreateManyOperation::new(self.engine().clone())
    }

    fn update_many(&self) -> crate::operations::UpdateManyOperation<E, Self::Model> {
        crate::operations::UpdateManyOperation::new(self.engine().clone())
    }
}
```

Adjust to match the existing `create()` / `update()` signatures (they may take `&self` and return `Operation<E, M>` directly; copy the pattern verbatim).

If `E: Clone` isn't already required by `ModelAccessor`, check how `create()` / `update()` work today — they presumably take `&E` or use a different pattern. Match it.

- [ ] **Step 2: `cargo check -p prax-query`**

- [ ] **Step 3: Commit**

```
feat(query): create_many / update_many on ModelAccessor
```

---

## Task 4: Codegen — emit `impl CreateInput` / `impl UpdateInput`

**Files:**
- Modify: `prax-codegen/src/generators/inputs/create_input.rs`
- Modify: `prax-codegen/src/generators/inputs/update_input.rs`

- [ ] **Step 1: For `<Model>CreateInput`, emit a trait impl**

```rust
impl ::prax_query::inputs::CreateInput for #model::CreateInput {
    type Model = #model::Self_Model_Type;
    type Data = <Self::Model as ::prax_query::traits::CreateData>::Data;

    fn into_ir(self) -> Self::Data {
        // Convert each Option<T> field on Self into the matching
        // entry in `Data`. If `Data = Vec<(String, FilterValue)>`,
        // emit a Vec literal of present fields.
        let mut __out: Self::Data = ::core::default::Default::default();
        // ... per-field push ...
        __out
    }
}
```

Match `Data` to whatever `CreateData::Data` actually is — read the trait def first.

- [ ] **Step 2: Same shape for `<Model>UpdateInput → impl UpdateInput`**

The update path needs to handle the `*FieldUpdate` wrapper structs (e.g. `IntFieldUpdate { set, increment, decrement }`). Each Option<FieldUpdate> wrapper field on `<Model>UpdateInput` lowers to the corresponding atomic-operator entry in `<Model as UpdateData>::Data`.

If the existing `UpdateData::Data` doesn't already support atomic operators, this task may need to extend it. Read first.

- [ ] **Step 3: Snapshot tests in `prax-codegen/tests/lower_snapshots.rs`** (extend the existing file)

Add insta snapshots for `<TestModel as CreateInput>::into_ir` token-stream output and `<TestModel as UpdateInput>::into_ir`. Confirm scalar-only round-trip lowers cleanly.

- [ ] **Step 4: Extend the derive_inputs_e2e tests**

Add tests that construct `<User>CreateInput`, call `.into_ir()`, and assert the result matches a hand-rolled equivalent `CreateData::Data` value.

- [ ] **Step 5: `cargo test -p prax-codegen && cargo test -p prax-orm --test derive_inputs_e2e`**

- [ ] **Step 6: Commit**

```
feat(codegen): emit CreateInput / UpdateInput trait impls
```

---

## Task 5: DSL — `data:` lowering for the create path

**Files:**
- Create: `prax-codegen/src/macros/lower/data_input.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs`

- [ ] **Step 1: Implement `lower_create_data(block, ctx) -> syn::Result<TokenStream>`**

For each `DslField::Pair`:
- If `key` matches a scalar field on `ctx.model` → emit `__c.<key> = Some(<value-lowering>);`
  - `value-lowering`: lit → `into()`; bare-ident → `<EnumPath>::<Ident>` if the field is an enum; path → as-is; expr → as-is.
- If `key` matches a relation field → return a phase-5b deferral error:
  > `nested write on relation \`{rel}\` is not supported in phase 5a — relation operators inside \`data:\` (create / connect / connect_or_create / set / update / delete) land in phase 5b`
- Logical operators (`and`/`or`/`not`) not valid here; reject with clear error.
- Unknown key → existing did-you-mean machinery from `validate.rs`, scoped to the create input's scalar fields plus this model's relation fields.

Emit a block expression that produces `<Model>CreateInput`:

```rust
{
    let mut __c = #crate_path::inputs::user::UserCreateInput::default();
    __c.email = Some("alice@example.com".into());
    /* ... */
    __c
}
```

- [ ] **Step 2: Spread support `..expr`**

Match the same pattern phase-3 lowering uses: replace the `default()` with `Clone::clone(&(<expr>))` when a spread appears first. Subsequent assignments overwrite.

- [ ] **Step 3: Bare-ident enum resolution**

A bare ident `Role` in a value position should resolve against the field's declared enum if applicable. Otherwise treat as expression. Already wired in `lower/scalar_filter.rs` for `equals: Admin` — reuse.

- [ ] **Step 4: Snapshot tests in `lower_snapshots.rs`**

Cover: pure-scalar create, spread create, bare-ident enum, expression escape `@(expr)`, relation-key rejection (assert the error variant, don't snapshot).

- [ ] **Step 5: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 6: Commit**

```
feat(codegen): lower DSL data: blocks to CreateInput
```

---

## Task 6: DSL — `data:` lowering for the update path

**Files:**
- Modify: `prax-codegen/src/macros/lower/data_input.rs`

- [ ] **Step 1: Implement `lower_update_data(block, ctx) -> syn::Result<TokenStream>`**

Same as `lower_create_data` but emit a `<Model>UpdateInput`. The key difference: scalar values can either be:
- A **literal/expr/path/bare-ident** → wraps with `<FieldUpdate>::set_to(value)` (i.e. `IntFieldUpdate { set: Some(value), increment: None, decrement: None }`)
- A **`{ set: V }` block** → explicit set
- A **`{ increment: N }` / `{ decrement: N }` / `{ multiply: N }` / `{ divide: N }` block** → atomic operator
- A **`{ unset: true }` block** (nullable fields only) → null

Look up the FieldUpdate wrapper struct for the scalar category (`IntFieldUpdate`, `BigIntFieldUpdate`, `FloatFieldUpdate`, `StringFieldUpdate`, etc.) — the mapping mirrors phase-3's `lower_scalar_filter` operator dispatch.

Reject `increment`/`decrement`/`multiply`/`divide` on non-numeric fields with a clear "operator `increment` is not valid for `String` field" error.

- [ ] **Step 2: Relation keys are phase 5b**

Same deferral message as `lower_create_data`.

- [ ] **Step 3: Snapshot tests**

Cover: plain set, explicit set block, increment, decrement, unset on nullable, increment on String (error), unset on non-nullable (error).

- [ ] **Step 4: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 5: Commit**

```
feat(codegen): lower DSL data: blocks to UpdateInput
```

---

## Task 7: `create!` macro

**Files:**
- Create: `prax-codegen/src/macros/ops/create.rs`
- Modify: `prax-codegen/src/macros/ops/mod.rs`
- Modify: `prax-codegen/src/lib.rs` (add `#[proc_macro] fn create`)
- Modify: `src/lib.rs` (umbrella re-export)
- Create / extend: `tests/write_macros_e2e.rs`

- [ ] **Step 1: `expand_create(input) -> syn::Result<TokenStream>`**

Follow the `find_many.rs` pattern. Allowed top-level keys: `data` (required), `include`, `select` (xor). Reject other keys with did-you-mean.

```rust
{
    #_schema_dep_const
    let __op = <_ as ::prax_query::ModelAccessor<_>>::create(&(#accessor_expr));
    let __data = #data_block;
    let __op = __op.with_create_input(__data);
    #(let __include = #include_block; let __op = __op.with_include_input(__include);)?
    #(let __select = #select_block; let __op = __op.with_select_input(__select);)?
    __op
}
```

- [ ] **Step 2: `#[proc_macro] fn create` in `prax-codegen/src/lib.rs`** + umbrella re-export

- [ ] **Step 3: Smoke-compile test in `tests/write_macros_e2e.rs`**

```rust
#[test]
fn create_macro_compiles() {
    let _op = prax::create!(client.user, {
        data: { email: "a@x.com", name: "Alice", age: 30 },
        select: { id: true },
    });
}
```

- [ ] **Step 4: `cargo test -p prax-orm --test write_macros_e2e`**

- [ ] **Step 5: Commit**

```
feat(codegen): create! proc-macro
```

---

## Task 8: `update!` macro

**Files:**
- Create: `prax-codegen/src/macros/ops/update.rs`
- Modify: same lib.rs + umbrella as Task 7
- Modify: `tests/write_macros_e2e.rs`

- [ ] **Step 1: `expand_update`**

Allowed top-level keys: `where` (required, unique), `data` (required), `include`, `select`. `lower_where` is the existing phase-3 helper; for `update!` it must target `WhereUniqueInput` not `WhereInput` — call the existing `lower_cursor` (which already targets WhereUniqueInput) or factor `lower_where_unique` out of it.

- [ ] **Step 2: Proc-macro wrapper + re-export**

- [ ] **Step 3: e2e tests**

Cover: scalar set, `{ increment: N }`, `{ unset: true }`, mixed.

- [ ] **Step 4: `cargo test -p prax-orm --test write_macros_e2e`**

- [ ] **Step 5: Commit**

```
feat(codegen): update! proc-macro
```

---

## Task 9: `upsert!` macro

**Files:**
- Create: `prax-codegen/src/macros/ops/upsert.rs`
- Modify: lib.rs + umbrella
- Modify: `tests/write_macros_e2e.rs`

- [ ] **Step 1: `expand_upsert`**

Allowed top-level keys: `where` (required, unique), `create` (required, lowers via `lower_create_data`), `update` (required, lowers via `lower_update_data`), `include`/`select`.

The macro emits both `with_create_input` and `with_update_input` calls on `UpsertOperation`.

- [ ] **Step 2: Proc-macro wrapper + re-export**

- [ ] **Step 3: e2e tests**

Cover: insert path (no existing row), update path (existing row), validation errors when `create` or `update` missing.

- [ ] **Step 4: Commit**

```
feat(codegen): upsert! proc-macro
```

---

## Task 10: `create_many!` macro

**Files:**
- Create: `prax-codegen/src/macros/ops/create_many.rs`
- Modify: lib.rs + umbrella
- Modify: `tests/write_macros_e2e.rs`

- [ ] **Step 1: `expand_create_many`**

Allowed top-level keys: `data` (required, list of blocks), `skip_duplicates` (optional bool).

```rust
prax::create_many!(client.user, {
    data: [
        { email: "a@x.com", name: "Alice" },
        { email: "b@x.com", name: "Bob" },
    ],
    skip_duplicates: true,
});
```

`data:` parsed as `DslValue::List` of `DslValue::Block`. Each block lowers via `lower_create_data`. The macro emits:
```rust
__op.with_create_inputs(vec![__data_0, __data_1, /* ... */])
    .skip_duplicates(true)
```

- [ ] **Step 2: Proc-macro wrapper + re-export**

- [ ] **Step 3: e2e tests**

- [ ] **Step 4: Commit**

```
feat(codegen): create_many! proc-macro
```

---

## Task 11: `update_many!` macro

**Files:**
- Create: `prax-codegen/src/macros/ops/update_many.rs`
- Modify: lib.rs + umbrella
- Modify: `tests/write_macros_e2e.rs`

- [ ] **Step 1: `expand_update_many`**

Allowed top-level keys: `where` (non-unique allowed, lowers via `lower_where`), `data` (required, lowers via `lower_update_data`).

- [ ] **Step 2: Proc-macro wrapper + re-export**

- [ ] **Step 3: e2e tests**

- [ ] **Step 4: Commit**

```
feat(codegen): update_many! proc-macro
```

---

## Task 12: trybuild UI tests

**Files:**
- Create: `tests/ui/write_macros/data_unknown_field.rs` + `.stderr`
- Create: `tests/ui/write_macros/data_nested_relation_phase_5b.rs` + `.stderr`
- Create: `tests/ui/write_macros/upsert_missing_create.rs` + `.stderr`
- Create: `tests/ui/write_macros/update_increment_on_string.rs` + `.stderr`
- Create: `tests/ui/write_macros/create_missing_data.rs` + `.stderr`
- Modify: `tests/trybuild_read_macros.rs` — extend glob to include `tests/ui/write_macros/*.rs`

- [ ] **Step 1: Author the five `.rs` fixtures**

Each fixture triggers exactly one diagnostic class. Use the existing `tests/ui/read_macros/` fixtures as a shape reference.

- [ ] **Step 2: Generate `.stderr` baselines**

`TRYBUILD=overwrite cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`. Inspect each generated `.stderr`. Confirm the diagnostic text is helpful for a real user; improve the validator messages if not.

The "phase 5b" deferral message in `data_nested_relation_phase_5b.stderr` deserves extra care — make sure the message is friendly and points users to the right phase rather than feeling like a generic "unknown field".

- [ ] **Step 3: `cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`** — green against committed baselines.

- [ ] **Step 4: Commit**

```
test(codegen): trybuild UI fixtures for write macros
```

---

## Task 13: e2e composition tests + CHANGELOG + push

**Files:**
- Modify: `tests/write_macros_e2e.rs`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Composition tests**

Prove the spread story works for write inputs:

```rust
#[test]
async fn create_with_spread() {
    let defaults = prax::r#where!(User, { active: true }); // bad example
    // ... actual test: build a *CreateInput value-like construct,
    //     spread into create!.
}
```

Actually, the spread-into-write story differs from spread-into-find because there is no `prax::create_data!(User, { ... })` shape macro in this phase (that's part of phase 4 follow-up or phase 5b). For phase 5a, the composition tests should cover:
- `create!` end-to-end exec returning the requested `select` shape
- `update!` with a mix of atomic + plain set
- `update_many!` against multiple rows
- `upsert!` insert path and update path
- `create_many!` with `skip_duplicates`

- [ ] **Step 2: CHANGELOG bullet under `[Unreleased]`**

```
### Added
- Flat write macros: `prax::create!`, `prax::update!`, `prax::upsert!`,
  `prax::create_many!`, `prax::update_many!`. Each supports `data:` for
  scalar fields with atomic operators (`increment`, `decrement`,
  `multiply`, `divide`, `set`, `unset`) and the same `include` / `select`
  return-shape contract as the read macros.
- `with_create_input` / `with_update_input` / `with_create_inputs`
  builder methods on the corresponding `Operation` types.
- `create_many` / `update_many` accessor methods on `ModelAccessor`.
- Generated `impl CreateInput for <Model>CreateInput` and
  `impl UpdateInput for <Model>UpdateInput` so the typed input structs
  can be passed directly to the new builder methods.

### Deferred to phase 5b
- Relation operators inside `data:` (`create`, `connect`, `disconnect`,
  `set`, `update`, `update_many`, `upsert`, `delete`, `delete_many`,
  `connect_or_create`).
- `NestedWritePlan` IR + executor.
- `SupportsNestedWrites` per-engine declarations.
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
feat(codegen): e2e tests + CHANGELOG for flat write macros
```

---

## Task 14: Push + open PR

**Files:**
- None.

- [ ] **Step 1: `git push -u origin feature/write-macros`**

Pre-push hook runs the full test suite (~3 min).

- [ ] **Step 2: Open PR**

```bash
gh pr create --base develop --head feature/write-macros \
  --title "feat(codegen): flat write macros (phase 5a)" \
  --body "..."
```

PR body should:
- Summarize the five macros
- Explicitly call out the 5a/5b split — phase 5a is scalar-only writes
- List the runtime additions (`with_*_input`, `create_many`/`update_many` on `ModelAccessor`, `impl CreateInput`/`UpdateInput` codegen)
- Link the spec and plan
- Test plan checklist

- [ ] **Step 3: Wait for CI**

---

## Out of scope for phase 5a (deferred to phase 5b)

- Relation operators inside `data:` blocks (nested writes)
- `NestedWritePlan` IR + executor (`prax-query`)
- `connect_or_create` single-statement and two-statement lowerings
- `SupportsNestedWrites` per-engine impls
- `set: [...]` full-relation-replacement semantics
- Generated `<Model><Relation>CreateNestedInput` / `<Model><Relation>UpdateNestedInput` and the `WithoutXxx` variants
- The CQL capability-gap trybuild fixture (phase 5b — `SupportsNestedWrites` doesn't yet need the gate at the macro level in phase 5a)
- Aggregate macros (phase 6)
- Computed/virtual field codegen (phase 5.5)
