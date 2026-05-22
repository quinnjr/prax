# Nested `connect_or_create` (Phase 5d) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land `connect_or_create` inside `create!`'s `data:` block:

```rust
let user = prax::create!(client.user, {
    data: {
        email: "alice@x.com",
        posts: {
            connect_or_create: [
                {
                    where: { id: 42 },
                    create: { title: "fallback if id=42 doesn't exist" },
                },
            ],
        },
    },
}).exec().await?;
```

After this phase, the only nested-write operator still deferred inside `create!`'s `data:` is `set:` (phase 5e).

**Architecture:**

`connect_or_create` semantically: "if a row matches the unique `where`, connect it (set its FK to the parent); else INSERT a new row with the parent's FK."

The executor uses a **two-statement engine-agnostic** approach mirroring phase-5c-mutations' Upsert:

```
UPDATE <child> SET <fk> = $1 WHERE <filter>
if affected_rows == 0:
    INSERT INTO <child> (create_cols + fk) VALUES (..., $parent_pk)
```

No `SELECT` round-trip needed. The UPDATE either touches the row (the connect path) or doesn't (the create path). If the filter matches multiple rows, they all get their FK pointed at the parent — documented behavior.

Single-statement vendor-specific upsert (Postgres `ON CONFLICT`, MySQL `ON DUPLICATE KEY`, MSSQL `MERGE`) is a separate optimization phase — same dispatch infrastructure would also apply to phase 5c-mutations' Upsert. Out of scope for 5d.

**New variant:**

```rust
pub enum NestedWriteOp {
    /* existing variants */
    ConnectOrCreate {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        where_filter: Filter,
        create_payload: Vec<(String, FilterValue)>,
    },
}
```

**Tech Stack:** Rust 2024, phase-3/4/5a-5c-mutations macro pipeline. No new external deps.

---

## File Structure

### Modified files

- `prax-query/src/nested.rs` — new variant + executor + unit tests
- `prax-codegen/src/macros/lower/data_relation.rs` — replace the `connect_or_create` deferral arm with real lowering; remove `phase_5d_deferral`-style helpers if any
- `tests/nested_writes_e2e.rs` — e2e coverage
- `tests/ui/nested_writes/nested_connect_or_create_phase_5d.{rs,stderr}` — delete (operator now ships)
- `CHANGELOG.md`

### Deleted

- `tests/ui/nested_writes/nested_connect_or_create_phase_5d.rs` + `.stderr`

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/connect-or-create rev-parse --abbrev-ref HEAD`
Expected: `feature/connect-or-create`.

- [ ] **Step 2: Confirm base**

Run: `git -C /home/joseph/Projects/prax/.worktrees/connect-or-create log --oneline -1`
Expected: starts with `f248194 feat(codegen): nested update/update_many/upsert (phase 5c-mutations)`.

- [ ] **Step 3: `cargo check --workspace --all-features`** — zero errors.

- [ ] **Step 4: `cargo test -p prax-query --lib && cargo test -p prax-codegen --lib`** — green.

- [ ] **Step 5: No commit — verification only.**

---

## Task 2: `NestedWriteOp::ConnectOrCreate` variant + two-statement executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant** (after the `Upsert` variant added in phase 5c-mutations):

```rust
/// Connect an existing child row if a `where` filter matches, else
/// insert a new one with the parent's FK spliced in.
///
/// Two-statement engine-agnostic lowering:
/// 1. `UPDATE <target_table> SET <foreign_key> = $1 WHERE <filter>`
///    (the connect path — points any matching row at the parent).
/// 2. If `affected_rows == 0`, emit
///    `INSERT INTO <target_table> (<create_cols + foreign_key>) VALUES (<...>)`
///    (the create path).
///
/// If the filter matches multiple rows, every match has its FK pointed
/// at the parent — `connect_or_create` is typically used with a unique
/// where, but this is not enforced at runtime.
ConnectOrCreate {
    relation: &'static str,
    target_table: &'static str,
    foreign_key: &'static str,
    where_filter: Filter,
    create_payload: Vec<(String, FilterValue)>,
},
```

- [ ] **Step 2: Implement the executor arm**

```rust
NestedWriteOp::ConnectOrCreate {
    relation: _,
    target_table,
    foreign_key,
    where_filter,
    create_payload,
} => {
    let dialect = engine.dialect();
    // Phase 1: attempt UPDATE to connect existing row(s).
    let (filter_sql, filter_params) =
        where_filter.to_sql(2, &crate::dialect::Postgres);
    let mut update_params: Vec<FilterValue> = Vec::with_capacity(filter_params.len() + 1);
    update_params.push(parent_pk.clone());
    update_params.extend(filter_params);
    let update_sql = format!(
        "UPDATE {} SET {} = {} WHERE {}",
        dialect.quote_ident(target_table),
        dialect.quote_ident(foreign_key),
        dialect.placeholder(1),
        filter_sql,
    );
    let affected = engine.execute_raw(&update_sql, update_params).await?;
    if affected > 0 {
        return Ok(());
    }
    // Phase 2: no row matched — INSERT with FK spliced in.
    if create_payload.is_empty() {
        return Err(crate::error::QueryError::not_found(target_table)
            .with_context("Nested ConnectOrCreate: no match and create payload empty"));
    }
    let mut columns: Vec<String> = create_payload.iter().map(|(c, _)| c.clone()).collect();
    let mut values: Vec<FilterValue> = create_payload.into_iter().map(|(_, v)| v).collect();
    columns.push(foreign_key.to_string());
    values.push(parent_pk.clone());
    let placeholders: Vec<String> =
        (1..=values.len()).map(|i| dialect.placeholder(i)).collect();
    let quoted_cols: Vec<String> = columns.iter().map(|c| dialect.quote_ident(c)).collect();
    let insert_sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        dialect.quote_ident(target_table),
        quoted_cols.join(", "),
        placeholders.join(", "),
    );
    engine.execute_raw(&insert_sql, values).await?;
    Ok(())
}
```

This mirrors the phase-5c-mutations Upsert pattern almost exactly — diff is that the UPDATE SET clause is just the FK column (vs Upsert's user-supplied SET payload) and the WHERE uses `where_filter` (vs Upsert's `target_pk`).

- [ ] **Step 3: Unit tests**

- `nested_op_connect_or_create_connect_path_when_affected` — `RecordingEngine::with_affected(vec![1])` so the UPDATE returns 1; assert only the UPDATE statement was emitted
- `nested_op_connect_or_create_create_path_when_zero_affected` — `RecordingEngine::with_affected(vec![0, 1])`; assert both UPDATE and INSERT emitted, INSERT params include parent_pk last

Reuse the `with_affected` constructor added in phase 5c-mutations' Upsert tests.

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::ConnectOrCreate variant + two-statement executor
```

---

## Task 3: DSL lowering for `connect_or_create`

**Files:**
- Modify: `prax-codegen/src/macros/lower/data_relation.rs`

- [ ] **Step 1: Replace the deferral arm** (currently around line 306):

```rust
"connect_or_create" => {
    let children = expect_list_of_blocks(value, &op_key, key.span())?;
    let target_ctx = ctx.for_model(target_model);
    for child_block in children {
        // Each entry is `{ where: { ... }, create: { ... } }`.
        let (where_expr, create_expr) =
            extract_connect_or_create_entry(child_block, &target_ctx)?;
        let op_expr = quote! {
            ::prax_query::nested::NestedWriteOp::ConnectOrCreate {
                relation: #relation_name_str,
                target_table: #target_table,
                foreign_key: #foreign_key,
                where_filter: <_ as ::prax_query::inputs::WhereInput>::into_ir(#where_expr),
                create_payload: <_ as ::prax_query::inputs::CreateInput>::into_ir(#create_expr),
            }
        };
        ops.push(NestedRelationOp { op_expr });
    }
}
```

- [ ] **Step 2: Add the `extract_connect_or_create_entry` helper**

Similar to phase 5c-mutations' `extract_upsert_entry` but without the `update:` key. Expects a child block with exactly two keys: `where` (filter block) and `create` (data block). Returns `(where_expr_tokenstream, create_expr_tokenstream)`. Reuse `take_named_field` / `reject_extra_keys` helpers from 5c-mutations.

The `where:` lowers via `super::where_input::lower_where` (any filter shape allowed at codegen-time; semantics at runtime use the resulting Filter directly).
The `create:` lowers via `super::data_input::lower_create_data` against the target's LowerCtx (FK column omitted from user input; appended at executor time).

- [ ] **Step 3: Update unknown-operator candidates list** — add `connect_or_create` to the suggestion candidates so unknown operators get correct did-you-mean.

- [ ] **Step 4: Remove or update the `connect_or_create_is_phase_5d` test**

The current test (around line 844 in `data_relation.rs::tests`) asserts a phase-5d deferral. The deferral is gone. Replace with a positive lowering test:

```rust
#[test]
fn lowers_nested_connect_or_create_to_nested_write_op() {
    let schema = parsed_schema();
    let user = schema.get_model("User").unwrap().clone();
    let ctx = LowerCtx::new(&schema, &user);
    let field = user.get_field("posts").unwrap();
    let value = DslValue::Block(parse_block(quote!({
        connect_or_create: [{ where: { id: 1 }, create: { title: "x" } }]
    })));
    let ops = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap();
    assert_eq!(ops.len(), 1);
    let s = pretty(ops[0].op_expr.clone());
    assert!(s.contains("ConnectOrCreate"), "got: {s}");
    assert!(s.contains("where_filter"), "got: {s}");
    assert!(s.contains("create_payload"), "got: {s}");
}
```

- [ ] **Step 5: `cargo test -p prax-codegen data_relation`**

- [ ] **Step 6: Commit**

```
feat(codegen): lower connect_or_create inside data: blocks
```

---

## Task 4: trybuild fixture cleanup

**Files:**
- Delete: `tests/ui/nested_writes/nested_connect_or_create_phase_5d.rs` + `.stderr`

- [ ] **Step 1: Delete the fixture**

```bash
git rm tests/ui/nested_writes/nested_connect_or_create_phase_5d.rs \
       tests/ui/nested_writes/nested_connect_or_create_phase_5d.stderr
```

- [ ] **Step 2: Verify trybuild still passes** — remaining fixtures (`nested_scalar_op_on_relation`, `nested_set_phase_5e`) should still cover the only-deferred operator (`set:`).

`cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`

- [ ] **Step 3: Commit**

```
test(codegen): drop obsolete connect_or_create phase-5d fixture
```

---

## Task 5: E2E tests

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Add two e2e tests**

Mirror phase 5c-mutations' Upsert e2e pattern (extends `RecordingEngine` with affected-override). Direct NestedWriteOp construction via `.with(nw)`:

- `nested_connect_or_create_connect_path` — affected-override `[1]` (parent insert) + `[1]` (UPDATE matches) → assert two statements (parent INSERT + UPDATE), no INSERT for the child
- `nested_connect_or_create_create_path` — affected-override `[1]` (parent insert) + `[0, 1]` (UPDATE no match, then INSERT) → assert three statements (parent INSERT + UPDATE + child INSERT) and that the INSERT params include the parent's PK

- [ ] **Step 2: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 3: Commit**

```
test(query): e2e for nested connect_or_create
```

---

## Task 6: CHANGELOG + final verification sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG bullet**

```
### Added
- **Nested `connect_or_create` inside `create!`'s `data:` (phase 5d).**
  New `NestedWriteOp::ConnectOrCreate` variant with two-statement
  engine-agnostic executor: `UPDATE child SET fk WHERE <filter>`
  (connect path); if zero affected rows, `INSERT INTO child (... + fk)
  VALUES (...)` (create path). Behaves correctly even when the where
  matches multiple rows — every match gets its FK pointed at the
  parent. Single-statement vendor-specific upsert (Postgres
  `ON CONFLICT`, MySQL `ON DUPLICATE KEY`, MSSQL `MERGE`) remains a
  separate optimization phase.

### Changed
- Inside `create!`'s `data:` block, only `set:` (phase 5e) remains a
  deferred nested operator. The unknown-operator did-you-mean
  candidate list grows to include `connect_or_create`.
- The `nested_connect_or_create_phase_5d` trybuild fixture is
  removed — the operator it tested now ships.
```

- [ ] **Step 2: Full sweep**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast
```

All three must be green.

- [ ] **Step 3: Commit**

```
docs(codegen): CHANGELOG for phase-5d connect_or_create
```

---

## Task 7: Push + open PR

- [ ] **Step 1: `git push -u origin feature/connect-or-create`**

- [ ] **Step 2: Open PR** — title `feat(codegen): nested connect_or_create (phase 5d)`

- [ ] **Step 3: Wait for CI.**

---

## Out of scope (deferred)

- `set: [...]` full-relation diff replacement — phase 5e
- Single-statement vendor-specific upsert/connect_or_create (Postgres `ON CONFLICT`, etc.) — separate optimization phase; would also re-apply to phase-5c-mutations' Upsert
- Nested writes inside `update!` / `upsert!` macros — needs `.with(nw)` infrastructure on those operations
- Aggregate macros — phase 6
- Computed/virtual fields — phase 5.5
