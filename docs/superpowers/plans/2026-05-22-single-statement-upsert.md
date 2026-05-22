# Vendor-Specific Single-Statement Upsert Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Optimize `NestedWriteOp::Upsert::execute` to emit a single statement on dialects that support it (Postgres `ON CONFLICT`, SQLite `ON CONFLICT`, DuckDB `ON CONFLICT`, MySQL `ON DUPLICATE KEY UPDATE`). MSSQL and CQL fall back to the existing two-statement form.

Current behavior (shipped in phase 5c-mutations):
```sql
UPDATE child SET col = $1 WHERE pk = $2;     -- if affected_rows == 0:
INSERT INTO child (cols + fk) VALUES (...);  -- run this
```

New single-statement behavior on Postgres:
```sql
INSERT INTO child (cols + fk) VALUES (...) ON CONFLICT (pk) DO UPDATE SET col = $N;
```

Reduces 2 round-trips to 1 on dialects that support it. Functional equivalence preserved.

**Scope decision: `NestedWriteOp::Upsert` only.** `NestedWriteOp::ConnectOrCreate`'s conflict-column extraction from arbitrary `where:` filters is structurally more nuanced (the user's where might match on any unique column, not just PK; create_payload may or may not include a value for that column). Deferred to follow-up.

**Architecture:**

The existing `SqlDialect::upsert_clause(conflict_cols, update_set)` already provides the dispatch:
- Postgres/SQLite: `" ON CONFLICT (cols) DO UPDATE SET <set>"`
- MySQL: `" ON DUPLICATE KEY UPDATE <set>"`
- MSSQL/CQL/NotSql: `""` (empty)

The executor checks if the returned clause is non-empty:
- Non-empty → single-statement form
- Empty → fall back to existing two-statement code path

The SET clause is built from `update_payload`'s `WriteOp` fragments (same shape as the existing UPDATE statement), then passed to `upsert_clause` to wrap with dialect-specific syntax.

---

## File Structure

### Modified files

- `prax-query/src/nested.rs` — refactor `NestedWriteOp::Upsert::execute`
- `tests/nested_writes_e2e.rs` — e2e coverage proving single-statement path on Postgres dialect
- `CHANGELOG.md`

---

## Task 1: Verify baseline

- [ ] **Step 1**: `git -C /home/joseph/Projects/prax/.worktrees/single-statement-upsert rev-parse --abbrev-ref HEAD` → `feature/single-statement-upsert`
- [ ] **Step 2**: `git log --oneline -1` starts with `6c9a5a5 feat(query): nested writes inside update! and upsert! macros`
- [ ] **Step 3**: `cargo check --workspace --all-features` — zero errors
- [ ] **Step 4**: `cargo test -p prax-query --lib nested` — green (baseline for nested-write executors)
- [ ] **Step 5**: No commit.

---

## Task 2: Refactor `NestedWriteOp::Upsert::execute` with dialect dispatch

**Files:**
- Modify: `prax-query/src/nested.rs`

- [ ] **Step 1: Read the current executor**

Locate the `NestedWriteOp::Upsert` arm in `execute()` (somewhere around line 700+). The current code:
1. Builds a SET clause from `update_payload`
2. Emits `UPDATE table SET ... WHERE pk = $1`
3. Checks `affected_rows`
4. If 0, emits a separate `INSERT INTO table (cols + fk) VALUES (...)`

- [ ] **Step 2: Add the dispatch**

Before the two-statement code, query the dialect:

```rust
let upsert_set = build_writeop_set_fragments(&update_payload, &dialect, &mut next_ph, &mut params)?;
let conflict_cols = [target_pk]; // single-column PK conflict target
let upsert_clause = dialect.upsert_clause(&conflict_cols, &upsert_set);
let supports_single = !upsert_clause.is_empty();
```

- [ ] **Step 3: Single-statement path**

When `supports_single`:
```rust
// INSERT INTO child (create_cols + fk) VALUES (...) ON CONFLICT (pk) DO UPDATE SET <set>
let insert_cols: Vec<_> = create_payload.iter().map(|(c, _)| c.clone()).collect();
let mut insert_values: Vec<_> = create_payload.into_iter().map(|(_, v)| v).collect();
let mut insert_cols_quoted: Vec<_> = insert_cols.iter().map(|c| dialect.quote_ident(c)).collect();
insert_cols_quoted.push(dialect.quote_ident(foreign_key));
insert_values.push(parent_pk.clone());

let mut placeholders = Vec::new();
for i in 1..=insert_values.len() {
    placeholders.push(dialect.placeholder(i));
}

let mut all_params = insert_values;
// Build the update_set fragments at higher positions
// IMPORTANT: the SET clause's parameters come AFTER the INSERT's ($N+1, $N+2, ...).
// Build update_payload's set fragments with `next_ph` starting at all_params.len() + 1.
let update_start_ph = all_params.len() + 1;
let (update_set_text, update_set_params) = build_writeop_set_fragments_at(&update_payload, &dialect, update_start_ph);
all_params.extend(update_set_params);

let upsert_clause_str = dialect.upsert_clause(&conflict_cols, &update_set_text);
let sql = format!(
    "INSERT INTO {} ({}) VALUES ({}){}",
    dialect.quote_ident(target_table),
    insert_cols_quoted.join(", "),
    placeholders.join(", "),
    upsert_clause_str,
);
engine.execute_raw(&sql, all_params).await?;
Ok(())
```

The exact API for `build_writeop_set_fragments_at` should produce: a SET text like `col1 = $4, col2 = col2 + $5` and a Vec of params positioned at `$N+1..`. Pull existing logic from the current two-statement Upsert code (which already builds the SET fragments for the UPDATE step) and refactor into a helper.

- [ ] **Step 4: Two-statement fallback**

When `!supports_single` (i.e. MSSQL, CQL, NotSql), keep the existing two-statement code path verbatim.

- [ ] **Step 5: Unit tests** in `nested.rs::tests`

- `nested_op_upsert_single_statement_on_postgres` — engine returns Postgres dialect; assert one `INSERT ... ON CONFLICT` statement
- `nested_op_upsert_two_statement_fallback_on_mssql` — build a mock engine that exposes the MSSQL dialect (`prax_query::dialect::Mssql`); assert two statements (UPDATE then INSERT)

For the MSSQL test you'll need a mock that overrides `dialect()` to return `&Mssql`. Either extend `RecordingEngine` with a configurable dialect or add a separate `MssqlRecordingEngine`.

- [ ] **Step 6: `cargo test -p prax-query --lib nested`** — all pass

- [ ] **Step 7: Commit**

```
perf(query): single-statement upsert on dialects with ON CONFLICT/ON DUPLICATE
```

---

## Task 3: E2E tests

**Files:**
- Modify: `tests/nested_writes_e2e.rs`

- [ ] **Step 1: Configurable-dialect RecordingEngine** (if not already trivial to add)

Extend the test mock or add a sibling that switches between dialect references. Simplest: a flag/enum (`PostgresLike` vs `MssqlLike`) that picks which static `&dyn SqlDialect` to return.

- [ ] **Step 2: Add e2e tests**

- `nested_upsert_single_statement_on_postgres_dialect` — verify the recording engine sees one `INSERT ... ON CONFLICT` statement and zero standalone `UPDATE` statements when the nested ops include an Upsert.
- `nested_upsert_two_statement_on_mssql_dialect` — verify two statements (UPDATE first, INSERT second) for the same logical operation.

- [ ] **Step 3: `cargo test -p prax-orm --test nested_writes_e2e`** — green

- [ ] **Step 4: Commit**

```
test(query): e2e for single-statement upsert dispatch
```

---

## Task 4: CHANGELOG + final sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: CHANGELOG entry**

```
### Changed
- **`NestedWriteOp::Upsert` now emits a single statement on dialects
  that support it** (Postgres `ON CONFLICT (pk) DO UPDATE SET ...`,
  SQLite, DuckDB, MySQL `ON DUPLICATE KEY UPDATE ...`). Halves the
  round-trips for nested upserts on those engines. MSSQL and CQL keep
  the existing two-statement fallback since neither has a clean
  single-statement upsert (MSSQL would need `MERGE`, CQL is
  last-write-wins by default and doesn't surface ON CONFLICT).
  Behavior unchanged on the fallback path.
- `NestedWriteOp::ConnectOrCreate` continues to use the two-statement
  form regardless of dialect — its conflict-column extraction from
  arbitrary `where:` filters is more nuanced and deferred to a
  follow-up.
```

- [ ] **Step 2: Full sweep**

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast
```

- [ ] **Step 3: Commit**

```
docs(query): CHANGELOG for single-statement upsert
```

---

## Task 5: Push + open PR

- [ ] `git push -u origin feature/single-statement-upsert`
- [ ] `gh pr create --base develop --head feature/single-statement-upsert --title "perf(query): single-statement upsert on Postgres/SQLite/MySQL/DuckDB"`

---

## Out of scope (deferred follow-ups)

- Single-statement `NestedWriteOp::ConnectOrCreate` — needs conflict-column extraction from arbitrary where filters
- MSSQL `MERGE` syntax for single-statement upsert — `MERGE` semantics need careful design around the source CTE
- Aggregate macros — phase 6
- Computed/virtual fields — phase 5.5
