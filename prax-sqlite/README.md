# prax-sqlite

SQLite query engine for Prax ORM.

## Overview

`prax-sqlite` provides an async SQLite backend using `tokio-rusqlite`.

## Features

- Async query execution with Tokio
- Connection pooling with reuse optimization
- WAL mode support for concurrent reads
- In-memory database support
- Transaction support

## Usage

```rust
use prax_sqlite::SqliteEngine;

// File-based database
let engine = SqliteEngine::new("sqlite:./data.db").await?;

// In-memory database
let engine = SqliteEngine::new("sqlite::memory:").await?;

// Execute queries through Prax client
let client = PraxClient::with_engine(engine);
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

When the feature is enabled, every new connection opened by `SqlitePool`
auto-registers the extension, so `vector_from_json`, `vector_distance`,
and the `vector` virtual table module are available without extra setup.

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

CREATE VIRTUAL TABLE "documents_vectors" USING vector(
    rowid_column='document_id',
    embedding='float4[1536] cosine hnsw'
);
```

### Similarity search

```rust
use prax_sqlite::vector::prelude::*;

let embedding = Embedding::new(vec![/* 1536 floats */])?;

let sql = VectorSearchBuilder::new("documents", "embedding")
    .query_embedding(&embedding)
    .metric(DistanceMetric::Cosine)
    .limit(10)
    .to_sql()?;
```

### Hybrid (vector + fts5) search via RRF

```rust
let sql = HybridSearchBuilder::new("documents")
    .vector_table("documents_vectors")
    .rowid_column("document_id")
    .vector_column("embedding")
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

