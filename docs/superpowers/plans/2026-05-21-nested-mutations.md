# Nested `update` / `update_many` / `upsert` (Phase 5c-mutations) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the nested-write surface in `create!`'s `data:` block with the three mutation operators left after phase 5c:

```rust
let user = prax::create!(client.user, {
    data: {
        email: "alice@x.com",
        posts: {
            update: [
                { where: { id: 1 }, data: { title: "renamed", views: { increment: 1 } } },
            ],
            update_many: { where: { published: false }, data: { views: { set: 0 } } },
            upsert: [
                { where: { id: 99 }, create: { title: "new" }, update: { views: { increment: 1 } } },
            ],
        },
    },
}).exec().await?;
```

**Out of scope** (deferred):
- `set: [...]` full-relation diff replacement — phase 5e
- `connect_or_create` engine-specific lowering — phase 5d (will also unlock single-statement upsert)
- Nested writes inside `update!` / `upsert!` macros — needs `.with(nw)` infrastructure on those operations; separate phase

**Architecture:**

Three new `NestedWriteOp` variants. Update and UpdateMany are straightforward SQL emissions. Upsert uses a **two-statement** approach (UPDATE first, INSERT if zero affected_rows) that's engine-agnostic; single-statement upserts via vendor-specific syntax (`ON CONFLICT`, `ON DUPLICATE KEY UPDATE`, `MERGE`) come in phase 5d alongside `connect_or_create`.

```rust
pub enum NestedWriteOp {
    Create { /* ... */ },     // phase 5b
    Connect { /* ... */ },    // phase 5b
    Disconnect { /* ... */ }, // phase 5c
    Delete { /* ... */ },     // phase 5c
    DeleteMany { /* ... */ }, // phase 5c

    Update {
        relation: &'static str,
        target_table: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
        payload: Vec<(String, WriteOp)>,
    },
    UpdateMany {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        filter: Filter,
        payload: Vec<(String, WriteOp)>,
    },
    Upsert {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        target_pk: &'static str,
        pk: FilterValue,
        create_payload: Vec<(String, FilterValue)>,
        update_payload: Vec<(String, WriteOp)>,
    },
}
```

SQL shapes:
- **Update**: `UPDATE child SET <writeop fragments> WHERE pk = $1`
- **UpdateMany**: `UPDATE child SET <writeop fragments> WHERE fk = $1 AND <filter>`
- **Upsert** (two-statement): `UPDATE child SET <writeop fragments> WHERE pk = $1` — check affected_rows; if zero, emit `INSERT INTO child (cols + fk) VALUES (...) ` with parent_pk for the FK column

Reuse the existing `WriteOp::to_sql_fragment(column, placeholder)` helper for the SET clause emission. Reuse `lower_create_data` / `lower_update_data` (from phase 5a) for the payload extraction at codegen time.

---

## File Structure

### New files

- `tests/ui/nested_writes/nested_update_mutations.rs` + `.stderr` — new fixture for the (now narrower) phase-5c-mutations deferral (since `update` is no longer deferred, the prior fixture file needs to be replaced or repurposed — see Task 6)

### Modified files

- `prax-query/src/nested.rs` — three new variants + executors + unit tests
- `prax-codegen/src/macros/lower/data_relation.rs` — three new operator arms; remove the `update | update_many | upsert` deferral arm entirely (only `connect_or_create` and `set` remain deferred at this point)
- `tests/nested_writes_e2e.rs` — e2e cases for each new operator
- `tests/ui/nested_writes/nested_unknown_op_phase_5c.rs` (existing) — file is now obsolete since all operators it referenced ship in phase 5c-mutations; delete it OR repurpose it (Task 6)
- `CHANGELOG.md`

### Deleted

- Possibly: `tests/ui/nested_writes/nested_unknown_op_phase_5c.rs` + `.stderr` (the deferred operator it tested now ships)

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/nested-mutations rev-parse --abbrev-ref HEAD`
Expected: `feature/nested-mutations`.

- [ ] **Step 2: Confirm base**

Run: `git -C /home/joseph/Projects/prax/.worktrees/nested-mutations log --oneline -1`
Expected: starts with `350d943 feat(codegen): nested disconnect/delete/delete_many inside data: (phase 5c)`.

- [ ] **Step 3: `cargo check --workspace --all-features`** — zero errors.

- [ ] **Step 4: Existing tests pass** — `cargo test -p prax-query --lib && cargo test -p prax-codegen --lib`.

- [ ] **Step 5: No commit.**

---

## Task 2: `NestedWriteOp::Update` variant + executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant**

Inside `pub enum NestedWriteOp` after the existing `DeleteMany` variant:

```rust
/// Update a child row by its primary key.
///
/// Lowers to `UPDATE <target_table> SET <writeop-fragments> WHERE <target_pk> = $1`.
/// Each entry in `payload` contributes one column assignment whose
/// shape is determined by the [`WriteOp`] variant (plain set, atomic
/// increment/decrement/multiply/divide, or null-out via Unset).
Update {
    relation: &'static str,
    target_table: &'static str,
    target_pk: &'static str,
    pk: FilterValue,
    payload: Vec<(String, crate::inputs::WriteOp)>,
},
```

Confirm the `WriteOp` path resolves — read `prax-query/src/inputs/write_payload.rs` or wherever the type lives.

- [ ] **Step 2: Implement the executor arm**

In the `match self` block:

```rust
NestedWriteOp::Update {
    relation: _,
    target_table,
    target_pk,
    pk,
    payload,
} => {
    if payload.is_empty() {
        return Ok(());
    }
    let dialect = engine.dialect();
    let mut set_fragments: Vec<String> = Vec::with_capacity(payload.len());
    let mut params: Vec<FilterValue> = Vec::with_capacity(payload.len() + 1);
    let mut next_placeholder = 1usize;
    for (col, op) in payload {
        let (frag, maybe_val) = op.to_sql_fragment(&dialect.quote_ident(&col), &dialect.placeholder(next_placeholder));
        set_fragments.push(frag);
        if let Some(val) = maybe_val {
            params.push(val);
            next_placeholder += 1;
        }
    }
    params.push(pk);
    let sql = format!(
        "UPDATE {} SET {} WHERE {} = {}",
        dialect.quote_ident(target_table),
        set_fragments.join(", "),
        dialect.quote_ident(target_pk),
        dialect.placeholder(next_placeholder),
    );
    let affected = engine.execute_raw(&sql, params).await?;
    if affected != 1 {
        return Err(crate::error::QueryError::not_found(target_table)
            .with_context("Nested Update by PK"));
    }
    Ok(())
}
```

**Verify `WriteOp::to_sql_fragment`'s exact signature first** — the plan's call site is best-guess. Read the actual function and adapt.

- [ ] **Step 3: Unit tests**

- `nested_op_update_plain_set` — single `WriteOp::Set("renamed".into())` → assert SQL is `UPDATE ... SET title = $1 WHERE id = $2`
- `nested_op_update_increment` — `WriteOp::Increment(1.into())` → assert SQL contains `views = views + $1`
- `nested_op_update_mixed_set_and_increment` — both operators in one update
- `nested_op_update_empty_payload_is_noop` — empty `payload` returns Ok without emitting SQL

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::Update variant + executor
```

---

## Task 3: `NestedWriteOp::UpdateMany` variant + executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant** — same shape as Update but FK-bound + filter instead of PK lookup:

```rust
/// Update many child rows matching a filter, scoped to the parent's
/// children only.
///
/// Lowers to `UPDATE <target_table> SET <writeop-fragments> WHERE <foreign_key> = $1 AND <filter>`.
/// The AND-with-parent-FK clause is a safety bound enforced at SQL
/// emit time — the user filter cannot reach rows belonging to other
/// parents.
UpdateMany {
    relation: &'static str,
    target_table: &'static str,
    foreign_key: &'static str,
    filter: Filter,
    payload: Vec<(String, crate::inputs::WriteOp)>,
},
```

- [ ] **Step 2: Implement the executor**

```rust
NestedWriteOp::UpdateMany {
    relation: _,
    target_table,
    foreign_key,
    filter,
    payload,
} => {
    if payload.is_empty() {
        return Ok(());
    }
    let dialect = engine.dialect();
    let mut set_fragments: Vec<String> = Vec::with_capacity(payload.len());
    let mut params: Vec<FilterValue> = Vec::with_capacity(payload.len() + 1);
    let mut next_placeholder = 1usize;
    for (col, op) in payload {
        let (frag, maybe_val) = op.to_sql_fragment(&dialect.quote_ident(&col), &dialect.placeholder(next_placeholder));
        set_fragments.push(frag);
        if let Some(val) = maybe_val {
            params.push(val);
            next_placeholder += 1;
        }
    }
    let fk_placeholder = dialect.placeholder(next_placeholder);
    next_placeholder += 1;
    params.push(parent_pk.clone());

    let is_unconstrained = matches!(filter, Filter::None);
    let sql = if is_unconstrained {
        format!(
            "UPDATE {} SET {} WHERE {} = {}",
            dialect.quote_ident(target_table),
            set_fragments.join(", "),
            dialect.quote_ident(foreign_key),
            fk_placeholder,
        )
    } else {
        let (filter_sql, filter_params) =
            filter.to_sql(next_placeholder, &crate::dialect::Postgres);
        params.extend(filter_params);
        format!(
            "UPDATE {} SET {} WHERE {} = {} AND ({})",
            dialect.quote_ident(target_table),
            set_fragments.join(", "),
            dialect.quote_ident(foreign_key),
            fk_placeholder,
            filter_sql,
        )
    };
    engine.execute_raw(&sql, params).await?;
    Ok(())
}
```

- [ ] **Step 3: Unit tests**

- `nested_op_update_many_with_filter` — `Filter::Equals("published", false)` + `WriteOp::Set("views" = 0)`
- `nested_op_update_many_with_empty_filter` — `Filter::None` → AND clause omitted

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::UpdateMany variant + executor
```

---

## Task 4: `NestedWriteOp::Upsert` variant + executor (two-statement)

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant**

```rust
/// Upsert: update if a row matches `pk`, else insert.
///
/// The two-statement engine-agnostic lowering:
/// 1. `UPDATE <target_table> SET <update_writeops> WHERE <target_pk> = $1`
/// 2. If `affected_rows == 0`, emit
///    `INSERT INTO <target_table> (<create_cols + foreign_key>) VALUES (<...>)`
///    with the parent PK spliced in for the FK column.
///
/// Single-statement upsert via vendor-specific syntax (Postgres
/// `ON CONFLICT`, MySQL `ON DUPLICATE KEY UPDATE`, MSSQL `MERGE`) is
/// planned for phase 5d alongside `connect_or_create`.
Upsert {
    relation: &'static str,
    target_table: &'static str,
    foreign_key: &'static str,
    target_pk: &'static str,
    pk: FilterValue,
    create_payload: Vec<(String, FilterValue)>,
    update_payload: Vec<(String, crate::inputs::WriteOp)>,
},
```

- [ ] **Step 2: Implement the executor**

```rust
NestedWriteOp::Upsert {
    relation: _,
    target_table,
    foreign_key,
    target_pk,
    pk,
    create_payload,
    update_payload,
} => {
    let dialect = engine.dialect();
    // Phase 1: attempt UPDATE.
    let mut set_fragments: Vec<String> = Vec::with_capacity(update_payload.len());
    let mut update_params: Vec<FilterValue> = Vec::with_capacity(update_payload.len() + 1);
    let mut next_placeholder = 1usize;
    for (col, op) in update_payload {
        let (frag, maybe_val) = op.to_sql_fragment(&dialect.quote_ident(&col), &dialect.placeholder(next_placeholder));
        set_fragments.push(frag);
        if let Some(val) = maybe_val {
            update_params.push(val);
            next_placeholder += 1;
        }
    }
    update_params.push(pk.clone());
    let update_sql = format!(
        "UPDATE {} SET {} WHERE {} = {}",
        dialect.quote_ident(target_table),
        set_fragments.join(", "),
        dialect.quote_ident(target_pk),
        dialect.placeholder(next_placeholder),
    );
    let affected = engine.execute_raw(&update_sql, update_params).await?;
    if affected > 0 {
        return Ok(());
    }
    // Phase 2: row didn't exist — INSERT it with the FK spliced in.
    if create_payload.is_empty() {
        // Defensive: an upsert with empty create_payload and zero
        // affected_rows means we have nothing to insert. Surface as
        // not_found so the caller doesn't silently no-op.
        return Err(crate::error::QueryError::not_found(target_table)
            .with_context("Nested Upsert: row absent and create payload empty"));
    }
    let mut columns: Vec<String> = create_payload.iter().map(|(c, _)| c.clone()).collect();
    let mut values: Vec<FilterValue> = create_payload.into_iter().map(|(_, v)| v).collect();
    columns.push(foreign_key.to_string());
    values.push(parent_pk.clone());
    let placeholders: Vec<String> = (1..=values.len()).map(|i| dialect.placeholder(i)).collect();
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

- [ ] **Step 3: Unit tests**

- `nested_op_upsert_update_path_when_affected` — Mock engine returns 1 from execute_raw; assert only the UPDATE statement was emitted (not the INSERT)
- `nested_op_upsert_insert_path_when_zero_affected` — Mock engine returns 0 from execute_raw on the UPDATE; assert both UPDATE and INSERT statements were emitted

**Note**: the default RecordingEngine in `nested.rs::tests` returns `Ok(1)` unconditionally. To test the insert path, build a recording engine that returns 0 for the UPDATE (e.g., a mode flag or sequence). Either extend the existing mock or write a one-off inline mock.

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::Upsert variant + two-statement executor
```

---

## Task 5: DSL lowering for `update` / `update_many` / `upsert`

**Files:**
- Modify: `prax-codegen/src/macros/lower/data_relation.rs`

- [ ] **Step 1: Add `update` arm**

```rust
"update" => {
    let children = expect_list_of_blocks(value, &op_key, key.span())?;
    let target_ctx = ctx.for_model(target_model);
    for child_block in children {
        // Each entry is `{ where: { id: N }, data: { ... } }`.
        let (pk_expr, data_expr) =
            extract_update_entry(child_block, target_model, &target_pk_column, &target_ctx)?;
        let op_expr = quote! {
            ::prax_query::nested::NestedWriteOp::Update {
                relation: #relation_name_str,
                target_table: #target_table,
                target_pk: #target_pk_column,
                pk: ::core::convert::Into::<::prax_query::filter::FilterValue>::into(#pk_expr),
                payload: <_ as ::prax_query::inputs::UpdateInput>::into_ir(#data_expr),
            }
        };
        ops.push(NestedRelationOp { op_expr });
    }
}
```

The `extract_update_entry` helper:
- Expects the child_block to have exactly two keys: `where` and `data`
- Validates `where` is a single-PK block (reuse `lower_connect_pk`'s shape)
- Lowers `data` via `super::data_input::lower_update_data` against the target's `LowerCtx`
- Returns `(pk_expr_tokenstream, update_input_tokenstream)`

Write it as a private helper in `data_relation.rs`.

- [ ] **Step 2: Add `update_many` arm**

```rust
"update_many" => {
    let DslValue::Block(entry_block) = value else {
        return Err(syn::Error::new(
            key.span(),
            "`update_many:` inside a relation expects `{ where: { ... }, data: { ... } }`",
        ));
    };
    let target_ctx = ctx.for_model(target_model);
    let (where_expr, data_expr) =
        extract_update_many_entry(entry_block, &target_ctx)?;
    let op_expr = quote! {
        ::prax_query::nested::NestedWriteOp::UpdateMany {
            relation: #relation_name_str,
            target_table: #target_table,
            foreign_key: #foreign_key,
            filter: <_ as ::prax_query::inputs::WhereInput>::into_ir(#where_expr),
            payload: <_ as ::prax_query::inputs::UpdateInput>::into_ir(#data_expr),
        }
    };
    ops.push(NestedRelationOp { op_expr });
}
```

The `extract_update_many_entry` helper:
- Expects exactly two keys: `where` (a filter block, possibly empty) and `data`
- Lowers `where` via `super::where_input::lower_where`
- Lowers `data` via `super::data_input::lower_update_data`

- [ ] **Step 3: Add `upsert` arm**

```rust
"upsert" => {
    let children = expect_list_of_blocks(value, &op_key, key.span())?;
    let target_ctx = ctx.for_model(target_model);
    for child_block in children {
        // Each entry is `{ where: { id: N }, create: { ... }, update: { ... } }`.
        let (pk_expr, create_expr, update_expr) =
            extract_upsert_entry(child_block, target_model, &target_pk_column, &target_ctx)?;
        let op_expr = quote! {
            ::prax_query::nested::NestedWriteOp::Upsert {
                relation: #relation_name_str,
                target_table: #target_table,
                foreign_key: #foreign_key,
                target_pk: #target_pk_column,
                pk: ::core::convert::Into::<::prax_query::filter::FilterValue>::into(#pk_expr),
                create_payload: <_ as ::prax_query::inputs::CreateInput>::into_ir(#create_expr),
                update_payload: <_ as ::prax_query::inputs::UpdateInput>::into_ir(#update_expr),
            }
        };
        ops.push(NestedRelationOp { op_expr });
    }
}
```

The `extract_upsert_entry` helper:
- Expects exactly three keys: `where`, `create`, `update`
- `where`: lowers via `lower_connect_pk` (single PK on target_pk column)
- `create`: lowers via `lower_create_data` (CreateInput for child, FK column stripped — note: the child's CreateInput should NOT include the FK column. The codegen's `<Child>CreateWithout<Parent>Input` types from phase 5b would be the right target here, but those weren't generated in phase 5b — we inlined nested ops. For phase 5c-mutations, just lower as `<Child>CreateInput` and trust the codegen to omit the FK column from the user-facing input. Verify by inspecting what `lower_create_data` produces for the test schema's Post model.)
- `update`: lowers via `lower_update_data`

- [ ] **Step 4: Remove the now-empty mutations deferral arm**

Find the `"update" | "update_many" | "upsert" => phase_5c_mutations_deferral(...)` arm and delete it. Also delete the `phase_5c_mutations_deferral` helper function (no callers left). The `phase_5e_deferral` (for `set`) and the `connect_or_create` arm stay.

Update the unknown-operator candidates list to include `update`, `update_many`, `upsert`.

- [ ] **Step 5: Snapshot tests**

In `data_relation.rs::tests`:
- `lowers_nested_update_to_nested_write_op_update` — `update: [{ where: { id: 1 }, data: { title: "x" } }]` → asserts a `NestedWriteOp::Update {` token in the output
- `lowers_nested_update_many_to_nested_write_op_update_many` — `update_many: { where: {...}, data: {...} }`
- `lowers_nested_upsert_to_nested_write_op_upsert` — single-entry upsert

- [ ] **Step 6: Remove the now-stale `update_op_inside_relation_block_is_phase_5c_deferral` test** — `update:` no longer triggers a deferral; the test invariant is gone.

- [ ] **Step 7: `cargo test -p prax-codegen data_relation`**

- [ ] **Step 8: Commit**

```
feat(codegen): lower update/update_many/upsert inside data: blocks
```

---

## Task 6: trybuild fixture cleanup

**Files:**
- Delete: `tests/ui/nested_writes/nested_unknown_op_phase_5c.rs` + `.stderr` (the fixture used `update:` which now ships)
- Repurpose or delete: any other stale fixture

- [ ] **Step 1: Delete the obsolete fixture**

```bash
git rm tests/ui/nested_writes/nested_unknown_op_phase_5c.rs tests/ui/nested_writes/nested_unknown_op_phase_5c.stderr
```

- [ ] **Step 2: Verify trybuild still passes**

`cargo test -p prax-orm --features ui-tests --test trybuild_read_macros` — remaining fixtures (`nested_connect_or_create_phase_5d`, `nested_scalar_op_on_relation`, `nested_set_phase_5e`) should still cover the remaining deferred-operator surface.

- [ ] **Step 3: Commit**

```
test(codegen): drop obsolete phase-5c trybuild fixture
```

---

## Task 7: E2E tests

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Add e2e tests** mirroring phase 5c's pattern (direct NestedWriteOp construction via `.with(nw)`):

- `nested_update_emits_parent_insert_then_update` — single `NestedWriteOp::Update` with plain Set
- `nested_update_increment_emits_arithmetic_set_clause` — verifies `views = views + $1` SQL shape
- `nested_update_many_with_filter_emits_fk_and_filter` — UpdateMany scoped by FK + filter
- `nested_upsert_update_path` — extend the existing test mock so the UPDATE returns 1; asserts only one statement
- `nested_upsert_insert_path` — mock returns 0 from UPDATE; asserts both UPDATE and INSERT statements emitted with FK spliced into INSERT

**Note**: the e2e file's `RecordingEngine` currently returns 1 unconditionally for execute_raw. For the upsert insert-path test, either configure the mock with a sequence or use a second mock variant. Match whatever pattern the existing `nested_delete_emits_parent_insert_then_delete_where_pk` test uses for the 1-vs-other affected_rows behaviour.

- [ ] **Step 2: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 3: Commit**

```
test(query): e2e for nested update/update_many/upsert
```

---

## Task 8: CHANGELOG + final verification sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG bullet**

```
### Added
- **Nested `update` / `update_many` / `upsert` inside `create!`'s
  `data:` (phase 5c-mutations).** Three new `NestedWriteOp` variants
  with executors. Update + UpdateMany emit standard `UPDATE SET ...
  WHERE ...` with `WriteOp` fragments (`set`, `increment`, `decrement`,
  `multiply`, `divide`, `unset` are all supported). Upsert uses a
  two-statement engine-agnostic path: UPDATE first, INSERT (with FK
  spliced in) when affected_rows == 0. Single-statement upsert via
  vendor-specific syntax (Postgres `ON CONFLICT` etc.) ships alongside
  `connect_or_create` in phase 5d.

### Changed
- The phase-5c deferral arm for mutation operators is gone; `update`,
  `update_many`, `upsert` are now first-class. Only `set:` (phase 5e)
  and `connect_or_create` (phase 5d) remain deferred.
- The `nested_unknown_op_phase_5c` trybuild fixture is removed — the
  operator it tested now ships.
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
docs(codegen): CHANGELOG for phase-5c-mutations
```

---

## Task 9: Push + open PR

- [ ] **Step 1: `git push -u origin feature/nested-mutations`**

- [ ] **Step 2: Open PR against `develop`** — title `feat(codegen): nested update/update_many/upsert (phase 5c-mutations)`

- [ ] **Step 3: Wait for CI.**

---

## Out of scope (deferred)

- `set: [...]` full-relation diff replacement — phase 5e
- `connect_or_create` engine-specific lowering (and single-statement upsert via same machinery) — phase 5d
- Nested writes inside `update!` / `upsert!` macros — needs `.with(nw)` infrastructure on those operations; separate phase
- Aggregate macros — phase 6
- Computed/virtual fields — phase 5.5
