use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use prax_import::prelude::*;

// Sample schemas for benchmarking
const SMALL_PRISMA_SCHEMA: &str = r#"
model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  name  String?
}
"#;

const MEDIUM_PRISMA_SCHEMA: &str = r#"
model User {
  id        Int      @id @default(autoincrement())
  email     String   @unique
  name      String?
  createdAt DateTime @default(now())
  updatedAt DateTime @updatedAt
  posts     Post[]
}

model Post {
  id        Int      @id @default(autoincrement())
  title     String
  content   String?
  published Boolean  @default(false)
  authorId  Int
  author    User     @relation(fields: [authorId], references: [id])
  tags      Tag[]
}

model Tag {
  id    Int    @id @default(autoincrement())
  name  String @unique
  posts Post[]
}
"#;

const LARGE_PRISMA_SCHEMA: &str = r#"
model User {
  id        Int      @id @default(autoincrement())
  email     String   @unique
  name      String?
  password  String
  role      Role     @default(USER)
  createdAt DateTime @default(now())
  updatedAt DateTime @updatedAt
  profile   Profile?
  posts     Post[]
  comments  Comment[]
  @@index([email])
}

model Profile {
  id        Int      @id @default(autoincrement())
  bio       String?
  avatar    String?
  userId    Int      @unique
  user      User     @relation(fields: [userId], references: [id])
}

model Post {
  id        Int      @id @default(autoincrement())
  title     String
  content   String?
  published Boolean  @default(false)
  authorId  Int
  author    User     @relation(fields: [authorId], references: [id])
  comments  Comment[]
  tags      Tag[]
  createdAt DateTime @default(now())
  updatedAt DateTime @updatedAt
  @@index([authorId])
  @@index([published])
}

model Comment {
  id        Int      @id @default(autoincrement())
  content   String
  postId    Int
  post      Post     @relation(fields: [postId], references: [id])
  authorId  Int
  author    User     @relation(fields: [authorId], references: [id])
  createdAt DateTime @default(now())
  @@index([postId])
  @@index([authorId])
}

model Tag {
  id    Int    @id @default(autoincrement())
  name  String @unique
  posts Post[]
}

enum Role {
  USER
  ADMIN
  MODERATOR
}
"#;

const SMALL_DIESEL_SCHEMA: &str = r#"
table! {
    users (id) {
        id -> Int4,
        email -> Varchar,
        name -> Nullable<Varchar>,
    }
}
"#;

const MEDIUM_DIESEL_SCHEMA: &str = r#"
table! {
    users (id) {
        id -> Int4,
        email -> Varchar,
        name -> Nullable<Varchar>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    posts (id) {
        id -> Int4,
        title -> Varchar,
        content -> Nullable<Text>,
        published -> Bool,
        author_id -> Int4,
        created_at -> Timestamp,
    }
}

joinable!(posts -> users (author_id));
"#;

const LARGE_DIESEL_SCHEMA: &str = r#"
table! {
    users (id) {
        id -> Int4,
        email -> Varchar,
        name -> Nullable<Varchar>,
        password -> Varchar,
        role -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    profiles (id) {
        id -> Int4,
        bio -> Nullable<Text>,
        avatar -> Nullable<Varchar>,
        user_id -> Int4,
    }
}

table! {
    posts (id) {
        id -> Int4,
        title -> Varchar,
        content -> Nullable<Text>,
        published -> Bool,
        author_id -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    comments (id) {
        id -> Int4,
        content -> Text,
        post_id -> Int4,
        author_id -> Int4,
        created_at -> Timestamp,
    }
}

table! {
    tags (id) {
        id -> Int4,
        name -> Varchar,
    }
}

joinable!(profiles -> users (user_id));
joinable!(posts -> users (author_id));
joinable!(comments -> posts (post_id));
joinable!(comments -> users (author_id));
"#;

const SMALL_SEAORM_ENTITY: &str = r#"
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment)]
    pub id: i32,
    pub email: String,
    pub name: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
"#;

const MEDIUM_SEAORM_ENTITY: &str = r#"
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment)]
    pub id: i32,
    #[sea_orm(unique)]
    pub email: String,
    pub name: Option<String>,
    pub password: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}
"#;

const LARGE_SEAORM_ENTITY: &str = r#"
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment)]
    pub id: i32,
    #[sea_orm(unique)]
    pub email: String,
    pub name: Option<String>,
    pub password: String,
    pub role: String,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub verified: bool,
    pub last_login: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
    #[sea_orm(has_many = "super::comment::Entity")]
    Comments,
    #[sea_orm(has_one = "super::profile::Entity")]
    Profile,
}
"#;

fn bench_prisma_import(c: &mut Criterion) {
    let mut group = c.benchmark_group("prisma_import");

    group.bench_with_input(
        BenchmarkId::from_parameter("small"),
        &SMALL_PRISMA_SCHEMA,
        |b, schema| {
            b.iter(|| import_prisma_schema(black_box(*schema)).unwrap());
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("medium"),
        &MEDIUM_PRISMA_SCHEMA,
        |b, schema| {
            b.iter(|| import_prisma_schema(black_box(*schema)).unwrap());
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("large"),
        &LARGE_PRISMA_SCHEMA,
        |b, schema| {
            b.iter(|| import_prisma_schema(black_box(*schema)).unwrap());
        },
    );

    group.finish();
}

fn bench_diesel_import(c: &mut Criterion) {
    let mut group = c.benchmark_group("diesel_import");

    group.bench_with_input(
        BenchmarkId::from_parameter("small"),
        &SMALL_DIESEL_SCHEMA,
        |b, schema| {
            b.iter(|| import_diesel_schema(black_box(*schema)).unwrap());
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("medium"),
        &MEDIUM_DIESEL_SCHEMA,
        |b, schema| {
            b.iter(|| import_diesel_schema(black_box(*schema)).unwrap());
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("large"),
        &LARGE_DIESEL_SCHEMA,
        |b, schema| {
            b.iter(|| import_diesel_schema(black_box(*schema)).unwrap());
        },
    );

    group.finish();
}

#[cfg(feature = "seaorm")]
fn bench_seaorm_import(c: &mut Criterion) {
    use prax_import::seaorm::import_seaorm_entity;

    let mut group = c.benchmark_group("seaorm_import");

    group.bench_with_input(
        BenchmarkId::from_parameter("small"),
        &SMALL_SEAORM_ENTITY,
        |b, schema| {
            b.iter(|| import_seaorm_entity(black_box(*schema)).unwrap());
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("medium"),
        &MEDIUM_SEAORM_ENTITY,
        |b, schema| {
            b.iter(|| import_seaorm_entity(black_box(*schema)).unwrap());
        },
    );

    group.bench_with_input(
        BenchmarkId::from_parameter("large"),
        &LARGE_SEAORM_ENTITY,
        |b, schema| {
            b.iter(|| import_seaorm_entity(black_box(*schema)).unwrap());
        },
    );

    group.finish();
}

#[cfg(not(feature = "seaorm"))]
fn bench_seaorm_import(_c: &mut Criterion) {
    // Skip SeaORM benchmarks if feature is not enabled
}

criterion_group!(
    benches,
    bench_prisma_import,
    bench_diesel_import,
    bench_seaorm_import
);
criterion_main!(benches);
