# prax-sqlite

SQLite query engine for Prax ORM.

## Overview

`prax-sqlite` provides an async SQLite backend on `rusqlite` (bundled SQLite), running each connection on its own thread.

## Features

- Async query execution with Tokio
- Connection pooling with reuse optimization
- WAL mode support for concurrent reads
- In-memory database support (see caveat below)
- Transaction support

> **In-memory caveat:** every connection opened for `sqlite::memory:` is its
> own isolated database, so an in-memory pool isolates every statement on its
> own database. Use `transaction()` to pin one connection, or a
> tempfile/shared-cache URI, for multi-statement workflows.

## Usage

```rust
use prax_orm::PraxClient;
use prax_sqlite::{SqliteEngine, SqlitePool};

// File-based database
let pool = SqlitePool::builder()
    .url("sqlite:./data.db")
    .build()
    .await?;
let engine = SqliteEngine::new(pool);

// In-memory database (see caveat above)
let pool = SqlitePool::builder()
    .url("sqlite::memory:")
    .build()
    .await?;
let engine = SqliteEngine::new(pool);

// Execute queries through Prax client
let client = PraxClient::new(engine);
let users = client.user().find_many().exec().await?;
```

## Performance

SQLite operations are highly optimized:
- **~145ns** connection acquisition (with pooling)
- WAL mode for concurrent read/write

## Vector Support (LLM / RAG)

Enable the `vector` feature to get typed vector columns, HNSW indexing,
and top-k similarity search backed by [sqlite-vector-rs](https://crates.io/crates/sqlite-vector-rs).

```toml
[dependencies]
prax-sqlite = { version = "0.7", features = ["vector"] }
```

sqlite-vector-rs is a loadable extension: build its library
(`cargo build --release --lib` in a checkout of
[sqlite-vector-rs](https://github.com/quinnjr/sqlite-vector-rs)) and point
`SQLITE_VECTOR_RS_LIB` at it, or place it next to your binary. Every new
connection opened by `SqlitePool` then loads the extension, so
`vector_from_json`, `knn_match` and the `vector` virtual table module are
available without extra setup.

### Schema

```prax
model Document {
  id        Int    @id @auto
  title     String
  content   String
  embedding Vector @dim(1536) @vectorType("float4") @metric("cosine") @index(hnsw)
}
```

`prax migrate` emits:

```sql
CREATE TABLE "documents" (
    "id" INTEGER PRIMARY KEY,
    "title" TEXT NOT NULL,
    "content" TEXT NOT NULL
);

CREATE VIRTUAL TABLE "documents_embedding_vectors" USING vector(dim=1536, type=float4, metric=cosine, mode=hnsw);
```

sqlite-vector-rs holds one vector per row, so each vector field gets its own
`{table}_{field}_vectors` table. Store a document's vector under the
document's id as the `rowid`:

```sql
INSERT INTO documents_embedding_vectors (rowid, vector)
VALUES (?1, vector_from_json(?2, 'float4'));
```

### Similarity search

```rust
use prax_sqlite::vector::prelude::*;

let embedding = Embedding::new(vec![/* 1536 floats */])?;

let sql = VectorSearchBuilder::new("documents", "embedding")
    .query_embedding(&embedding)
    .limit(10)
    .to_sql()?;
// SELECT "documents".*, v.distance AS distance
// FROM (SELECT rowid AS match_id, distance FROM "documents_embedding_vectors"
//       WHERE knn_match(distance, vector_from_json('[...]', 'float4')) LIMIT 10) v
// JOIN "documents" ON "documents"."id" = v.match_id
// ORDER BY v.distance ASC
```

The distance metric belongs to the vector table (`metric=` in its DDL), so
the search doesn't take one.

### Hybrid (vector + fts5) search via RRF

```rust
let sql = HybridSearchBuilder::new("documents")
    .vector_table(vector_table_name("documents", "embedding"))
    .fts_table("documents_fts")
    .query_embedding(&embedding)
    .query_text("large wild cats")
    .vector_weight(0.7)
    .text_weight(0.3)
    .limit(10)
    .to_sql()?;
```

See `examples/vector_rag.rs` for an end-to-end sample.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

