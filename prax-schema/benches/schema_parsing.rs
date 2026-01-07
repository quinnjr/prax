//! Benchmarks for schema parsing operations.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use prax_schema::parser::parse_schema;

/// A minimal schema with a single model.
const MINIMAL_SCHEMA: &str = r#"
model User {
    id    Int    @id @auto
    name  String
}
"#;

/// A small schema with a few models and relations.
const SMALL_SCHEMA: &str = r#"
model User {
    id        Int      @id @auto
    email     String   @unique
    name      String?
    posts     Post[]
    profile   Profile?
    createdAt DateTime @default(now())
}

model Post {
    id        Int      @id @auto
    title     String
    content   String?
    published Boolean  @default(false)
    author    User     @relation(fields: [authorId], references: [id])
    authorId  Int
    createdAt DateTime @default(now())
}

model Profile {
    id     Int    @id @auto
    bio    String?
    user   User   @relation(fields: [userId], references: [id])
    userId Int    @unique
}
"#;

/// A medium schema with multiple models, enums, and complex relations.
const MEDIUM_SCHEMA: &str = r#"
enum Role {
    USER
    ADMIN
    MODERATOR
}

enum PostStatus {
    DRAFT
    PUBLISHED
    ARCHIVED
}

model User {
    id        Int       @id @auto
    email     String    @unique
    name      String?
    role      Role      @default(USER)
    posts     Post[]
    comments  Comment[]
    profile   Profile?
    followers Follow[]  @relation("followers")
    following Follow[]  @relation("following")
    createdAt DateTime  @default(now())
    updatedAt DateTime  @updatedAt
}

model Post {
    id        Int        @id @auto
    title     String
    content   String?
    status    PostStatus @default(DRAFT)
    author    User       @relation(fields: [authorId], references: [id])
    authorId  Int
    comments  Comment[]
    tags      Tag[]
    createdAt DateTime   @default(now())
    updatedAt DateTime   @updatedAt
}

model Comment {
    id        Int      @id @auto
    content   String
    author    User     @relation(fields: [authorId], references: [id])
    authorId  Int
    post      Post     @relation(fields: [postId], references: [id])
    postId    Int
    createdAt DateTime @default(now())
}

model Profile {
    id        Int      @id @auto
    bio       String?
    avatar    String?
    website   String?
    user      User     @relation(fields: [userId], references: [id])
    userId    Int      @unique
    updatedAt DateTime @updatedAt
}

model Tag {
    id    Int    @id @auto
    name  String @unique
    posts Post[]
}

model Follow {
    id          Int      @id @auto
    follower    User     @relation("followers", fields: [followerId], references: [id])
    followerId  Int
    following   User     @relation("following", fields: [followingId], references: [id])
    followingId Int
    createdAt   DateTime @default(now())

    @@unique([followerId, followingId])
}
"#;

/// A large schema with many models and complex attributes.
fn generate_large_schema(model_count: usize) -> String {
    let mut schema = String::new();

    // Add some enums
    schema.push_str(
        r#"
enum Status {
    ACTIVE
    INACTIVE
    PENDING
}

"#,
    );

    // Generate models
    for i in 0..model_count {
        schema.push_str(&format!(
            r#"
model Model{i} {{
    id          Int      @id @auto
    name        String
    description String?
    status      Status   @default(ACTIVE)
    value       Float?
    count       Int      @default(0)
    active      Boolean  @default(true)
    metadata    Json?
    createdAt   DateTime @default(now())
    updatedAt   DateTime @updatedAt

    @@index([name])
    @@index([status, createdAt])
}}
"#
        ));
    }

    schema
}

/// Benchmark minimal schema parsing.
fn bench_parse_minimal(c: &mut Criterion) {
    c.bench_function("parse_minimal_schema", |b| {
        b.iter(|| black_box(parse_schema(MINIMAL_SCHEMA).unwrap()))
    });
}

/// Benchmark small schema parsing.
fn bench_parse_small(c: &mut Criterion) {
    c.bench_function("parse_small_schema", |b| {
        b.iter(|| black_box(parse_schema(SMALL_SCHEMA).unwrap()))
    });
}

/// Benchmark medium schema parsing.
fn bench_parse_medium(c: &mut Criterion) {
    c.bench_function("parse_medium_schema", |b| {
        b.iter(|| black_box(parse_schema(MEDIUM_SCHEMA).unwrap()))
    });
}

/// Benchmark large schema parsing with varying model counts.
fn bench_parse_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_large_schema");

    for model_count in [10, 25, 50, 100].iter() {
        let schema = generate_large_schema(*model_count);
        group.throughput(Throughput::Bytes(schema.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("models", model_count),
            &schema,
            |b, schema| b.iter(|| black_box(parse_schema(schema).unwrap())),
        );
    }

    group.finish();
}

/// Benchmark schema with many attributes.
fn bench_parse_with_attributes(c: &mut Criterion) {
    let schema_with_validation = r#"
model User {
    /// The user's unique identifier
    id Int @id @auto

    /// The user's email address
    /// @validate(email)
    email String @unique

    /// The user's display name
    /// @validate(minLength: 2, maxLength: 100)
    name String

    /// The user's age (must be 18+)
    /// @validate(min: 18, max: 150)
    age Int?

    /// User's website URL
    /// @validate(url)
    website String?

    /// User's phone number
    /// @validate(pattern: "^\+?[0-9]{10,15}$")
    phone String?

    /// User's bio
    /// @validate(maxLength: 1000)
    bio String?

    /// @hidden
    passwordHash String

    /// @deprecated("Use 'email' instead")
    username String?

    createdAt DateTime @default(now())
    updatedAt DateTime @updatedAt

    @@index([email])
    @@index([name])
}
"#;

    c.bench_function("parse_schema_with_validation", |b| {
        b.iter(|| black_box(parse_schema(schema_with_validation).unwrap()))
    });
}

/// Benchmark schema with views.
fn bench_parse_with_views(c: &mut Criterion) {
    let schema_with_views = r#"
model User {
    id    Int    @id @auto
    email String @unique
    name  String
    posts Post[]
}

model Post {
    id        Int      @id @auto
    title     String
    content   String?
    authorId  Int
    author    User     @relation(fields: [authorId], references: [id])
    createdAt DateTime @default(now())
}

view UserPostCount {
    userId    Int
    userName  String
    postCount Int

    @@sql("SELECT u.id as userId, u.name as userName, COUNT(p.id) as postCount FROM User u LEFT JOIN Post p ON p.authorId = u.id GROUP BY u.id, u.name")
}

view RecentPosts {
    postId    Int
    title     String
    authorName String
    createdAt DateTime

    @@sql("SELECT p.id as postId, p.title, u.name as authorName, p.createdAt FROM Post p JOIN User u ON u.id = p.authorId ORDER BY p.createdAt DESC")
}
"#;

    c.bench_function("parse_schema_with_views", |b| {
        b.iter(|| black_box(parse_schema(schema_with_views).unwrap()))
    });
}

/// Benchmark repeated parsing (cache warmup simulation).
fn bench_repeated_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("repeated_parsing");

    group.bench_function("parse_10_times", |b| {
        b.iter(|| {
            for _ in 0..10 {
                black_box(parse_schema(MEDIUM_SCHEMA).unwrap());
            }
        })
    });

    group.finish();
}

/// Benchmark string handling in schema.
fn bench_string_heavy_schema(c: &mut Criterion) {
    // Generate a schema with many long string defaults
    let mut schema = String::from("model Config {\n    id Int @id @auto\n");
    for i in 0..20 {
        schema.push_str(&format!(
            r#"    field{} String @default("{}")"#,
            i,
            "x".repeat(100)
        ));
        schema.push('\n');
    }
    schema.push_str("}\n");

    c.bench_function("parse_string_heavy_schema", |b| {
        b.iter(|| black_box(parse_schema(&schema).unwrap()))
    });
}

criterion_group!(
    benches,
    bench_parse_minimal,
    bench_parse_small,
    bench_parse_medium,
    bench_parse_large,
    bench_parse_with_attributes,
    bench_parse_with_views,
    bench_repeated_parsing,
    bench_string_heavy_schema,
);

criterion_main!(benches);
