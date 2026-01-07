# prax-import

Import schemas from Prisma, Diesel, and SeaORM to Prax ORM.

## Features

- **Prisma Import**: Parse Prisma schema files (`.prisma`) and convert to Prax
- **Diesel Import**: Parse Diesel schema files (Rust code with `table!` macros) and convert to Prax
- **SeaORM Import**: Parse SeaORM entity files (Rust code with `DeriveEntityModel`) and convert to Prax
- **Type Mapping**: Automatic conversion of types between ORMs
- **Relation Mapping**: Preserve relations and foreign keys
- **Attribute Mapping**: Convert attributes and constraints

## Status

✅ **Production Ready** - Full support for Prisma, Diesel, and SeaORM schema imports with comprehensive test coverage.

### Supported Features

- ✅ Prisma schema parsing (models, fields, enums, relations)
- ✅ Diesel schema parsing (table! macros, joinables)
- ✅ SeaORM entity parsing (DeriveEntityModel, DeriveRelation)
- ✅ Type mapping with full Prax AST support
- ✅ Attribute conversion (@id, @unique, @default, @relation, etc.)
- ✅ CLI integration via `prax import` command
- ✅ Comprehensive test coverage

## Usage

### Import from Prisma

```rust
use prax_import::prisma::import_prisma_schema;

let prisma_schema = r#"
model User {
  id        Int      @id @default(autoincrement())
  email     String   @unique
  name      String?
  posts     Post[]
  createdAt DateTime @default(now())
}

model Post {
  id        Int      @id @default(autoincrement())
  title     String
  content   String?
  published Boolean  @default(false)
  authorId  Int
  author    User     @relation(fields: [authorId], references: [id])
}
"#;

let prax_schema = import_prisma_schema(prisma_schema)?;
// Write to .prax file
std::fs::write("schema.prax", format!("{}", prax_schema))?;
```

### Import from Diesel

```rust
use prax_import::diesel::import_diesel_schema;

let diesel_schema = r#"
table! {
    users (id) {
        id -> Int4,
        email -> Varchar,
        name -> Nullable<Varchar>,
        created_at -> Timestamp,
    }
}

table! {
    posts (id) {
        id -> Int4,
        title -> Varchar,
        content -> Nullable<Text>,
        published -> Bool,
        author_id -> Int4,
    }
}

joinable!(posts -> users (author_id));
"#;

let prax_schema = import_diesel_schema(diesel_schema)?;
// Write to .prax file
std::fs::write("schema.prax", format!("{}", prax_schema))?;
```

### Import from SeaORM

```rust
use prax_import::seaorm::import_seaorm_entity;

let seaorm_entity = r#"
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment)]
    pub id: i32,
    #[sea_orm(unique)]
    pub email: String,
    pub name: Option<String>,
    pub created_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}
"#;

let prax_schema = import_seaorm_entity(seaorm_entity)?;
// Write to .prax file
std::fs::write("schema.prax", format!("{}", prax_schema))?;
```

## CLI Usage

```bash
# Import from Prisma
prax import --from prisma --input schema.prisma --output schema.prax

# Import from Diesel
prax import --from diesel --input schema.rs --output schema.prax

# Import from SeaORM
prax import --from sea-orm --input entity/user.rs --output schema.prax

# Print to stdout instead of file
prax import --from prisma --input schema.prisma --print

# Force overwrite existing file
prax import --from diesel --input schema.rs --output schema.prax --force
```

## Type Mappings

### Prisma to Prax

| Prisma Type | Prax Type |
|-------------|-----------|
| `Int` | `Int` |
| `BigInt` | `BigInt` |
| `Float` | `Float` |
| `Decimal` | `Decimal` |
| `String` | `String` |
| `Boolean` | `Boolean` |
| `DateTime` | `DateTime` |
| `Json` | `Json` |
| `Bytes` | `Bytes` |

### Diesel to Prax

| Diesel Type | Prax Type |
|-------------|-----------|
| `Int4` | `Int` |
| `Int8` | `BigInt` |
| `Float4`, `Float8` | `Float` |
| `Numeric` | `Decimal` |
| `Varchar`, `Text` | `String` |
| `Bool` | `Boolean` |
| `Timestamp` | `DateTime` |
| `Json`, `Jsonb` | `Json` |
| `Bytea` | `Bytes` |

### SeaORM to Prax

| SeaORM Type | Prax Type |
|-------------|-----------|
| `i32`, `i64` | `Int` |
| `f32`, `f64` | `Float` |
| `String` | `String` |
| `bool` | `Boolean` |
| `DateTime` | `DateTime` |
| `Date` | `Date` |
| `Time` | `Time` |
| `Decimal` | `Decimal` |
| `serde_json::Value` | `Json` |
| `Vec<u8>` | `Bytes` |
| `Uuid` | `Uuid` |

## Performance

The import crate is highly optimized for performance:

- **Prisma**: ~7,675 small schemas/sec, ~1,221 medium schemas/sec
- **Diesel**: ~8,135 small schemas/sec, ~1,625 medium schemas/sec
- **SeaORM**: ~7,799 small schemas/sec, ~5,049 medium schemas/sec

Recent optimizations include regex compilation caching, resulting in **1.4-2.3x speedup** for Prisma and Diesel imports.

See [BENCHMARKS.md](BENCHMARKS.md) for detailed performance metrics.

## Testing

Run the test suite:

```bash
# Test all import functionality
cargo test -p prax-import --all-features

# Test specific ORM import
cargo test -p prax-import --features prisma
cargo test -p prax-import --features diesel
cargo test -p prax-import --features seaorm
```

## Benchmarking

Run performance benchmarks:

```bash
# Run all benchmarks
cargo bench -p prax-import --all-features

# View HTML reports
open target/criterion/report/index.html
```

## Contributing

Contributions are welcome! To add support for a new ORM:

1. Create a new module in `src/<orm_name>/`
2. Define intermediate types in `src/<orm_name>/types.rs`
3. Implement parser in `src/<orm_name>/parser.rs`
4. Use the converter builders in `src/converter.rs`
5. Add comprehensive tests
6. Update CLI integration in `prax-cli/src/commands/import.rs`

## License

Dual-licensed under MIT or Apache-2.0, matching the main Prax ORM project.
