# Nested `create` + `connect` Inside `create!` (Phase 5b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Light up the most common nested-write pattern — inserting a parent row plus its child rows in a single `create!` call. After this phase ships:

```rust
let user = prax::create!(client.user, {
    data: {
        email: "alice@x.com",
        name: "Alice",
        posts: {
            create: [
                { title: "First post",  body: "Hi"     },
                { title: "Second post", body: "World"  },
            ],
            connect: [
                { id: 42 },
            ],
        },
    },
    select: { id: true, email: true },
}).exec().await?;
```

Phase 5b is deliberately narrower than the spec §6 surface — it ships **only** the `create` and `connect` operators inside `data:` blocks, only on the **create** path (not `update!` / `upsert!`). The other operators land in follow-up phases:

| Phase | Operators added |
|-------|-----------------|
| 5b (this PR)  | `create`, `connect` inside `data:` on `create!`               |
| 5c (later)    | `update`, `update_many`, `upsert`, `delete`, `delete_many`, `disconnect` inside `data:` on `update!` |
| 5d (later)    | `connect_or_create` (engine-specific lowerings)               |
| 5e (later)    | `set:` (diff-based full relation replacement)                 |

**Why this slice:** the spec's full phase 5 is 25+ tasks across 5-7 days. The codebase already has the runtime IR (`NestedWriteOp::Create`, `NestedWriteOperations`, `in_transaction()`) and the phase-5a DSL infrastructure. The narrowest valuable shipment is the Create+Connect pair on `create!` — it's the Prisma pattern users actually reach for first, it unblocks `NestedWriteOp::Connect` (currently returns `QueryError::internal` because of missing relation metadata), and the work is contained enough to fit one PR review.

**Architecture:**

The runtime side is mostly built. Phase 5b focuses on (a) unblocking the `Connect` executor, (b) emitting the per-relation nested-input types in codegen, (c) extending the phase-5a `data:` DSL lowering to recognize relation keys, and (d) wiring the `create!` macro to attach `NestedWriteOp` values to `CreateOperation` via the existing `.with(nw)` builder.

Specifically:

- **Runtime (`prax-query`)**:
  - Replace the `QueryError::internal("not implemented")` body of `NestedWriteOp::Connect`'s executor with an actual implementation: build `UPDATE <target_table> SET <foreign_key> = <parent_pk> WHERE <target_pk> = <pk>` and execute. Target-PK column name comes from the relation's `RelationMeta::Target::PRIMARY_KEY[0]`.
  - Extend `NestedWriteOp::Connect` to carry the metadata the executor needs: change from `{ relation: String, pk: FilterValue }` to `{ relation: String, target_table: String, foreign_key: String, target_pk: String, pk: FilterValue }`. Codegen of relation-helpers (Task 4-5) fills these from the per-relation type's `RelationMeta` consts.
- **Codegen (`prax-codegen`)**:
  - New `inputs/create_without.rs` generator emits `<RelatedModel>CreateWithout<Owner>Input` per relation back-pointer. This is a `<RelatedModel>CreateInput` minus the FK column that points back at the owner. (Existing `CreateInput` already includes all scalars; the `Without` variant omits the FK.)
  - New `inputs/relation_nested_create.rs` generator emits `<Model><Relation>CreateNestedInput` per relation, containing `create: Option<Vec<<RelatedModel>CreateWithout<Owner>Input>>` and `connect: Option<Vec<<RelatedModel>WhereUniqueInput>>`. (Two fields in phase 5b; later phases extend.)
  - Extend `inputs/create_input.rs` to include relation fields on `<Model>CreateInput`: `posts: Option<<Model>PostsCreateNestedInput>`. The existing `impl CreateInput` trait impl iterates over relation fields after scalars and emits `NestedWriteOp::Create` / `NestedWriteOp::Connect` values into the resulting payload's nested-op vec.
  - New `macros/lower/data_relation.rs` lowers a relation key inside `data:` to a `<Model><Relation>CreateNestedInput` struct literal. Inside the relation block, the parser recognizes the operator keys `create:` and `connect:` — anything else returns a "phase 5c+" deferral with a friendly diagnostic. Scalar children inside `create: [{ ... }]` lower via the existing `lower_create_data` recursively against the related model's `LowerCtx`.
- **Engine capability declarations**:
  - Each SQL engine + MongoDB: `impl SupportsNestedWrites for <Engine>` in its `capabilities.rs` (Task 11 — most or all engines may already have this; verify and add only what's missing).
- **DSL surface**:
  - The phase-5a `lower_create_data` already rejects relation keys with a "phase 5b" diagnostic. Phase 5b replaces that rejection with the new relation lowering. Phase 5c will replace the still-rejected operators (`update`/`upsert`/etc.) inside relation blocks with their own lowerings.

**Tech Stack:** Rust 2024, `proc_macro2`, `quote`, `syn 2.0`, plus the phase-3/4/5a macro pipeline. No new external dependencies.

---

## File Structure

### New files

- `prax-codegen/src/generators/inputs/create_without.rs` — emit `<RelatedModel>CreateWithout<Owner>Input` per relation back-pointer
- `prax-codegen/src/generators/inputs/relation_nested_create.rs` — emit `<Model><Relation>CreateNestedInput` per relation
- `prax-codegen/src/macros/lower/data_relation.rs` — lower a relation key inside `data:` to a nested-input struct literal
- `tests/ui/nested_writes/cql_capability_gap.rs` + `.stderr` — `create!` with nested `posts: { create: [...] }` against a CQL-backed accessor → does not compile
- `tests/ui/nested_writes/unknown_nested_op_phase_5c.rs` + `.stderr` — using `update:` inside a relation block on phase 5b's `create!` → friendly phase-5c deferral
- `tests/ui/nested_writes/scalar_op_on_relation.rs` + `.stderr` — using `{ equals: ... }` (a where-style operator) inside a relation block in `data:` → clear error
- `tests/nested_writes_e2e.rs` — workspace-level integration tests against the in-process engine used in earlier phases

### Modified files

- `prax-query/src/nested.rs` — extend `NestedWriteOp::Connect` variant fields; implement the executor body
- `prax-codegen/src/generators/inputs/create_input.rs` — emit relation fields on `<Model>CreateInput` and extend `impl CreateInput::into_ir` to accumulate nested ops
- `prax-codegen/src/generators/inputs/mod.rs` — register the two new generators
- `prax-codegen/src/macros/lower/data_input.rs` — replace the relation-key rejection with a call into the new `data_relation::lower_relation_value`
- `prax-codegen/src/macros/lower/mod.rs` — `pub mod data_relation;`
- `prax-codegen/src/macros/ops/create.rs` — accept nested ops from the lowered `data:` block and emit `.with(nw)` chained calls
- (Possibly) `prax-postgres/src/capabilities.rs`, `prax-mysql/src/capabilities.rs`, `prax-sqlite/src/capabilities.rs`, `prax-mssql/src/capabilities.rs`, `prax-duckdb/src/capabilities.rs`, `prax-mongodb/src/capabilities.rs` — add `impl SupportsNestedWrites` if absent. Verify against the existing `capabilities.rs` files; phase 2 may already have these.
- `tests/trybuild_read_macros.rs` — extend glob to include `tests/ui/nested_writes/*.rs`
- `CHANGELOG.md` — `[Unreleased]` bullets

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/nested-writes rev-parse --abbrev-ref HEAD`
Expected: `feature/nested-writes`.

- [ ] **Step 2: Confirm base**

Run: `git -C /home/joseph/Projects/prax/.worktrees/nested-writes log --oneline -1`
Expected: starts with `797a768 feat(codegen): flat write macros (phase 5a) (#104)`.

- [ ] **Step 3: Workspace builds**

Run: `cargo check --workspace --all-features`
Expected: zero errors.

- [ ] **Step 4: Existing tests pass**

Run: `cargo test -p prax-query --lib && cargo test -p prax-codegen --lib`
Expected: all pass.

- [ ] **Step 5: No commit — verification only.**

---

## Task 2: Unblock `NestedWriteOp::Connect` executor

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Extend the `Connect` variant**

Change the existing:

```rust
Connect {
    relation: String,
    pk: FilterValue,
}
```

…to:

```rust
Connect {
    /// Name of the relation on the parent model (for diagnostics).
    relation: String,
    /// Target child table.
    target_table: String,
    /// FK column on the child table that references the parent's PK.
    foreign_key: String,
    /// PK column on the child table (where the connect filter matches).
    target_pk: String,
    /// PK value of the child row to connect.
    pk: FilterValue,
}
```

Look up every existing constructor of `NestedWriteOp::Connect` in the repo (likely codegen / tests) and update them to fill the new fields. Run `cargo check -p prax-query` after to surface the call sites.

- [ ] **Step 2: Implement the executor body**

In the `execute` impl for `NestedWriteOp::Connect`, replace the `QueryError::internal(...)` with:

```rust
NestedWriteOp::Connect { target_table, foreign_key, target_pk, pk, .. } => {
    let sql = format!(
        "UPDATE {} SET {} = $1 WHERE {} = $2",
        target_table, foreign_key, target_pk
    );
    let params = vec![parent_pk.clone(), pk.clone()];
    engine.execute_raw(&sql, params).await?;
    Ok(())
}
```

Adjust the API call (`execute_raw` / equivalent) to whatever the existing `Create` executor uses for raw SQL execution.

Identifier-safety: `target_table`, `foreign_key`, and `target_pk` come from `RelationMeta` constants (compile-time `&'static str`s on codegen-emitted types), so they're trusted; the dollar-placeholder values are parameterized. Add a brief comment noting this matches the `.cursor/rules/sql-safety.mdc` boundary.

- [ ] **Step 3: Update doc comments**

Remove the "not yet implemented" caveat from `NestedWriteOp::Connect`'s doc comment. Add a one-liner explaining the SQL shape.

- [ ] **Step 4: Add a unit test**

In `prax-query/src/nested.rs::tests` (or a sibling test module), construct a `NestedWriteOp::Connect`, run it against a `MockEngine`, assert the recorded SQL matches the expected `UPDATE ... SET ... WHERE ...` shape.

- [ ] **Step 5: `cargo test -p prax-query --lib`**

Expected: all pass.

- [ ] **Step 6: Commit**

```
feat(query): implement NestedWriteOp::Connect executor
```

---

## Task 3: Codegen — `<RelatedModel>CreateWithout<Owner>Input`

**Files:**
- Create: `prax-codegen/src/generators/inputs/create_without.rs`
- Modify: `prax-codegen/src/generators/inputs/mod.rs`

- [ ] **Step 1: Build the generator**

For each relation on a parent model, identify the relation's target and back-pointer:
- `posts: Post[]` on `User` (HasMany) → back-pointer is `Post.authorId`, target is `Post`. Emit `PostCreateWithoutUserInput`.

The emitted struct is `<RelatedModel>CreateInput` minus the FK column that points back at the owner. Reuse the existing `<RelatedModel>CreateInput` generator's per-field iteration; skip the FK column.

Emit:

```rust
#[derive(Debug, Clone, Default)]
pub struct PostCreateWithoutUserInput {
    pub title: String,
    pub body:  Option<String>,
    // FK column (author_id) omitted — the parent insert fills it in.
}

impl ::prax_query::inputs::CreateInput for PostCreateWithoutUserInput {
    type Model = Post;
    type Data = ::prax_query::traits::CreatePayload;
    fn into_ir(self) -> Self::Data { /* like the existing impl but no FK */ }
}
```

- [ ] **Step 2: Wire into both derive + `prax_schema!` paths**

The existing phase-2/5a `create_input.rs` generator is called from both the derive and `prax_schema!` entry points. Add a parallel call site for `create_without` next to it.

- [ ] **Step 3: Insta snapshot tests**

Extend `prax-codegen/tests/lower_snapshots.rs` with a snapshot of `PostCreateWithoutUserInput` for the fixture schema. Confirm:
- All scalar fields except the FK back-pointer column are present
- `into_ir()` lowers cleanly to `CreatePayload`

- [ ] **Step 4: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 5: Commit**

```
feat(codegen): emit <RelatedModel>CreateWithout<Owner>Input per relation
```

---

## Task 4: Codegen — `<Model><Relation>CreateNestedInput` wrapper

**Files:**
- Create: `prax-codegen/src/generators/inputs/relation_nested_create.rs`
- Modify: `prax-codegen/src/generators/inputs/mod.rs`

- [ ] **Step 1: Build the generator**

For each relation on each parent model, emit:

```rust
#[derive(Debug, Clone, Default)]
pub struct UserPostsCreateNestedInput {
    pub create:  Option<Vec<PostCreateWithoutUserInput>>,
    pub connect: Option<Vec<PostWhereUniqueInput>>,
}
```

Two fields in phase 5b: `create` and `connect`. Later phases will extend this with `connect_or_create`, etc. The struct uses `Option<Vec<_>>` (not just `Vec<_>`) so that absent keys lower to `None` rather than empty lists, matching Prisma semantics.

- [ ] **Step 2: Emit a helper trait or method to flatten into `NestedWriteOp`s**

Add an inherent method on the nested-input struct that the runtime CreateOperation can call to extract its NestedWriteOps:

```rust
impl UserPostsCreateNestedInput {
    pub fn into_nested_ops(self) -> Vec<::prax_query::nested::NestedWriteOp> {
        let mut ops = Vec::new();
        if let Some(children) = self.create {
            for child in children {
                let payload = ::prax_query::inputs::CreateInput::into_ir(child);
                ops.push(::prax_query::nested::NestedWriteOp::Create {
                    relation:    "posts".to_string(),
                    target_table: <Post as ::prax_query::Model>::TABLE_NAME.to_string(),
                    foreign_key:  <UserPostsRelation as ::prax_query::relations::RelationMeta>::FOREIGN_KEY.to_string(),
                    payload:      vec![payload.into_columns()], // adapt to the actual CreatePayload→Vec<(col,val)> shape
                });
            }
        }
        if let Some(connect_specs) = self.connect {
            for spec in connect_specs {
                let pk = /* extract single PK value from PostWhereUniqueInput */;
                ops.push(::prax_query::nested::NestedWriteOp::Connect {
                    relation:    "posts".to_string(),
                    target_table: <Post as ::prax_query::Model>::TABLE_NAME.to_string(),
                    foreign_key:  <UserPostsRelation as ::prax_query::relations::RelationMeta>::FOREIGN_KEY.to_string(),
                    target_pk:    <Post as ::prax_query::Model>::PRIMARY_KEY[0].to_string(),
                    pk,
                });
            }
        }
        ops
    }
}
```

The exact pathing (`UserPostsRelation`, `<Post as Model>::TABLE_NAME`, etc.) depends on how codegen currently names per-relation RelationMeta types — read `inputs/relation_meta.rs` first. If the per-relation RelationMeta isn't yet name-discoverable, generate this `into_nested_ops` inline using the captured target_table/foreign_key strings from the relation analysis pass (no trait lookup at runtime). Inline is simpler.

- [ ] **Step 3: Wire into both derive + `prax_schema!` paths**

- [ ] **Step 4: Snapshot tests**

Snapshot the generated `UserPostsCreateNestedInput` struct + its `into_nested_ops` body for the fixture schema.

- [ ] **Step 5: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 6: Commit**

```
feat(codegen): emit <Model><Relation>CreateNestedInput wrapper
```

---

## Task 5: Extend `<Model>CreateInput` with relation fields

**Files:**
- Modify: `prax-codegen/src/generators/inputs/create_input.rs`

- [ ] **Step 1: Add relation fields to the struct**

For each relation on the parent model, append a field:

```rust
pub struct UserCreateInput {
    // ... existing scalar fields ...
    pub posts:   Option<UserPostsCreateNestedInput>,
    pub profile: Option<UserProfileCreateNestedInput>, // to-one HasOne
}
```

(Profile / HasOne relations also get a `*CreateNestedInput`. For phase 5b minimum, focus on HasMany relations; HasOne is structurally similar but only allows a single child — defer if it doesn't fit cleanly, document the deferral.)

- [ ] **Step 2: Extend `impl CreateInput::into_ir` to collect nested ops**

Today's `into_ir` returns a `CreatePayload` (or whatever the `Data` type is) of scalar columns. Extend it to also accumulate the nested ops by calling each relation field's `.into_nested_ops()` and appending to the payload's `Vec<NestedWriteOp>` field.

If `CreatePayload` doesn't currently carry a `Vec<NestedWriteOp>`, add one. The `CreateOperation::with(...)` builder already accepts `NestedWriteOp`, so the integration point is on the operation side: when consuming a `CreateInput`'s `Data`, the operation pulls the scalar columns into its `.set()` calls and the nested ops into a `.with(...)` call per op.

This is the trickiest task in phase 5b — get the runtime payload's shape right before continuing. Read `prax-query/src/traits.rs::CreatePayload` and the existing `with_create_input` in `operations/create.rs` first.

- [ ] **Step 3: Snapshot test**

Verify `UserCreateInput` includes the relation fields and that `into_ir()` token output references each relation's `into_nested_ops()` accumulator.

- [ ] **Step 4: `cargo test -p prax-codegen && cargo test -p prax-orm --test derive_inputs_e2e`**

Confirm the phase-2 `derive_inputs_e2e` still passes — adding optional relation fields shouldn't break existing tests.

- [ ] **Step 5: Commit**

```
feat(codegen): include relation fields on <Model>CreateInput
```

---

## Task 6: DSL — `data:` relation-key lowering (entry point)

**Files:**
- Create: `prax-codegen/src/macros/lower/data_relation.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs` — `pub mod data_relation;`
- Modify: `prax-codegen/src/macros/lower/data_input.rs` — replace relation-key rejection with delegation

- [ ] **Step 1: `lower_create_relation(rel, value, ctx)` in `data_relation.rs`**

`rel` is the schema's `Relation` metadata (name, kind, target, FK column, back-pointer). `value` is the DSL value the user wrote — expected to be a `DslValue::Block`.

For each pair inside the relation block:
- `create:` → expect `DslValue::List` of `DslValue::Block`. Each inner block lowers via the existing `lower_create_data` against the **target** model's `LowerCtx` (build a new ctx for the target). Skip the FK column on the target — the codegen-emitted `<RelatedModel>CreateWithout<Owner>Input` struct already omits it, so the inner block lowers into that variant, not the full CreateInput.
- `connect:` → expect `DslValue::List` of `DslValue::Block`. Each inner block is a `WhereUniqueInput` lowering — reuse the existing `lower_cursor` / `lower_where_unique`.
- `update:` / `update_many:` / `upsert:` / `delete:` / `delete_many:` / `disconnect:` / `set:` → emit phase-5c deferral error:
  > `nested operator '{op}' inside 'data:' relation block is not supported in phase 5b. Update/upsert/delete operators on relations land in phase 5c.`
- `connect_or_create:` → emit phase-5d deferral with the same friendly wording.
- Unknown operator → did-you-mean against the allowed-in-5b set `["create", "connect"]`.

Emit a struct literal of the per-relation nested-input wrapper:

```rust
::prax::inputs::user::UserPostsCreateNestedInput {
    create:  Some(vec![ /* lowered create children */ ]),
    connect: Some(vec![ /* lowered connect specs */ ]),
}
```

When only one of `create` / `connect` is present, the other field gets `None`.

- [ ] **Step 2: Update `data_input.rs` to delegate to relation lowering**

Find the existing match arm in `lower_create_data` that rejects relation keys with a phase-5b message. Replace with:

```rust
// (existing relation-name detection)
Some(rel) => {
    let lowered = data_relation::lower_create_relation(rel, &field.value, ctx)?;
    quote! { __c.#field_ident = Some(#lowered); }
}
```

Keep the rejection logic for relation keys on the **update** path inside `lower_update_data` — phase 5c will replace that.

- [ ] **Step 3: Snapshot tests**

In `lower_snapshots.rs` add cases for:
- `data: { name: "Alice", posts: { create: [...] } }`
- `data: { posts: { connect: [...] } }`
- `data: { posts: { update: [...] } }` → should produce a phase-5c diagnostic (assert error, don't snapshot)

- [ ] **Step 4: `cargo test -p prax-codegen lower_snapshots`**

- [ ] **Step 5: Commit**

```
feat(codegen): lower DSL nested create/connect inside data:
```

---

## Task 7: Wire `create!` macro to emit `.with(nw)` calls

**Files:**
- Modify: `prax-codegen/src/macros/ops/create.rs`

- [ ] **Step 1: The `create!` macro already passes a `<Model>CreateInput` value to `with_create_input`**

…which calls `into_ir()` and applies scalar columns. With Task 5's changes, the same `into_ir()` now also returns the nested ops inside the payload.

Two design choices for how `CreateOperation` consumes those ops:

(a) `with_create_input` becomes responsible for both — it sets columns AND calls `self = self.with(nw)` for each accumulated op. The macro is unchanged.

(b) The macro emits an explicit chain: `let __op = __op.with_create_input(__data); for nw in __data.nested_ops() { __op = __op.with(nw); }`. The macro is changed; `with_create_input` stays purely scalar.

**Prefer (a)** — pushing the nested-op consumption inside `with_create_input` keeps the macro emission clean. The macro doesn't need to be changed.

Verify by reading the existing `with_create_input` impl in `operations/create.rs`. If `CreateInput::Data` doesn't already include nested ops (Task 5 added this), make sure `with_create_input` consumes both halves.

Actually — if the macro changes are zero, **document that and move on**. The work for `create!` to support nested writes is already done by Tasks 2-6.

- [ ] **Step 2: If macro changes are needed (i.e. you chose option b), implement them**

Otherwise: no-op task, no commit needed. Update Step 1's reasoning in the commit message to explain why this task is empty.

- [ ] **Step 3: Smoke compile test in `tests/nested_writes_e2e.rs`**

```rust
#[test]
fn create_with_nested_create() {
    let _op = prax::create!(client.user, {
        data: {
            email: "alice@x.com",
            name: "Alice",
            posts: {
                create: [
                    { title: "p1" },
                    { title: "p2" },
                ],
            },
        },
        select: { id: true },
    });
}
```

This must compile. Run `cargo build -p prax-orm --tests`. If it doesn't, fix the path that's broken.

- [ ] **Step 4: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 5: Commit (if any code changed)**

```
feat(codegen): wire create! macro for nested create/connect
```

(If Step 1 found nothing to change, skip this commit and move to Task 8.)

---

## Task 8: Engine capability declarations

**Files:**
- Possibly modify: `prax-postgres/src/capabilities.rs`, `prax-mysql/src/capabilities.rs`, `prax-sqlite/src/capabilities.rs`, `prax-mssql/src/capabilities.rs`, `prax-duckdb/src/capabilities.rs`, `prax-mongodb/src/capabilities.rs`

- [ ] **Step 1: Audit existing `capabilities.rs` files**

Run: `grep -rn "SupportsNestedWrites" prax-postgres prax-mysql prax-sqlite prax-mssql prax-duckdb prax-mongodb`

For each engine that doesn't already declare `impl SupportsNestedWrites for <Engine>`, add it.

CQL engines (`prax-scylladb`, `prax-cassandra`) must NOT impl `SupportsNestedWrites` — phase 5b is the first phase where this trait is enforced at the type level for nested-write codepaths.

- [ ] **Step 2: Per-engine `assert_all<E: SupportsNestedWrites + ...>()` compile-only test**

Append to each engine's existing `tests/capabilities.rs` (or wherever the marker-trait assertions live). Should be a one-liner.

- [ ] **Step 3: `cargo check --workspace --all-features`**

Expected: zero errors. If a CQL engine breaks because something downstream now requires `SupportsNestedWrites`, the gate is wired correctly — that should ONLY happen if a user wrote `prax::create!(client.user, { data: { posts: { create: [...] } } })` against a CQL engine, which is the trybuild fixture in Task 9.

- [ ] **Step 4: Commit**

```
feat(query): SupportsNestedWrites engine impls (SQL + MongoDB)
```

---

## Task 9: trybuild UI tests

**Files:**
- Create: `tests/ui/nested_writes/cql_capability_gap.rs` + `.stderr`
- Create: `tests/ui/nested_writes/unknown_nested_op_phase_5c.rs` + `.stderr`
- Create: `tests/ui/nested_writes/scalar_op_on_relation.rs` + `.stderr`
- Modify: `tests/trybuild_read_macros.rs` — extend glob to include `tests/ui/nested_writes/*.rs`

- [ ] **Step 1: Author the three `.rs` fixtures**

- `cql_capability_gap.rs` — schema + `prax::create!` against a CQL-backed accessor with a nested `posts: { create: [...] }`. Should fail to compile because the engine doesn't impl `SupportsNestedWrites`.
- `unknown_nested_op_phase_5c.rs` — uses `update:` inside `posts: { ... }`. Should produce the friendly phase-5c deferral diagnostic from Task 6.
- `scalar_op_on_relation.rs` — uses `{ equals: 1 }` inside a relation block in `data:` (treating it like a where filter). Should produce a clear "scalar operator on relation" error.

- [ ] **Step 2: Generate `.stderr` baselines**

`TRYBUILD=overwrite cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`. Inspect each generated `.stderr`. The CQL gap message comes from the `#[diagnostic::on_unimplemented]` on `SupportsNestedWrites` — confirm the message is helpful; if it's missing or unhelpful, add/improve it on the trait def.

- [ ] **Step 3: `cargo test -p prax-orm --features ui-tests --test trybuild_read_macros`** — green against committed baselines.

- [ ] **Step 4: Commit**

```
test(codegen): trybuild UI fixtures for nested create/connect
```

---

## Task 10: e2e tests + composition

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Cover the three core scenarios**

- Nested create only: parent + 2 children.
- Nested connect only: parent + 1 connected existing child.
- Both: parent + 1 created child + 1 connected child.

For each: build the operation via `prax::create!`, run `.exec().await?`, inspect the engine's recorded SQL statements.

The expected SQL for "parent + 1 created child + 1 connected child":
1. `BEGIN`
2. `INSERT INTO users (email, name) VALUES ($1, $2) RETURNING id`
3. `INSERT INTO posts (title, body, author_id) VALUES ($1, $2, $3)` (FK = parent's returned id)
4. `UPDATE posts SET author_id = $1 WHERE id = $2` (the connected child's existing row)
5. `COMMIT`

Adapt expectations to whatever the in-process engine actually records.

- [ ] **Step 2: A "spread" composition test**

```rust
#[test]
fn create_with_spread_and_nested() {
    let defaults = user::UserCreateInput {
        email: "default@x.com".into(),
        active: Some(true),
        ..Default::default()
    };
    let _op = prax::create!(client.user, {
        data: {
            ..defaults,
            name: "Alice",
            posts: {
                create: [{ title: "p1" }],
            },
        },
    });
}
```

Confirm spread doesn't clobber the new relation field.

- [ ] **Step 3: `cargo test -p prax-orm --test nested_writes_e2e`**

- [ ] **Step 4: Commit**

```
test(codegen): e2e tests for nested create/connect
```

---

## Task 11: CHANGELOG + final verification sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG bullet under `[Unreleased]`**

```
### Added
- **Nested create/connect (phase 5b).** `prax::create!`'s `data:` block
  now accepts relation keys with `{ create: [...], connect: [...] }`,
  building a single transaction that inserts the parent, inserts the
  nested children with the parent's returned PK as their FK, and updates
  the FK of any existing child rows targeted by `connect`.
- `NestedWriteOp::Connect` executor is now functional (was returning
  `QueryError::internal`).
- Codegen emits `<RelatedModel>CreateWithout<Owner>Input` (FK-omitted
  variants) and `<Model><Relation>CreateNestedInput` wrappers per
  relation. `<Model>CreateInput` now includes optional relation fields
  alongside its scalar columns.
- `SupportsNestedWrites` is now enforced at the type level on the
  create-side nested-write code path. SQL engines and MongoDB declare it;
  CQL engines (ScyllaDB, Cassandra) intentionally do not — nested writes
  against CQL fail to compile with a friendly diagnostic.

### Deferred to phase 5c+
- `update`, `update_many`, `upsert`, `delete`, `delete_many`,
  `disconnect`, `set` operators inside relation blocks — phase 5c
- `connect_or_create` (engine-specific lowerings) — phase 5d
- `set:` full relation replacement (diff-based) — phase 5e
- Nested writes inside `update!` / `upsert!` `data:` blocks — phase 5c
```

- [ ] **Step 2: Full verification sweep**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

All three must be green.

- [ ] **Step 3: Commit**

```
docs(codegen): CHANGELOG for nested create/connect
```

---

## Task 12: Push + open PR

**Files:**
- None.

- [ ] **Step 1: `git push -u origin feature/nested-writes`**

Pre-push hook runs the full test suite.

- [ ] **Step 2: Open PR**

```bash
gh pr create --base develop --head feature/nested-writes \
  --title "feat(codegen): nested create/connect inside data: (phase 5b)" \
  --body "..."
```

PR body should:
- Summarize the slice scope (nested create + connect on `create!`)
- Explicitly enumerate what's deferred to phase 5c/5d/5e
- Link the spec and plan
- Test plan checklist

- [ ] **Step 3: Wait for CI**

---

## Out of scope for phase 5b (deferred to phase 5c+)

- Nested operators on `update!` / `upsert!` (whole new code path; phase 5c)
- `update` / `update_many` / `upsert` / `delete` / `delete_many` / `disconnect` operators inside relation blocks (phase 5c)
- `connect_or_create` with engine-specific single-statement lowerings (phase 5d)
- `set: [...]` (diff-based full relation replacement) (phase 5e)
- Returning the typed shape after a nested write — phase 5b ships the writes but the post-write `.exec()` returns whatever the parent operation's `RETURNING` clause produces; aggregating a fully-hydrated tree across the transaction lands in phase 5c
- Aggregate macros (phase 6)
- Computed/virtual field codegen (phase 5.5)
