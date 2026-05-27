# Prisma-Parity Gap Closure — Decomposition & Roadmap

**Date:** 2026-05-27
**Status:** Decomposition / index (not an implementation plan)
**Scope:** Decomposes the Tier 1–3 gaps found in the 2026-05-27 Prax-vs-Prisma capability audit into discrete, independently-shippable sub-projects with dependencies and a recommended build order.

This is a roadmap/index document. Each sub-project below should get its own
`docs/superpowers/specs/<date>-<topic>-design.md` → `docs/superpowers/plans/<date>-<topic>.md`
cycle and link back here. `T3-SCHEMA` is itself an epic and will decompose further.

> Baseline: audited at workspace version 0.10.0. Tiers/severity come from the audit;
> "Tier 4" (Studio, realtime, managed services) is intentionally out of scope here.

---

## Tier 1 — Migration / `db` CLI + introspection

The `prax-migrate` engine (schema diff, 5 SQL dialects, CQL, event sourcing,
shadow DB, drift detection) is real and comprehensive. `prax migrate deploy`
and `prax migrate status` already work end-to-end. The gap is **CLI wiring +
one missing capability (cross-dialect introspection)**, not the engine. Several
commands shipped in 0.10.0 as scaffolds that print success but do not execute
(verified TODOs):

- `db.rs:84`, `:348` — "Execute SQL" never runs (`db push`, `db execute` diff path)
- `db.rs:54`, `:378` — current DB state / schema diffing not implemented
- `migrate.rs:186/189` — `migrate reset` DB drop/create
- `migrate.rs:293/304` — `migrate resolve` history-table update
- `migrate.rs:332` — `migrate dev` database introspection
- `migrate.rs:378/390` — `migrate rollback` placeholder
- `migrate.rs:410/423` — `migrate history` placeholder
- `migrate.rs:655` — generic "Execute SQL against database"

### Sub-projects

| ID | Sub-project | What it is | Depends on | Size/Risk |
|----|-------------|------------|------------|-----------|
| **T1-EXEC** | Apply primitive | Extract the "apply generated `MigrationSql` to a live connection" path that `deploy` already uses into a shared primitive reused by push/reset/rollback. | — (mostly exists) | S |
| **T1-INTROSPECT** | Cross-dialect introspection | An `Introspector` trait + MySQL/SQLite/MSSQL implementations (Postgres exists). Reads tables, columns, indexes, FKs (+actions), enums, views. | T1-EXEC | **L — long pole** |
| **T1-DEV** | `migrate dev` | Diff schema against **live DB state**, emit migration files, apply. | T1-EXEC + T1-INTROSPECT + diff engine | M |
| **T1-PUSH** | `db push` | Diff schema vs DB, apply directly (no migration files), with destructive-change guard. | T1-EXEC + T1-INTROSPECT | M |
| **T1-RESET** | `migrate reset` | Per-dialect DB drop/create + replay (+ optional seed). | T1-EXEC | S–M |
| **T1-EVENTCLI** | `rollback` / `resolve` / `history` | Wire CLI to the existing event store: execute `down.sql`, append `RolledBack`/`Resolved` events, replay log for history. | T1-EXEC + event store (exists) | M |

**Internal order:** T1-EXEC + T1-INTROSPECT first (parallelizable) → DEV / PUSH / RESET / EVENTCLI fan out.

**Stopgap:** until these land, the stubbed commands should **error loudly**
("not yet implemented") rather than print success — shipping silent no-ops is a footgun. Candidate for a 0.10.1.

---

## Tier 2 — Client API gaps

Mostly independent features. One natural cluster (`T2-FILTEROPS`) shares a body of work.

| ID | Sub-project | Items | Depends on | Size |
|----|-------------|-------|------------|------|
| **T2-THROW** | OrThrow variants | `findUniqueOrThrow`, `findFirstOrThrow` | — | XS |
| **T2-RETURN** | Bulk insert + return | `createManyAndReturn` (uses existing `returning_clause`) | — | S |
| **T2-FILTEROPS** | Filter operator expansion *(cluster)* | JSON path filters + scalar list ops (`has`/`hasEvery`/`hasSome`/`isEmpty`) + `mode: insensitive` lowering. All extend the same surface: `Filter` IR → per-dialect WHERE lowering → `where!` macro. Gated by existing `Supports*` markers. | — | **L (do as one)** |
| **T2-TXSEQ** | Sequential transaction | `$transaction([op1, op2, …])` over the existing batch API | — | S |
| **T2-ORDER** | Ordering | `orderBy` nulls first/last (small); `_relevance` (pairs with full-text) | `_relevance` → T3-FULLTEXT | S + M |

---

## Tier 3 — Schema language

Each touches parser → AST → validator → migration DDL, and wants
introspection round-trip (hence the T1-INTROSPECT dependency).

| ID | Sub-project | Item | Depends on | Size |
|----|-------------|------|------------|------|
| **T3-IGNORE** | `@ignore` | Exclude field/model from codegen; round-trip through introspection | T1-INTROSPECT | S |
| **T3-CHECK** | `@@check` | CHECK constraints: DDL emit + introspect | T1-INTROSPECT | M |
| **T3-FULLTEXT** | `@@fulltext` | Complete the defined-but-unwired index type: DDL + query path | `search.rs`; pairs with T2-ORDER `_relevance` | M |
| **T3-SCHEMA** | `@@schema` | Multi-schema: qualified names across parser/AST/validator/**every** SQL generator/introspection/query builder | T1-INTROSPECT | **XL — own epic** |

---

## Cross-cutting dependencies & clusters

- **`T1-INTROSPECT` is the keystone.** Needed by: non-Postgres `db pull`,
  the `migrate dev` / `db push` diff source, and Tier-3 round-tripping
  (`@@check`, `@ignore`, `@@schema`).
- **Full-text cluster:** `T2-ORDER` (`_relevance`) + `T3-FULLTEXT` + `search.rs` —
  design together.
- **Filter cluster:** `T2-FILTEROPS` is three "features" but one body of work
  (same files: `filter.rs`, per-dialect WHERE lowering, `where!` macro). Splitting
  them means touching the same surface three times.

## Recommended build order — 3 parallel tracks

1. **Track A (critical path):** T1-EXEC + **T1-INTROSPECT** → T1-DEV / T1-PUSH /
   T1-RESET / T1-EVENTCLI. Closes the shipped-half-working gap and is the
   keystone for Tier 3.
2. **Track B (independent quick wins, anytime):** T2-THROW → T2-RETURN →
   T2-TXSEQ → T2-ORDER (nulls). No dependency on Track A; good parallel work.
3. **Track C (after introspection lands):** T2-FILTEROPS cluster → T3-IGNORE →
   T3-CHECK → full-text cluster (T2 `_relevance` + T3-FULLTEXT).
   **T3-SCHEMA is its own epic — schedule last, separately.**

Each box is sized to be one spec → plan → implement cycle, except `T3-SCHEMA`.

## Out of scope (Tier 4, for reference)

Prisma Studio (data-browser GUI), realtime/CDC (Pulse), managed edge cache
(Accelerate), query advisor (Optimize), CockroachDB native driver, first-class
soft-delete/lifecycle directives. Tracked separately if/when prioritized.

## Prax advantages to preserve (not gaps)

DuckDB / ScyllaDB / Cassandra / pgvector engines; built-in multi-tenancy (RLS,
task-local tenant context, per-tenant pools); security (RLS builder, column
grants, data masking, `policy`/`serverGroup` schema blocks); `prax-import`
(Prisma/Diesel/SeaORM → Prax); first-party `prax-axum`/`prax-actix`/`prax-armature`;
`prax-typegen` (TS + Zod).
