//! End-to-end parser tests: .prax source → FieldAttributes payloads.

use prax_schema::ast::AggregateKind;

const SCHEMA: &str = r#"
datasource db {
    provider = "postgresql"
    url = env("DATABASE_URL")
}

model Post {
    id         Int    @id @auto
    author_id  Int
    title      String
    views      Int
    created_at String
}

model User {
    id         Int    @id @auto
    email      String @unique
    first_name String
    last_name  String
    posts      Post[] @relation(fields: [author_id], references: [id])

    full_name   String  @generated("first_name || ' ' || last_name") @stored
    search_key  String  @generated("LOWER(email)") @virtual

    post_count  Int     @count(posts)
    total_views Int     @sum(posts.views)
    last_post   String  @max(posts.created_at)
}
"#;

fn parse(source: &str) -> prax_schema::ast::Schema {
    prax_schema::parse_schema(source).expect("schema parses")
}

fn user_field<'a>(schema: &'a prax_schema::ast::Schema, name: &str) -> &'a prax_schema::ast::Field {
    let model = schema.get_model("User").expect("User model");
    model
        .get_field(name)
        .unwrap_or_else(|| panic!("field {name}"))
}

#[test]
fn parses_generated_stored_default() {
    let s = parse(SCHEMA);
    let g = user_field(&s, "full_name").generated().unwrap();
    assert_eq!(g.expression, "first_name || ' ' || last_name");
    assert!(g.stored);
}

#[test]
fn parses_generated_virtual() {
    let s = parse(SCHEMA);
    let g = user_field(&s, "search_key").generated().unwrap();
    assert!(!g.stored);
}

#[test]
fn parses_count() {
    let s = parse(SCHEMA);
    let a = user_field(&s, "post_count").aggregate().unwrap();
    assert_eq!(a.kind, AggregateKind::Count);
    assert_eq!(a.relation.as_str(), "posts");
    assert!(a.field.is_none());
}

#[test]
fn parses_sum_with_dotted_field() {
    let s = parse(SCHEMA);
    let a = user_field(&s, "total_views").aggregate().unwrap();
    assert_eq!(a.kind, AggregateKind::Sum);
    assert_eq!(a.relation.as_str(), "posts");
    assert_eq!(a.field.as_deref(), Some("views"));
}

#[test]
fn parses_max_with_dotted_field() {
    let s = parse(SCHEMA);
    let a = user_field(&s, "last_post").aggregate().unwrap();
    assert_eq!(a.kind, AggregateKind::Max);
    assert_eq!(a.relation.as_str(), "posts");
    assert_eq!(a.field.as_deref(), Some("created_at"));
}
