# Typed Input Codegen (Phase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire phase-1's typed input traits into the codegen pipeline. After this lands, `#[derive(Model)]` and `prax_schema!` emit per-model `WhereInput` / `WhereUniqueInput` / `IncludeInput` / `SelectInput` / `OrderByInput` / `CreateInput` / `UpdateInput` types plus `RelationFilterMeta` impls per relation, and every SQL engine declares which capability traits it satisfies.

**Architecture:**
- Add new generator modules under `prax-codegen/src/generators/inputs_*.rs` — one per input type. Each emits a `TokenStream` for a given parsed model. Generators are pure functions of the parsed schema; no state.
- Wire the new emitters into both codegen entry points (`derive.rs` for `#[derive(Model)]` and `model.rs` for `prax_schema!`). The shared `generators/mod.rs` re-exports the new emitters.
- Defer aggregate-related inputs (`CountSelect`, `AggregateInput`, `GroupByInput`) to phase 6. Defer nested-write fields in `CreateInput` / `UpdateInput` to phase 5 — phase 2 only emits flat scalar fields.
- Add `capabilities.rs` to each SQL engine crate declaring its marker-trait impls. CQL engines (`prax-scylladb`, `prax-cassandra`) intentionally implement nothing.

**Tech Stack:** Rust 2024, `proc_macro2`, `quote`, `syn 2.0`, `convert_case`, `prax-schema` AST, `insta` (snapshot tests of generated tokens), `trybuild` (compile-pass / compile-fail tests).

---

## File Structure

### New files (in `prax-codegen/`)

- `prax-codegen/src/generators/inputs/mod.rs` — module root + helper for per-model field iteration
- `prax-codegen/src/generators/inputs/where_input.rs` — emit `<Model>WhereInput`
- `prax-codegen/src/generators/inputs/where_unique_input.rs` — emit `<Model>WhereUniqueInput` (enum over unique columns)
- `prax-codegen/src/generators/inputs/include_input.rs` — emit `<Model>Include` + per-relation `<Relation>IncludeArgs`
- `prax-codegen/src/generators/inputs/select_input.rs` — emit `<Model>Select`
- `prax-codegen/src/generators/inputs/order_by_input.rs` — emit `<Model>OrderBy`
- `prax-codegen/src/generators/inputs/create_input.rs` — emit `<Model>CreateInput` (flat scalars only at phase 2)
- `prax-codegen/src/generators/inputs/update_input.rs` — emit `<Model>UpdateInput` (flat scalars only at phase 2)
- `prax-codegen/src/generators/inputs/relation_meta.rs` — emit `impl RelationFilterMeta for <Model><Relation>Meta` per relation

### New files (per engine crate, all parallel layout)

- `prax-postgres/src/capabilities.rs`
- `prax-mysql/src/capabilities.rs`
- `prax-sqlite/src/capabilities.rs`
- `prax-mssql/src/capabilities.rs`
- `prax-duckdb/src/capabilities.rs`
- `prax-mongodb/src/capabilities.rs`

Each registers `pub mod capabilities;` in its crate's `lib.rs`.

### Test fixtures

- `prax-codegen/tests/fixtures/inputs_schema.rs` — a hand-rolled `#[derive(Model)]` test model with: scalar fields (String, i32, Option<String>, bool, DateTime), one to-many relation, one to-one relation, one enum column.
- `prax-codegen/tests/derive_inputs.rs` — derive-driven tests asserting each generated input struct can be constructed + lowered.
- `prax-codegen/tests/snapshots/` — `insta` snapshots of generated token streams for the fixture model.

### Modified files

- `prax-codegen/src/generators/mod.rs` — `mod inputs;` + re-exports
- `prax-codegen/src/generators/derive.rs` (line ~700, end of `derive_model_impl`) — emit calls to the new generators
- `prax-codegen/src/generators/model.rs` (similar location, end of `generate_model_module`) — emit calls to the new generators
- Each engine crate's `src/lib.rs` — add `pub mod capabilities;`
- `prax-codegen/Cargo.toml` — add `insta` to `[dev-dependencies]` if not already present

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None.

- [ ] **Step 1: Confirm worktree + branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/typed-inputs-codegen rev-parse --abbrev-ref HEAD`
Expected: `feature/typed-inputs-codegen`

- [ ] **Step 2: Workspace check**

Run: `cargo check --workspace --all-features` (from the worktree root)
Expected: zero compile errors.

- [ ] **Step 3: Phase-1 tests still pass**

Run: `cargo test -p prax-query`
Expected: all pass.

- [ ] **Step 4: No commit — verification only**

---

## Task 2: Scaffold `generators/inputs/` module + shared helpers

**Files:**
- Create: `prax-codegen/src/generators/inputs/mod.rs`
- Modify: `prax-codegen/src/generators/mod.rs`

- [ ] **Step 1: Create `inputs/mod.rs`** with this shape:

```rust
//! Generators for the typed input shapes (phase 2 of the typed-query-traits work).
//!
//! Each submodule is a pure function from a parsed schema model
//! (`prax_schema::ast::Model` or a derive-parsed `FieldInfo` list) to a
//! `TokenStream` containing one input type per model. The `derive.rs`
//! and `model.rs` entry points call all of these in turn and concat
//! the streams into the per-model module.

pub mod create_input;
pub mod include_input;
pub mod order_by_input;
pub mod relation_meta;
pub mod select_input;
pub mod update_input;
pub mod where_input;
pub mod where_unique_input;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

/// Tag for a field's filter category — drives which scalar filter
/// wrapper is referenced in the generated struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCategory {
    String,
    Int,
    BigInt,
    Float,
    Decimal,
    Bool,
    Bytes,
    Uuid,
    Json,
    DateTime,
    Date,
    Time,
    Enum,
}

/// Map a schema-level type string to the right `FilterCategory`.
/// Returns `None` for unknown / relation fields.
pub fn filter_category_for(type_name: &str) -> Option<FilterCategory> {
    match type_name {
        "String" => Some(FilterCategory::String),
        "Int" | "i32" => Some(FilterCategory::Int),
        "BigInt" | "i64" => Some(FilterCategory::BigInt),
        "Float" | "f64" => Some(FilterCategory::Float),
        "Decimal" | "rust_decimal::Decimal" => Some(FilterCategory::Decimal),
        "Boolean" | "bool" => Some(FilterCategory::Bool),
        "Bytes" | "Vec<u8>" => Some(FilterCategory::Bytes),
        "Uuid" | "uuid::Uuid" => Some(FilterCategory::Uuid),
        "Json" | "serde_json::Value" => Some(FilterCategory::Json),
        "DateTime" | "chrono::DateTime<chrono::Utc>" => Some(FilterCategory::DateTime),
        "Date" | "chrono::NaiveDate" => Some(FilterCategory::Date),
        "Time" | "chrono::NaiveTime" => Some(FilterCategory::Time),
        _ => None,
    }
}

/// Resolve the `prax_query::inputs` filter wrapper type ident for a
/// given category + nullability.
pub fn filter_wrapper_ident(cat: FilterCategory, nullable: bool) -> Ident {
    let name = match (cat, nullable) {
        (FilterCategory::String, false) => "StringFilter",
        (FilterCategory::String, true) => "StringNullableFilter",
        (FilterCategory::Int, false) => "IntFilter",
        (FilterCategory::Int, true) => "IntNullableFilter",
        (FilterCategory::BigInt, false) => "BigIntFilter",
        (FilterCategory::BigInt, true) => "BigIntNullableFilter",
        (FilterCategory::Float, false) => "FloatFilter",
        (FilterCategory::Float, true) => "FloatNullableFilter",
        (FilterCategory::Decimal, false) => "DecimalFilter",
        (FilterCategory::Decimal, true) => "DecimalNullableFilter",
        (FilterCategory::Bool, false) => "BoolFilter",
        (FilterCategory::Bool, true) => "BoolNullableFilter",
        (FilterCategory::Bytes, false) => "BytesFilter",
        (FilterCategory::Bytes, true) => "BytesNullableFilter",
        (FilterCategory::Uuid, false) => "UuidFilter",
        (FilterCategory::Uuid, true) => "UuidNullableFilter",
        (FilterCategory::Json, false) => "InputJsonFilter",          // crate-root alias from phase 1
        (FilterCategory::Json, true) => "InputJsonNullableFilter",   // crate-root alias from phase 1
        (FilterCategory::DateTime, false) => "DateTimeFilter",
        (FilterCategory::DateTime, true) => "DateTimeNullableFilter",
        (FilterCategory::Date, false) => "DateFilter",
        (FilterCategory::Date, true) => "DateNullableFilter",
        (FilterCategory::Time, false) => "TimeFilter",
        (FilterCategory::Time, true) => "TimeNullableFilter",
        (FilterCategory::Enum, false) => "EnumFilter",
        (FilterCategory::Enum, true) => "EnumNullableFilter",
    };
    format_ident!("{}", name)
}

/// Resolve the field-update wrapper ident.
pub fn update_wrapper_ident(cat: FilterCategory, nullable: bool) -> Ident {
    let name = match (cat, nullable) {
        (FilterCategory::String, false) => "StringFieldUpdate",
        (FilterCategory::String, true) => "StringNullableFieldUpdate",
        (FilterCategory::Int, false) => "IntFieldUpdate",
        (FilterCategory::Int, true) => "IntNullableFieldUpdate",
        (FilterCategory::BigInt, false) => "BigIntFieldUpdate",
        (FilterCategory::BigInt, true) => "BigIntNullableFieldUpdate",
        (FilterCategory::Float, false) => "FloatFieldUpdate",
        (FilterCategory::Float, true) => "FloatNullableFieldUpdate",
        (FilterCategory::Decimal, false) => "DecimalFieldUpdate",
        (FilterCategory::Decimal, true) => "DecimalNullableFieldUpdate",
        (FilterCategory::Bool, false) => "BoolFieldUpdate",
        (FilterCategory::Bool, true) => "BoolNullableFieldUpdate",
        (FilterCategory::Bytes, false) => "BytesFieldUpdate",
        (FilterCategory::Bytes, true) => "BytesNullableFieldUpdate",
        (FilterCategory::Uuid, false) => "UuidFieldUpdate",
        (FilterCategory::Uuid, true) => "UuidNullableFieldUpdate",
        (FilterCategory::Json, false) => "JsonFieldUpdate",
        (FilterCategory::Json, true) => "JsonNullableFieldUpdate",
        (FilterCategory::DateTime, false) => "DateTimeFieldUpdate",
        (FilterCategory::DateTime, true) => "DateTimeNullableFieldUpdate",
        (FilterCategory::Date, false) => "DateTimeFieldUpdate",      // dates use the DateTime update wrapper
        (FilterCategory::Date, true) => "DateTimeNullableFieldUpdate",
        (FilterCategory::Time, false) => "DateTimeFieldUpdate",      // times use the DateTime update wrapper
        (FilterCategory::Time, true) => "DateTimeNullableFieldUpdate",
        (FilterCategory::Enum, false) => "EnumFieldUpdate",
        (FilterCategory::Enum, true) => "EnumNullableFieldUpdate",
    };
    format_ident!("{}", name)
}

/// Resolve the Rust scalar payload type that the filter / update wrapper
/// expects. Used for the `equals: Option<Self::Payload>` field types
/// during input-struct emission.
pub fn scalar_payload_type(cat: FilterCategory) -> TokenStream {
    match cat {
        FilterCategory::String => quote! { ::std::string::String },
        FilterCategory::Int => quote! { i32 },
        FilterCategory::BigInt => quote! { i64 },
        FilterCategory::Float => quote! { f64 },
        FilterCategory::Decimal => quote! { ::rust_decimal::Decimal },
        FilterCategory::Bool => quote! { bool },
        FilterCategory::Bytes => quote! { ::std::vec::Vec<u8> },
        FilterCategory::Uuid => quote! { ::uuid::Uuid },
        FilterCategory::Json => quote! { ::serde_json::Value },
        FilterCategory::DateTime => quote! { ::chrono::DateTime<::chrono::Utc> },
        FilterCategory::Date => quote! { ::chrono::NaiveDate },
        FilterCategory::Time => quote! { ::chrono::NaiveTime },
        FilterCategory::Enum => panic!("enum payload requires the enum ident — caller must construct"),
    }
}
```

- [ ] **Step 2: Register in `generators/mod.rs`**

In `prax-codegen/src/generators/mod.rs`, after the existing `mod` lines (around line 14), add:

```rust
mod inputs;

pub use inputs::{FilterCategory, filter_category_for, filter_wrapper_ident, update_wrapper_ident, scalar_payload_type};
```

- [ ] **Step 3: Confirm it compiles**

Run: `cargo check -p prax-codegen`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add prax-codegen/src/generators/inputs/mod.rs prax-codegen/src/generators/mod.rs
git commit -m "feat(codegen): scaffold inputs generator module

Shared FilterCategory enum + helpers map schema types to the
prax_query::inputs filter/update wrapper idents. Subsequent tasks
fill in the per-input emitters (where, include, select, ...)."
```

---

## Task 3: `where_input` generator + derive entrypoint wiring

**Files:**
- Create: `prax-codegen/src/generators/inputs/where_input.rs`
- Create: `prax-codegen/tests/fixtures/inputs_schema.rs`
- Create: `prax-codegen/tests/derive_inputs.rs`
- Modify: `prax-codegen/src/generators/derive.rs` (emit calls)
- Modify: `prax-codegen/src/generators/mod.rs` (re-export)

- [ ] **Step 1: Write the failing integration test**

Create `prax-codegen/tests/fixtures/inputs_schema.rs`:

```rust
//! Test fixture for the input-codegen tests. Models below cover:
//! scalar fields (String, i32, Option<String>, bool, DateTime), one
//! to-many relation (User.posts), one to-one relation (User.profile),
//! one enum column (User.role).

use prax_codegen::Model;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Role {
    Admin,
    Member,
}

impl ::core::fmt::Display for Role {
    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        match self {
            Role::Admin => f.write_str("Admin"),
            Role::Member => f.write_str("Member"),
        }
    }
}

#[derive(Model, Debug, Clone)]
#[prax(table = "users")]
pub struct User {
    #[prax(id)]
    pub id: i64,
    #[prax(unique)]
    pub email: String,
    pub name: Option<String>,
    pub age: Option<i32>,
    pub active: bool,
    // `role` deliberately uses the String column type at codegen-time;
    // a future task can wire enum-aware codegen. Phase 2 treats unknown
    // type strings as opaque.
    pub role: String,
}
```

Create `prax-codegen/tests/derive_inputs.rs`:

```rust
//! End-to-end tests that the generated *Input structs for the fixture
//! model compile and lower correctly through phase-1's traits.

mod fixtures {
    pub mod inputs_schema;
}

use fixtures::inputs_schema::User;
use prax_query::filter::Filter;
use prax_query::inputs::{StringFilter, WhereInput};

#[test]
fn user_where_input_default_lowers_to_filter_none() {
    let w = user::UserWhereInput::default();
    assert!(matches!(w.into_ir(), Filter::None));
}

#[test]
fn user_where_input_email_contains_lowers_to_contains_filter() {
    let w = user::UserWhereInput {
        email: Some(StringFilter::contains("@example.com")),
        ..Default::default()
    };
    match w.into_ir() {
        Filter::Contains(col, _) => assert_eq!(col, "email"),
        other => panic!("expected Filter::Contains, got {:?}", other),
    }
}
```

`user::UserWhereInput` is the module path — the `#[derive(Model)]` macro currently emits a `user` module per model. Phase 2 puts `UserWhereInput` inside that same module.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-codegen --test derive_inputs`
Expected: compile error — `UserWhereInput` doesn't exist yet.

- [ ] **Step 3: Implement the `where_input` generator**

Create `prax-codegen/src/generators/inputs/where_input.rs`:

```rust
//! Generate `<Model>WhereInput` for a parsed model.
//!
//! The struct has one `Option<ScalarFilter>` field per scalar column +
//! `Option<ListRelationFilter<...>>` for each to-many relation +
//! `Option<SingleRelationFilter<...>>` for each to-one. Plus the
//! `and` / `or` / `not` logical combinators.
//!
//! `WhereInput::into_ir(self) -> Filter` ANDs together the active
//! per-field filters.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, filter_category_for, filter_wrapper_ident};
use crate::generators::snake_ident;

/// One field's metadata as seen by the where-input generator.
pub struct WhereField {
    /// Field name in the source code (snake_case ident).
    pub name: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Filter category — only Some for scalar fields. Relation fields
    /// emit `ListRelationFilter` / `SingleRelationFilter` instead.
    pub category: Option<FilterCategory>,
    /// Whether the field is `Option<T>` (nullable).
    pub nullable: bool,
    /// For relation fields: the target model's `WhereInput` type.
    /// `None` for scalar fields.
    pub relation_target_where_input: Option<Ident>,
    /// For relation fields: `true` = to-many, `false` = to-one. Unused
    /// for scalars.
    pub is_to_many: bool,
}

/// Emit `<Model>WhereInput` + its `WhereInput` trait impl.
pub fn generate(model_ident: &Ident, fields: &[WhereField]) -> TokenStream {
    let where_input_ident = format_ident!("{}WhereInput", model_ident);

    let scalar_fields = fields.iter().filter(|f| f.category.is_some());
    let relation_fields = fields.iter().filter(|f| f.relation_target_where_input.is_some());

    let scalar_field_decls = scalar_fields.clone().map(|f| {
        let name = &f.name;
        let cat = f.category.expect("scalar field has category");
        let wrapper = filter_wrapper_ident(cat, f.nullable);
        quote! {
            pub #name: ::core::option::Option<::prax_query::inputs::#wrapper>
        }
    });

    let relation_field_decls = relation_fields.clone().map(|f| {
        let name = &f.name;
        let target = f.relation_target_where_input.as_ref().expect("relation");
        if f.is_to_many {
            quote! {
                pub #name: ::core::option::Option<::prax_query::inputs::ListRelationFilter<#target>>
            }
        } else {
            quote! {
                pub #name: ::core::option::Option<::prax_query::inputs::SingleRelationFilter<#target>>
            }
        }
    });

    let scalar_lowerings = scalar_fields.clone().map(|f| {
        let name = &f.name;
        let col = &f.column;
        quote! {
            if let ::core::option::Option::Some(__inner) = self.#name {
                use ::prax_query::inputs::ScalarFilter as _;
                parts.push(__inner.into_filter(#col));
            }
        }
    });

    // Relation-filter lowering needs the per-relation `RelationFilterMeta`
    // impl emitted by `relation_meta.rs` (Task 9). The meta type is named
    // `<Model><Relation>FilterMeta`.
    let relation_lowerings = fields.iter().filter(|f| f.relation_target_where_input.is_some()).map(|f| {
        let name = &f.name;
        let meta_ident = format_ident!("{}{}FilterMeta", model_ident, pascal(&f.name.to_string()));
        quote! {
            if let ::core::option::Option::Some(__inner) = self.#name {
                use ::prax_query::inputs::relation::LowerRelationFilter as _;
                let f = __inner.lower::<#meta_ident>();
                if !matches!(f, ::prax_query::filter::Filter::None) {
                    parts.push(f);
                }
            }
        }
    });

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #where_input_ident {
            #(#scalar_field_decls,)*
            #(#relation_field_decls,)*
            pub and: ::core::option::Option<::std::vec::Vec<#where_input_ident>>,
            pub or: ::core::option::Option<::std::vec::Vec<#where_input_ident>>,
            pub not: ::core::option::Option<::std::boxed::Box<#where_input_ident>>,
        }

        impl ::prax_query::inputs::WhereInput for #where_input_ident {
            type Model = super::#model_ident;
            fn into_ir(self) -> ::prax_query::filter::Filter {
                let mut parts: ::std::vec::Vec<::prax_query::filter::Filter> = ::std::vec::Vec::new();

                #(#scalar_lowerings)*
                #(#relation_lowerings)*

                if let ::core::option::Option::Some(ands) = self.and {
                    let inner: ::std::vec::Vec<::prax_query::filter::Filter> = ands
                        .into_iter()
                        .map(|w| <Self as ::prax_query::inputs::WhereInput>::into_ir(w))
                        .collect();
                    parts.push(::prax_query::filter::Filter::and(inner));
                }
                if let ::core::option::Option::Some(ors) = self.or {
                    let inner: ::std::vec::Vec<::prax_query::filter::Filter> = ors
                        .into_iter()
                        .map(|w| <Self as ::prax_query::inputs::WhereInput>::into_ir(w))
                        .collect();
                    parts.push(::prax_query::filter::Filter::or(inner));
                }
                if let ::core::option::Option::Some(n) = self.not {
                    parts.push(::prax_query::filter::Filter::Not(::std::boxed::Box::new(
                        <Self as ::prax_query::inputs::WhereInput>::into_ir(*n),
                    )));
                }

                match parts.len() {
                    0 => ::prax_query::filter::Filter::None,
                    1 => parts.into_iter().next().unwrap(),
                    _ => ::prax_query::filter::Filter::and(parts),
                }
            }
        }
    }
}

fn pascal(s: &str) -> proc_macro2::Ident {
    use convert_case::{Case, Casing};
    format_ident!("{}", s.to_case(Case::Pascal))
}

fn _suppress_unused(_: ::syn::Ident) {}
let _ = snake_ident;  // keeps the unused-import elision happy at module bootstrap
```

Remove the trailing dead-code lines once the rest of the file references `snake_ident` — they're scaffolding to satisfy clippy during bootstrap.

- [ ] **Step 4: Re-export from `generators/mod.rs`**

Add to `generators/mod.rs`:

```rust
pub use inputs::where_input::{generate as generate_where_input, WhereField};
```

- [ ] **Step 5: Wire into `derive.rs`**

In `prax-codegen/src/generators/derive.rs`, locate the `derive_model_impl` function's terminal `quote! { ... }`. **Before** the closing `}` of that quote, add a call to the new generator:

```rust
        // Phase-2 typed-input emission.
        #[allow(unused_imports)]
        use crate::generators::inputs::where_input::WhereField;
        let where_fields: ::std::vec::Vec<WhereField> = field_infos
            .iter()
            .filter_map(|f| {
                if f.is_list {
                    return Some(WhereField {
                        name: f.name.clone(),
                        column: f.column_name.clone(),
                        category: None,
                        nullable: false,
                        relation_target_where_input: Some(format_ident!("{}WhereInput", f.related_type.as_ref()?)),
                        is_to_many: true,
                    });
                }
                let cat = crate::generators::inputs::filter_category_for(&f.type_str)?;
                Some(WhereField {
                    name: f.name.clone(),
                    column: f.column_name.clone(),
                    category: Some(cat),
                    nullable: f.is_optional,
                    relation_target_where_input: None,
                    is_to_many: false,
                })
            })
            .collect();
        let where_input_tokens = crate::generators::inputs::where_input::generate(&name, &where_fields);
```

Then in the final `quote!` add `#where_input_tokens` inside the `pub mod #module_name { ... }` block alongside the existing per-model emissions.

The `FieldInfo` struct in `derive.rs` may not have `type_str` / `related_type` / `is_optional` fields with those exact names. Locate the actual fields with:

```bash
grep -n "struct FieldInfo\|pub.*:" prax-codegen/src/generators/derive.rs | head -30
```

and adapt the mapping. **If the FieldInfo layout doesn't expose type / nullability cleanly, report BLOCKED — that schema-side gap should be fixed before continuing.**

- [ ] **Step 6: Run the new test**

Run: `cargo test -p prax-codegen --test derive_inputs`
Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add prax-codegen/src/generators/inputs/where_input.rs prax-codegen/src/generators/derive.rs prax-codegen/src/generators/mod.rs prax-codegen/tests/derive_inputs.rs prax-codegen/tests/fixtures/inputs_schema.rs
git commit -m "feat(codegen): emit <Model>WhereInput from #[derive(Model)]

The generator walks parsed FieldInfo and produces one Option-typed
field per scalar column (using the prax_query::inputs scalar
filter wrappers) + relation filter fields keyed by the
<Model><Relation>FilterMeta impl emitted in a later task. WhereInput's
into_ir method AND-composes the active per-field filters and folds
in and/or/not combinators."
```

---

## Task 4: `where_unique_input` generator

**Files:**
- Create: `prax-codegen/src/generators/inputs/where_unique_input.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Modify: `prax-codegen/tests/derive_inputs.rs`

- [ ] **Step 1: Append failing test**

Append to `prax-codegen/tests/derive_inputs.rs`:

```rust
use prax_query::inputs::WhereUniqueInput;

#[test]
fn user_where_unique_input_id_lowers_to_id_equals() {
    let w = user::UserWhereUniqueInput::Id(42);
    let filter = <_ as WhereUniqueInput>::into_ir(w);
    match filter {
        Filter::Equals(col, prax_query::filter::FilterValue::Int(v)) => {
            assert_eq!(col, "id");
            assert_eq!(v, 42);
        }
        other => panic!("expected Filter::Equals on id, got {:?}", other),
    }
}

#[test]
fn user_where_unique_input_email_lowers_to_email_equals() {
    let w = user::UserWhereUniqueInput::Email("a@b.com".into());
    let filter = <_ as WhereUniqueInput>::into_ir(w);
    match filter {
        Filter::Equals(col, prax_query::filter::FilterValue::String(s)) => {
            assert_eq!(col, "email");
            assert_eq!(s, "a@b.com");
        }
        other => panic!("expected Filter::Equals on email, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-codegen --test derive_inputs`
Expected: compile errors.

- [ ] **Step 3: Implement the generator**

Create `prax-codegen/src/generators/inputs/where_unique_input.rs`:

```rust
//! Generate `<Model>WhereUniqueInput` — an enum over the model's
//! primary-key and `@unique` columns.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, filter_category_for, scalar_payload_type};
use crate::generators::pascal_ident;

/// One unique-key column.
pub struct UniqueColumn {
    /// The variant name (PascalCase of the column).
    pub variant: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Filter category for the scalar payload.
    pub category: FilterCategory,
    /// For enum columns: the enum's PascalCase ident. Phase 2 leaves
    /// this `None` (enum-aware codegen lands later).
    pub enum_ident: Option<Ident>,
}

pub fn generate(model_ident: &Ident, columns: &[UniqueColumn]) -> TokenStream {
    let where_unique_ident = format_ident!("{}WhereUniqueInput", model_ident);

    if columns.is_empty() {
        // Models with no unique columns get an uninhabited enum so
        // find_unique / update / delete fail to compile.
        return quote! {
            /// Uninhabited because the model has no unique key.
            #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
            pub enum #where_unique_ident {}

            impl ::prax_query::inputs::WhereUniqueInput for #where_unique_ident {
                type Model = super::#model_ident;
                fn into_ir(self) -> ::prax_query::filter::Filter {
                    match self {}
                }
            }
        };
    }

    let variant_decls = columns.iter().map(|c| {
        let v = &c.variant;
        let payload = if let Some(enum_ident) = &c.enum_ident {
            quote! { #enum_ident }
        } else {
            scalar_payload_type(c.category)
        };
        quote! { #v(#payload) }
    });

    let lower_arms = columns.iter().map(|c| {
        let v = &c.variant;
        let col = &c.column;
        let body = match c.category {
            FilterCategory::Int => quote! { ::prax_query::filter::FilterValue::Int(value as i64) },
            FilterCategory::BigInt => quote! { ::prax_query::filter::FilterValue::Int(value) },
            FilterCategory::Float => quote! { ::prax_query::filter::FilterValue::Float(value) },
            FilterCategory::Bool => quote! { ::prax_query::filter::FilterValue::Bool(value) },
            FilterCategory::String => quote! { ::prax_query::filter::FilterValue::String(value) },
            FilterCategory::Decimal => quote! { ::prax_query::filter::FilterValue::String(value.to_string()) },
            FilterCategory::Uuid => quote! { ::prax_query::filter::FilterValue::String(value.to_string()) },
            FilterCategory::Bytes => quote! {
                {
                    use base64::Engine as _;
                    ::prax_query::filter::FilterValue::String(
                        base64::engine::general_purpose::STANDARD.encode(&value)
                    )
                }
            },
            FilterCategory::DateTime => quote! { ::prax_query::filter::FilterValue::String(value.to_rfc3339()) },
            FilterCategory::Date => quote! { ::prax_query::filter::FilterValue::String(value.to_string()) },
            FilterCategory::Time => quote! { ::prax_query::filter::FilterValue::String(value.format("%H:%M:%S").to_string()) },
            FilterCategory::Json => quote! { ::prax_query::filter::FilterValue::Json(value) },
            FilterCategory::Enum => quote! { ::prax_query::filter::FilterValue::String(value.to_string()) },
        };
        quote! {
            Self::#v(value) => ::prax_query::filter::Filter::Equals(
                ::std::borrow::Cow::Borrowed(#col),
                #body,
            )
        }
    });

    quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum #where_unique_ident {
            #(#variant_decls,)*
        }

        impl ::prax_query::inputs::WhereUniqueInput for #where_unique_ident {
            type Model = super::#model_ident;
            fn into_ir(self) -> ::prax_query::filter::Filter {
                match self {
                    #(#lower_arms,)*
                }
            }
        }
    }
}
```

- [ ] **Step 4: Re-export + wire derive.rs**

Add `pub use inputs::where_unique_input::{generate as generate_where_unique_input, UniqueColumn};` to `generators/mod.rs`.

In `derive.rs`, after collecting `where_fields`, gather unique columns:

```rust
        use crate::generators::inputs::where_unique_input::UniqueColumn;
        let unique_columns: Vec<UniqueColumn> = field_infos.iter()
            .filter(|f| f.is_id || f.is_unique)
            .filter_map(|f| {
                let cat = crate::generators::inputs::filter_category_for(&f.type_str)?;
                Some(UniqueColumn {
                    variant: format_ident!("{}", f.name.to_string().to_case(Case::Pascal)),
                    column: f.column_name.clone(),
                    category: cat,
                    enum_ident: None,
                })
            })
            .collect();
        let where_unique_tokens = crate::generators::inputs::where_unique_input::generate(&name, &unique_columns);
```

Add `#where_unique_tokens` to the final `quote!` alongside `#where_input_tokens`.

If `FieldInfo` doesn't have `is_unique`, locate the right field (likely `is_unique` or `unique` flag) — adapt.

- [ ] **Step 5: Run tests**

Run: `cargo test -p prax-codegen --test derive_inputs`
Expected: all 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add prax-codegen/src/generators/inputs/where_unique_input.rs prax-codegen/src/generators/derive.rs prax-codegen/src/generators/mod.rs prax-codegen/tests/derive_inputs.rs
git commit -m "feat(codegen): emit <Model>WhereUniqueInput enum

One variant per @id / @unique column. WhereUniqueInput::into_ir
lowers to Filter::Equals keyed by the column name. Models with no
unique keys get an uninhabited enum so find_unique fails to compile."
```

---

## Task 5: `include_input` + `select_input` generators

**Files:**
- Create: `prax-codegen/src/generators/inputs/include_input.rs`
- Create: `prax-codegen/src/generators/inputs/select_input.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Modify: `prax-codegen/tests/derive_inputs.rs`

- [ ] **Step 1: Append failing tests**

```rust
use prax_query::inputs::{IncludeInput, SelectInput};

#[test]
fn user_include_default_is_empty() {
    let i = user::UserInclude::default();
    let inc = <_ as IncludeInput>::into_ir(i);
    assert!(inc.is_empty());
}

#[test]
fn user_select_default_is_empty() {
    let s = user::UserSelect::default();
    let _sel = <_ as SelectInput>::into_ir(s);
}
```

- [ ] **Step 2: Implement `include_input.rs`**

```rust
//! Generate `<Model>Include` — one Option-typed slot per relation.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct IncludeField {
    /// Field name in the source code (snake_case).
    pub name: Ident,
    /// SQL relation name (for the IncludeSpec).
    pub relation: String,
}

pub fn generate(model_ident: &Ident, relations: &[IncludeField]) -> TokenStream {
    let include_ident = format_ident!("{}Include", model_ident);

    let decls = relations.iter().map(|r| {
        let n = &r.name;
        quote! {
            pub #n: ::core::option::Option<bool>
        }
    });

    let lowerings = relations.iter().map(|r| {
        let n = &r.name;
        let rel = &r.relation;
        quote! {
            if self.#n == ::core::option::Option::Some(true) {
                inc = inc.with(::prax_query::relations::IncludeSpec::new(#rel));
            }
        }
    });

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #include_ident {
            #(#decls,)*
        }

        impl ::prax_query::inputs::IncludeInput for #include_ident {
            type Model = super::#model_ident;
            fn into_ir(self) -> ::prax_query::relations::Include {
                let mut inc = ::prax_query::relations::Include::new();
                #(#lowerings)*
                inc
            }
        }
    }
}
```

(Phase 2 keeps `IncludeInput` minimal — just `Option<bool>` per relation. Phase 3+ macros may extend with per-relation `where`/`order_by`/`take`/`include` recursion; that lands when the macro DSL needs it.)

- [ ] **Step 3: Implement `select_input.rs`**

```rust
//! Generate `<Model>Select` — one Option<bool> per column + per relation.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct SelectField {
    pub name: Ident,
    pub column: String,
    pub is_relation: bool,
}

pub fn generate(model_ident: &Ident, fields: &[SelectField]) -> TokenStream {
    let select_ident = format_ident!("{}Select", model_ident);

    let decls = fields.iter().map(|f| {
        let n = &f.name;
        quote! {
            pub #n: ::core::option::Option<bool>
        }
    });

    let lowerings = fields.iter().filter(|f| !f.is_relation).map(|f| {
        let n = &f.name;
        let col = &f.column;
        quote! {
            if self.#n == ::core::option::Option::Some(true) {
                cols.push(#col.to_string());
            }
        }
    });

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #select_ident {
            #(#decls,)*
        }

        impl ::prax_query::inputs::SelectInput for #select_ident {
            type Model = super::#model_ident;
            fn into_ir(self) -> ::prax_query::types::Select {
                let mut cols: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
                #(#lowerings)*
                if cols.is_empty() {
                    ::prax_query::types::Select::All
                } else {
                    ::prax_query::types::Select::Columns(cols)
                }
            }
        }
    }
}
```

If `prax_query::types::Select` doesn't have a `Columns(Vec<String>)` variant, locate the existing variant for column projection and adapt. Run `grep -n "pub enum Select" prax-query/src/types.rs`.

- [ ] **Step 4: Wire derive.rs**

Gather the include + select field lists from `field_infos` and emit the new tokens. Same pattern as Tasks 3-4.

- [ ] **Step 5: Run tests + commit**

Run: `cargo test -p prax-codegen --test derive_inputs`
Expected: 6 tests pass.

Commit:

```bash
git add prax-codegen/src/generators/inputs/include_input.rs prax-codegen/src/generators/inputs/select_input.rs prax-codegen/src/generators/derive.rs prax-codegen/src/generators/mod.rs prax-codegen/tests/derive_inputs.rs
git commit -m "feat(codegen): emit <Model>Include and <Model>Select

Include carries one Option<bool> per relation; select carries one
Option<bool> per column + relation. Lowerings produce Include /
Select runtime IR via the phase-1 traits. Phase 3+ may extend
Include to per-relation args (where/take/include recursion)."
```

---

## Task 6: `order_by_input` generator

**Files:**
- Create: `prax-codegen/src/generators/inputs/order_by_input.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Modify: `prax-codegen/tests/derive_inputs.rs`

- [ ] **Step 1: Append failing test**

```rust
use prax_query::inputs::OrderByInput;
use prax_query::types::SortOrder;

#[test]
fn user_order_by_email_asc_lowers() {
    let o = user::UserOrderBy::Email(SortOrder::Asc);
    let order = <_ as OrderByInput>::into_ir(o);
    // We can't deeply assert OrderBy's internal shape here without
    // knowing its accessors; just confirm it isn't None.
    assert!(!matches!(order, ::prax_query::types::OrderBy::None));
}
```

If `OrderBy::None` isn't the right "empty" representation, adapt — look at `prax_query::types::OrderBy`.

- [ ] **Step 2: Implement the generator**

```rust
//! Generate `<Model>OrderBy` — an enum over the model's sortable columns.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct OrderByField {
    /// Variant name (PascalCase of the column).
    pub variant: Ident,
    /// SQL column name (string literal).
    pub column: String,
    /// Whether the column is nullable (allows NULLS FIRST/LAST).
    pub nullable: bool,
}

pub fn generate(model_ident: &Ident, fields: &[OrderByField]) -> TokenStream {
    let order_by_ident = format_ident!("{}OrderBy", model_ident);

    let variant_decls = fields.iter().map(|f| {
        let v = &f.variant;
        // Phase 2 ignores NULLS FIRST/LAST — lands with the dialect layer.
        quote! { #v(::prax_query::types::SortOrder) }
    });

    let lower_arms = fields.iter().map(|f| {
        let v = &f.variant;
        let col = &f.column;
        quote! {
            Self::#v(dir) => ::prax_query::types::OrderBy::single(#col, dir)
        }
    });

    quote! {
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum #order_by_ident {
            #(#variant_decls,)*
        }

        impl ::prax_query::inputs::OrderByInput for #order_by_ident {
            type Model = super::#model_ident;
            fn into_ir(self) -> ::prax_query::types::OrderBy {
                match self {
                    #(#lower_arms,)*
                }
            }
        }
    }
}
```

If `OrderBy::single(col, dir)` doesn't exist, locate the right constructor and adapt — try `OrderByField::asc(col)` / `OrderByField::desc(col)` from `prax_query::types`.

- [ ] **Step 3: Wire + commit**

Same pattern as previous tasks.

```bash
git commit -m "feat(codegen): emit <Model>OrderBy enum

One variant per sortable column carrying a SortOrder. Nullable
columns get NULLS FIRST/LAST support via the dialect layer in a
follow-up."
```

---

## Task 7: `create_input` + `update_input` generators (flat scalars)

**Files:**
- Create: `prax-codegen/src/generators/inputs/create_input.rs`
- Create: `prax-codegen/src/generators/inputs/update_input.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Modify: `prax-codegen/tests/derive_inputs.rs`

Phase 2 emits flat scalar fields only. Nested writes (`PostsCreateNestedInput` / `connect_or_create` / etc.) land in phase 5.

- [ ] **Step 1: Append failing tests**

```rust
use prax_query::inputs::{CreateInput, UpdateInput};

#[test]
fn user_create_input_has_required_email_field() {
    let _c = user::UserCreateInput {
        email: "x@y.com".into(),
        name: None,
        age: None,
        active: true,
        role: "Admin".into(),
    };
}

#[test]
fn user_update_input_default_is_empty() {
    let _u = user::UserUpdateInput::default();
}
```

Note: `CreateInput::into_ir` needs a `Data` associated type matching what the existing `CreateOperation` consumes. Phase-2 punts: the generated `*CreateInput` does not yet impl the `CreateInput` trait. It is a plain struct that future tasks will plug in once `CreateOperation`'s data field is restructured (phase 5). Phase 2 only emits the **struct shape**; trait impls land with the operation rework.

- [ ] **Step 2: Implement `create_input.rs`**

```rust
//! Generate `<Model>CreateInput` — flat scalar fields, no nested writes
//! (those land in phase 5).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, scalar_payload_type};

pub struct CreateField {
    pub name: Ident,
    pub category: FilterCategory,
    pub nullable: bool,
    pub has_default: bool,  // database default or @default attribute
    pub enum_ident: Option<Ident>,
}

pub fn generate(model_ident: &Ident, fields: &[CreateField]) -> TokenStream {
    let create_ident = format_ident!("{}CreateInput", model_ident);

    let field_decls = fields.iter().map(|f| {
        let n = &f.name;
        let payload = if let Some(e) = &f.enum_ident {
            quote! { #e }
        } else {
            scalar_payload_type(f.category)
        };
        if f.nullable || f.has_default {
            quote! { pub #n: ::core::option::Option<#payload> }
        } else {
            quote! { pub #n: #payload }
        }
    });

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #create_ident {
            #(#field_decls,)*
        }
    }
}
```

Note: this struct derives `Default` so phase-3 macros can call `..Default::default()`. Required fields without `has_default` must still be set — `Default` here puts them at `Default::default()` of the payload type (empty String, 0 for ints, etc.) which is fine for the macro DSL's "leaf may be omitted" semantics. Phase 5's nested-write codegen will replace this with a strict variant that doesn't derive Default.

- [ ] **Step 3: Implement `update_input.rs`**

```rust
//! Generate `<Model>UpdateInput` — flat scalar fields wrapped in
//! `*FieldUpdate` wrappers. No nested writes (phase 5).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use crate::generators::inputs::{FilterCategory, update_wrapper_ident};

pub struct UpdateField {
    pub name: Ident,
    pub category: FilterCategory,
    pub nullable: bool,
    pub enum_ident: Option<Ident>,
}

pub fn generate(model_ident: &Ident, fields: &[UpdateField]) -> TokenStream {
    let update_ident = format_ident!("{}UpdateInput", model_ident);

    let field_decls = fields.iter().map(|f| {
        let n = &f.name;
        let wrapper = update_wrapper_ident(f.category, f.nullable);
        if matches!(f.category, FilterCategory::Enum) {
            let e = f.enum_ident.as_ref().expect("enum field requires enum ident");
            quote! {
                pub #n: ::core::option::Option<::prax_query::inputs::#wrapper<#e>>
            }
        } else {
            quote! {
                pub #n: ::core::option::Option<::prax_query::inputs::#wrapper>
            }
        }
    });

    quote! {
        #[derive(Debug, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct #update_ident {
            #(#field_decls,)*
        }
    }
}
```

- [ ] **Step 4: Wire + commit**

```bash
git commit -m "feat(codegen): emit flat <Model>CreateInput + <Model>UpdateInput

Phase 2 emits struct shapes only — required scalar fields plus
Option-wrapped optional fields for create; *FieldUpdate-wrapped
fields for update. Nested writes (connect / connect_or_create /
disconnect / upsert / delete / set) land in phase 5 along with
the NestedWritePlan executor."
```

---

## Task 8: `relation_meta` generator

**Files:**
- Create: `prax-codegen/src/generators/inputs/relation_meta.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Modify: `prax-codegen/tests/derive_inputs.rs`

This emits the `impl ::prax_query::inputs::relation::RelationFilterMeta for <Model><Relation>FilterMeta` that `WhereInput`'s relation fields reference.

- [ ] **Step 1: Append failing test**

For the fixture, `User` has no declared relations (kept the fixture minimal for phase 2). Add a relation:

In `prax-codegen/tests/fixtures/inputs_schema.rs`, append:

```rust
#[derive(Model, Debug, Clone)]
#[prax(table = "posts")]
pub struct Post {
    #[prax(id)]
    pub id: i64,
    pub title: String,
    pub author_id: i64,
}
```

Then in `User`, add a relation field (if the `#[derive(Model)]` macro supports relations natively — verify with `grep -n "is_list\|related_type" prax-codegen/src/generators/derive.rs`). If derive doesn't support relations cleanly, defer the relation_meta integration test to Task 11 (the `prax_schema!` path), where the schema language defines relations directly.

Append to `derive_inputs.rs`:

```rust
#[test]
fn user_posts_relation_filter_meta_constants() {
    use prax_query::inputs::relation::RelationFilterMeta;
    type M = user::UserPostsFilterMeta;
    assert_eq!(M::PARENT_TABLE, "users");
    assert_eq!(M::PARENT_PK, "id");
    assert_eq!(M::CHILD_TABLE, "posts");
    assert_eq!(M::CHILD_FK, "author_id");
}
```

- [ ] **Step 2: Implement the generator**

```rust
//! Generate per-relation `RelationFilterMeta` impls.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct RelationMetaSpec {
    /// Marker struct name, e.g. `UserPostsFilterMeta`.
    pub meta_ident: Ident,
    pub parent_table: String,
    pub parent_pk: String,
    pub child_table: String,
    pub child_fk: String,
}

pub fn generate(specs: &[RelationMetaSpec]) -> TokenStream {
    let impls = specs.iter().map(|s| {
        let m = &s.meta_ident;
        let pt = &s.parent_table;
        let pp = &s.parent_pk;
        let ct = &s.child_table;
        let cf = &s.child_fk;
        quote! {
            #[doc(hidden)]
            pub struct #m;
            impl ::prax_query::inputs::relation::RelationFilterMeta for #m {
                const PARENT_TABLE: &'static str = #pt;
                const PARENT_PK: &'static str = #pp;
                const CHILD_TABLE: &'static str = #ct;
                const CHILD_FK: &'static str = #cf;
            }
        }
    });
    quote! { #(#impls)* }
}
```

- [ ] **Step 3: Wire derive.rs + commit**

If derive's `FieldInfo` doesn't expose `parent_pk` / `child_fk` cleanly, the relation_meta generator may not be wireable from the derive path — the relation direction info lives in `#[prax(relation(...))]` attributes. Defer to Task 11 (schema-path wiring) if needed.

```bash
git commit -m "feat(codegen): emit RelationFilterMeta impls per relation

Each declared relation emits a zero-sized marker type with the
RelationFilterMeta trait impl pointing at the parent/child table
and column names. Powers EXISTS / NOT EXISTS lowering for
relation filters introduced in phase 1."
```

---

## Task 9: Wire `prax_schema!` macro path

**Files:**
- Modify: `prax-codegen/src/generators/model.rs`
- Possibly create: `prax-codegen/tests/schema_inputs.rs`

The `prax_schema!` macro consumes a parsed `prax_schema::ast::Schema` and emits one module per model. Mirror the wiring done in Task 3-8 but feeding from the AST instead of `FieldInfo`.

- [ ] **Step 1: Locate the emit point**

Run: `grep -n "fn generate_model_module\|fn generate_model_module_with_style" prax-codegen/src/generators/model.rs`

Identify where each model's tokens are concatenated. Add the same `where_input::generate(...)`, `where_unique_input::generate(...)`, etc. calls feeding from `ast::Model` field metadata.

- [ ] **Step 2: Map AST fields to the generator inputs**

For each model `m: &Schema::Model`:
- Iterate `m.fields` for `WhereField` / `CreateField` / `UpdateField` mappings.
- Iterate `m.relations` for relation fields + `RelationMetaSpec` entries.
- Resolve target model PK column names by looking up the relation's target model in the schema.

The schema AST may differ from derive's `FieldInfo` — adapter functions in `inputs/mod.rs` can normalize.

- [ ] **Step 3: Run schema-path tests**

Create `prax-codegen/tests/schema_inputs.rs` that invokes `prax_schema!` on a small schema and asserts the same things as `derive_inputs.rs`.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(codegen): wire input generators into prax_schema! macro

Both #[derive(Model)] and prax_schema! now emit the seven typed
input types per model plus per-relation RelationFilterMeta impls.
Codegen feeds derive's FieldInfo or the parsed schema AST
through the same emitter functions in generators::inputs."
```

---

## Task 10: Engine capability impls — PG, MySQL, SQLite

**Files (per engine):**
- Create: `prax-postgres/src/capabilities.rs`, `prax-mysql/src/capabilities.rs`, `prax-sqlite/src/capabilities.rs`
- Modify: each crate's `src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `prax-postgres/tests/capabilities.rs`:

```rust
use prax_postgres::Engine;
use prax_query::capabilities::{
    SupportsCorrelatedSubquery, SupportsGeneratedColumns, SupportsJsonPath,
    SupportsNestedWrites, SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

fn assert_traits<E: SupportsRelationFilter + SupportsCorrelatedSubquery + SupportsJsonPath + SupportsGeneratedColumns + SupportsNestedWrites + SupportsScalarSubqueryInSelect>() {}

#[test]
fn postgres_engine_impls_all_supported_capabilities() {
    assert_traits::<Engine>();
}
```

(If the engine type isn't named `Engine`, locate the correct concrete type — `grep -n "pub struct.*Engine\|impl QueryEngine" prax-postgres/src/`. Common names: `PostgresEngine`, `PgEngine`.)

- [ ] **Step 2: Implement `prax-postgres/src/capabilities.rs`**

```rust
//! Postgres engine capability declarations.
//!
//! Postgres supports the broadest set of features. CQL engines are the
//! only ones that decline most of these.

use crate::engine::Engine;  // adapt to the actual concrete engine type
use prax_query::capabilities::{
    SupportsArrayOps, SupportsCaseInsensitiveMode, SupportsCorrelatedSubquery,
    SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath,
    SupportsNestedWrites, SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};

impl SupportsRelationFilter for Engine {}
impl SupportsCorrelatedSubquery for Engine {}
impl SupportsJsonPath for Engine {}
impl SupportsCaseInsensitiveMode for Engine {}
impl SupportsFullTextSearch for Engine {}
impl SupportsArrayOps for Engine {}
impl SupportsGeneratedColumns for Engine {}
impl SupportsScalarSubqueryInSelect for Engine {}
impl SupportsNestedWrites for Engine {}
```

Add `pub mod capabilities;` to `prax-postgres/src/lib.rs`.

- [ ] **Step 3: Implement `prax-mysql/src/capabilities.rs`**

MySQL impls: `SupportsRelationFilter`, `SupportsCorrelatedSubquery`, `SupportsJsonPath` (5.7+), `SupportsFullTextSearch`, `SupportsGeneratedColumns`, `SupportsScalarSubqueryInSelect`, `SupportsNestedWrites`. Skip `SupportsArrayOps` (not native) and `SupportsCaseInsensitiveMode` (handled at collation level).

- [ ] **Step 4: Implement `prax-sqlite/src/capabilities.rs`**

SQLite impls: `SupportsRelationFilter`, `SupportsCorrelatedSubquery`, `SupportsJsonPath` (requires JSON1 extension at runtime), `SupportsGeneratedColumns`, `SupportsScalarSubqueryInSelect`, `SupportsNestedWrites`.

- [ ] **Step 5: Run tests + commit**

Run the three new test files. All should compile + pass.

```bash
git commit -m "feat(postgres,mysql,sqlite): impl capability marker traits

Each SQL engine declares which prax_query::capabilities marker
traits it satisfies. Phase-3+ macros use these to compile-time-block
unsupported features per engine (e.g., relation filters on CQL).
Postgres has the broadest set; MySQL and SQLite trim FTS / array
ops; CQL engines decline everything in a later task."
```

---

## Task 11: Engine capability impls — MSSQL, DuckDB, MongoDB

**Files:**
- Create: `prax-mssql/src/capabilities.rs`, `prax-duckdb/src/capabilities.rs`, `prax-mongodb/src/capabilities.rs`
- Modify: each crate's `src/lib.rs`

MSSQL: same set as Postgres minus `SupportsArrayOps`.
DuckDB: `SupportsRelationFilter`, `SupportsCorrelatedSubquery`, `SupportsGeneratedColumns`, `SupportsScalarSubqueryInSelect`, `SupportsNestedWrites`.
MongoDB: only `SupportsRelationFilter` and `SupportsNestedWrites` (uses different SQL primitives). `SupportsScalarSubqueryInSelect` defers to a follow-up plan (the `$lookup`-lowering plan).

- [ ] **Steps 1-3:** Mirror Task 10. Add tests, impls, lib.rs registration.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(mssql,duckdb,mongodb): impl capability marker traits

MSSQL gets the full SQL surface minus array ops; DuckDB declines
FTS / JSON path / case-insensitive mode; MongoDB only impls
SupportsRelationFilter and SupportsNestedWrites (different SQL
primitives; scalar-subquery-in-select awaits the \$lookup
lowering plan)."
```

---

## Task 12: Confirm CQL engines decline (no capability impls)

**Files:**
- None — verification + sanity test only.

- [ ] **Step 1: Create compile-fail tests**

Create `prax-scylladb/tests/capability_compile_fail.rs`:

```rust
#[test]
fn scylla_does_not_impl_relation_filter() {
    // This file uses trybuild — see prax-scylladb/tests/ui/.
}
```

And `prax-scylladb/tests/ui/cql_no_relation_filter.rs`:

```rust
use prax_query::capabilities::SupportsRelationFilter;
use prax_scylladb::Engine;  // adapt name
fn assert_relation_filter<E: SupportsRelationFilter>() {}
fn main() {
    assert_relation_filter::<Engine>();  // should NOT compile
}
```

And a matching `prax-scylladb/tests/ui/cql_no_relation_filter.stderr` snapshot (let `trybuild` capture it on first run).

Repeat for `prax-cassandra`.

- [ ] **Step 2: Add `trybuild` to dev-deps**

`prax-scylladb/Cargo.toml` and `prax-cassandra/Cargo.toml`:

```toml
[dev-dependencies]
trybuild = { workspace = true }
```

- [ ] **Step 3: Run trybuild tests**

```bash
TRYBUILD=overwrite cargo test -p prax-scylladb --test capability_compile_fail
TRYBUILD=overwrite cargo test -p prax-cassandra --test capability_compile_fail
```

The first run captures the stderr snapshot. Inspect the captured snapshots — they should show `the trait bound 'Engine: SupportsRelationFilter' is not satisfied` with the phase-1 diagnostic note.

- [ ] **Step 4: Commit the snapshots**

```bash
git commit -m "test(scylladb,cassandra): trybuild capability compile-fail tests

CQL engines must not implement the SQL capability marker traits.
trybuild captures the rustc diagnostic so future regressions
(someone accidentally adding an impl) are caught at test time."
```

---

## Task 13: End-to-end derive test exercising all phase-2 inputs

**Files:**
- Modify: `prax-codegen/tests/derive_inputs.rs` — add a comprehensive end-to-end test.

- [ ] **Step 1: Append end-to-end test**

```rust
#[test]
fn user_where_input_with_relation_combines_via_and() {
    use prax_query::inputs::{BoolFilter, IntFilter, ScalarFilter, StringFilter};

    let w = user::UserWhereInput {
        email: Some(StringFilter::contains("@example.com")),
        active: Some(BoolFilter::equals(true)),
        age: Some(prax_query::inputs::IntNullableFilter {
            gte: Some(18),
            ..Default::default()
        }),
        ..Default::default()
    };

    let f = <_ as WhereInput>::into_ir(w);
    // 3 active filters → AND of 3.
    match f {
        Filter::And(parts) => assert_eq!(parts.len(), 3),
        other => panic!("expected Filter::And(3), got {:?}", other),
    }
}

#[test]
fn user_where_input_serde_round_trip() {
    let w = user::UserWhereInput {
        email: Some(StringFilter::contains("@x.com")),
        ..Default::default()
    };
    let json = serde_json::to_string(&w).unwrap();
    let back: user::UserWhereInput = serde_json::from_str(&json).unwrap();
    assert_eq!(back.email.as_ref().and_then(|f| f.contains.as_deref()), Some("@x.com"));
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p prax-codegen --test derive_inputs`
Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git commit -m "test(codegen): exercise the full input surface end-to-end

A single test constructs UserWhereInput with three operators across
two scalar types + serde round-trip, asserting the lowered Filter
shape. Catches regressions if any of the generators emit a
non-composable struct or a broken into_ir."
```

---

## Task 14: Final workspace verification

**Files:**
- None — verification.

- [ ] **Step 1: Format**: `cargo fmt --all -- --check` — clean.
- [ ] **Step 2: Clippy**: `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean.
- [ ] **Step 3: Workspace tests**: `cargo test --workspace --lib` — all pass.
- [ ] **Step 4: Integration tests**: `cargo test -p prax-codegen --test derive_inputs --test schema_inputs` — pass.
- [ ] **Step 5: Capability tests on each engine**: `cargo test -p prax-postgres --test capabilities` (and the other 5 engines + the 2 CQL trybuild). All pass.
- [ ] **Step 6: Docs**: `cargo doc --workspace --no-deps --all-features` — clean.
- [ ] **Step 7: Phase complete — no commit needed here.**

---

## Acceptance criteria

- [ ] `#[derive(Model)]` emits `UserWhereInput`, `UserWhereUniqueInput`, `UserInclude`, `UserSelect`, `UserOrderBy`, `UserCreateInput`, `UserUpdateInput` per model, plus per-relation `<Model><Relation>FilterMeta` impls of `RelationFilterMeta`.
- [ ] `prax_schema!` does the same from a `.prax` schema string.
- [ ] Six SQL engines (PG, MySQL, SQLite, MSSQL, DuckDB) declare the capability marker traits they satisfy. MongoDB declares the two it does. ScyllaDB / Cassandra deliberately do not, and trybuild tests pin that they fail to compile when used as `SupportsRelationFilter`.
- [ ] Phase-1 tests still pass (no regressions in `prax-query`).
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace` all pass.
- [ ] `UserWhereInput { email: Some(StringFilter::contains("@x.com")) }` round-trips through `serde_json` and lowers to a runtime `Filter::Contains`.

---

## Self-review notes

- **Scope.** Phase 2 narrowly: codegen of phase-1's typed-input shapes + engine capability impls. Nested writes (phase 5), aggregate macros (phase 6), `prax generate` CLI orchestration (touches the same emitters but adds file I/O — folded in implicitly since both derive and prax_schema use the same generators), and macro-level diagnostics (phase 3+) are deliberately excluded.
- **Generators are pure.** Each takes parsed model metadata and returns a `TokenStream`. No side effects, no I/O. Lets the derive path and schema path share them trivially.
- **Phase-2 simplification: `IncludeInput` only carries `Option<bool>`.** Phase 3+ macros may need the per-relation `Args` shape from the spec (`r#where` / `order_by` / `take` etc. inside an include). Adding that field is additive — defer to when the macro DSL actually needs it.
- **Phase-2 simplification: `CreateInput` derives `Default`.** Phase 5 will replace with a strict variant.
- **Engine capability impls are mechanical.** Each engine's file is ~15 lines.
- **CQL trybuild tests document the intentional capability gap.** Without them, a future contributor adding `impl SupportsRelationFilter for ScyllaEngine` would slip through review.
