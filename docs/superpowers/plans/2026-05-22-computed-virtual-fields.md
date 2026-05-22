# Computed and Virtual Fields Implementation Plan (Phase 5.5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land phase 5.5 of the typed-query-traits initiative — DB-generated columns (`@generated`), relation-aggregate virtual fields (`@count`/`@sum`/`@avg`/`@min`/`@max`), and the `select: { _count: { rel: true } }` ad-hoc accessor — in a single PR.

**Architecture:** Schema AST grows two structured attribute payloads (`GeneratedAttribute`, `AggregateAttribute`) on `FieldAttributes`. Parser & derive both produce them. `prax-migrate` per-dialect generators emit `GENERATED ALWAYS AS (...)` DDL behind a `SupportsGeneratedColumns` marker; CQL rejects. `prax-query` reuses the existing `Filter::ScalarSubquery` IR variant for WHERE/ORDER BY aggregates and gains a new `ScalarProjection` runtime type for SELECT; both gated on `SupportsScalarSubqueryInSelect`. Codegen excludes both classes from Create/Update inputs, includes them in Where/Select/OrderBy, emits a `<Model>Count` substruct, and adds a `_count` key to the `select:` macro DSL.

**Tech Stack:** Rust 2024, Cargo workspace. Uses `syn` / `pest` for codegen / schema parsing; `trybuild` for macro UI; `insta`-style snapshot tests where they already exist; live-Postgres integration via the existing testcontainer harness in `prax-postgres/tests/`.

**Spec:** `docs/superpowers/specs/2026-05-22-computed-virtual-fields-design.md`

**Worktree:** `/home/joseph/Projects/prax/.worktrees/computed-virtual-fields/`, branch `feature/computed-virtual-fields`.

---

## Task 1: Baseline check

- [ ] **Step 1:** Confirm worktree state:
  ```
  cd /home/joseph/Projects/prax/.worktrees/computed-virtual-fields
  git rev-parse --abbrev-ref HEAD     # feature/computed-virtual-fields
  git log --oneline -2                # 24e995e + 14f2d82 (spec + clarification)
  ```
- [ ] **Step 2:** `cargo check --workspace --all-features` — zero errors.
- [ ] **Step 3:** `cargo test -p prax-schema --lib` — green baseline.
- [ ] **Step 4:** No commit.

---

## Task 2: Schema AST — `GeneratedAttribute` and `AggregateAttribute` on `FieldAttributes`

**Files:**
- Modify: `prax-schema/src/ast/attribute.rs`
- Modify: `prax-schema/src/ast/mod.rs` (re-exports)
- Test: `prax-schema/tests/computed_field_attributes.rs` (new)

- [ ] **Step 1:** In `prax-schema/src/ast/attribute.rs`, after `RelationAttribute`, add two new structs and an enum:

  ```rust
  /// Aggregate kind for relation-aggregate virtual fields.
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
  pub enum AggregateKind {
      Count,
      Sum,
      Avg,
      Min,
      Max,
  }

  /// `@generated("expr") @stored|@virtual` attribute.
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct GeneratedAttribute {
      /// SQL expression text. Emitted verbatim into DDL — no dialect translation.
      pub expression: String,
      /// True if STORED, false if VIRTUAL. Default is STORED.
      pub stored: bool,
  }

  /// `@count(rel)` / `@sum(rel.field)` / etc.
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct AggregateAttribute {
      pub kind: AggregateKind,
      /// Outgoing relation name on the parent model.
      pub relation: SmolStr,
      /// Target field on the related model. Required for non-Count.
      pub field: Option<SmolStr>,
  }
  ```

- [ ] **Step 2:** Add two fields to `FieldAttributes` (around line 207):

  ```rust
  /// `@generated("...")` attribute, if present.
  pub generated: Option<GeneratedAttribute>,
  /// Relation-aggregate attribute (`@count`/`@sum`/etc.), if present.
  pub aggregate: Option<AggregateAttribute>,
  ```

- [ ] **Step 3:** Add convenience accessors on `Field` (in `prax-schema/src/ast/field.rs`):

  ```rust
  pub fn is_generated(&self) -> bool { self.has_attribute("generated") }
  pub fn is_aggregate(&self) -> bool {
      self.has_attribute("count") || self.has_attribute("sum")
          || self.has_attribute("avg") || self.has_attribute("min")
          || self.has_attribute("max")
  }
  /// True if this field is computed by the database or by a query-time
  /// aggregate. Such fields are excluded from CreateInput and UpdateInput.
  pub fn is_computed(&self) -> bool { self.is_generated() || self.is_aggregate() }
  ```

- [ ] **Step 4:** Re-export `GeneratedAttribute`, `AggregateAttribute`, `AggregateKind` from `prax-schema/src/ast/mod.rs`.

- [ ] **Step 5:** Write `prax-schema/tests/computed_field_attributes.rs`:

  ```rust
  use prax_schema::ast::{
      AggregateAttribute, AggregateKind, FieldAttributes, GeneratedAttribute,
  };

  #[test]
  fn generated_attribute_round_trip() {
      let g = GeneratedAttribute { expression: "a || b".into(), stored: true };
      let json = serde_json::to_string(&g).unwrap();
      let back: GeneratedAttribute = serde_json::from_str(&json).unwrap();
      assert_eq!(g, back);
  }

  #[test]
  fn aggregate_count_has_no_field() {
      let a = AggregateAttribute { kind: AggregateKind::Count, relation: "posts".into(), field: None };
      assert_eq!(a.kind, AggregateKind::Count);
      assert!(a.field.is_none());
  }

  #[test]
  fn aggregate_sum_has_field() {
      let a = AggregateAttribute { kind: AggregateKind::Sum, relation: "posts".into(), field: Some("views".into()) };
      assert_eq!(a.field.as_deref(), Some("views"));
  }

  #[test]
  fn field_attributes_default_has_none() {
      let f = FieldAttributes::default();
      assert!(f.generated.is_none());
      assert!(f.aggregate.is_none());
  }
  ```

- [ ] **Step 6:** `cargo test -p prax-schema --test computed_field_attributes` — green.
- [ ] **Step 7:** Commit:

  ```
  feat(schema): add GeneratedAttribute and AggregateAttribute to FieldAttributes
  ```

---

## Task 3: `.prax` parser support for `@generated` / `@stored` / `@virtual` / aggregates

**Files:**
- Modify: `prax-schema/src/parser/prax.pest`
- Modify: `prax-schema/src/parser/grammar.rs`
- Modify: `prax-schema/src/ast/attribute.rs` (extraction logic in `FieldAttributes::from_attributes` or wherever `Attribute → FieldAttributes` happens)
- Test: `prax-schema/tests/computed_field_parser.rs` (new)

- [ ] **Step 1:** Inspect the `.pest` grammar to confirm field attributes already accept arbitrary `@name(...)` forms:

  ```
  grep -n "attribute\|@" prax-schema/src/parser/prax.pest | head
  ```
  Most directives are parsed generically as `Attribute { name, args }` — no grammar changes needed if so. If a hard-coded list exists (e.g. `field_attribute = { "id" | "unique" | ... }`), extend it to include `generated`, `stored`, `virtual`, `count`, `sum`, `avg`, `min`, `max`.

- [ ] **Step 2:** Find the conversion path from `Vec<Attribute>` → `FieldAttributes` (try `grep -rn "fn extract_attributes\|FieldAttributes::from" prax-schema/src/`) and extend it:

  ```rust
  // For each attribute in field.attributes:
  match attr.name.as_str() {
      // ... existing cases ...
      "generated" => {
          let expression = attr
              .first_string_arg()
              .ok_or_else(|| validation_error(&attr.span, "@generated requires a string expression"))?;
          // Look for @stored / @virtual on the same field.
          let stored = !field.has_attribute("virtual");
          attrs.generated = Some(GeneratedAttribute { expression, stored });
      }
      "stored" | "virtual" => {
          // Consumed by the @generated branch above; verify it pairs with @generated.
          if !field.has_attribute("generated") {
              return Err(validation_error(
                  &attr.span,
                  "@stored / @virtual only valid alongside @generated",
              ));
          }
      }
      "count" => {
          let rel = attr.first_ident_arg().ok_or_else(|| ...)?;
          if attr.args.len() > 1 {
              return Err(validation_error(&attr.span, "@count takes exactly one relation name"));
          }
          attrs.aggregate = Some(AggregateAttribute {
              kind: AggregateKind::Count, relation: rel.into(), field: None,
          });
      }
      "sum" | "avg" | "min" | "max" => {
          let path = attr.first_path_arg().ok_or_else(|| ...)?; // expects `rel.field`
          let (rel, field) = path.split_once('.').ok_or_else(|| {
              validation_error(&attr.span, &format!("@{} requires `relation.field` form", attr.name))
          })?;
          let kind = match attr.name.as_str() {
              "sum" => AggregateKind::Sum,
              "avg" => AggregateKind::Avg,
              "min" => AggregateKind::Min,
              "max" => AggregateKind::Max,
              _ => unreachable!(),
          };
          attrs.aggregate = Some(AggregateAttribute {
              kind, relation: rel.into(), field: Some(field.into()),
          });
      }
      _ => { /* existing fallthrough */ }
  }
  ```

  If `first_string_arg`/`first_ident_arg`/`first_path_arg` helpers don't exist, add minimal versions in `prax-schema/src/ast/attribute.rs::impl Attribute`.

- [ ] **Step 3:** Add a `Field::generated()` / `Field::aggregate()` accessor wrapping `extract_attributes()`:

  ```rust
  pub fn generated(&self) -> Option<GeneratedAttribute> { self.extract_attributes().generated.clone() }
  pub fn aggregate(&self) -> Option<AggregateAttribute> { self.extract_attributes().aggregate.clone() }
  ```

- [ ] **Step 4:** Write `prax-schema/tests/computed_field_parser.rs`:

  ```rust
  use prax_schema::ast::AggregateKind;
  use prax_schema::parser::parse_schema;

  const SCHEMA: &str = r#"
  datasource db { provider = "postgresql"; url = env("DATABASE_URL") }

  model Post {
      id        Int    @id @auto
      author_id Int
      title     String
      views     Int
      created_at DateTime
  }

  model User {
      id         Int    @id @auto
      email      String @unique
      first_name String
      last_name  String
      posts      Post[] @relation(fields: [], references: [])

      full_name  String   @generated("first_name || ' ' || last_name") @stored
      search_key String   @generated("LOWER(email)") @virtual

      post_count  Int      @count(posts)
      total_views Int      @sum(posts.views)
      last_post   DateTime? @max(posts.created_at)
  }
  "#;

  #[test]
  fn parses_generated_stored() {
      let schema = parse_schema(SCHEMA).expect("schema parses");
      let user = schema.find_model("User").unwrap();
      let f = user.find_field("full_name").unwrap();
      let g = f.generated().unwrap();
      assert_eq!(g.expression, "first_name || ' ' || last_name");
      assert!(g.stored);
  }

  #[test]
  fn parses_generated_virtual() {
      let schema = parse_schema(SCHEMA).unwrap();
      let user = schema.find_model("User").unwrap();
      let g = user.find_field("search_key").unwrap().generated().unwrap();
      assert!(!g.stored);
  }

  #[test]
  fn parses_count() {
      let schema = parse_schema(SCHEMA).unwrap();
      let f = schema.find_model("User").unwrap().find_field("post_count").unwrap();
      let a = f.aggregate().unwrap();
      assert_eq!(a.kind, AggregateKind::Count);
      assert_eq!(a.relation.as_str(), "posts");
      assert!(a.field.is_none());
  }

  #[test]
  fn parses_sum_with_dotted_field() {
      let schema = parse_schema(SCHEMA).unwrap();
      let a = schema
          .find_model("User").unwrap()
          .find_field("total_views").unwrap()
          .aggregate().unwrap();
      assert_eq!(a.kind, AggregateKind::Sum);
      assert_eq!(a.relation.as_str(), "posts");
      assert_eq!(a.field.as_deref(), Some("views"));
  }
  ```

  If the test helpers (`Schema::find_model`, `Model::find_field`) don't exist, use `schema.models.iter().find(|m| m.name() == "User")` etc.

- [ ] **Step 5:** Run `cargo test -p prax-schema --test computed_field_parser` until green.

- [ ] **Step 6:** Commit:

  ```
  feat(schema): parse @generated and aggregate directives in .prax files
  ```

---

## Task 4: Schema validation — illegal combinations and error messages

**Files:**
- Modify: `prax-schema/src/validator.rs`
- Test: `prax-schema/tests/computed_field_validation.rs` (new)

- [ ] **Step 1:** Locate the validator entry point: `grep -n "validate\|Validator" prax-schema/src/validator.rs | head`.

- [ ] **Step 2:** Add `validate_computed_fields(schema, errors)`:

  ```rust
  fn validate_computed_fields(schema: &Schema, errors: &mut Vec<ValidationError>) {
      for model in &schema.models {
          for field in &model.fields {
              let attrs = field.extract_attributes();
              if let Some(g) = &attrs.generated {
                  if attrs.is_id || attrs.is_auto {
                      errors.push(ValidationError::new(
                          field.span.clone(),
                          format!("field `{}` cannot be both @generated and @id/@auto", field.name()),
                      ));
                  }
                  if attrs.aggregate.is_some() {
                      errors.push(ValidationError::new(
                          field.span.clone(),
                          format!("field `{}` cannot be both @generated and an aggregate", field.name()),
                      ));
                  }
                  if g.expression.trim().is_empty() {
                      errors.push(ValidationError::new(
                          field.span.clone(),
                          "@generated expression must not be empty",
                      ));
                  }
              }
              if let Some(a) = &attrs.aggregate {
                  // Relation must resolve to an outgoing relation on the parent model.
                  if model.find_relation(a.relation.as_str()).is_none() {
                      errors.push(ValidationError::new(
                          field.span.clone(),
                          format!("unknown relation `{}` in @{:?}", a.relation, a.kind),
                      ));
                  }
                  // Non-Count aggregates require `relation.field`; Count rejects field.
                  match (a.kind, &a.field) {
                      (AggregateKind::Count, Some(_)) => errors.push(ValidationError::new(
                          field.span.clone(),
                          "@count takes a relation name, not `relation.field`",
                      )),
                      (k, None) if k != AggregateKind::Count => errors.push(ValidationError::new(
                          field.span.clone(),
                          format!("@{:?} requires `relation.field`", k),
                      )),
                      _ => {}
                  }
              }
          }
      }
  }
  ```

  Call this from the main validator.

- [ ] **Step 3:** Write `prax-schema/tests/computed_field_validation.rs` covering each error case (empty expression, @generated+@id, @count(rel.field), @sum(rel) without field, unknown relation).

  ```rust
  fn assert_error_contains(schema_text: &str, expected_fragment: &str) {
      let err = parse_schema(schema_text).expect_err("expected validation error");
      let msg = format!("{err}");
      assert!(msg.contains(expected_fragment), "missing `{expected_fragment}` in `{msg}`");
  }

  #[test]
  fn rejects_generated_with_id() {
      assert_error_contains(
          r#"
          datasource db { provider = "postgresql"; url = env("X") }
          model User { id Int @id @auto @generated("1") }
          "#,
          "cannot be both @generated and @id",
      );
  }
  // ... four more test functions for the other cases ...
  ```

- [ ] **Step 4:** `cargo test -p prax-schema --test computed_field_validation` — green.

- [ ] **Step 5:** Commit:

  ```
  feat(schema): validate @generated and @count/@sum field combinations
  ```

---

## Task 5: `#[derive(Model)]` derive-attribute support

**Files:**
- Modify: `prax-codegen/src/generators/derive.rs` (or whichever module parses `#[prax(...)]`)
- Modify: `prax-codegen/src/generators/fields.rs`
- Test: `prax-codegen/tests/derive_computed.rs` (new)

- [ ] **Step 1:** Locate the `#[prax(...)]` attribute parser. Try `grep -rn "prax::AttrSet\|parse_prax_attrs\|fn parse.*Attr" prax-codegen/src/`.

- [ ] **Step 2:** Add parsing for:
  - `#[prax(generated = "expr")]` → `GeneratedAttribute { expression, stored: true }` (default stored)
  - `#[prax(generated = "expr", stored)]` → stored true
  - `#[prax(generated = "expr", virtual)]` → stored false
  - `#[prax(count(rel))]` → `AggregateAttribute { Count, rel, None }`
  - `#[prax(sum(rel.field))]` → likewise (and `avg`, `min`, `max`)

  The parser builds an `AggregateAttribute` / `GeneratedAttribute` per field and stores it on the codegen's internal `FieldMeta` struct (find via `grep -n "struct .*Field" prax-codegen/src/generators/fields.rs`).

- [ ] **Step 3:** Write a `prax-codegen/tests/derive_computed.rs` smoke test that derives a model and asserts the generated metadata constants are present (e.g., emitted `<Model>::COMPUTED_FIELDS` array).

  Actual surface: extend the `Model` trait or `<Model>::COLUMN_LIST` machinery to include `<Model>::GENERATED_FIELDS: &[(&str, &str, bool)]` (name, expression, stored) for use by migration. Add a `<Model>::AGGREGATE_FIELDS: &[(&str, AggregateKindConst, &str, Option<&str>)]` similarly. Both are zero-cost statics.

  ```rust
  #[derive(prax_orm::Model)]
  #[prax(table = "users")]
  struct User {
      #[prax(id, auto)]
      id: i32,
      first_name: String,
      last_name: String,
      #[prax(generated = "first_name || ' ' || last_name", stored)]
      full_name: String,
      #[prax(count(posts))]
      post_count: i64,
  }

  #[test]
  fn user_emits_generated_field_metadata() {
      assert_eq!(
          User::GENERATED_FIELDS,
          &[("full_name", "first_name || ' ' || last_name", true)],
      );
      assert_eq!(User::AGGREGATE_FIELDS.len(), 1);
      assert_eq!(User::AGGREGATE_FIELDS[0].0, "post_count");
  }
  ```

- [ ] **Step 4:** Implement the metadata constants in the `derive_model_trait.rs` (or sibling) emitter.

- [ ] **Step 5:** `cargo test -p prax-codegen --test derive_computed` — green.

- [ ] **Step 6:** Commit:

  ```
  feat(codegen): #[prax(generated)] and #[prax(count/sum/avg/min/max)] in derive
  ```

---

## Task 6: Migration capability marker `SupportsGeneratedColumns`

**Files:**
- Modify: `prax-migrate/src/dialect.rs`
- Modify: `prax-migrate/src/sql.rs` (impl on SQL generators)
- Modify: `prax-migrate/src/cql/generator.rs` (does not impl)

- [ ] **Step 1:** In `prax-migrate/src/dialect.rs`, add:

  ```rust
  /// Marker trait — dialects that support `GENERATED ALWAYS AS (expr) STORED|VIRTUAL`
  /// (or equivalent) computed columns.
  pub trait SupportsGeneratedColumns {}
  ```

- [ ] **Step 2:** In `prax-migrate/src/sql.rs`, impl it on every SQL generator type:

  ```rust
  impl SupportsGeneratedColumns for PostgresSqlGenerator {}
  impl SupportsGeneratedColumns for MySqlGenerator {}
  impl SupportsGeneratedColumns for SqliteGenerator {}
  impl SupportsGeneratedColumns for MssqlGenerator {}
  impl SupportsGeneratedColumns for DuckDbSqlGenerator {}
  ```

- [ ] **Step 3:** Do **not** impl on `prax-migrate/src/cql/generator.rs::CqlGenerator`.

- [ ] **Step 4:** `cargo check -p prax-migrate --all-features` — green.

- [ ] **Step 5:** Commit:

  ```
  feat(migrate): add SupportsGeneratedColumns capability marker
  ```

---

## Task 7: Per-dialect DDL emission for `@generated` columns

**Files:**
- Modify: `prax-migrate/src/sql.rs` (every SQL generator's `column_definition` or equivalent)
- Modify: `prax-migrate/src/diff.rs` if needed to surface `GeneratedAttribute` to the generator
- Test: `prax-migrate/tests/generated_columns.rs` (new)

- [ ] **Step 1:** Inspect how column definitions get built today: `grep -n "fn column_definition\|emit_column" prax-migrate/src/sql.rs | head`.

- [ ] **Step 2:** Extend the column-emitter for each dialect (`PostgresSqlGenerator`, `MySqlGenerator`, `SqliteGenerator`, `MssqlGenerator`, `DuckDbSqlGenerator`) so that when the column metadata carries a `GeneratedAttribute`, it appends the dialect's suffix:

  | Dialect | Suffix when `stored=true` | Suffix when `stored=false` |
  |---------|---------------------------|----------------------------|
  | Postgres | `GENERATED ALWAYS AS (<expr>) STORED` | **reject** — return `MigrationError::unsupported("Postgres does not support @virtual generated columns yet")` |
  | MySQL | `AS (<expr>) STORED` | `AS (<expr>) VIRTUAL` |
  | SQLite | `GENERATED ALWAYS AS (<expr>) STORED` | `GENERATED ALWAYS AS (<expr>) VIRTUAL` |
  | MSSQL | `AS (<expr>) PERSISTED` (type elided in MSSQL — see below) | `AS (<expr>)` (type elided) |
  | DuckDB | `GENERATED ALWAYS AS (<expr>) STORED` | `GENERATED ALWAYS AS (<expr>) VIRTUAL` |

  For MSSQL `@generated`, omit the column type — MSSQL infers from the expression. Other dialects keep the declared type.

- [ ] **Step 3:** Make sure the `SchemaDiff` pipeline carries the `GeneratedAttribute` through to the SQL generator. Likely path: schema → diff → column-meta. Add a `generated: Option<GeneratedAttribute>` field to whatever column-metadata struct the diff produces, and populate it from `Field::generated()`.

- [ ] **Step 4:** Write `prax-migrate/tests/generated_columns.rs` with one test per dialect:

  ```rust
  use prax_migrate::sql::{PostgresSqlGenerator, MySqlGenerator, SqliteGenerator, MssqlGenerator, DuckDbSqlGenerator};
  // (helper: build a SchemaDiff that creates a `users` table with a stored
  // generated column `full_name VARCHAR(255) @generated("first_name || ' ' || last_name") @stored`)

  fn fixture_diff_stored() -> SchemaDiff { /* hand-build */ }
  fn fixture_diff_virtual() -> SchemaDiff { /* hand-build */ }

  #[test]
  fn postgres_emits_generated_stored() {
      let sql = PostgresSqlGenerator.generate(&fixture_diff_stored()).up.join("\n");
      assert!(sql.contains("\"full_name\" VARCHAR(255) GENERATED ALWAYS AS (first_name || ' ' || last_name) STORED"));
  }

  #[test]
  fn postgres_rejects_virtual() {
      let err = PostgresSqlGenerator.generate(&fixture_diff_virtual()).expect_err("must reject");
      assert!(format!("{err}").contains("does not support @virtual"));
  }

  #[test]
  fn mysql_emits_virtual() {
      let sql = MySqlGenerator.generate(&fixture_diff_virtual()).unwrap().up.join("\n");
      assert!(sql.contains("AS (first_name || ' ' || last_name) VIRTUAL"));
  }
  // ... per-dialect tests for sqlite, mssql (PERSISTED), duckdb ...
  ```

- [ ] **Step 5:** `cargo test -p prax-migrate --test generated_columns` — green.

- [ ] **Step 6:** Commit:

  ```
  feat(migrate): emit GENERATED ALWAYS AS (...) DDL per SQL dialect
  ```

---

## Task 8: CQL rejection for `@generated`

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`
- Test: extend `prax-migrate/tests/generated_columns.rs`

- [ ] **Step 1:** In `CqlGenerator` (or `ScyllaCqlGenerator` — find the actual type), wherever it converts model fields → CQL `CREATE TABLE` definitions, add a check at the top:

  ```rust
  for field in &model.fields {
      if field.is_generated() {
          return Err(MigrationError::unsupported(format!(
              "@generated columns are not supported on CQL engines (field `{}.{}`)",
              model.name(), field.name(),
          )));
      }
      if field.is_aggregate() {
          // Aggregates have no DDL anyway — they're query-time. Skip the field entirely
          // rather than rejecting (the model can still be created without the virtual).
          continue;
      }
  }
  ```

- [ ] **Step 2:** Add tests to `prax-migrate/tests/generated_columns.rs`:

  ```rust
  #[test]
  fn cql_rejects_generated() {
      let err = CqlGenerator.generate(&fixture_diff_stored()).expect_err("must reject");
      assert!(format!("{err}").contains("@generated columns are not supported on CQL"));
  }

  #[test]
  fn cql_skips_aggregate_fields() {
      // build a model with one @count field and no @generated.
      let sql = CqlGenerator.generate(&fixture_diff_aggregate_only()).unwrap();
      assert!(!sql.up.join("\n").contains("post_count"));
  }
  ```

- [ ] **Step 3:** `cargo test -p prax-migrate --test generated_columns` — green.

- [ ] **Step 4:** Commit:

  ```
  feat(migrate): reject @generated on CQL; skip aggregate fields silently
  ```

---

## Task 9: `prax-query::ScalarProjection` runtime type + Operation field

**Files:**
- Create: `prax-query/src/projection.rs`
- Modify: `prax-query/src/lib.rs` (module + re-export)
- Modify: `prax-query/src/operations/mod.rs` (extend the shared `Operation` builder type)
- Modify: `prax-query/src/sql.rs` (SqlBuilder integration — find the SELECT-clause assembly)
- Test: `prax-query/tests/projection.rs` (new)

- [ ] **Step 1:** Create `prax-query/src/projection.rs`:

  ```rust
  //! Scalar-subquery projections for relation-aggregate virtual fields and
  //! the `select: { _count: { rel: true } }` ad-hoc accessor.

  use std::borrow::Cow;
  use crate::filter::FilterValue;

  /// A scalar-subquery column added to a SELECT clause.
  ///
  /// `sql` may contain `{N}` placeholders that resolve to the dialect's
  /// positional placeholder for `params[N]`, identical to
  /// [`crate::filter::Filter::ScalarSubquery`].
  #[derive(Debug, Clone)]
  pub struct ScalarProjection {
      pub sql: Cow<'static, str>,
      pub params: Vec<FilterValue>,
      /// Output column alias. Emitted as `(...) AS "alias"`. Must be a
      /// codegen-controlled static string — never user input.
      pub alias: &'static str,
  }

  impl ScalarProjection {
      pub fn new(
          sql: impl Into<Cow<'static, str>>,
          params: Vec<FilterValue>,
          alias: &'static str,
      ) -> Self {
          Self { sql: sql.into(), params, alias }
      }
  }
  ```

- [ ] **Step 2:** Re-export from `prax-query/src/lib.rs`:

  ```rust
  pub mod projection;
  pub use projection::ScalarProjection;
  ```

- [ ] **Step 3:** Find the central `Operation` builder type (try `grep -rn "pub struct .*Operation\b" prax-query/src/operations/`). Add a field:

  ```rust
  pub struct FindManyOperation<'a, T: Model> {
      // ... existing ...
      pub extra_projections: Vec<crate::projection::ScalarProjection>,
  }
  ```
  Repeat for `FindFirstOperation`, `FindUniqueOperation`, and any `SelectInput`-carrying operation. Constructors initialize to `Vec::new()`.

- [ ] **Step 4:** Find the SqlBuilder logic that emits the SELECT column list (`grep -n "fn build_select\|push_columns\|SELECT" prax-query/src/sql.rs` and `prax-query/src/builder.rs`). After the regular column list, append projections:

  ```rust
  for proj in &operation.extra_projections {
      sql.push_str(", ");
      let next_ph = self.placeholders_so_far();
      let rewritten = substitute_brace_placeholders(&proj.sql, next_ph, dialect);
      sql.push_str(&rewritten);
      sql.push_str(" AS ");
      sql.push_str(&dialect.quote_ident(proj.alias));
      self.extend_params(proj.params.clone());
  }
  ```

  `substitute_brace_placeholders` is the same helper used by `Filter::ScalarSubquery::to_sql`. If it's currently private, expose it `pub(crate)` and reuse.

- [ ] **Step 5:** Write `prax-query/tests/projection.rs`:

  ```rust
  use prax_query::dialect::Postgres;
  use prax_query::filter::FilterValue;
  use prax_query::projection::ScalarProjection;

  // (Hand-build a minimal FindManyOperation with one ScalarProjection and one
  // simple Filter, then call the SqlBuilder and assert the emitted SQL.)

  #[test]
  fn single_projection_emits_alias() {
      let proj = ScalarProjection::new(
          "(SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"author_id\" = \"users\".\"id\")",
          vec![],
          "_count_posts",
      );
      let sql = build_test_select(&Postgres, vec![proj]);
      assert!(sql.contains(", (SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"author_id\" = \"users\".\"id\") AS \"_count_posts\""));
  }

  #[test]
  fn placeholders_renumbered_after_where_params() {
      // outer query has WHERE … = $1, projection sql has `> {0}`. After splicing,
      // the projection placeholder must become $2.
      let outer_filter_param = FilterValue::Int(7);
      let proj = ScalarProjection::new(
          "(SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"views\" > {0})",
          vec![FilterValue::Int(100)],
          "_high_view_count",
      );
      let sql = build_test_select_with_filter(&Postgres, proj, outer_filter_param);
      assert!(sql.contains("> $2)"));
      assert!(sql.contains("WHERE \"users\".\"id\" = $1"));
  }
  ```

- [ ] **Step 6:** `cargo test -p prax-query --test projection` — green.

- [ ] **Step 7:** Commit:

  ```
  feat(query): ScalarProjection runtime type for SELECT-side scalar subqueries
  ```

---

## Task 10: `SupportsScalarSubqueryInSelect` capability + engine impls

**Files:**
- Modify: `prax-query/src/capabilities.rs`
- Modify: `prax-postgres/src/lib.rs` (or its capability file)
- Modify: `prax-mysql/src/lib.rs`
- Modify: `prax-sqlite/src/lib.rs`
- Modify: `prax-mssql/src/lib.rs`
- Modify: `prax-duckdb/src/lib.rs`
- Test: `prax-query/tests/compile_fail/scalar_subquery_not_supported.rs` (trybuild)

- [ ] **Step 1:** Add the marker trait to `prax-query/src/capabilities.rs`:

  ```rust
  /// Engine supports scalar subqueries in the SELECT and ORDER BY clauses.
  /// Required for relation-aggregate virtual fields and the `_count` accessor.
  pub trait SupportsScalarSubqueryInSelect {}
  ```

- [ ] **Step 2:** Find each engine's main type and impl. Run `grep -rn "impl SupportsNestedWrites" prax-*` to locate the analogous capability impls for each, then mirror.

  ```rust
  impl SupportsScalarSubqueryInSelect for prax_postgres::PostgresEngine {}
  // ... mysql, sqlite, mssql, duckdb ...
  ```

  Do NOT impl on `prax-mongodb`, `prax-scylladb`, `prax-cassandra`.

- [ ] **Step 3:** Gate `Operation::with_scalar_projection`:

  ```rust
  impl<'a, T: Model, E: SupportsScalarSubqueryInSelect> FindManyOperation<'a, T, E> {
      pub fn with_scalar_projection(mut self, proj: ScalarProjection) -> Self {
          self.extra_projections.push(proj);
          self
      }
  }
  ```

- [ ] **Step 4:** Add a `compile_fail` trybuild fixture asserting MongoDB rejects:

  ```rust
  // prax-query/tests/compile_fail/scalar_subquery_not_supported.rs
  use prax_query::projection::ScalarProjection;
  use prax_mongodb::MongoEngine;

  fn main() {
      let engine: MongoEngine = unimplemented!();
      let op = engine.find_many::<User>();
      op.with_scalar_projection(ScalarProjection::new("…", vec![], "x"));
      //~^ ERROR the trait `SupportsScalarSubqueryInSelect` is not implemented
  }
  ```

  Register this fixture wherever the existing compile_fail tests are wired.

- [ ] **Step 5:** `cargo test -p prax-query` — green.

- [ ] **Step 6:** Commit:

  ```
  feat(query): SupportsScalarSubqueryInSelect capability and engine impls
  ```

---

## Task 11: Codegen — result struct fields and `<Model>FullColumns` adjustments

**Files:**
- Modify: `prax-codegen/src/generators/model.rs` (struct emit)
- Modify: `prax-codegen/src/generators/fields.rs` (FULL_COLUMNS emit)
- Modify: `prax-codegen/src/generators/derive_from_row.rs` (FromRow handling of `@generated` and aggregates)
- Test: `prax-codegen/tests/derive_computed.rs` (extend Task 5's test)

- [ ] **Step 1:** In the model struct emitter, when a field is `is_generated()`, emit it as a normal scalar field of its declared type. When `is_aggregate()` with `Count`, force the Rust type to `i64`. For `Sum`/`Min`/`Max`, use `Option<<declared type>>`. For `Avg`, use `Option<f64>`. Note: the user-declared Rust type on the field is a hint; codegen verifies it matches (warn-and-coerce or hard-error — pick hard-error for explicitness):

  ```rust
  match attrs.aggregate.as_ref().map(|a| a.kind) {
      Some(AggregateKind::Count)
          if field_type.is_i64_compatible() => /* emit i64 */,
      Some(AggregateKind::Sum | AggregateKind::Min | AggregateKind::Max)
          if field_type.is_option() => /* emit declared */,
      Some(AggregateKind::Avg)
          if field_type.is_option_f64() => /* emit Option<f64> */,
      None => /* normal */,
      _ => return Err(compile_error("aggregate field type does not match aggregate kind")),
  }
  ```

- [ ] **Step 2:** In FULL_COLUMNS emit, skip fields where `is_aggregate()` returns true (no underlying column). Include `is_generated()` (it has a real column).

- [ ] **Step 3:** In `derive_from_row.rs`, generate `FromRow` reads that:
  - Skip aggregate fields if they're not selected (default the Option to `None` / `0` for Count).
  - Read `@generated` like a normal column.

  Actually simplification: have aggregate fields always default to `None` (`Option<_>`) / `0` (Count) when the row doesn't carry them. SqlBuilder won't include them unless they're projected via Task 13.

- [ ] **Step 4:** Extend `prax-codegen/tests/derive_computed.rs` with:

  ```rust
  #[test]
  fn aggregate_field_is_excluded_from_full_columns() {
      let cols: Vec<&str> = User::FULL_COLUMNS.iter().copied().collect();
      assert!(cols.contains(&"full_name"));
      assert!(!cols.contains(&"post_count"));
  }

  #[test]
  fn count_field_type_is_i64() {
      let u = User { id: 1, post_count: 42, ..Default::default() };
      let _: i64 = u.post_count;
  }
  ```

- [ ] **Step 5:** `cargo test -p prax-codegen` — green.

- [ ] **Step 6:** Commit:

  ```
  feat(codegen): result struct + FULL_COLUMNS for @generated and aggregate fields
  ```

---

## Task 12: Codegen — Where/Select/OrderBy/Create/Update input membership

**Files:**
- Modify: `prax-codegen/src/generators/inputs/where_input.rs`
- Modify: `prax-codegen/src/generators/inputs/select_input.rs`
- Modify: `prax-codegen/src/generators/inputs/order_by_input.rs`
- Modify: `prax-codegen/src/generators/inputs/create_input.rs`
- Modify: `prax-codegen/src/generators/inputs/update_input.rs`
- Test: extend `prax-codegen/tests/derive_computed.rs`

- [ ] **Step 1:** In `where_input.rs`: when iterating fields, include both `is_generated()` and `is_aggregate()` fields. The emitted `<Model>WhereInput` gets a `pub post_count: Option<IntFilter>` (and similar) field per aggregate/generated. The `into_filter` lowering for aggregates produces `Filter::ScalarSubquery { sql, params }` — see Task 14 for the body. For now, just emit the field; lowering follows.

- [ ] **Step 2:** In `select_input.rs`: include both classes. Add a `_count` field (Task 13 follow-up handles its specific shape).

- [ ] **Step 3:** In `order_by_input.rs`: include both. Lowering treats aggregate fields by producing `Order { expr: ScalarSubqueryExpr, dir }`. Add a `OrderByItem::ScalarSubquery { sql, params, direction }` variant in `prax-query::inputs` if the existing OrderBy IR doesn't already support this.

- [ ] **Step 4:** In `create_input.rs` and `update_input.rs`: exclude both `is_generated()` and `is_aggregate()`. Add a compile_error path if user tries to set them (Task 15 trybuild).

- [ ] **Step 5:** Extend `prax-codegen/tests/derive_computed.rs`:

  ```rust
  #[test]
  fn create_input_excludes_computed_fields() {
      // UserCreateInput's struct should not contain `full_name` or `post_count` fields.
      let _: UserCreateInput = UserCreateInput {
          email: "a@b".into(),
          first_name: "A".into(),
          last_name: "B".into(),
          // post_count and full_name must NOT be required here.
      };
  }
  ```

- [ ] **Step 6:** `cargo test -p prax-codegen` — green.

- [ ] **Step 7:** Commit:

  ```
  feat(codegen): input-struct membership for @generated and aggregate fields
  ```

---

## Task 13: Codegen — `<Model>Count` synthetic struct + `_count` field on result

**Files:**
- Create: `prax-codegen/src/generators/count_struct.rs`
- Modify: `prax-codegen/src/generators/mod.rs` (wire it in)
- Modify: `prax-codegen/src/generators/model.rs` (`_count` field on the result struct)
- Modify: `prax-codegen/src/generators/derive_from_row.rs` (hydrate `_count`)
- Test: extend `prax-codegen/tests/derive_computed.rs`

- [ ] **Step 1:** Create `count_struct.rs` that emits `<Model>Count` with one `pub <rel>: Option<i64>` per outgoing relation. Models with zero outgoing relations: skip emission entirely.

  ```rust
  pub fn emit_count_struct(model: &ModelMeta) -> Option<TokenStream> {
      if model.outgoing_relations.is_empty() { return None; }
      let name = format_ident!("{}Count", model.name);
      let fields = model.outgoing_relations.iter().map(|rel| {
          let f = format_ident!("{}", rel.name);
          quote! { pub #f: Option<i64> }
      });
      Some(quote! {
          #[derive(Debug, Clone, Default)]
          pub struct #name {
              #(#fields,)*
          }
      })
  }
  ```

- [ ] **Step 2:** In `model.rs`, when emitting the result struct, append `pub _count: Option<<Model>Count>` if outgoing relations exist. Add `Default for User` that defaults this to `None`.

- [ ] **Step 3:** In `derive_from_row.rs`, generate FromRow that probes for `_count_<rel>` columns. If any exist, populate `_count = Some(<Model>Count { posts: row.get_opt("_count_posts"), comments: row.get_opt("_count_comments") })`. Otherwise leave `None`.

- [ ] **Step 4:** Extend tests:

  ```rust
  #[test]
  fn model_with_relations_emits_count_struct() {
      let _ = UserCount { posts: Some(3), comments: None };
      let u = User::default();
      assert!(u._count.is_none());
  }
  ```

- [ ] **Step 5:** Compile-error case for `_count` on relation-less models: covered by Task 15 trybuild.

- [ ] **Step 6:** `cargo test -p prax-codegen` — green.

- [ ] **Step 7:** Commit:

  ```
  feat(codegen): <Model>Count synthetic struct and _count field on results
  ```

---

## Task 14: Macro DSL — `_count: { rel: true }` in `select:`, schema-level aggregate lowering

**Files:**
- Modify: `prax-codegen/src/macros/lower/select_input.rs`
- Modify: `prax-codegen/src/macros/lower/where_input.rs`
- Modify: `prax-codegen/src/macros/lower/order_by_input.rs`
- Modify: `prax-codegen/src/generators/inputs/select_input.rs`

- [ ] **Step 1:** In `select_input.rs` lowering, accept a `_count` key whose value is a brace-block of `<rel>: true` entries. For each entry, push a `ScalarProjection` constructor call into the lowered output:

  ```rust
  // Lowered output looks like:
  builder.with_scalar_projection(prax_query::ScalarProjection::new(
      Cow::Borrowed("(SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"author_id\" = \"users\".\"id\")"),
      vec![],
      "_count_posts",
  ))
  ```

  The SQL string is constructed at macro expansion using schema knowledge (table name, FK column, parent PK column). It is a `&'static str` because it's a const string literal in the codegen output — no runtime concatenation, satisfying SQL-safety rules.

- [ ] **Step 2:** Diagnostics on bad `_count` keys:
  - Unknown relation name → did-you-mean against `model.outgoing_relations`.
  - Value other than `true` → "use `<rel>: true` (filtered or detailed counts are a follow-up)".
  - `_count` on a model with no relations → "model `X` has no outgoing relations to count".

- [ ] **Step 3:** Schema-level aggregate lowering — when a field on the model has `aggregate.is_some()`, and the `select:` block names it (or selects all columns), the corresponding `ScalarProjection` is added automatically using the schema metadata. Same SQL construction, alias is the field name (not `_count_*`).

- [ ] **Step 4:** Same machinery for `where:` lowering when the field has `aggregate.is_some()`: lower to `Filter::ScalarSubquery { sql, params }` with the full boolean predicate (e.g., `(SELECT COUNT(*) ...) > {0}`).

- [ ] **Step 5:** Same machinery for `order_by:` aggregates.

- [ ] **Step 6:** `cargo build -p prax-codegen --all-features` — green.

- [ ] **Step 7:** Commit:

  ```
  feat(codegen): _count in select! and lowering for schema-level aggregates
  ```

---

## Task 15: trybuild diagnostics — happy and unhappy paths

**Files:**
- Create: `prax-codegen/tests/ui/computed_generated_happy.rs`
- Create: `prax-codegen/tests/ui/computed_count_happy.rs`
- Create: `prax-codegen/tests/ui/computed_set_aggregate_in_data.rs` (expect compile_fail)
- Create: `prax-codegen/tests/ui/computed_unknown_relation_in_count.rs` (compile_fail)
- Create: `prax-codegen/tests/ui/computed_count_on_relationless_model.rs` (compile_fail)
- Modify: `prax-codegen/tests/ui.rs` (or wherever trybuild is wired)

- [ ] **Step 1:** Create one trybuild fixture per scenario above. Happy fixtures should compile; unhappy ones get a locked `.stderr` baseline.

- [ ] **Step 2:** Each compile_fail fixture exercises one diagnostic the previous tasks set up:
  - `set("post_count", FilterValue::Int(7))` in a `create!` `data:` → "field is a computed virtual and cannot be assigned".
  - `_count: { unknown_rel: true }` → "unknown relation; did you mean `posts`?".
  - `_count: { posts: true }` on a model declared without any relation → "no outgoing relations".

- [ ] **Step 3:** Run `cargo test -p prax-codegen --test ui` and accept the new `.stderr` baselines (`TRYBUILD=overwrite`).

- [ ] **Step 4:** Commit:

  ```
  test(codegen): trybuild fixtures for @generated, @count, and _count diagnostics
  ```

---

## Task 16: e2e mock-engine tests (workspace-root `tests/`)

**Files:**
- Create: `tests/computed_fields_e2e.rs`

- [ ] **Step 1:** Copy the `RecordingEngine` pattern from `tests/nested_writes_e2e.rs` (the one we just extended with `DialectKind`). Add a derived `User` with `@generated` + `@count` fields:

  ```rust
  #[derive(prax_orm::Model, Debug, Clone, Default)]
  #[prax(table = "users")]
  struct User {
      #[prax(id, auto)]
      id: i32,
      #[prax(unique)]
      email: String,
      first_name: String,
      last_name: String,
      #[prax(generated = "first_name || ' ' || last_name", stored)]
      full_name: String,
      #[prax(relation(target = "Post", foreign_key = "author_id"))]
      posts: Vec<Post>,
      #[prax(count(posts))]
      post_count: i64,
  }

  #[derive(prax_orm::Model, Debug, Clone, Default)]
  #[prax(table = "posts")]
  struct Post {
      #[prax(id, auto)]
      id: i32,
      author_id: i32,
      views: i32,
  }
  ```

- [ ] **Step 2:** Tests (each ~20 lines):

  - `filter_by_post_count_emits_scalar_subquery_in_where`:
    Issue `find_many!(client.user, { where: { post_count: { gt: 5 } } })`. Assert the recorded SQL contains `(SELECT COUNT(*) FROM "posts" WHERE "posts"."author_id" = "users"."id") > $1` and params = `[Int(5)]`.

  - `select_count_emits_scalar_subquery_in_select`:
    Issue `find_many!(client.user, { select: { id: true, _count: { posts: true } } })`. Assert the SQL contains `, (SELECT COUNT(*) FROM "posts" WHERE …) AS "_count_posts"`.

  - `order_by_post_count_emits_scalar_subquery`:
    Assert `ORDER BY (SELECT COUNT(*) …) DESC`.

  - `full_name_in_full_columns_but_not_in_create`:
    Compile-time: `UserCreateInput` does not have a `full_name` field.

  - `create_input_omits_computed_fields`:
    Issue `create!(client.user, { data: { email: …, first_name: …, last_name: … } })`. Recorded INSERT statement must omit `full_name` and `post_count`.

- [ ] **Step 3:** `cargo test --test computed_fields_e2e` — green.

- [ ] **Step 4:** Commit:

  ```
  test(query): e2e RecordingEngine coverage for @generated, @count, _count
  ```

---

## Task 17: Live Postgres integration test

**Files:**
- Create: `prax-postgres/tests/computed_fields.rs`

- [ ] **Step 1:** Mirror an existing live-Postgres test (e.g., `prax-postgres/tests/nested_write_postgres.rs`) for the testcontainer harness. Create a `users` table with:

  ```sql
  CREATE TABLE users (
      id SERIAL PRIMARY KEY,
      email TEXT UNIQUE NOT NULL,
      first_name TEXT NOT NULL,
      last_name TEXT NOT NULL,
      full_name TEXT GENERATED ALWAYS AS (first_name || ' ' || last_name) STORED
  );
  CREATE TABLE posts (
      id SERIAL PRIMARY KEY,
      author_id INT REFERENCES users(id),
      views INT NOT NULL DEFAULT 0
  );
  ```

  Run the test as:

  ```rust
  #[tokio::test]
  #[ignore = "requires postgres container; run with --ignored"]
  async fn computed_fields_round_trip() {
      let pool = test_pool().await;
      // CREATE TABLE …
      // INSERT user; assert full_name was populated by the DB.
      // INSERT three posts; assert find_many with select _count: { posts } returns post_count = 3.
      // Filter by where: { post_count: { gt: 2 } } returns the user.
  }
  ```

- [ ] **Step 2:** `cargo test -p prax-postgres --test computed_fields -- --ignored` — green (skipped in default test pass).

- [ ] **Step 3:** Commit:

  ```
  test(postgres): live integration for @generated and @count
  ```

---

## Task 18: CHANGELOG + workspace sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1:** Add under `[Unreleased] > Added`:

  ```
  - **Computed and virtual fields (phase 5.5).** Three new field classes:
    - `@generated("expr") @stored|@virtual` — DB-side computed columns,
      DDL emitted per dialect (`GENERATED ALWAYS AS (...) STORED|VIRTUAL`
      on PG/SQLite/DuckDB, `AS (...) [VIRTUAL|STORED]` on MySQL,
      `AS (...) [PERSISTED]` on MSSQL). CQL engines reject at migrate
      time. `SupportsGeneratedColumns` capability marker.
    - `@count(rel)`, `@sum/@avg/@min/@max(rel.field)` — relation-aggregate
      virtuals. Result-struct types: Count → `i64`, Avg → `Option<f64>`,
      others → `Option<T>`. WHERE/ORDER BY lower via existing
      `Filter::ScalarSubquery`; SELECT lowers via a new
      `ScalarProjection` runtime type. `SupportsScalarSubqueryInSelect`
      capability — implemented by all SQL engines, not by MongoDB or CQL.
    - `select: { _count: { rel: true } }` ad-hoc accessor — synthesizes
      a per-model `<Model>Count` struct exposed as `Option<_>` on the
      result. Compile-time error against relation-less models.
  - All computed/aggregate fields are excluded from `<Model>CreateInput`
    and `<Model>UpdateInput`; included in WhereInput, SelectInput,
    OrderByInput.

  ### Known limitations

  - Postgres rejects `@virtual` generated columns (PG 17+ support
    deferred — use `@stored`).
  - The ad-hoc `_count` accessor only supports counts; sum/avg/min/max
    require a schema-level `@sum/@avg/...` attribute.
  - MongoDB engines fail to compile against aggregate fields and the
    `_count` accessor until the `$lookup` follow-up ships.
  ```

- [ ] **Step 2:** Full workspace sweep:

  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace --all-features --no-fail-fast
  ```

- [ ] **Step 3:** Commit:

  ```
  docs(schema): CHANGELOG for phase 5.5 computed/virtual fields
  ```

---

## Task 19: Push + PR (handled outside subagent)

- [ ] `git push -u origin feature/computed-virtual-fields`
- [ ] `gh pr create --base develop --title "feat: computed and virtual fields (phase 5.5)"`

PR body sketch:

```
Closes phase 5.5 of the typed-query-traits initiative. Spec:
docs/superpowers/specs/2026-05-22-computed-virtual-fields-design.md.

## What ships
- @generated("expr") @stored|@virtual on schema fields
- @count/@sum/@avg/@min/@max relation aggregate virtuals
- select: { _count: { rel: true } } ad-hoc accessor

## What's deferred
- MongoDB $lookup lowering for aggregates
- include: { _count: ... }
- PG ≥ 17 @virtual generated columns
- _count with where filter

## Test plan
- prax-schema parser/validation
- prax-migrate per-dialect DDL snapshots + CQL rejection
- prax-query ScalarProjection unit tests
- prax-codegen trybuild
- workspace e2e (tests/computed_fields_e2e.rs)
- prax-postgres live integration (`-- --ignored`)
```

---

## Out of scope (deferred follow-ups, listed in spec §12)

- MongoDB `$lookup` lowering.
- `include: { _count: … }` form.
- Lateral-join optimization for scalar subqueries.
- Postgres ≥ 17 virtual generated columns.
- Cross-dialect `@generated` expression translator.
- Filtered `_count` (`_count: { posts: { where: { … } } }`).
