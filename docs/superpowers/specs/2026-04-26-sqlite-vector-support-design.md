# prax-sqlite Vector Support Design

**Date:** 2026-04-26  
**Status:** Approved  
**Type:** Feature Addition

## Overview

Add LLM/RAG vector storage and retrieval to `prax-sqlite` by integrating the `sqlite-vector-rs` crate (v0.2+) behind a new `vector` feature flag. Mirrors the `prax-pgvector` API where possible, extends the Prax schema language with a `Vector` field type, and teaches the SQLite migration generator to emit `CREATE VIRTUAL TABLE ... USING vector(...)` from schema diffs.

Scope spans four crates:
- `prax-sqlite` — runtime API, auto-registration, runtime types
- `prax-schema` — parser support for `Vector` fields and vector attributes
- `prax-migrate` — `FieldDiff` extension with vector metadata, `SqliteGenerator` emits vector DDL
- `prax-codegen` — optional: derive macro recognition of vector fields (covered if trivial, deferred otherwise)

## Goals

1. **Feature-gated integration** via `vector` feature on `prax-sqlite` (no impact when disabled)
2. **Auto-registration** of the sqlite-vector-rs extension on every `SqlitePool` connection when the feature is on
3. **Schema declaration** of vector columns via `Vector` field type + `@dim`, `@vectorType`, `@metric`, `@index` attributes
4. **Migration emission** of `CREATE VIRTUAL TABLE ... USING vector(...)` from schema diffs
5. **Runtime query API** mirroring prax-pgvector: `Embedding`, `DistanceMetric`, `VectorSearchBuilder`, `HybridSearchBuilder` (fts5 + vector via RRF), `VectorIndex`
6. **Support all six element types** that sqlite-vector-rs offers: `float2`, `float4` (default), `float8`, `int1`, `int2`, `int4`

## Non-Goals

- Built-in embedding model inference — users bring their own embeddings as `Vec<f32>` etc.
- Schema-driven Arrow import/export — sqlite-vector-rs's Arrow functions remain available as raw SQL for users who need them, but we don't wrap them
- Auto-generation of embedding columns from text fields — out of scope

## Background

`sqlite-vector-rs` (owned and published by the same author as Prax) provides:

- `sqlite_vector_rs::register(&conn)` to activate on a rusqlite connection
- SQL scalar functions: `vector_from_json`, `vector_to_json`, `vector_distance`, `vector_dims`, `vector_rebuild_index`, `vector_export_arrow`, `vector_insert_arrow`
- Virtual table module `vector`: `CREATE VIRTUAL TABLE name USING vector(dim=..., type=..., metric=...)` with typed vector columns (`float2` / `float4` / `float8` / `int1` / `int2` / `int4`) and HNSW indexing
- `library` feature that pulls in rusqlite (already used by prax-sqlite)

`prax-sqlite` already uses rusqlite (via `tokio-rusqlite`), so integration is native — no FFI or subprocess.

`prax-pgvector` is the reference API shape. We mirror its module layout, type names, and builder patterns where behavior is equivalent.

## Architecture

### Feature Flag

Add to `prax-sqlite/Cargo.toml`:

```toml
[features]
default = []
vector = ["dep:sqlite-vector-rs"]

[dependencies]
sqlite-vector-rs = { workspace = true, optional = true, features = ["library"] }
```

Add to workspace root `Cargo.toml`:

```toml
[workspace.dependencies]
sqlite-vector-rs = "0.2"
```

### Module Layout

New `prax-sqlite/src/vector/` directory (all files gated `#[cfg(feature = "vector")]` from `lib.rs`):

```
prax-sqlite/src/vector/
├── mod.rs           # module entry, re-exports, prelude
├── types.rs         # Embedding, HalfEmbedding, IntVector<T>, conversions
├── metric.rs        # DistanceMetric, VectorElementType
├── error.rs         # VectorError
├── index.rs         # VectorIndex builder for CREATE VIRTUAL TABLE
├── search.rs        # VectorSearchBuilder
├── hybrid.rs        # HybridSearchBuilder (vector + fts5 via RRF)
└── register.rs      # register_vector_extension helper (called internally)
```

`prax-sqlite/src/lib.rs`:

```rust
#[cfg(feature = "vector")]
pub mod vector;
```

### Auto-registration

`prax-sqlite/src/pool.rs` (existing file) — in the rusqlite connection factory used by the pool, after the connection is established:

```rust
#[cfg(feature = "vector")]
crate::vector::register::register_vector_extension(&conn)
    .map_err(|e| SqliteError::Vector(e))?;
```

This runs before the pool hands out any connection, so user queries always see the extension loaded.

### Schema Parser Extension (`prax-schema`)

Extend the parser to recognize:

```prax
model Document {
  id        Int    @id @auto
  content   String
  embedding Vector @dim(1536) @vectorType("float4") @metric("cosine") @index(hnsw)
}
```

New additions to `prax-schema`:

- `FieldType::Vector` variant in the AST
- Attribute parsers for `@dim(N)`, `@vectorType("..."|identifier)`, `@metric("..."|identifier)`, `@index(hnsw)`
- Validation:
  - `Vector` fields require `@dim(N)` with `N > 0`
  - `@vectorType` values: `float2`, `float4` (default), `float8`, `int1`, `int2`, `int4`
  - `@metric` values: `cosine` (default), `l2`, `inner`
  - `@index` values: `hnsw` (only option supported by sqlite-vector-rs currently)

### Schema Diff Extension (`prax-migrate/src/diff.rs`)

Extend `FieldDiff`:

```rust
#[derive(Debug, Clone, Default)]
pub struct FieldDiff {
    // ... existing fields ...
    /// Optional vector column metadata for SQLite vector support.
    pub vector: Option<VectorColumnInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorColumnInfo {
    pub dimensions: u32,
    pub element_type: VectorElementType,
    pub metric: VectorDistanceMetric,
    pub index: Option<VectorIndexKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorElementType {
    Float2,
    Float4,
    Float8,
    Int1,
    Int2,
    Int4,
}

impl VectorElementType {
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorElementType::Float2 => "float2",
            VectorElementType::Float4 => "float4",
            VectorElementType::Float8 => "float8",
            VectorElementType::Int1 => "int1",
            VectorElementType::Int2 => "int2",
            VectorElementType::Int4 => "int4",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorDistanceMetric {
    Cosine,
    L2,
    InnerProduct,
}

impl VectorDistanceMetric {
    pub fn as_sql(&self) -> &'static str {
        match self {
            VectorDistanceMetric::Cosine => "cosine",
            VectorDistanceMetric::L2 => "l2",
            VectorDistanceMetric::InnerProduct => "inner",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorIndexKind {
    Hnsw,
}
```

### SqliteGenerator Extension (`prax-migrate/src/sql.rs`)

When `SqliteGenerator::create_table(model)` detects any field with `vector: Some(_)`:

1. Emit `CREATE TABLE name (...)` for all non-vector columns (existing logic)
2. Emit a companion `CREATE VIRTUAL TABLE {table}_vectors USING vector(...)` listing every vector column as `{col_name}='{element_type}[{dim}] {metric} {index}'`
3. In the down migration, drop the virtual table first (reverse order)

Example output for `model Document { id Int @id @auto, content String, embedding Vector @dim(1536) @vectorType("float4") @metric("cosine") @index(hnsw), summary_vec Vector @dim(384) @vectorType("float4") @metric("cosine") @index(hnsw) }`:

```sql
-- up
CREATE TABLE "documents" (
    "id" INTEGER PRIMARY KEY AUTOINCREMENT,
    "content" TEXT NOT NULL
);

CREATE VIRTUAL TABLE "documents_vectors" USING vector(
    rowid_column='document_id',
    embedding='float4[1536] cosine hnsw',
    summary_vec='float4[384] cosine hnsw'
);
```

```sql
-- down
DROP TABLE IF EXISTS "documents_vectors";
DROP TABLE IF EXISTS "documents";
```

One virtual table per model (not per column) — sqlite-vector-rs supports multiple vector columns in a single virtual table, and this matches its native design.

### Runtime API

**Types:**

```rust
// Most common — f32 vector (maps to float4)
pub struct Embedding {
    data: Vec<f32>,
}

// Half-precision (float2) when `halfvec` feature is enabled
#[cfg(feature = "halfvec")]
pub struct HalfEmbedding { /* f16 data */ }

// Double-precision (float8)
pub struct DoubleEmbedding {
    data: Vec<f64>,
}

// Integer vectors
pub struct IntVector<T: IntVectorElement> {
    data: Vec<T>,
}

pub trait IntVectorElement: Copy + private::Sealed {
    const TYPE: VectorElementType;
}

// Implemented for i8 (int1), i16 (int2), i32 (int4)
```

**Search:**

```rust
use prax_sqlite::vector::prelude::*;

let embedding = Embedding::new(vec![0.1, 0.2, /* ... */]);

let search = VectorSearchBuilder::new("documents", "embedding")
    .query(embedding)
    .metric(DistanceMetric::Cosine)
    .limit(10)
    .build();

// Yields SQL like:
// SELECT documents.*, vector_distance(v.embedding, ?, 'cosine', 'float4') AS distance
// FROM documents_vectors v
// JOIN documents ON documents.id = v.document_id
// ORDER BY distance
// LIMIT 10
```

**Hybrid search (Reciprocal Rank Fusion):**

Requires both a vector column and a `fts5` virtual table on a text column. The builder composes a CTE that unions ranked results:

```rust
let hybrid = HybridSearchBuilder::new("documents")
    .vector_column("embedding")
    .fts_column("content")          // an existing fts5 virtual table: documents_fts
    .query_vector(embedding)
    .query_text("machine learning")
    .vector_weight(0.7)
    .text_weight(0.3)
    .rrf_k(60)                       // default RRF constant
    .limit(10)
    .build();
```

Generates an RRF query:

```sql
WITH vec_ranked AS (
    SELECT document_id, ROW_NUMBER() OVER (ORDER BY vector_distance(embedding, ?, 'cosine', 'float4')) AS rank
    FROM documents_vectors
),
fts_ranked AS (
    SELECT rowid AS document_id, ROW_NUMBER() OVER (ORDER BY bm25(documents_fts)) AS rank
    FROM documents_fts WHERE documents_fts MATCH ?
)
SELECT d.*,
    COALESCE(0.7 / (60 + v.rank), 0) + COALESCE(0.3 / (60 + f.rank), 0) AS score
FROM documents d
LEFT JOIN vec_ranked v ON d.id = v.document_id
LEFT JOIN fts_ranked f ON d.id = f.document_id
WHERE v.document_id IS NOT NULL OR f.document_id IS NOT NULL
ORDER BY score DESC
LIMIT 10;
```

**Index builder** (for users creating tables outside of migrations):

```rust
use prax_sqlite::vector::VectorIndex;

let ddl = VectorIndex::new("documents_vectors")
    .rowid_column("document_id")
    .column("embedding", VectorElementType::Float4, 1536)
    .metric(DistanceMetric::Cosine)
    .index(VectorIndexKind::Hnsw)
    .to_create_sql();
```

## Error Handling

New types in `prax-sqlite/src/vector/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("Dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("Unsupported metric for element type {element_type}")]
    UnsupportedMetric { element_type: &'static str },

    #[error("Extension not loaded on connection")]
    ExtensionNotLoaded,

    #[error("Rusqlite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    #[error("sqlite-vector-rs error: {0}")]
    Driver(String),
}

pub type VectorResult<T> = Result<T, VectorError>;
```

Existing `prax-sqlite` `SqliteError` gains a `Vector(VectorError)` variant (feature-gated).

In `prax-schema`, new parser errors:

```rust
pub enum SchemaError {
    // ... existing ...
    MissingVectorDimension { field: String },
    InvalidVectorType { value: String },
    InvalidVectorMetric { value: String },
    InvalidVectorIndex { value: String },
}
```

In `prax-migrate`, `MigrationError::SqlGeneration` covers any vector-related generation errors — no new variants.

## Data Flow

```
┌────────────────────────┐     ┌────────────────────────┐     ┌────────────────────────┐
│ schema.prax            │────▶│ prax-schema parser     │────▶│ SchemaDiff             │
│ Vector @dim(...)       │     │ (Vector + attrs)       │     │ + VectorColumnInfo     │
└────────────────────────┘     └────────────────────────┘     └────────────────────────┘
                                                                          │
                                                                          ▼
┌────────────────────────┐     ┌────────────────────────┐     ┌────────────────────────┐
│ up.sql / down.sql      │◀────│ SqliteGenerator        │◀────│ MigrationDialect       │
│ CREATE VIRTUAL TABLE   │     │ (vector-aware)         │     │ (SqlDialect::generate) │
└────────────────────────┘     └────────────────────────┘     └────────────────────────┘
         │
         ▼
┌────────────────────────┐     ┌────────────────────────┐     ┌────────────────────────┐
│ SQLite DB              │◀────│ SqlitePool             │◀────│ VectorSearchBuilder    │
│ + sqlite-vector-rs ext │     │ (auto-register ext)    │     │ HybridSearchBuilder    │
└────────────────────────┘     └────────────────────────┘     └────────────────────────┘
```

## Testing Strategy

### Unit tests

**prax-sqlite `vector` module** (inline `#[cfg(test)] mod tests`):
- `types::Embedding::new` rejects empty, stores dimensions correctly
- `DistanceMetric::as_sql()` and `VectorElementType::as_sql()` produce expected strings
- `VectorIndex::to_create_sql()` generates expected DDL for each metric × element-type combination
- `VectorSearchBuilder::build().to_sql()` — snapshot assertions against expected query shape
- `HybridSearchBuilder::build().to_sql()` — snapshot assertion for RRF structure
- Error variants construct and display correctly

**prax-migrate `SqliteGenerator`** (extend existing tests in `prax-migrate/src/sql.rs`):
- Vector field emits correct `CREATE VIRTUAL TABLE ... USING vector(...)` clause
- Multiple vector fields on one model produce one virtual table with multiple columns
- Down migration drops the virtual table before the main table
- Mix of regular + vector fields: main table has non-vector columns, virtual table has vector columns
- Each `VectorElementType` × `VectorDistanceMetric` × `VectorIndexKind` combination produces valid SQL

**prax-schema parser**:
- Valid `Vector @dim(1536) @vectorType("float4") @metric("cosine") @index(hnsw)` parses to expected AST
- Defaults: `Vector @dim(1536)` produces `float4` / `cosine` / no index
- Missing `@dim` returns `MissingVectorDimension`
- Invalid `@vectorType("invalid")` returns `InvalidVectorType`
- Invalid `@metric("manhattan")` returns `InvalidVectorMetric`

### Integration tests (`prax-sqlite/tests/vector_integration.rs`, feature-gated)

Only run with `cargo test -p prax-sqlite --features vector`:

- **Auto-registration**: create pool, open connection, verify `vector_from_json` SQL function is callable
- **End-to-end top-k search**:
  1. Create pool (in-memory SQLite)
  2. Run migration created from a schema with a vector column (via prax-migrate)
  3. Insert 100 random embeddings
  4. Query top-10 nearest to a target embedding using `VectorSearchBuilder`
  5. Assert results are ordered by distance
- **Hybrid search**: seed 50 documents with text + embeddings, create fts5 virtual table, query with both a text string and a vector, assert RRF ordering prefers documents matching both
- **Multiple vector columns**: model with two vector columns, verify both work in a single virtual table

### Migration tests (`prax-migrate/tests/sqlite_vector_migration.rs`)

- Full workflow: `CqlSchemaDiff` for SQLite with vector fields → generate → apply via rusqlite → verify table exists and has the right shape
- Round-trip: generate migration, apply, check `sqlite_master` for expected virtual table definition

### Parser tests (`prax-schema/tests/vector_parse.rs`)

- Fixture schemas with vector fields parse successfully
- Error cases produce the correct `SchemaError` variant

## Migration Path

No breaking changes. `vector` is an additive feature flag.

**For existing prax-sqlite users:**
- No changes required — feature off by default, existing schemas continue to work
- To enable: `prax-sqlite = { version = "0.7.4", features = ["vector"] }` + declare `Vector` fields in schema

**For existing prax-schema/prax-migrate users:**
- `FieldDiff::vector` is `Option<VectorColumnInfo>`, defaults to `None`
- `SqliteGenerator` passes through unchanged for non-vector fields
- Other generators (Postgres, MySQL, MSSQL, DuckDB) ignore the `vector` field entirely — vector fields on those dialects should be rejected at the schema-parse or SchemaDiffer level with a clear "vector columns are only supported on SQLite; use prax-pgvector for Postgres" error

## Dependencies

### New workspace dependency
- `sqlite-vector-rs = "0.2"`

### Modified
- `prax-sqlite/Cargo.toml` gains `vector` feature + optional `sqlite-vector-rs` dep
- No version bumps to existing dependency versions

## Success Criteria

**Functional:**
- `cargo build -p prax-sqlite --features vector` succeeds
- `cargo test -p prax-sqlite --features vector` passes (unit + integration)
- Auto-registration: fresh pool exposes `vector_from_json` SQL function
- Migration generation produces valid `CREATE VIRTUAL TABLE ... USING vector(...)` for schemas with vector fields
- `VectorSearchBuilder` returns results ordered by distance metric
- `HybridSearchBuilder` combines fts5 and vector scores via RRF

**Compatibility:**
- `cargo build -p prax-sqlite` (no features) remains identical — zero impact when feature off
- Existing `prax-sqlite`, `prax-migrate`, `prax-schema` tests unchanged
- Postgres/MySQL/SQLite/MSSQL/DuckDB generators remain byte-identical on non-vector schemas

**Testing:**
- Unit tests cover each DistanceMetric × VectorElementType combination for DDL generation
- Integration tests exercise the full schema → migration → runtime query path
- Parser tests cover every new attribute + error case

**Documentation:**
- `prax-sqlite/README.md` updated with vector quick-start (feature flag, schema example, search example)
- Each new module has crate-level doc comment
- Example at `prax-sqlite/examples/vector_rag.rs` demonstrating the full RAG workflow

## Future Work

- Embedding model inference helpers (e.g., integration with `candle` or `ort` for local models)
- Arrow-based bulk import/export (`vector_export_arrow` / `vector_insert_arrow` wrappers)
- Additional index types if sqlite-vector-rs adds them (IVFFlat, etc.)
- Cross-dialect vector abstraction: unified `Vector` type across pgvector + sqlite-vector so application code can target both
- `prax-codegen` derive support for vector columns in `#[derive(Model)]`
- Live benchmarks against prax-pgvector for equivalent workloads

## Example End-to-End

**schema.prax:**
```prax
model Document {
  id        Int    @id @auto
  title     String
  content   String
  embedding Vector @dim(1536) @vectorType("float4") @metric("cosine") @index(hnsw)
}
```

**Generated migration:**
```sql
-- up
CREATE TABLE "documents" (
    "id" INTEGER PRIMARY KEY AUTOINCREMENT,
    "title" TEXT NOT NULL,
    "content" TEXT NOT NULL
);

CREATE VIRTUAL TABLE "documents_vectors" USING vector(
    rowid_column='document_id',
    embedding='float4[1536] cosine hnsw'
);

-- down
DROP TABLE IF EXISTS "documents_vectors";
DROP TABLE IF EXISTS "documents";
```

**Application code:**
```rust
use prax_sqlite::{SqliteConfig, SqlitePool};
use prax_sqlite::vector::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect(SqliteConfig::from_path("./rag.db")).await?;

    // Insert embeddings (generated by user's embedding model)
    let embedding = Embedding::new(compute_embedding("my query text"));
    pool.execute(
        "INSERT INTO documents_vectors (document_id, embedding) VALUES (?, vector_from_json(?, 'float4'))",
        // ...
    ).await?;

    // Top-k retrieval
    let search = VectorSearchBuilder::new("documents", "embedding")
        .query(embedding)
        .metric(DistanceMetric::Cosine)
        .limit(10)
        .build();

    let results = pool.query_vector_search::<Document>(&search).await?;
    for doc in results {
        println!("{}: {}", doc.distance, doc.title);
    }

    Ok(())
}

fn compute_embedding(_text: &str) -> Vec<f32> {
    // User's embedding model here
    vec![0.1; 1536]
}
```
