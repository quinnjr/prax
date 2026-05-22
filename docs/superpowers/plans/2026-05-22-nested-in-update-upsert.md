# Nested Writes Inside `update!` / `upsert!` Macros Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend nested-write support to the `update!` and `upsert!` macros (so far only `create!` accepts nested ops in `data:`).

```rust
// update! with nested ops in data:
prax::update!(client.user, {
    where: { id: 1 },
    data: {
        name: "Renamed",
        posts: {
            create: [{ title: "new post" }],
            disconnect: [{ id: 5 }],
        },
    },
}).exec().await?;

// upsert! with nested ops in both create: and update: branches
prax::upsert!(client.user, {
    where: { id: 1 },
    create: {
        email: "alice@x.com",
        posts: { create: [{ title: "from create branch" }] },
    },
    update: {
        name: "Renamed",
        posts: { disconnect: [{ id: 5 }] },
    },
}).exec().await?;
```

After this lands, every read+write macro accepts the full nested-write operator set. The only deferred nested-writes work is vendor-specific single-statement upsert (perf optimization).

**Architecture:**

Mirror phase 5b's `CreateOperation::with(nw)` pattern on `UpdateOperation` and extend `UpsertOperation` with two nested vecs (one per branch):

- `UpdateOperation`: add `nested: Vec<NestedWriteOp>` field + `with(nw)` builder. Exec runs the parent UPDATE first, then iterates nested ops with the affected row's PK as parent_pk. The parent_pk is fetched via a SELECT against the where-unique filter (post-UPDATE, since UPDATE-RETURNING isn't engine-agnostic).
- `UpsertOperation`: add `create_nested: Vec<NestedWriteOp>` and `update_nested: Vec<NestedWriteOp>` fields + `with_create_nested(nw)` and `with_update_nested(nw)` builders. Exec dispatches by which branch ran: create_nested fires when the UPDATE returned zero affected, update_nested fires when it returned non-zero.

**DSL changes:**

- New `lower_update_data_with_nested` in `data_input.rs` (parallel to phase-5b's `lower_create_data_with_nested`) that accepts relation keys and delegates to `data_relation::lower_create_relation` (which is misnamed — it works for any data block's relation keys, since it produces token streams referencing `NestedWriteOp::*`, not tied to a parent op type).
- `update!` macro switches from `lower_update_data` to `lower_update_data_with_nested` for its `data:` block.
- `upsert!` macro:
  - `create:` branch uses `lower_create_data_with_nested` (already exists)
  - `update:` branch uses the new `lower_update_data_with_nested`
  - The emit emits `.with_create_nested(nw)` and `.with_update_nested(nw)` per branch

**Tech Stack:** Rust 2024, phase-3 to 5e infrastructure. No new external deps.

---

## File Structure

### Modified files

- `prax-query/src/operations/update.rs` — add nested field + with(nw) builder + exec changes
- `prax-query/src/operations/upsert.rs` — add two nested fields + branch-specific builders + exec dispatch
- `prax-codegen/src/macros/lower/data_input.rs` — add `lower_update_data_with_nested`
- `prax-codegen/src/macros/lower/data_relation.rs` — rename `lower_create_relation` to `lower_data_relation` (it's used by both create and update paths now) OR leave the name; document the misnomer
- `prax-codegen/src/macros/ops/update.rs` — switch to `_with_nested`, emit `.with(nw)` chain
- `prax-codegen/src/macros/ops/upsert.rs` — switch both branches, emit `.with_create_nested(...)` / `.with_update_nested(...)`
- `tests/nested_writes_e2e.rs` — e2e coverage
- `CHANGELOG.md`

---

## Task 1: Verify baseline

- [ ] **Step 1**: `git -C /home/joseph/Projects/prax/.worktrees/nested-in-update-upsert rev-parse --abbrev-ref HEAD` → `feature/nested-in-update-upsert`
- [ ] **Step 2**: `git log --oneline -1` starts with `34b05a7 feat(codegen): nested set relation replacement (phase 5e)`
- [ ] **Step 3**: `cargo check --workspace --all-features` — zero errors
- [ ] **Step 4**: `cargo test -p prax-query --lib && cargo test -p prax-codegen --lib` — green
- [ ] **Step 5**: No commit.

---

## Task 2: `UpdateOperation::with(nw)` + nested exec

**Files:**
- Modify: `prax-query/src/operations/update.rs`

- [ ] **Step 1: Add nested field** to `UpdateOperation` (mirror `CreateOperation::nested: Vec<NestedWriteOp>` from phase 5b around line 33 of create.rs)

```rust
pub struct UpdateOperation<E: QueryEngine, M: Model> {
    /* existing fields */
    nested: Vec<crate::nested::NestedWriteOp>,
}
```

Initialize `nested: Vec::new()` in `new()`.

- [ ] **Step 2: Add `with(nw)` builder** gated on `SupportsNestedWrites`:

```rust
pub fn with(mut self, nw: crate::nested::NestedWriteOp) -> Self
where
    E: crate::capabilities::SupportsNestedWrites,
{
    self.nested.push(nw);
    self
}
```

- [ ] **Step 3: Extend `exec()` to run nested ops**

After the main UPDATE runs successfully:
1. Read the parent_pk from `self.filter` (the `WhereUniqueInput` filter). If the filter equals on the PK column directly, extract its value. Otherwise SELECT the PK back via a quick lookup.
2. For each `nw` in `self.nested`, call `nw.execute(&engine, &parent_pk).await?` — mirror create.rs's batching logic for consecutive Connect ops.

Hardest part: extracting parent_pk from arbitrary `Filter`. The filter's structure for a where-unique is typically `Filter::Equals("id".into(), FilterValue::Int(N))`. Pattern-match on that — if it's an Equals on the model's PK column, use the value directly. Otherwise fall back to a SELECT lookup:

```rust
let (sql, params) = self.filter.to_sql(1, dialect);
let select_sql = format!(
    "SELECT {} FROM {} WHERE {}",
    dialect.quote_ident(M::PRIMARY_KEY[0]),
    dialect.quote_ident(M::TABLE_NAME),
    sql,
);
let pk_row: Option<PkOnly> = engine.query_optional(&select_sql, params).await?;
let parent_pk = pk_row.ok_or_else(|| QueryError::not_found(M::TABLE_NAME))?.pk;
```

Where `PkOnly` is a tiny FromRow newtype. Actually — even simpler: emit `SELECT <pk> FROM ...` and use `engine.count` to get *something* — no, that doesn't give us the PK value.

**Simplification**: require the user's where-unique to be a PK lookup for now. Extract via pattern match on `Filter::Equals(field, value)` where `field == M::PRIMARY_KEY[0]`. If it's a non-PK unique field (e.g. email), error with a clear "nested writes inside update! require a where-unique that targets the primary key" message. Phase-followup can lift this restriction.

That's a real limitation but it's documented and the simple path. Implementation:

```rust
fn extract_pk_from_filter(filter: &Filter, pk_col: &str) -> Option<FilterValue> {
    match filter {
        Filter::Equals(name, value) if name.as_ref() == pk_col => Some(value.clone()),
        _ => None,
    }
}
```

If `nested.is_empty()`, skip the PK extraction entirely.

- [ ] **Step 4: Batched Connect support** (mirror create.rs's partition logic)

Reuse the same `if let NestedWriteOp::Connect { target_table, foreign_key, target_pk, .. } = &nested[idx]` pattern from create.rs to batch consecutive Connects with the same target.

- [ ] **Step 5: Unit tests**

- `update_with_nested_create` — Update with a single nested Create; assert UPDATE statement followed by child INSERT
- `update_with_nested_disconnect` — assert UPDATE + child UPDATE SET fk = NULL
- `update_nested_requires_pk_in_where` — Update with a non-PK where (e.g. email) and nested ops → returns a clear error

- [ ] **Step 6: `cargo test -p prax-query --lib`**

- [ ] **Step 7: Commit**

```
feat(query): UpdateOperation nested-writes via .with(nw)
```

---

## Task 3: `UpsertOperation` nested support (two branches)

**Files:**
- Modify: `prax-query/src/operations/upsert.rs`

- [ ] **Step 1: Add two nested fields** to `UpsertOperation`:

```rust
pub struct UpsertOperation<E: QueryEngine, M: Model> {
    /* existing */
    create_nested: Vec<crate::nested::NestedWriteOp>,
    update_nested: Vec<crate::nested::NestedWriteOp>,
}
```

- [ ] **Step 2: Add two builders**:

```rust
pub fn with_create_nested(mut self, nw: crate::nested::NestedWriteOp) -> Self
where
    E: crate::capabilities::SupportsNestedWrites,
{ self.create_nested.push(nw); self }

pub fn with_update_nested(mut self, nw: crate::nested::NestedWriteOp) -> Self
where
    E: crate::capabilities::SupportsNestedWrites,
{ self.update_nested.push(nw); self }
```

- [ ] **Step 3: Extend `exec()` to dispatch nested ops by branch**

After the existing UPDATE/INSERT logic, determine which branch ran:
- If the existing UPDATE affected > 0 rows → "update branch" → run `update_nested` ops with the parent_pk extracted from the where filter
- If the INSERT ran → "create branch" → run `create_nested` ops with the parent_pk being the newly-inserted row's PK (need to capture this from the INSERT)

Same PK-extraction limitation as Task 2 (where must equal-match the PK column).

- [ ] **Step 4: Unit tests**

- `upsert_with_nested_in_update_branch` — affected=1 on UPDATE → update_nested runs, create_nested doesn't
- `upsert_with_nested_in_create_branch` — affected=0 on UPDATE → INSERT runs → create_nested fires
- `upsert_both_branches_carry_nested` — verify both vecs are populated and only the right one fires

- [ ] **Step 5: `cargo test -p prax-query --lib`**

- [ ] **Step 6: Commit**

```
feat(query): UpsertOperation per-branch nested writes
```

---

## Task 4: `lower_update_data_with_nested` in data_input.rs

**Files:**
- Modify: `prax-codegen/src/macros/lower/data_input.rs`

- [ ] **Step 1: Implement `lower_update_data_with_nested`** (mirror `lower_create_data_with_nested` from phase 5b):

Returns `(TokenStream, Vec<TokenStream>)` — the UpdateInput value + a list of NestedWriteOp expression token streams (one per relation entry encountered).

For each relation key inside the data block, delegate to `data_relation::lower_create_relation` (which produces `NestedRelationOp` values regardless of whether the parent op is create or update — the produced `NestedWriteOp::*` tokens are agnostic).

Same overall structure as `lower_create_data_with_nested`: walk the block, route scalar fields normally, route relation fields to `lower_create_relation`, collect both halves.

- [ ] **Step 2: Tests** mirror the existing `lower_create_data_with_nested` tests in `data_input.rs::tests`.

- [ ] **Step 3: `cargo test -p prax-codegen data_input`**

- [ ] **Step 4: Commit**

```
feat(codegen): lower_update_data_with_nested for nested writes inside update!
```

---

## Task 5: Wire `update!` macro for nested ops

**Files:**
- Modify: `prax-codegen/src/macros/ops/update.rs`

- [ ] **Step 1: Switch the `data:` lowering call**

Find the line that calls `lower_update_data(b, ctx)?` (around the data: arm). Replace with `lower_update_data_with_nested(b, ctx)?` which returns both the data payload TokenStream and a Vec of nested-op token streams.

- [ ] **Step 2: Emit `.with(nw)` chain in the generated code**

```rust
let __op = #accessor.update();
let __op = __op.with_where_input(__where);
let __op = __op.with_update_input(__data);
#( let __op = __op.with(#nested_ops); )*
__op
```

Mirror create.rs::expand_create's emit pattern for nested ops.

- [ ] **Step 3: Smoke compile + e2e test scaffolding**

- [ ] **Step 4: Commit**

```
feat(codegen): update! macro accepts nested writes in data:
```

---

## Task 6: Wire `upsert!` macro for nested ops in both branches

**Files:**
- Modify: `prax-codegen/src/macros/ops/upsert.rs`

- [ ] **Step 1: Switch both lowering calls**

`create:` arm calls `lower_create_data_with_nested` (already exists from phase 5b) and collects both `create_data_payload` + `create_nested_ops`.

`update:` arm calls `lower_update_data_with_nested` (from Task 4) and collects `update_data_payload` + `update_nested_ops`.

- [ ] **Step 2: Emit dual `.with_create_nested(...)` / `.with_update_nested(...)` chains**

```rust
let __op = #accessor.upsert();
let __op = __op.with_where_input(__where);
let __op = __op.with_create_input(__create_data);
let __op = __op.with_update_input(__update_data);
#( let __op = __op.with_create_nested(#create_nested_ops); )*
#( let __op = __op.with_update_nested(#update_nested_ops); )*
__op
```

- [ ] **Step 3: Smoke compile + e2e test scaffolding**

- [ ] **Step 4: Commit**

```
feat(codegen): upsert! macro accepts nested writes in create: and update: branches
```

---

## Task 7: E2E tests

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Add update! + nested tests** (use direct API to avoid the schema-path bug from phase 5b):

- `update_with_nested_create` — Update operation with nested Create; assert UPDATE + child INSERT statements
- `update_with_nested_disconnect_and_delete` — Update + nested Disconnect + nested Delete; assert proper order

- [ ] **Step 2: Add upsert! + nested tests**

- `upsert_update_branch_runs_update_nested` — affected=1 → only update_nested fires; create_nested doesn't
- `upsert_create_branch_runs_create_nested` — affected=0 → INSERT path → create_nested fires
- `upsert_with_nested_in_both_branches` — populate both; verify only one fires per branch

- [ ] **Step 3: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 4: Commit**

```
test(query): e2e for nested writes inside update! and upsert!
```

---

## Task 8: CHANGELOG + final sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG entry**

```
### Added
- **Nested writes inside `update!` and `upsert!` macros.** `update!`'s
  `data:` block and `upsert!`'s `create:`/`update:` branches now accept
  the full Prisma nested-write operator set (`create`, `connect`,
  `disconnect`, `delete`, `delete_many`, `update`, `update_many`,
  `upsert`, `connect_or_create`, `set`).
- `UpdateOperation::with(NestedWriteOp)` and
  `UpsertOperation::with_create_nested(NestedWriteOp)` /
  `with_update_nested(NestedWriteOp)` runtime builders, gated on
  `SupportsNestedWrites`.
- `UpsertOperation` dispatches nested ops by branch: update_nested
  fires when the existing-row UPDATE matched; create_nested fires
  when the row was newly inserted.

### Known limitations
- Nested writes inside `update!`/`upsert!` currently require the
  `where:` clause to equal-match on the primary key column. Non-PK
  unique columns (e.g. `where: { email: "..." }`) error with a clear
  diagnostic. Lifting this restriction is a separate follow-up — it
  needs a SELECT-then-update pattern to capture the row's PK.
```

- [ ] **Step 2: Full sweep**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast
```

- [ ] **Step 3: Commit**

```
docs(codegen): CHANGELOG for nested writes inside update! and upsert!
```

---

## Task 9: Push + open PR

- [ ] `git push -u origin feature/nested-in-update-upsert`
- [ ] `gh pr create --base develop --head feature/nested-in-update-upsert --title "feat(query): nested writes inside update! and upsert! macros"`

---

## Out of scope (deferred)

- Vendor-specific single-statement upsert (Postgres `ON CONFLICT` etc.) — next PR
- Lifting the PK-only where-unique limitation — needs SELECT-then-update; separate follow-up
- Aggregate macros — phase 6
- Computed/virtual fields — phase 5.5
