# Nested `disconnect` / `delete` / `delete_many` (Phase 5c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend phase 5b's nested-write surface with three "remove relation" operators inside `create!`'s `data:` block:

```rust
let user = prax::create!(client.user, {
    data: {
        email: "alice@x.com",
        posts: {
            create: [{ title: "kept" }],   // phase 5b
            connect: [{ id: 1 }],          // phase 5b
            disconnect: [{ id: 2 }],       // phase 5c — clear FK on row #2
            delete: [{ id: 3 }],           // phase 5c — DELETE row #3
            delete_many: { published: false }, // phase 5c — DELETE WHERE filter
        },
    },
}).exec().await?;
```

Phase 5c does **not** extend nested writes into `update!` / `upsert!` macros — those need their own `.with(nw)` infrastructure on `UpdateOperation` / `UpsertOperation`, and that's a separate phase. Phase 5c only extends `create!`'s already-wired nested path.

The mutation operators (`update`, `update_many`, `upsert`) and `set:` (full-relation diff replacement) plus `connect_or_create` (engine-specific lowering) remain deferred.

**Architecture:**

Three new `NestedWriteOp` variants — mirroring phase 5b's `Connect` pattern with `&'static str` identifier fields:

```rust
pub enum NestedWriteOp {
    Create { /* ... */ },                  // phase 5b
    Connect { /* ... */ },                 // phase 5b
    Disconnect {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
    },
    Delete {
        relation: &'static str,
        target_table: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
    },
    DeleteMany {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        filter: Filter,
    },
}
```

SQL emissions:
- **Disconnect**: `UPDATE child SET fk = NULL WHERE pk = $1` (clears the FK; row stays)
- **Delete**: `DELETE FROM child WHERE pk = $1`
- **DeleteMany**: `DELETE FROM child WHERE fk = $parent_pk AND <filter>` — note the AND-with-parent-FK is a safety bound that prevents nuking rows outside this relation

DSL lowering extends `data_relation.rs` to accept these three operators. The existing `phase_5c_deferral` arm narrows to the still-deferred set (`update`, `update_many`, `upsert`, `set` → "phase 5c-mutations"; `connect_or_create` → "phase 5d").

**Tech Stack:** Rust 2024, plus the phase-3/4/5a/5b macro pipeline.

---

## File Structure

### New files

- `tests/ui/nested_writes/nested_set_phase_5e.rs` + `.stderr` — replaces the now-narrower phase-5c deferral with separate deferral fixtures (one for `set`, one for `update`)
- `tests/ui/nested_writes/nested_update_phase_5c_mutations.rs` + `.stderr` — deferral for `update:` inside relation

### Modified files

- `prax-query/src/nested.rs` — add `Disconnect`, `Delete`, `DeleteMany` variants + executors
- `prax-codegen/src/macros/lower/data_relation.rs` — add `disconnect` / `delete` / `delete_many` arms; narrow the deferral arms
- `tests/nested_writes_e2e.rs` — e2e coverage for new ops
- `tests/ui/nested_writes/nested_unknown_op_phase_5c.rs` (existing) — update to test an operator that's still deferred (e.g. `update`) since `delete` is no longer deferred
- `CHANGELOG.md`

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/nested-update-ops rev-parse --abbrev-ref HEAD`
Expected: `feature/nested-update-ops`.

- [ ] **Step 2: Confirm base**

Run: `git -C /home/joseph/Projects/prax/.worktrees/nested-update-ops log --oneline -1`
Expected: starts with `cdd097e feat(codegen): nested create/connect + tech-debt sweep (phase 5b)`.

- [ ] **Step 3: `cargo check --workspace --all-features`** — zero errors.

- [ ] **Step 4: Existing tests pass** — `cargo test -p prax-query --lib && cargo test -p prax-codegen --lib`.

- [ ] **Step 5: No commit — verification only.**

---

## Task 2: `NestedWriteOp::Disconnect` variant + executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant**

Inside `pub enum NestedWriteOp` (around line 609), after the existing `Connect` variant:

```rust
/// Disconnect a child row by clearing its FK column to NULL.
///
/// Lowers to `UPDATE <target_table> SET <foreign_key> = NULL WHERE <target_pk> = <pk>`.
/// The child row persists; only the FK is cleared. Use [`Delete`] to remove
/// the row entirely.
Disconnect {
    relation: &'static str,
    target_table: &'static str,
    foreign_key: &'static str,
    target_pk: &'static str,
    pk: FilterValue,
},
```

- [ ] **Step 2: Implement the executor arm**

In the `match self` block inside `NestedWriteOp::execute`, add a Disconnect arm after Connect:

```rust
NestedWriteOp::Disconnect { target_table, foreign_key, target_pk, pk, .. } => {
    let dialect = engine.dialect();
    let sql = format!(
        "UPDATE {} SET {} = NULL WHERE {} = {}",
        dialect.quote_ident(target_table),
        dialect.quote_ident(foreign_key),
        dialect.quote_ident(target_pk),
        dialect.placeholder(1),
    );
    engine.execute_raw(&sql, vec![pk]).await?;
    Ok(())
}
```

Note: `parent_pk` is unused for Disconnect (we don't need to know which parent we're disconnecting from — the child PK uniquely identifies the row). This is a deliberate asymmetry vs Connect.

- [ ] **Step 3: Unit test**

In the `#[cfg(test)] mod tests` block (around line 1035), add:

```rust
#[tokio::test]
async fn nested_op_disconnect_emits_update_set_null() {
    let engine = RecordingEngine::new();
    let op = NestedWriteOp::Disconnect {
        relation: "posts",
        target_table: "posts",
        foreign_key: "author_id",
        target_pk: "id",
        pk: FilterValue::Int(42),
    };
    op.execute(&engine, &FilterValue::Int(7)).await.unwrap();
    let stmts = engine.statements();
    assert_eq!(stmts.len(), 1);
    let (sql, params) = &stmts[0];
    assert!(sql.contains("UPDATE"));
    assert!(sql.contains("SET"));
    assert!(sql.contains("NULL"));
    assert!(sql.contains("WHERE"));
    assert_eq!(params.len(), 1);
    assert!(matches!(params[0], FilterValue::Int(42)));
}
```

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::Disconnect variant + executor
```

---

## Task 3: `NestedWriteOp::Delete` variant + executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant**

```rust
/// Delete a child row by its primary key.
///
/// Lowers to `DELETE FROM <target_table> WHERE <target_pk> = <pk>`.
/// Unlike [`Disconnect`], this removes the row entirely.
Delete {
    relation: &'static str,
    target_table: &'static str,
    target_pk: &'static str,
    pk: FilterValue,
},
```

Note: no `foreign_key` field — DELETE only needs the PK.

- [ ] **Step 2: Implement the executor**

```rust
NestedWriteOp::Delete { target_table, target_pk, pk, .. } => {
    let dialect = engine.dialect();
    let sql = format!(
        "DELETE FROM {} WHERE {} = {}",
        dialect.quote_ident(target_table),
        dialect.quote_ident(target_pk),
        dialect.placeholder(1),
    );
    let affected = engine.execute_raw(&sql, vec![pk]).await?;
    if affected != 1 {
        return Err(crate::error::QueryError::not_found(target_table)
            .with_context("Nested Delete by PK"));
    }
    Ok(())
}
```

The affected-rows check matches the Connect-batch pattern from PR #105 — deleting a non-existent PK is a `RecordNotFound`.

- [ ] **Step 3: Unit test** (mirror Task 2's pattern)

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::Delete variant + executor
```

---

## Task 4: `NestedWriteOp::DeleteMany` variant + executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant**

```rust
/// Delete many child rows by FK + scalar filter.
///
/// Lowers to `DELETE FROM <target_table> WHERE <foreign_key> = <parent_pk> AND <filter>`.
/// The AND-with-parent-FK is a safety bound — the filter is scoped to the
/// parent's children only; it cannot remove rows belonging to other parents.
DeleteMany {
    relation: &'static str,
    target_table: &'static str,
    foreign_key: &'static str,
    filter: Filter,
},
```

- [ ] **Step 2: Implement the executor**

```rust
NestedWriteOp::DeleteMany { target_table, foreign_key, filter, .. } => {
    let dialect = engine.dialect();
    // The user's filter starts at $2 since $1 is the parent PK.
    let (filter_sql, mut params) = filter.to_sql(2, dialect);
    params.insert(0, parent_pk.clone());
    let sql = if filter_sql.is_empty() {
        format!(
            "DELETE FROM {} WHERE {} = {}",
            dialect.quote_ident(target_table),
            dialect.quote_ident(foreign_key),
            dialect.placeholder(1),
        )
    } else {
        format!(
            "DELETE FROM {} WHERE {} = {} AND ({})",
            dialect.quote_ident(target_table),
            dialect.quote_ident(foreign_key),
            dialect.placeholder(1),
            filter_sql,
        )
    };
    engine.execute_raw(&sql, params).await?;
    Ok(())
}
```

Note: DeleteMany doesn't check affected-rows — deleting zero rows when the filter doesn't match is a valid no-op, not a `RecordNotFound`.

- [ ] **Step 3: Unit test**

Cover both: filter-only (e.g. `Filter::Equals("published", false.into())`) and empty-filter (the AND clause is omitted).

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::DeleteMany variant + executor
```

---

## Task 5: DSL lowering for `disconnect` / `delete` / `delete_many`

**Files:**
- Modify: `prax-codegen/src/macros/lower/data_relation.rs`

- [ ] **Step 1: Add `disconnect` lowering**

In `lower_create_relation` (or wherever the operator dispatch lives — currently the `match op_key.as_str()` block around line 144-180), add a new arm after `connect`:

```rust
"disconnect" => {
    let children = expect_list_of_blocks(value, &op_key, key.span())?;
    for child_block in children {
        let pk_expr = lower_connect_pk(child_block, target_model, &target_pk_column)?;
        let op_expr = quote! {
            ::prax_query::nested::NestedWriteOp::Disconnect {
                relation: #relation_name_str,
                target_table: #target_table,
                foreign_key: #foreign_key,
                target_pk: #target_pk_column,
                pk: ::core::convert::Into::<
                    ::prax_query::filter::FilterValue
                >::into(#pk_expr),
            }
        };
        ops.push(NestedRelationOp { op_expr });
    }
}
```

The `lower_connect_pk` helper from phase 5b is reused — same shape (single PK value per child block).

- [ ] **Step 2: Add `delete` lowering**

Same pattern as `disconnect` but emit `NestedWriteOp::Delete` (which has no `foreign_key` field):

```rust
"delete" => {
    let children = expect_list_of_blocks(value, &op_key, key.span())?;
    for child_block in children {
        let pk_expr = lower_connect_pk(child_block, target_model, &target_pk_column)?;
        let op_expr = quote! {
            ::prax_query::nested::NestedWriteOp::Delete {
                relation: #relation_name_str,
                target_table: #target_table,
                target_pk: #target_pk_column,
                pk: ::core::convert::Into::<
                    ::prax_query::filter::FilterValue
                >::into(#pk_expr),
            }
        };
        ops.push(NestedRelationOp { op_expr });
    }
}
```

- [ ] **Step 3: Add `delete_many` lowering**

`delete_many:` takes a single filter block (not a list). The filter lowers against the **child** model's WhereInput. Reuse the existing `lower_where` from phase 3:

```rust
"delete_many" => {
    let DslValue::Block(filter_block) = value else {
        return Err(syn::Error::new(
            key.span(),
            "`delete_many:` inside a relation expects a filter block `{ ... }`",
        ));
    };
    let target_ctx = LowerCtx::new(ctx.schema, target_model);
    let filter_expr = super::where_input::lower_where(filter_block, &target_ctx)?;
    // The where-input lowers to a <Child>WhereInput value; to extract a `Filter`,
    // call into_ir() on it.
    let op_expr = quote! {
        ::prax_query::nested::NestedWriteOp::DeleteMany {
            relation: #relation_name_str,
            target_table: #target_table,
            foreign_key: #foreign_key,
            filter: <_ as ::prax_query::inputs::WhereInput>::into_ir(#filter_expr),
        }
    };
    ops.push(NestedRelationOp { op_expr });
}
```

- [ ] **Step 4: Narrow the phase-5c deferral arm**

Find the arm matching `"update" | "update_many" | "upsert" | "delete" | "delete_many" | "disconnect" | "set"` (around line 172) and shrink to the still-deferred set:

```rust
"update" | "update_many" | "upsert" => {
    return Err(phase_5c_mutations_deferral(&op_key, relation_field.name(), key.span()));
}
"set" => {
    return Err(phase_5e_deferral("set", relation_field.name(), key.span()));
}
```

`connect_or_create` keeps its own phase-5d arm.

Rename `phase_5c_deferral` to `phase_5c_mutations_deferral` and update the wording to reflect the narrower scope (mention update/upsert specifically, not the now-shipped operators). Add a new `phase_5e_deferral` for `set`.

- [ ] **Step 5: Snapshot/unit tests**

In `data_relation.rs::tests`, add tests proving:
- A `disconnect:` block lowers to a `NestedWriteOp::Disconnect` token stream
- A `delete:` block lowers to `Delete`
- A `delete_many: { ... }` block lowers to `DeleteMany`
- An `update:` block STILL produces the (renamed) `phase_5c_mutations_deferral` error
- A `set:` block produces the new `phase_5e_deferral` error

- [ ] **Step 6: `cargo test -p prax-codegen data_relation`**

- [ ] **Step 7: Commit**

```
feat(codegen): lower disconnect/delete/delete_many inside data: blocks
```

---

## Task 6: trybuild fixture updates

**Files:**
- Modify: `tests/ui/nested_writes/nested_unknown_op_phase_5c.rs` (existing) — change the deferred operator under test from one we just shipped (e.g. `delete`) to one still deferred (e.g. `update`)
- Modify: `tests/ui/nested_writes/nested_unknown_op_phase_5c.stderr` — regenerate baseline
- Create: `tests/ui/nested_writes/nested_set_phase_5e.rs` + `.stderr` — new fixture for `set:`

- [ ] **Step 1: Update existing fixture**

The existing `nested_unknown_op_phase_5c.rs` uses `update:` (or whichever operator). Verify it still triggers the deferral diagnostic after the Task 5 narrowing. The diagnostic wording will have changed (now mentions "phase 5c-mutations" or similar) — regenerate the stderr.

Run: `TRYBUILD=overwrite cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`

Inspect the new baseline; confirm it's still helpful.

- [ ] **Step 2: Create `nested_set_phase_5e.rs`**

Mirror the existing fixture but use `set: [...]` inside a relation block. The diagnostic should mention phase 5e (full-relation replacement is deferred).

Run `TRYBUILD=overwrite` again to write the stderr.

- [ ] **Step 3: `cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`**

- [ ] **Step 4: Commit**

```
test(codegen): trybuild fixtures for narrowed phase-5c deferral + set
```

---

## Task 7: E2E tests

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Add three e2e tests**

Mirror the existing phase-5b e2e tests (`mixed_create_and_connect_in_order`, etc.). Don't use the `prax::create!` macro path due to the schema-helpers latent bug; instead build the operation via the runtime API with direct `NestedWriteOp` construction:

- `nested_disconnect_emits_update_set_null` — single Disconnect, asserts UPDATE child SET fk=NULL WHERE pk=$1
- `nested_delete_emits_delete_where_pk` — single Delete, asserts DELETE FROM child WHERE pk=$1
- `nested_delete_many_with_filter_emits_delete_where_fk_and_filter` — DeleteMany with `Filter::Equals("published", false.into())`, asserts the DELETE includes both the FK bound and the user filter

Plus one combined test:
- `create_with_disconnect_and_delete_in_same_transaction` — Create with 1 connect + 1 disconnect + 1 delete, all three nested ops inside one transaction; assert the recorded SQL order

- [ ] **Step 2: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 3: Commit**

```
test(query): e2e for nested disconnect/delete/delete_many
```

---

## Task 8: CHANGELOG + final verification sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG bullets under `[Unreleased]`**

```
### Added
- Nested **disconnect** / **delete** / **delete_many** operators inside
  `create!`'s `data:` block (phase 5c). `delete_many` is filter-scoped to
  the parent's children — the AND-with-parent-FK is enforced at SQL emit
  time and cannot be bypassed by user filters.
- `NestedWriteOp::Disconnect`, `NestedWriteOp::Delete`,
  `NestedWriteOp::DeleteMany` runtime variants + executors.
- New trybuild fixture covering the `set:` operator's phase-5e deferral.

### Changed
- The phase-5c deferral diagnostic narrowed: previously rejected
  `disconnect` / `delete` / `delete_many` (now shipped) plus
  `update` / `update_many` / `upsert` / `set`. The renamed
  "phase 5c-mutations" deferral now covers only `update` / `update_many`
  / `upsert`; `set` has its own "phase 5e" deferral.
```

- [ ] **Step 2: Full sweep**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

All three must be green.

- [ ] **Step 3: Commit**

```
docs(codegen): CHANGELOG for phase-5c nested disconnect/delete
```

---

## Task 9: Push + open PR

**Files:**
- None.

- [ ] **Step 1: `git push -u origin feature/nested-update-ops`**

- [ ] **Step 2: Open PR**

```bash
gh pr create --base develop --head feature/nested-update-ops \
  --title "feat(codegen): nested disconnect/delete/delete_many (phase 5c)" \
  --body "..."
```

PR body should:
- Summarize the three new operators
- Note what's still deferred (update/upsert/update_many → 5c-mutations; set → 5e; connect_or_create → 5d; nested inside update!/upsert! → separate phase)
- Reference spec §6 and this plan

- [ ] **Step 3: Wait for CI**

---

## Out of scope for phase 5c (deferred)

- `update` / `update_many` / `upsert` operators inside relation blocks — phase 5c-mutations
- Nested writes (any operator) inside `update!` / `upsert!` macros — requires `.with(nw)` infrastructure on `UpdateOperation` / `UpsertOperation`; separate phase
- `connect_or_create` — phase 5d (engine-specific lowerings)
- `set: [...]` full-relation replacement — phase 5e
- Aggregate macros — phase 6
- Computed/virtual field codegen — phase 5.5
