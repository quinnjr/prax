//! Validator coverage for @generated and aggregate field combinations.

use prax_schema::SchemaError;

/// Parse and validate the given schema text; return an error string that
/// contains the concatenated messages of every sub-error inside
/// `ValidationFailed`, or the top-level error text if it isn't that variant.
fn error_text(schema_text: &str) -> String {
    match prax_schema::validate_schema(schema_text) {
        Ok(_) => String::new(), // no error
        Err(SchemaError::ValidationFailed { errors, .. }) => errors
            .iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => format!("{e}"),
    }
}

fn assert_error_contains(schema_text: &str, expected_fragment: &str) {
    let result = prax_schema::validate_schema(schema_text);
    assert!(
        result.is_err(),
        "expected validation error containing `{expected_fragment}`, but schema parsed clean"
    );
    let text = error_text(schema_text);
    assert!(
        text.contains(expected_fragment),
        "expected `{expected_fragment}` in error output, got:\n{text}"
    );
}

// ── helpers ────────────────────────────────────────────────────────────────

const DS: &str = r#"
datasource db {
    provider = "postgresql"
    url = env("DATABASE_URL")
}
"#;

// ── tests ──────────────────────────────────────────────────────────────────

#[test]
fn rejects_generated_with_id() {
    let schema = format!(
        r#"{DS}
model User {{
    id Int @id @auto @generated("1")
}}
"#
    );
    assert_error_contains(&schema, "cannot be both @generated and @id/@auto");
}

#[test]
fn rejects_empty_generated_expression() {
    let schema = format!(
        r#"{DS}
model User {{
    id   Int    @id @auto
    name String @generated("   ")
}}
"#
    );
    assert_error_contains(&schema, "@generated expression must not be empty");
}

#[test]
fn rejects_count_with_field_path() {
    let schema = format!(
        r#"{DS}
model Post {{
    id        Int @id @auto
    author_id Int
}}

model User {{
    id    Int    @id @auto
    posts Post[]  @relation(fields: [author_id], references: [id])
    bad   Int     @count(posts.id)
}}
"#
    );
    assert_error_contains(&schema, "@count takes a relation name");
}

#[test]
fn rejects_sum_without_field() {
    let schema = format!(
        r#"{DS}
model Post {{
    id        Int @id @auto
    author_id Int
    views     Int
}}

model User {{
    id    Int    @id @auto
    posts Post[]  @relation(fields: [author_id], references: [id])
    bad   Int     @sum(posts)
}}
"#
    );
    assert_error_contains(&schema, "requires `relation.field`");
}

#[test]
fn rejects_unknown_relation_in_count() {
    let schema = format!(
        r#"{DS}
model User {{
    id  Int @id @auto
    bad Int @count(nope)
}}
"#
    );
    assert_error_contains(&schema, "unknown relation `nope`");
}

#[test]
fn rejects_orphan_stored() {
    let schema = format!(
        r#"{DS}
model User {{
    id   Int    @id @auto
    name String @stored
}}
"#
    );
    assert_error_contains(&schema, "@stored is only valid alongside @generated");
}
