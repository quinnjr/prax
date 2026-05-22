# Nested `set:` Full-Relation Replacement (Phase 5e) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the last remaining nested-write operator inside `create!`'s `data:` block:

```rust
let user = prax::create!(client.user, {
    data: {
        email: "alice@x.com",
        posts: {
            set: [{ id: 1 }, { id: 2 }, { id: 7 }],
        },
    },
}).exec().await?;
```

Semantics: after the operation, **exactly the listed child rows** are connected to the parent. Any child currently connected to the parent that isn't in the list gets disconnected (FK cleared to NULL). Any child in the list that isn't currently connected gets connected (FK pointed at the parent). Pre-existing FK values on the listed children are overwritten — `set:` claims the rows for this parent regardless of who owned them before.

After this lands, **every nested-write operator inside `create!`'s `data:` is shipped**.

**Architecture:**

Two-statement engine-agnostic executor — no SELECT needed:

```
-- Disconnect: clear FK on any current child not in the set
UPDATE <child> SET <fk> = NULL WHERE <fk> = $parent AND <pk> NOT IN (set_pks)

-- Connect: point FK at the parent on every set member (idempotent for already-connected rows)
UPDATE <child> SET <fk> = $parent WHERE <pk> IN (set_pks)
```

Edge cases:
- **Empty set `set: []`** → disconnect ALL current children. The `NOT IN ()` clause is invalid SQL; special-case to `UPDATE <child> SET <fk> = NULL WHERE <fk> = $parent` (no NOT IN clause), then skip the connect statement.
- **Single-element set** → `IN ($pk)` is valid; no special case needed.
- **Already-correct relation state**: both UPDATEs run anyway. The second is idempotent (already-connected rows get their FK re-set to the same value). The first may touch zero rows. Cost is one wasted UPDATE; correctness preserved.

```rust
pub enum NestedWriteOp {
    /* existing variants */
    Set {
        relation: &'static str,
        target_table: &'static str,
        foreign_key: &'static str,
        target_pk: &'static str,
        set_pks: Vec<FilterValue>,
    },
}
```

---

## File Structure

### Modified files

- `prax-query/src/nested.rs` — new variant + executor + unit tests
- `prax-codegen/src/macros/lower/data_relation.rs` — replace the `set:` phase-5e deferral arm with real lowering; remove `phase_5e_deferral` helper if no remaining callers
- `tests/nested_writes_e2e.rs` — e2e coverage
- `tests/ui/nested_writes/nested_set_phase_5e.{rs,stderr}` — delete (operator now ships)
- `CHANGELOG.md`

### Deleted

- `tests/ui/nested_writes/nested_set_phase_5e.rs` + `.stderr`

---

## Task 1: Verify baseline

- [ ] **Step 1**: `git -C /home/joseph/Projects/prax/.worktrees/set-replacement rev-parse --abbrev-ref HEAD` → `feature/set-replacement`
- [ ] **Step 2**: `git log --oneline -1` starts with `77475d0 feat(codegen): nested connect_or_create (phase 5d)`
- [ ] **Step 3**: `cargo check --workspace --all-features` — zero errors
- [ ] **Step 4**: `cargo test -p prax-query --lib && cargo test -p prax-codegen --lib` — green
- [ ] **Step 5**: No commit.

---

## Task 2: `NestedWriteOp::Set` variant + two-statement executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Add the variant** (after the `ConnectOrCreate` variant from phase 5d):

```rust
/// Replace the relation contents — after execution, exactly the
/// listed child rows are connected to the parent. Rows currently
/// connected that aren't in `set_pks` get their FK cleared; rows in
/// `set_pks` that aren't currently connected (or are connected to a
/// different parent) get their FK pointed at this parent.
///
/// Two-statement engine-agnostic lowering:
/// 1. `UPDATE <target_table> SET <foreign_key> = NULL WHERE <foreign_key> = $parent AND <target_pk> NOT IN (set_pks)`
/// 2. `UPDATE <target_table> SET <foreign_key> = $parent WHERE <target_pk> IN (set_pks)`
///
/// When `set_pks` is empty, step 1's `NOT IN ()` is invalid SQL —
/// the executor special-cases this to `UPDATE <child> SET <fk> = NULL
/// WHERE <fk> = $parent` (no NOT IN clause), then skips step 2.
///
/// `set:` claims rows for this parent regardless of who they belonged
/// to before — pre-existing FK values get overwritten. This matches
/// Prisma's relation-replacement semantics.
Set {
    relation: &'static str,
    target_table: &'static str,
    foreign_key: &'static str,
    target_pk: &'static str,
    set_pks: Vec<FilterValue>,
},
```

- [ ] **Step 2: Implement the executor arm**

```rust
NestedWriteOp::Set {
    relation: _,
    target_table,
    foreign_key,
    target_pk,
    set_pks,
} => {
    let dialect = engine.dialect();

    // Phase 1: disconnect current children not in set_pks.
    if set_pks.is_empty() {
        // No NOT IN clause needed — clear every child of this parent.
        let sql = format!(
            "UPDATE {} SET {} = NULL WHERE {} = {}",
            dialect.quote_ident(target_table),
            dialect.quote_ident(foreign_key),
            dialect.quote_ident(foreign_key),
            dialect.placeholder(1),
        );
        engine.execute_raw(&sql, vec![parent_pk.clone()]).await?;
        return Ok(());
    }
    // set_pks is non-empty — emit disconnect with NOT IN clause + connect.
    let mut disconnect_params: Vec<FilterValue> =
        Vec::with_capacity(set_pks.len() + 1);
    disconnect_params.push(parent_pk.clone());
    let mut not_in_placeholders: Vec<String> = Vec::with_capacity(set_pks.len());
    for (i, pk) in set_pks.iter().enumerate() {
        disconnect_params.push(pk.clone());
        not_in_placeholders.push(dialect.placeholder(i + 2));
    }
    let disconnect_sql = format!(
        "UPDATE {} SET {} = NULL WHERE {} = {} AND {} NOT IN ({})",
        dialect.quote_ident(target_table),
        dialect.quote_ident(foreign_key),
        dialect.quote_ident(foreign_key),
        dialect.placeholder(1),
        dialect.quote_ident(target_pk),
        not_in_placeholders.join(", "),
    );
    engine.execute_raw(&disconnect_sql, disconnect_params).await?;

    // Phase 2: connect every row in set_pks (idempotent for already-connected).
    let mut connect_params: Vec<FilterValue> = Vec::with_capacity(set_pks.len() + 1);
    connect_params.push(parent_pk.clone());
    let mut in_placeholders: Vec<String> = Vec::with_capacity(set_pks.len());
    for (i, pk) in set_pks.iter().enumerate() {
        connect_params.push(pk.clone());
        in_placeholders.push(dialect.placeholder(i + 2));
    }
    let connect_sql = format!(
        "UPDATE {} SET {} = {} WHERE {} IN ({})",
        dialect.quote_ident(target_table),
        dialect.quote_ident(foreign_key),
        dialect.placeholder(1),
        dialect.quote_ident(target_pk),
        in_placeholders.join(", "),
    );
    engine.execute_raw(&connect_sql, connect_params).await?;
    Ok(())
}
```

- [ ] **Step 3: Unit tests**

- `nested_op_set_with_empty_list_clears_all_children` — `set_pks: vec![]` → one UPDATE with `WHERE fk = $1`, no NOT IN clause, no connect step
- `nested_op_set_with_non_empty_list_emits_disconnect_then_connect` — two UPDATEs; first has NOT IN, second has IN; both reference same target table + FK
- `nested_op_set_with_single_element_uses_single_placeholder_in_lists` — `set_pks: vec![FilterValue::Int(5)]` → IN ($2) and NOT IN ($2)
- Optional: `nested_op_set_disconnect_clears_only_current_parents_children` — verify the disconnect's `WHERE fk = $1 AND ...` clause is present (safety check that we don't accidentally clear children of other parents)

- [ ] **Step 4: `cargo test -p prax-query --lib nested`**

- [ ] **Step 5: Commit**

```
feat(query): NestedWriteOp::Set variant + two-statement executor
```

---

## Task 3: DSL lowering for `set:`

**Files:**
- Modify: `prax-codegen/src/macros/lower/data_relation.rs`

- [ ] **Step 1: Replace the `"set" => phase_5e_deferral(...)` arm** with real lowering:

```rust
"set" => {
    let children = expect_list_of_blocks(value, &op_key, key.span())?;
    let mut pk_exprs: Vec<TokenStream> = Vec::with_capacity(children.len());
    for child_block in children {
        let pk_expr = lower_connect_pk(child_block, target_model, &target_pk_column)?;
        pk_exprs.push(quote! {
            ::core::convert::Into::<::prax_query::filter::FilterValue>::into(#pk_expr)
        });
    }
    let op_expr = quote! {
        ::prax_query::nested::NestedWriteOp::Set {
            relation: #relation_name_str,
            target_table: #target_table,
            foreign_key: #foreign_key,
            target_pk: #target_pk_column,
            set_pks: ::std::vec![ #( #pk_exprs ),* ],
        }
    };
    ops.push(NestedRelationOp { op_expr });
}
```

`set:` takes a list of `{ id: N }` blocks — same shape as `connect:` from phase 5b. Reuse `lower_connect_pk` for each entry. Empty list is allowed (means "disconnect everything").

- [ ] **Step 2: Remove `phase_5e_deferral` helper** if no other callers remain. (It was only used by the `set:` arm.)

- [ ] **Step 3: Update unknown-operator candidates list** to include `set`.

- [ ] **Step 4: Replace the `set_op_inside_relation_block_is_phase_5e_deferral` test**

The existing test asserts a phase-5e deferral. Replace with a positive lowering test mirroring phase-5d's pattern:

```rust
#[test]
fn lowers_nested_set_to_nested_write_op() {
    let schema = parsed_schema();
    let user = schema.get_model("User").unwrap().clone();
    let ctx = LowerCtx::new(&schema, &user);
    let field = user.get_field("posts").unwrap();
    let value = DslValue::Block(parse_block(quote!({
        set: [{ id: 1 }, { id: 2 }]
    })));
    let ops = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap();
    assert_eq!(ops.len(), 1);
    let s = pretty(ops[0].op_expr.clone());
    assert!(s.contains("NestedWriteOp :: Set"), "got: {s}");
    assert!(s.contains("set_pks"), "got: {s}");
}
```

Add an empty-list test:

```rust
#[test]
fn lowers_nested_set_with_empty_list() {
    let schema = parsed_schema();
    let user = schema.get_model("User").unwrap().clone();
    let ctx = LowerCtx::new(&schema, &user);
    let field = user.get_field("posts").unwrap();
    let value = DslValue::Block(parse_block(quote!({
        set: []
    })));
    let ops = lower_create_relation(field, &value, Span::call_site(), &ctx).unwrap();
    assert_eq!(ops.len(), 1);
    let s = pretty(ops[0].op_expr.clone());
    assert!(s.contains("NestedWriteOp :: Set"), "got: {s}");
    // Empty vec — the macro emits `::std::vec![]`
}
```

- [ ] **Step 5: `cargo test -p prax-codegen data_relation`**

- [ ] **Step 6: Commit**

```
feat(codegen): lower set inside data: blocks
```

---

## Task 4: trybuild fixture cleanup

- [ ] **Step 1**: `git rm tests/ui/nested_writes/nested_set_phase_5e.rs tests/ui/nested_writes/nested_set_phase_5e.stderr`

- [ ] **Step 2**: `cargo test -p prax-orm --features ui-tests --test trybuild_read_macros` — the remaining `nested_scalar_op_on_relation` fixture covers the only-still-relevant negative path.

- [ ] **Step 3: Commit**

```
test(codegen): drop obsolete set phase-5e fixture
```

---

## Task 5: E2E tests

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Add three e2e tests**

- `nested_set_empty_list_emits_disconnect_all` — `Set { set_pks: vec![] }` → assert parent INSERT + single UPDATE with `WHERE fk = $1`, no NOT IN clause
- `nested_set_with_pks_emits_disconnect_then_connect` — `Set { set_pks: vec![Int(1), Int(2), Int(3)] }` → assert three statements (parent INSERT + disconnect UPDATE + connect UPDATE), confirm both contain the expected SQL shapes
- `nested_set_combined_with_other_ops` — combine `Set` with a `Create` in the same transaction, verify order

- [ ] **Step 2: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 3: Commit**

```
test(query): e2e for nested set relation replacement
```

---

## Task 6: CHANGELOG + final sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG bullet**

```
### Added
- **Nested `set:` full-relation replacement inside `create!`'s `data:`
  (phase 5e).** New `NestedWriteOp::Set` variant with two-statement
  engine-agnostic executor: disconnect (`UPDATE child SET fk = NULL
  WHERE fk = $parent AND pk NOT IN (...)`) followed by connect
  (`UPDATE child SET fk = $parent WHERE pk IN (...)`). Empty
  `set: []` clears every current child without the invalid `NOT IN ()`
  clause. Pre-existing FK values are overwritten — `set:` claims rows
  regardless of prior ownership.

### Changed
- **The nested-write operator surface inside `create!`'s `data:` is
  now complete.** Every Prisma-style operator ships: `create`,
  `connect`, `disconnect`, `delete`, `delete_many`, `update`,
  `update_many`, `upsert`, `connect_or_create`, `set`. Single-statement
  vendor-specific upsert/connect_or_create (Postgres `ON CONFLICT`,
  MySQL `ON DUPLICATE KEY`, MSSQL `MERGE`) remains a separate
  optimization phase; current behavior uses engine-agnostic
  two-statement paths everywhere.
- The unknown-operator did-you-mean candidate list grows to include
  `set`. The `nested_set_phase_5e` trybuild fixture is removed — the
  operator it tested now ships.
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
docs(codegen): CHANGELOG for phase-5e set relation replacement
```

---

## Task 7: Push + open PR

- [ ] **Step 1**: `git push -u origin feature/set-replacement`
- [ ] **Step 2**: `gh pr create --base develop --head feature/set-replacement --title "feat(codegen): nested set relation replacement (phase 5e)"`
- [ ] **Step 3**: Wait for CI.

---

## Out of scope (still deferred)

- Single-statement vendor-specific upsert/connect_or_create (Postgres `ON CONFLICT`, etc.) — separate optimization phase
- Nested writes inside `update!` / `upsert!` macros — needs `.with(nw)` infrastructure on those operations
- Aggregate macros — phase 6
- Computed/virtual fields — phase 5.5
