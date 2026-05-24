# Aggregate Macros Implementation Plan (Phase 6)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Ship phase 6 — Prisma-style `count!` `select:` extension, `aggregate!`, and `group_by!` macros — in a single PR on top of the existing runtime `AggregateOperation` / `GroupByOperation`.

**Architecture:** Codegen emits per-model input structs (`<Model>{Count,Sum,Avg,Min,Max}Select`), result structs (`<Model>{Count,Sum,Avg,Min,Max}Result`, `<Model>AggregateResult`, `<Model>GroupByResult`), an args struct family, a `<Model>GroupByColumn` enum, plus `aggregate()` / `group_by()` accessors and `with_aggregate_args()` / `with_group_by_args()` extension methods. Three macro entry points (`count!` extended, `aggregate!` new, `group_by!` new) parse brace-block DSL via shared lowering helpers (`aggregate_select.rs`, `having.rs`) and emit typed-input + builder-method tokens.

**Tech Stack:** Rust 2024, Cargo workspace. Codegen via `syn` 2.x / `proc-macro2` / `quote`. Lowering helpers live in `prax-codegen/src/macros/lower/`, macro entry points in `prax-codegen/src/macros/ops/`. Runtime is the existing `prax-query/src/operations/aggregate.rs` (already provides `AggregateOperation`, `GroupByOperation`, `AggregateResult`, `HavingCondition`).

**Spec:** `docs/superpowers/specs/2026-05-23-aggregate-macros-design.md`.

**Worktree:** `/home/joseph/Projects/prax/.worktrees/aggregate-macros/`, branch `feature/aggregate-macros`.

---

## Task 1: Baseline check

- [ ] **Step 1:** Confirm worktree state:
  ```
  cd /home/joseph/Projects/prax/.worktrees/aggregate-macros
  git rev-parse --abbrev-ref HEAD    # feature/aggregate-macros
  git log --oneline -2               # 4ad0292 docs(query): design spec + c916afa develop tip
  ```
- [ ] **Step 2:** `cargo check --workspace --all-features` — zero errors.
- [ ] **Step 3:** `cargo test -p prax-query --lib operations::aggregate` — green baseline (existing AggregateOperation/GroupByOperation tests).
- [ ] **Step 4:** No commit.

---

## Task 2: Codegen — per-model select-shape input structs

**Files:**
- Create: `prax-codegen/src/generators/aggregate.rs`
- Modify: `prax-codegen/src/generators/mod.rs` — `pub mod aggregate;`
- Modify: `prax-codegen/src/generators/derive.rs` — call `aggregate::emit_select_inputs(...)` and splice output into the model's input module

- [ ] **Step 1: Create the new module** `prax-codegen/src/generators/aggregate.rs`:

  ```rust
  //! Aggregate-macro support: per-model select-shape input structs,
  //! result structs, args structs, GroupByColumn enum, and the
  //! `aggregate()` / `group_by()` accessor + `with_aggregate_args` /
  //! `with_group_by_args` extension methods on AggregateOperation /
  //! GroupByOperation. Used by `count!` (extended in phase 6),
  //! `aggregate!`, and `group_by!`.

  use proc_macro2::TokenStream;
  use quote::{format_ident, quote};

  /// Information about one scalar (non-relation, non-aggregate) field
  /// on a model. Built by the caller from the existing FieldInfo loop.
  pub struct ScalarFieldMeta<'a> {
      pub ident: &'a syn::Ident,
      pub ty: &'a syn::Type,
      pub column_name: &'a str,
      pub is_numeric: bool,
      pub is_sortable: bool,
  }

  /// Emit the five per-model select-shape input structs.
  pub fn emit_select_inputs(
      model_ident: &syn::Ident,
      scalars: &[ScalarFieldMeta<'_>],
  ) -> TokenStream {
      let count_name = format_ident!("{}CountSelect", model_ident);
      let sum_name = format_ident!("{}SumSelect", model_ident);
      let avg_name = format_ident!("{}AvgSelect", model_ident);
      let min_name = format_ident!("{}MinSelect", model_ident);
      let max_name = format_ident!("{}MaxSelect", model_ident);

      // All scalars are eligible for Count and Min/Max if sortable.
      // Numeric scalars are eligible for Sum/Avg.
      let count_fields = scalars.iter().map(|f| {
          let ident = f.ident;
          quote! { pub #ident: ::core::option::Option<bool> }
      });
      let sortable_fields = scalars.iter().filter(|f| f.is_sortable).map(|f| {
          let ident = f.ident;
          quote! { pub #ident: ::core::option::Option<bool> }
      });
      let numeric_fields: Vec<_> = scalars.iter().filter(|f| f.is_numeric).map(|f| {
          let ident = f.ident;
          quote! { pub #ident: ::core::option::Option<bool> }
      }).collect();
      let sortable_for_min_max: Vec<_> = scalars.iter().filter(|f| f.is_sortable).map(|f| {
          let ident = f.ident;
          quote! { pub #ident: ::core::option::Option<bool> }
      }).collect();

      quote! {
          #[derive(Debug, Default, Clone)]
          pub struct #count_name {
              pub _all: ::core::option::Option<bool>,
              #(#count_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #sum_name {
              #(#numeric_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #avg_name {
              #(#numeric_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #min_name {
              #(#sortable_for_min_max,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #max_name {
              #(#sortable_for_min_max,)*
          }
      }
  }
  ```

  Note: the `numeric_fields` and `sortable_for_min_max` iterators are collected first because they're used twice (Sum+Avg and Min+Max respectively); `quote!` consumes them once.

- [ ] **Step 2: Register the module** in `prax-codegen/src/generators/mod.rs`:

  Add `pub mod aggregate;` next to the existing `pub mod count_struct;` (or wherever the existing per-model emitters are registered).

- [ ] **Step 3: Wire into derive emit path**.

  In `prax-codegen/src/generators/derive.rs`, locate where per-model input structs are assembled (look for where `count_struct::emit_count_struct(...)` from phase 5.5 is called). After that call, build a `Vec<ScalarFieldMeta>` from the existing `FieldInfo` collection:

  ```rust
  let scalar_meta: Vec<aggregate::ScalarFieldMeta> = fields
      .iter()
      .filter(|f| !f.is_relation && f.aggregate.is_none())
      .map(|f| aggregate::ScalarFieldMeta {
          ident: &f.ident,
          ty: &f.ty,
          column_name: f.column_name.as_str(),
          is_numeric: rust_type_is_numeric(&f.ty),
          is_sortable: rust_type_is_sortable(&f.ty),
      })
      .collect();
  let aggregate_select_inputs = aggregate::emit_select_inputs(&model_ident, &scalar_meta);
  ```

  Splice `aggregate_select_inputs` into the model's input module (alongside where `<Model>WhereInput`/etc. land).

- [ ] **Step 4: Add the type-classification helpers** at the top of `aggregate.rs`:

  ```rust
  /// True for Rust types we know are numeric. Recognises i8/16/32/64,
  /// u8/16/32/64, i128/u128, f32/f64, usize/isize, and Option<...>
  /// wrapping any of the above. Heuristic — codegen would need richer
  /// type info to handle Decimal / BigDecimal etc.
  pub fn rust_type_is_numeric(ty: &syn::Type) -> bool {
      let name = type_leaf_ident(ty);
      matches!(
          name.as_deref(),
          Some("i8" | "i16" | "i32" | "i64" | "i128"
              | "u8" | "u16" | "u32" | "u64" | "u128"
              | "f32" | "f64"
              | "isize" | "usize"
              | "Decimal" | "BigDecimal")
      )
  }

  /// True for Rust types eligible as MIN/MAX target. Numerics, strings,
  /// DateTime/NaiveDateTime/Date/Time, Uuid. Excludes JSON, bytes,
  /// vectors.
  pub fn rust_type_is_sortable(ty: &syn::Type) -> bool {
      if rust_type_is_numeric(ty) {
          return true;
      }
      let name = type_leaf_ident(ty);
      matches!(
          name.as_deref(),
          Some("String" | "str"
              | "DateTime" | "NaiveDateTime" | "NaiveDate" | "NaiveTime" | "Date" | "Time"
              | "Uuid")
      )
  }

  /// For `Foo`, returns Some("Foo"). For `Option<Foo>` returns Some("Foo").
  /// For path types like `std::time::SystemTime`, returns the last segment.
  fn type_leaf_ident(ty: &syn::Type) -> Option<String> {
      if let syn::Type::Path(tp) = ty {
          if let Some(seg) = tp.path.segments.last() {
              // Unwrap Option<T>
              if seg.ident == "Option" {
                  if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                      if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                          return type_leaf_ident(inner);
                      }
                  }
              }
              return Some(seg.ident.to_string());
          }
      }
      None
  }
  ```

- [ ] **Step 5:** Add unit tests in `aggregate.rs::tests`:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use syn::parse_quote;

      #[test]
      fn numeric_detects_basic_ints_and_floats() {
          let ty: syn::Type = parse_quote!(i32);
          assert!(rust_type_is_numeric(&ty));
          let ty: syn::Type = parse_quote!(Option<f64>);
          assert!(rust_type_is_numeric(&ty));
          let ty: syn::Type = parse_quote!(u128);
          assert!(rust_type_is_numeric(&ty));
          let ty: syn::Type = parse_quote!(String);
          assert!(!rust_type_is_numeric(&ty));
      }

      #[test]
      fn sortable_includes_string_and_datetime() {
          let ty: syn::Type = parse_quote!(String);
          assert!(rust_type_is_sortable(&ty));
          let ty: syn::Type = parse_quote!(DateTime<Utc>);
          assert!(rust_type_is_sortable(&ty));
          let ty: syn::Type = parse_quote!(Option<NaiveDateTime>);
          assert!(rust_type_is_sortable(&ty));
          let ty: syn::Type = parse_quote!(serde_json::Value);
          assert!(!rust_type_is_sortable(&ty));
      }
  }
  ```

- [ ] **Step 6:** Run `cargo test -p prax-codegen --lib generators::aggregate` — both unit tests pass. Then `cargo build --workspace --all-features` to confirm the emit doesn't break existing per-model codegen.

- [ ] **Step 7: Commit**

  ```
  feat(codegen): emit per-model aggregate-select input structs
  ```

  Body: notes the new `generators/aggregate.rs` module, `ScalarFieldMeta` parameter shape, type-classification helpers, and that emit is wired into the derive path.

---

## Task 3: Codegen — per-model result-shape output structs

**Files:**
- Modify: `prax-codegen/src/generators/aggregate.rs`
- Modify: `prax-codegen/src/generators/derive.rs` (splice into per-model output)

- [ ] **Step 1:** Extend `aggregate.rs` with `emit_result_structs`:

  ```rust
  /// Emit the five per-model result-shape output structs plus the
  /// composite UserAggregateResult / UserGroupByResult.
  pub fn emit_result_structs(
      model_ident: &syn::Ident,
      scalars: &[ScalarFieldMeta<'_>],
  ) -> TokenStream {
      let count_result = format_ident!("{}CountSelectResult", model_ident);
      let sum_result = format_ident!("{}SumResult", model_ident);
      let avg_result = format_ident!("{}AvgResult", model_ident);
      let min_result = format_ident!("{}MinResult", model_ident);
      let max_result = format_ident!("{}MaxResult", model_ident);
      let agg_result = format_ident!("{}AggregateResult", model_ident);
      let gb_result = format_ident!("{}GroupByResult", model_ident);

      let count_fields = scalars.iter().map(|f| {
          let ident = f.ident;
          quote! { pub #ident: i64 }
      });

      let numeric_fields_owned: Vec<_> = scalars.iter().filter(|f| f.is_numeric).map(|f| {
          let ident = f.ident;
          // Sum widens to i64 / f64 depending on input; simplification:
          // use Option<f64> for all sum results to handle int+float uniformly.
          // (Aggregate result types are dialect-coerced at runtime via FilterValue.)
          quote! { pub #ident: ::core::option::Option<f64> }
      }).collect();

      let avg_fields: Vec<_> = scalars.iter().filter(|f| f.is_numeric).map(|f| {
          let ident = f.ident;
          quote! { pub #ident: ::core::option::Option<f64> }
      }).collect();

      let min_max_fields: Vec<_> = scalars.iter().filter(|f| f.is_sortable).map(|f| {
          let ident = f.ident;
          let ty = f.ty;
          // Wrap T in Option<T>. If already Option<T>, leave as-is.
          let outer_ty = if is_option_type(ty) {
              quote! { #ty }
          } else {
              quote! { ::core::option::Option<#ty> }
          };
          quote! { pub #ident: #outer_ty }
      }).collect();

      // Every scalar appears in the GroupByResult as Option<T> so callers
      // can `by:` on any subset.
      let gb_scalar_fields = scalars.iter().map(|f| {
          let ident = f.ident;
          let ty = f.ty;
          let outer_ty = if is_option_type(ty) {
              quote! { #ty }
          } else {
              quote! { ::core::option::Option<#ty> }
          };
          quote! { pub #ident: #outer_ty }
      });

      quote! {
          #[derive(Debug, Default, Clone)]
          pub struct #count_result {
              pub _all: i64,
              #(#count_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #sum_result {
              #(#numeric_fields_owned,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #avg_result {
              #(#avg_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #min_result {
              #(#min_max_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #max_result {
              #(#min_max_fields,)*
          }

          #[derive(Debug, Default, Clone)]
          pub struct #agg_result {
              pub _sum:   ::core::option::Option<#sum_result>,
              pub _avg:   ::core::option::Option<#avg_result>,
              pub _min:   ::core::option::Option<#min_result>,
              pub _max:   ::core::option::Option<#max_result>,
              pub _count: ::core::option::Option<#count_result>,
          }

          #[derive(Debug, Default, Clone)]
          pub struct #gb_result {
              #(#gb_scalar_fields,)*
              pub _sum:   ::core::option::Option<#sum_result>,
              pub _avg:   ::core::option::Option<#avg_result>,
              pub _min:   ::core::option::Option<#min_result>,
              pub _max:   ::core::option::Option<#max_result>,
              pub _count: ::core::option::Option<#count_result>,
          }
      }
  }

  fn is_option_type(ty: &syn::Type) -> bool {
      if let syn::Type::Path(tp) = ty {
          if let Some(seg) = tp.path.segments.last() {
              return seg.ident == "Option";
          }
      }
      false
  }
  ```

- [ ] **Step 2:** In `derive.rs`, after the `emit_select_inputs` call, add:

  ```rust
  let aggregate_result_structs = aggregate::emit_result_structs(&model_ident, &scalar_meta);
  ```

  Splice both `aggregate_select_inputs` and `aggregate_result_structs` into the model's input/output module.

- [ ] **Step 3:** Add tests at `aggregate.rs::tests`:

  ```rust
  #[test]
  fn count_result_has_i64_all_and_one_field_per_scalar() {
      // Build a synthetic ScalarFieldMeta slice for a 2-column model.
      // Run emit_result_structs. Parse the output as a syn::File.
      // Assert that the emitted CountSelectResult struct contains `_all: i64`
      // and exactly two additional fields.
      use syn::parse_quote;
      let ident: syn::Ident = parse_quote!(User);
      let scalars = sample_scalars(&[
          ("id", parse_quote!(i32), true, true),
          ("email", parse_quote!(String), false, true),
      ]);
      let tokens = emit_result_structs(&ident, &scalars);
      let s = tokens.to_string();
      assert!(s.contains("struct UserCountSelectResult"));
      assert!(s.contains("_all : i64"));
      assert!(s.contains("id : i64"));
      assert!(s.contains("email : i64"));
      assert!(s.contains("struct UserSumResult"));
      // SumResult should only contain numeric fields (id), not email.
      let sum_block_idx = s.find("struct UserSumResult").unwrap();
      let next_close = s[sum_block_idx..].find('}').unwrap();
      let sum_body = &s[sum_block_idx..sum_block_idx + next_close];
      assert!(sum_body.contains("id :"));
      assert!(!sum_body.contains("email :"));
  }

  // Helper to build ScalarFieldMeta from a tuple slice.
  #[cfg(test)]
  fn sample_scalars(items: &[(&str, syn::Type, bool, bool)]) -> Vec<ScalarFieldMeta<'_>> {
      // Note: lifetime gymnastics — for tests, prefer building owned Vec then
      // returning references via a helper that lives in the test fn scope.
      // Simplest workable approach: write the helper inline in the test, not as a
      // module-level fn.
      unimplemented!()
  }
  ```

  The `sample_scalars` helper is awkward because of lifetimes — inline the build in each test instead. Trim to:

  ```rust
  #[test]
  fn count_result_has_i64_all_and_one_field_per_scalar() {
      use syn::parse_quote;
      let ident: syn::Ident = parse_quote!(User);
      let id_ident: syn::Ident = parse_quote!(id);
      let email_ident: syn::Ident = parse_quote!(email);
      let id_ty: syn::Type = parse_quote!(i32);
      let email_ty: syn::Type = parse_quote!(String);
      let scalars = vec![
          ScalarFieldMeta { ident: &id_ident, ty: &id_ty, column_name: "id", is_numeric: true, is_sortable: true },
          ScalarFieldMeta { ident: &email_ident, ty: &email_ty, column_name: "email", is_numeric: false, is_sortable: true },
      ];
      let s = emit_result_structs(&ident, &scalars).to_string();
      assert!(s.contains("struct UserCountSelectResult"));
      assert!(s.contains("_all : i64"));
      assert!(s.contains("struct UserSumResult"));
      let sum_idx = s.find("struct UserSumResult").unwrap();
      let close = s[sum_idx..].find('}').unwrap();
      let sum_body = &s[sum_idx..sum_idx + close];
      assert!(sum_body.contains("id :"));
      assert!(!sum_body.contains("email :"), "Sum body should not include non-numeric `email`: {sum_body}");
  }
  ```

- [ ] **Step 4:** Run `cargo test -p prax-codegen --lib generators::aggregate`. Pass.

- [ ] **Step 5: Commit**

  ```
  feat(codegen): emit per-model aggregate-result output structs
  ```

---

## Task 4: Codegen — `<Model>GroupByColumn` enum, args structs, `having`/`order_by` value types

**Files:**
- Modify: `prax-codegen/src/generators/aggregate.rs`
- Modify: `prax-codegen/src/generators/derive.rs`

- [ ] **Step 1:** Extend `aggregate.rs` with `emit_args_and_columns_enum`:

  ```rust
  pub fn emit_args_and_columns_enum(
      model_ident: &syn::Ident,
      scalars: &[ScalarFieldMeta<'_>],
  ) -> TokenStream {
      let columns_enum = format_ident!("{}GroupByColumn", model_ident);
      let args_agg = format_ident!("{}AggregateArgs", model_ident);
      let args_gb = format_ident!("{}GroupByArgs", model_ident);
      let where_input = format_ident!("{}WhereInput", model_ident);
      let count_select = format_ident!("{}CountSelect", model_ident);
      let sum_select = format_ident!("{}SumSelect", model_ident);
      let avg_select = format_ident!("{}AvgSelect", model_ident);
      let min_select = format_ident!("{}MinSelect", model_ident);
      let max_select = format_ident!("{}MaxSelect", model_ident);
      let having_ty = format_ident!("{}GroupByHaving", model_ident);
      let order_by_ty = format_ident!("{}GroupByOrderBy", model_ident);

      // Each scalar becomes a variant of the GroupByColumn enum;
      // method `column_name()` returns the SQL column string.
      let variants = scalars.iter().map(|f| {
          let v = format_ident!("{}", to_pascal_case(&f.ident.to_string()));
          quote! { #v }
      });
      let column_arms = scalars.iter().map(|f| {
          let v = format_ident!("{}", to_pascal_case(&f.ident.to_string()));
          let col = f.column_name;
          quote! { Self::#v => #col }
      });

      quote! {
          #[derive(Debug, Clone, Copy, PartialEq, Eq)]
          pub enum #columns_enum {
              #(#variants,)*
          }

          impl #columns_enum {
              pub fn column_name(&self) -> &'static str {
                  match self {
                      #(#column_arms,)*
                  }
              }
          }

          #[derive(Debug, Default, Clone)]
          pub struct #args_agg {
              pub where_input: ::core::option::Option<#where_input>,
              pub _sum:   ::core::option::Option<#sum_select>,
              pub _avg:   ::core::option::Option<#avg_select>,
              pub _min:   ::core::option::Option<#min_select>,
              pub _max:   ::core::option::Option<#max_select>,
              pub _count: ::core::option::Option<#count_select>,
          }

          #[derive(Debug, Default, Clone)]
          pub struct #args_gb {
              pub by:           ::std::vec::Vec<#columns_enum>,
              pub where_input:  ::core::option::Option<#where_input>,
              pub _sum:         ::core::option::Option<#sum_select>,
              pub _avg:         ::core::option::Option<#avg_select>,
              pub _min:         ::core::option::Option<#min_select>,
              pub _max:         ::core::option::Option<#max_select>,
              pub _count:       ::core::option::Option<#count_select>,
              pub having:       ::core::option::Option<#having_ty>,
              pub order_by:     ::core::option::Option<#order_by_ty>,
          }

          // Sketch types for having and order_by — concrete fields land in
          // Task 9 / Task 10. For now, define empty structs so the args
          // struct compiles end-to-end and the macros can populate them
          // later.
          #[derive(Debug, Default, Clone)]
          pub struct #having_ty {
              pub conditions: ::std::vec::Vec<::prax_query::operations::aggregate::HavingCondition>,
          }

          #[derive(Debug, Default, Clone)]
          pub struct #order_by_ty {
              pub items: ::std::vec::Vec<(::std::string::String, ::prax_query::operations::aggregate::SortDirection)>,
          }
      }
  }

  fn to_pascal_case(snake: &str) -> String {
      let mut out = String::with_capacity(snake.len());
      let mut upper = true;
      for c in snake.chars() {
          if c == '_' {
              upper = true;
          } else if upper {
              out.push(c.to_ascii_uppercase());
              upper = false;
          } else {
              out.push(c);
          }
      }
      out
  }
  ```

  Note on `HavingCondition` and `SortDirection`: these are runtime types in `prax-query/src/operations/aggregate.rs`. Check the actual type and module path with `grep -nE "pub struct HavingCondition\|pub enum SortDirection\|pub enum Sort" prax-query/src/operations/aggregate.rs` before committing — adjust the `::prax_query::operations::aggregate::...` path if it's `Sort` instead of `SortDirection`, or if `HavingCondition` lives at a different module path.

- [ ] **Step 2:** Wire into `derive.rs`:

  ```rust
  let aggregate_args = aggregate::emit_args_and_columns_enum(&model_ident, &scalar_meta);
  ```

  Splice into the model's input module alongside the select inputs and result structs.

- [ ] **Step 3:** Add a test:

  ```rust
  #[test]
  fn group_by_column_enum_has_variant_per_scalar() {
      use syn::parse_quote;
      let ident: syn::Ident = parse_quote!(User);
      let id_ident: syn::Ident = parse_quote!(team_id);
      let region_ident: syn::Ident = parse_quote!(region);
      let i32_ty: syn::Type = parse_quote!(i32);
      let str_ty: syn::Type = parse_quote!(String);
      let scalars = vec![
          ScalarFieldMeta { ident: &id_ident, ty: &i32_ty, column_name: "team_id", is_numeric: true, is_sortable: true },
          ScalarFieldMeta { ident: &region_ident, ty: &str_ty, column_name: "region", is_numeric: false, is_sortable: true },
      ];
      let s = emit_args_and_columns_enum(&ident, &scalars).to_string();
      assert!(s.contains("enum UserGroupByColumn"));
      assert!(s.contains("TeamId"));
      assert!(s.contains("Region"));
      assert!(s.contains("Self :: TeamId => \"team_id\""));
      assert!(s.contains("struct UserAggregateArgs"));
      assert!(s.contains("struct UserGroupByArgs"));
  }
  ```

- [ ] **Step 4:** Run tests, fix anything that doesn't quote as expected (whitespace in `quote!` output is normalized — use `s.replace(' ', "")` for tighter assertions if needed).

- [ ] **Step 5: Commit**

  ```
  feat(codegen): emit per-model GroupByColumn enum and aggregate args structs
  ```

---

## Task 5: Codegen — `aggregate()` / `group_by()` accessors + `with_*_args` extensions

**Files:**
- Modify: `prax-codegen/src/generators/aggregate.rs`
- Modify: `prax-codegen/src/generators/derive.rs`

- [ ] **Step 1:** Add `emit_accessors_and_extensions`:

  ```rust
  pub fn emit_accessors_and_extensions(
      model_ident: &syn::Ident,
      scalars: &[ScalarFieldMeta<'_>],
  ) -> TokenStream {
      let accessor_ty = format_ident!("{}Accessor", model_ident);   // existing per-model accessor
      let agg_args = format_ident!("{}AggregateArgs", model_ident);
      let gb_args = format_ident!("{}GroupByArgs", model_ident);
      let count_select = format_ident!("{}CountSelect", model_ident);
      let sum_select = format_ident!("{}SumSelect", model_ident);
      let avg_select = format_ident!("{}AvgSelect", model_ident);
      let min_select = format_ident!("{}MinSelect", model_ident);
      let max_select = format_ident!("{}MaxSelect", model_ident);
      let columns_enum = format_ident!("{}GroupByColumn", model_ident);

      // Build `fields_set()` impls for each select struct: returns
      // Vec<&'static str> of the column names whose Option<bool> is Some(true).
      let count_fields_set = scalars.iter().map(|f| {
          let ident = f.ident;
          let col = f.column_name;
          quote! {
              if matches!(self.#ident, ::core::option::Option::Some(true)) {
                  out.push(#col);
              }
          }
      });
      let numeric_fields_set = scalars.iter().filter(|f| f.is_numeric).map(|f| {
          let ident = f.ident;
          let col = f.column_name;
          quote! {
              if matches!(self.#ident, ::core::option::Option::Some(true)) {
                  out.push(#col);
              }
          }
      });
      let sortable_fields_set = scalars.iter().filter(|f| f.is_sortable).map(|f| {
          let ident = f.ident;
          let col = f.column_name;
          quote! {
              if matches!(self.#ident, ::core::option::Option::Some(true)) {
                  out.push(#col);
              }
          }
      });

      let numeric_fields_set_2 = numeric_fields_set.clone(); // used twice (Sum + Avg)
      let sortable_fields_set_2 = sortable_fields_set.clone(); // Min + Max

      quote! {
          impl #count_select {
              /// Column names whose Option<bool> is Some(true), excluding `_all`.
              pub fn fields_set(&self) -> ::std::vec::Vec<&'static str> {
                  let mut out = ::std::vec::Vec::new();
                  #(#count_fields_set)*
                  out
              }
              pub fn all_set(&self) -> bool {
                  matches!(self._all, ::core::option::Option::Some(true))
              }
          }
          impl #sum_select {
              pub fn fields_set(&self) -> ::std::vec::Vec<&'static str> {
                  let mut out = ::std::vec::Vec::new();
                  #(#numeric_fields_set)*
                  out
              }
          }
          impl #avg_select {
              pub fn fields_set(&self) -> ::std::vec::Vec<&'static str> {
                  let mut out = ::std::vec::Vec::new();
                  #(#numeric_fields_set_2)*
                  out
              }
          }
          impl #min_select {
              pub fn fields_set(&self) -> ::std::vec::Vec<&'static str> {
                  let mut out = ::std::vec::Vec::new();
                  #(#sortable_fields_set)*
                  out
              }
          }
          impl #max_select {
              pub fn fields_set(&self) -> ::std::vec::Vec<&'static str> {
                  let mut out = ::std::vec::Vec::new();
                  #(#sortable_fields_set_2)*
                  out
              }
          }

          impl<E: ::prax_query::traits::QueryEngine> #accessor_ty<E> {
              pub fn aggregate(&self) -> ::prax_query::operations::aggregate::AggregateOperation<#model_ident, E> {
                  ::prax_query::operations::aggregate::AggregateOperation::with_engine(self.engine.clone())
              }
              pub fn group_by(&self, by: ::std::vec::Vec<#columns_enum>) -> ::prax_query::operations::aggregate::GroupByOperation<#model_ident, E> {
                  let cols: ::std::vec::Vec<::std::string::String> =
                      by.iter().map(|c| c.column_name().to_string()).collect();
                  ::prax_query::operations::aggregate::GroupByOperation::with_engine(self.engine.clone(), cols)
              }
          }

          impl<E: ::prax_query::traits::QueryEngine + ::core::clone::Clone> ::prax_query::operations::aggregate::AggregateOperation<#model_ident, E> {
              pub fn with_aggregate_args(mut self, args: #agg_args) -> Self {
                  if let ::core::option::Option::Some(w) = args.where_input {
                      self = self.with_where_input(w);
                  }
                  if let ::core::option::Option::Some(c) = args._count {
                      if c.all_set() { self = self.count(); }
                      for col in c.fields_set() { self = self.count_column(col); }
                  }
                  if let ::core::option::Option::Some(s) = args._sum {
                      for col in s.fields_set() { self = self.sum(col); }
                  }
                  if let ::core::option::Option::Some(a) = args._avg {
                      for col in a.fields_set() { self = self.avg(col); }
                  }
                  if let ::core::option::Option::Some(m) = args._min {
                      for col in m.fields_set() { self = self.min(col); }
                  }
                  if let ::core::option::Option::Some(m) = args._max {
                      for col in m.fields_set() { self = self.max(col); }
                  }
                  self
              }
          }

          impl<E: ::prax_query::traits::QueryEngine + ::core::clone::Clone> ::prax_query::operations::aggregate::GroupByOperation<#model_ident, E> {
              pub fn with_group_by_args(mut self, args: #gb_args) -> Self {
                  if let ::core::option::Option::Some(w) = args.where_input {
                      self = self.with_where_input(w);
                  }
                  if let ::core::option::Option::Some(c) = args._count {
                      if c.all_set() { self = self.count(); }
                      for col in c.fields_set() { self = self.count_column(col); }
                  }
                  if let ::core::option::Option::Some(s) = args._sum {
                      for col in s.fields_set() { self = self.sum(col); }
                  }
                  if let ::core::option::Option::Some(a) = args._avg {
                      for col in a.fields_set() { self = self.avg(col); }
                  }
                  if let ::core::option::Option::Some(m) = args._min {
                      for col in m.fields_set() { self = self.min(col); }
                  }
                  if let ::core::option::Option::Some(m) = args._max {
                      for col in m.fields_set() { self = self.max(col); }
                  }
                  if let ::core::option::Option::Some(h) = args.having {
                      for cond in h.conditions { self = self.having(cond); }
                  }
                  // order_by lowering lands when Task 8 wires the macro DSL.
                  let _ = args.order_by;
                  self
              }
          }
      }
  }
  ```

  **Verify** before committing:
  - `GroupByOperation` actually accepts `Vec<String>` for columns (or `Vec<&str>` — check signature). Adapt if needed.
  - `AggregateOperation::with_where_input` and `count_column` / `count_distinct` actually exist with those names. If `count_column` doesn't take `impl Into<String>`, adapt the call.
  - The accessor field is named `engine` and is `Clone`-able (or use a different binding name per existing patterns in `prax-codegen/src/generators/relation_accessors.rs`).

- [ ] **Step 2:** Wire `emit_accessors_and_extensions` into `derive.rs`:

  ```rust
  let aggregate_accessors = aggregate::emit_accessors_and_extensions(&model_ident, &scalar_meta);
  ```

  Splice into the model's output module **outside** the input mod block (these are `impl` blocks for the engine accessor and operation types).

- [ ] **Step 3:** Add an integration smoke test in `tests/aggregate_macros_e2e.rs` (created in Task 13 — placeholder for now):

  Or skip and rely on the compile-check sweep:

  ```bash
  cd /home/joseph/Projects/prax/.worktrees/aggregate-macros
  cargo build --workspace --all-features
  ```

  All emitted impls compile.

- [ ] **Step 4: Commit**

  ```
  feat(codegen): emit aggregate/group_by accessors + with_*_args extension impls
  ```

---

## Task 6: Lowering helper — `aggregate_select.rs`

**Files:**
- Create: `prax-codegen/src/macros/lower/aggregate_select.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs` — `pub mod aggregate_select;`

- [ ] **Step 1:** Create `aggregate_select.rs`:

  ```rust
  //! Lower a `_<agg>: { col: true, ... }` DSL brace block to a typed
  //! `<Model><Agg>Select` constructor TokenStream. Shared between
  //! `count!`, `aggregate!`, and `group_by!`.

  use proc_macro2::TokenStream;
  use quote::{format_ident, quote};

  use super::LowerCtx;
  use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};

  /// Aggregate kinds the lowering recognises.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum AggKind {
      Count,
      Sum,
      Avg,
      Min,
      Max,
  }

  impl AggKind {
      pub fn select_struct_suffix(&self) -> &'static str {
          match self {
              Self::Count => "CountSelect",
              Self::Sum => "SumSelect",
              Self::Avg => "AvgSelect",
              Self::Min => "MinSelect",
              Self::Max => "MaxSelect",
          }
      }
      pub fn key(&self) -> &'static str {
          match self {
              Self::Count => "_count",
              Self::Sum => "_sum",
              Self::Avg => "_avg",
              Self::Min => "_min",
              Self::Max => "_max",
          }
      }
  }

  /// Lower one `_<agg>: { col: true, ... }` block to a tokens expression
  /// that constructs `<Model><Agg>Select { <col>: Some(true), ... }`.
  pub fn lower_agg_select(
      kind: AggKind,
      block: &DslBlock,
      ctx: &LowerCtx<'_>,
  ) -> syn::Result<TokenStream> {
      let struct_ident = format_ident!("{}{}", ctx.model.name(), kind.select_struct_suffix());

      let mut setters: Vec<TokenStream> = Vec::new();

      for entry in &block.fields {
          let DslField::Pair { key, value, .. } = entry else {
              return Err(syn::Error::new(
                  proc_macro2::Span::call_site(),
                  format!("`{}` block does not support spread or conditional fields", kind.key()),
              ));
          };
          let key_str = key.to_string();

          // Validate value is `true` literal.
          if !matches!(value, DslValue::Bool(true)) {
              return Err(syn::Error::new(
                  key.span(),
                  format!(
                      "value for `{}.{}` must be `true` (only opt-in is supported)",
                      kind.key(), key_str
                  ),
              ));
          }

          // Special: `_all` only valid in Count blocks.
          if key_str == "_all" {
              if kind != AggKind::Count {
                  return Err(syn::Error::new(
                      key.span(),
                      format!("`_all` is only valid inside `_count`, not `{}`", kind.key()),
                  ));
              }
              setters.push(quote! { __s._all = ::core::option::Option::Some(true); });
              continue;
          }

          // Validate column exists on the model and is appropriate for kind.
          let field = ctx.model.get_field(&key_str).ok_or_else(|| {
              let candidates: Vec<String> =
                  ctx.model.fields.keys().map(|k| k.to_string()).collect();
              let suggestion = crate::macros::validate::suggest(&key_str, &candidates);
              let msg = match suggestion {
                  Some(s) => format!(
                      "unknown column `{}` on model `{}`; did you mean `{}`?",
                      key_str, ctx.model.name(), s
                  ),
                  None => format!(
                      "unknown column `{}` on model `{}`",
                      key_str, ctx.model.name()
                  ),
              };
              syn::Error::new(key.span(), msg)
          })?;

          if field.is_relation() {
              return Err(syn::Error::new(
                  key.span(),
                  format!(
                      "field `{}` is a relation; aggregates require a scalar column",
                      key_str
                  ),
              ));
          }
          if field.aggregate().is_some() {
              return Err(syn::Error::new(
                  key.span(),
                  format!(
                      "field `{}` is itself an aggregate; cannot aggregate an aggregate",
                      key_str
                  ),
              ));
          }

          // For Sum/Avg, the field must be numeric (rough heuristic at codegen).
          if matches!(kind, AggKind::Sum | AggKind::Avg) {
              if !field_is_numeric_scalar(field) {
                  return Err(syn::Error::new(
                      key.span(),
                      format!(
                          "field `{}` is not numeric; `{}` requires a numeric column",
                          key_str, kind.key()
                      ),
                  ));
              }
          }

          let col_ident = format_ident!("{}", key_str);
          setters.push(quote! {
              __s.#col_ident = ::core::option::Option::Some(true);
          });
      }

      if setters.is_empty() {
          return Err(syn::Error::new(
              proc_macro2::Span::call_site(),
              format!("`{}` block is empty; specify at least one column or remove the block", kind.key()),
          ));
      }

      let module_ident = format_ident!("{}", ctx.model.name().to_case(convert_case::Case::Snake));
      Ok(quote! {{
          let mut __s: #module_ident::#struct_ident =
              <#module_ident::#struct_ident as ::core::default::Default>::default();
          #(#setters)*
          __s
      }})
  }

  /// Determine whether a schema field's type is numeric. Look at
  /// `field.field_type` — `FieldType::Scalar(ScalarType::Int|BigInt|Float|...)`.
  fn field_is_numeric_scalar(field: &prax_schema::ast::Field) -> bool {
      use prax_schema::ast::{FieldType, ScalarType};
      matches!(
          field.field_type,
          FieldType::Scalar(ScalarType::Int)
              | FieldType::Scalar(ScalarType::BigInt)
              | FieldType::Scalar(ScalarType::Float)
              | FieldType::Scalar(ScalarType::Decimal)
      )
  }
  ```

  Adjust the `ScalarType` variant names to whatever the actual enum has (`grep -n "pub enum ScalarType" prax-schema/src/ast/types.rs`).

- [ ] **Step 2:** Register the module in `prax-codegen/src/macros/lower/mod.rs`: `pub mod aggregate_select;`.

- [ ] **Step 3:** Unit tests in `aggregate_select.rs::tests`:

  ```rust
  #[cfg(test)]
  mod tests {
      // Build a minimal LowerCtx with a fake Model that has a numeric `id`
      // and a string `email` field. Verify:
      //  - lower_agg_select(Count, { _all: true, email: true }) → success
      //  - lower_agg_select(Sum, { email: true }) → Err "is not numeric"
      //  - lower_agg_select(Min, { unknown: true }) → Err "unknown column"
      //  - lower_agg_select(Count, { _all: false }) → Err "must be true"
      //
      // The exact mechanics depend on how other lowering tests build a
      // LowerCtx — copy the pattern from
      // prax-codegen/src/macros/lower/select_input.rs::tests.
  }
  ```

- [ ] **Step 4:** `cargo test -p prax-codegen --lib macros::lower::aggregate_select` — all pass.

- [ ] **Step 5: Commit**

  ```
  feat(codegen): aggregate_select lowering helper
  ```

---

## Task 7: Lowering helper — `having.rs`

**Files:**
- Create: `prax-codegen/src/macros/lower/having.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs` — `pub mod having;`

- [ ] **Step 1:** Create `having.rs`:

  ```rust
  //! Lower a `having: { _count: { _all: { gt: 5 } }, _sum: { views: { gte: 100 } } }`
  //! block to a Vec<HavingCondition> token expression.

  use proc_macro2::TokenStream;
  use quote::{format_ident, quote};

  use super::LowerCtx;
  use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
  use crate::macros::lower::aggregate_select::AggKind;

  /// Lower the having block into a Vec<HavingCondition> expression.
  pub fn lower_having(
      block: &DslBlock,
      ctx: &LowerCtx<'_>,
  ) -> syn::Result<TokenStream> {
      let mut conditions: Vec<TokenStream> = Vec::new();

      for entry in &block.fields {
          let DslField::Pair { key, value, .. } = entry else {
              return Err(syn::Error::new(
                  proc_macro2::Span::call_site(),
                  "having block does not support spread or conditional fields",
              ));
          };
          let agg_key = key.to_string();
          let kind = match agg_key.as_str() {
              "_count" => AggKind::Count,
              "_sum" => AggKind::Sum,
              "_avg" => AggKind::Avg,
              "_min" => AggKind::Min,
              "_max" => AggKind::Max,
              other => return Err(syn::Error::new(
                  key.span(),
                  format!("unknown having key `{}`; use one of _count/_sum/_avg/_min/_max", other),
              )),
          };
          let DslValue::Block(inner) = value else {
              return Err(syn::Error::new(
                  key.span(),
                  format!("having `{}` value must be a `{{ col: {{ op: value }} }}` block", agg_key),
              ));
          };

          // Inner: { col_name: { op: value } }, e.g. _all: { gt: 5 }
          for col_entry in &inner.fields {
              let DslField::Pair { key: col_key, value: col_val, .. } = col_entry else {
                  return Err(syn::Error::new(
                      proc_macro2::Span::call_site(),
                      "having column block does not support spread",
                  ));
              };
              let col = col_key.to_string();
              let DslValue::Block(op_block) = col_val else {
                  return Err(syn::Error::new(
                      col_key.span(),
                      format!("having `{}.{}` must be a `{{ op: value }}` block", agg_key, col),
                  ));
              };
              // Inner-inner: { op: value }, e.g. { gt: 5 }
              for op_entry in &op_block.fields {
                  let DslField::Pair { key: op_key, value: op_val, .. } = op_entry else { continue; };
                  let op = op_key.to_string();
                  // Convert the DSL value to a Rust expression.
                  let expr = dsl_value_to_expr(op_val)?;
                  let ctor = match (kind, op.as_str()) {
                      (AggKind::Count, "gt") => quote! { ::prax_query::operations::aggregate::HavingCondition::count_gt(#expr as f64) },
                      (AggKind::Count, "gte") => quote! { ::prax_query::operations::aggregate::HavingCondition::count_gte(#expr as f64) },
                      (AggKind::Count, "lt") => quote! { ::prax_query::operations::aggregate::HavingCondition::count_lt(#expr as f64) },
                      (AggKind::Count, "lte") => quote! { ::prax_query::operations::aggregate::HavingCondition::count_lte(#expr as f64) },
                      (AggKind::Count, "equals") => quote! { ::prax_query::operations::aggregate::HavingCondition::count_eq(#expr as f64) },
                      // Add other (Sum, Avg, Min, Max) × (op) variants by mirroring the
                      // existing HavingCondition constructor names — inspect
                      // prax-query/src/operations/aggregate.rs for the actual list.
                      _ => return Err(syn::Error::new(
                          op_key.span(),
                          format!("unsupported having operator `{}` for `{}`", op, agg_key),
                      )),
                  };
                  conditions.push(ctor);
              }
          }
      }

      let _ = ctx;  // future: schema-aware col validation against the agg block
      Ok(quote! {
          {
              let mut __conds: ::std::vec::Vec<::prax_query::operations::aggregate::HavingCondition> = ::std::vec::Vec::new();
              #( __conds.push(#conditions); )*
              __conds
          }
      })
  }

  fn dsl_value_to_expr(v: &DslValue) -> syn::Result<TokenStream> {
      match v {
          DslValue::Int(i) => Ok(quote! { #i }),
          DslValue::Float(f) => Ok(quote! { #f }),
          DslValue::String(s) => Ok(quote! { #s }),
          _ => Err(syn::Error::new(
              proc_macro2::Span::call_site(),
              "having operator value must be a numeric or string literal",
          )),
      }
  }
  ```

  **IMPORTANT**: inspect `prax-query/src/operations/aggregate.rs` for the actual `HavingCondition` constructor names. They likely cover only `count_*` currently — if `sum_gt`, `avg_gt`, etc. don't exist, extend `HavingCondition` with the missing constructors as part of this task (small additive change to the runtime crate). Document any added constructors in the commit.

- [ ] **Step 2:** Register module in `lower/mod.rs`.

- [ ] **Step 3:** Tests:

  ```rust
  #[cfg(test)]
  mod tests {
      // - lower_having({ _count: { _all: { gt: 5 } } }) → contains count_gt(5_f64)
      // - lower_having({ _bad_key: ... }) → Err "unknown having key"
      // - lower_having({ _count: { _all: { like: 5 } } }) → Err "unsupported having operator"
  }
  ```

- [ ] **Step 4: Commit**

  ```
  feat(codegen): having lowering helper
  ```

  If you had to add `HavingCondition::sum_gt` etc., split into two commits with `feat(query): expand HavingCondition for non-count aggregates` first.

---

## Task 8: Extend `count!` with `select:` block

**Files:**
- Modify: `prax-codegen/src/macros/ops/count.rs`

- [ ] **Step 1:** Locate the existing `count!` lowering. There's a rejection at "select: on count! is a phase-6 feature" — replace it with real parsing using `aggregate_select::lower_agg_select(AggKind::Count, ...)`.

- [ ] **Step 2:** Sketch:

  ```rust
  // Top-level count! block parser:
  let mut where_block: Option<&DslBlock> = None;
  let mut select_block: Option<&DslBlock> = None;
  for field in &input.fields {
      let DslField::Pair { key, value, .. } = field else { continue; };
      match key.to_string().as_str() {
          "where" => { /* existing handling */ where_block = ... }
          "select" => {
              let DslValue::Block(b) = value else { return Err(...); };
              select_block = Some(b);
          }
          other => return Err(syn::Error::new(key.span(), format!("unknown key `{}`", other))),
      }
  }

  if let Some(select) = select_block {
      // Lower to: client.<accessor>.aggregate().with_aggregate_args(UserAggregateArgs { _count: Some(<select>), where_input: <where>, ..Default::default() })
      // Result type: <Model>CountSelectResult
      let select_ts = crate::macros::lower::aggregate_select::lower_agg_select(
          crate::macros::lower::aggregate_select::AggKind::Count,
          select,
          &ctx,
      )?;
      // Wrap as Some(..) and place in UserAggregateArgs._count.
      // Lower where_block if present.
      let where_ts = match where_block {
          Some(w) => {
              let wts = crate::macros::lower::where_input::lower_where_input(w, &ctx)?.where_input;
              quote! { ::core::option::Option::Some(#wts) }
          }
          None => quote! { ::core::option::Option::None },
      };
      let args_ident = format_ident!("{}AggregateArgs", ctx.model.name());
      let module_ident = format_ident!("{}", ctx.model.name().to_case(convert_case::Case::Snake));

      return Ok(quote! {
          {
              let __args: #module_ident::#args_ident = #module_ident::#args_ident {
                  where_input: #where_ts,
                  _count: ::core::option::Option::Some(#select_ts),
                  ..::core::default::Default::default()
              };
              <#accessor_expr>.aggregate().with_aggregate_args(__args)
          }
      });
  }
  // Fall through to existing plain-i64 count! lowering.
  ```

  The exact accessor-expression form depends on the existing `count!` plumbing — match the pattern (probably `ctx.accessor_expr`).

- [ ] **Step 3:** A small unit test in `ops/count.rs::tests` (or wherever existing count tests live) that the `select:` path emits a `with_aggregate_args` call. Use the `lower_macro_input_to_string` helper if one exists, else add one.

- [ ] **Step 4: Commit**

  ```
  feat(codegen): count! select: block (phase 6 Prisma-style per-column counts)
  ```

---

## Task 9: New `aggregate!` macro entry point

**Files:**
- Create: `prax-codegen/src/macros/ops/aggregate.rs`
- Modify: `prax-codegen/src/macros/ops/mod.rs` — `pub mod aggregate;`
- Modify: `prax-codegen/src/lib.rs` — register `#[proc_macro] pub fn aggregate(...)` re-export
- Modify: `prax-orm/src/lib.rs` — re-export `aggregate!` from `prax_codegen`

- [ ] **Step 1:** Create `ops/aggregate.rs`:

  ```rust
  //! `aggregate!` proc-macro entry point.

  use proc_macro2::TokenStream;
  use quote::{format_ident, quote};
  use convert_case::{Case, Casing};

  use crate::macros::accessor::AccessorCall;
  use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
  use crate::macros::lower::aggregate_select::{AggKind, lower_agg_select};
  use crate::macros::lower::where_input;
  use crate::macros::schema_resolve::resolve_model_for_accessor;
  use crate::macros::validate;

  pub fn aggregate_macro_impl(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
      let parsed: AccessorCall = match syn::parse(input) {
          Ok(x) => x,
          Err(e) => return e.to_compile_error().into(),
      };
      match lower_aggregate(parsed) {
          Ok(ts) => ts.into(),
          Err(e) => e.to_compile_error().into(),
      }
  }

  fn lower_aggregate(call: AccessorCall) -> syn::Result<TokenStream> {
      let ctx = resolve_model_for_accessor(&call)?;
      let block = &call.block;

      let mut where_block: Option<&DslBlock> = None;
      let mut count_block: Option<&DslBlock> = None;
      let mut sum_block: Option<&DslBlock> = None;
      let mut avg_block: Option<&DslBlock> = None;
      let mut min_block: Option<&DslBlock> = None;
      let mut max_block: Option<&DslBlock> = None;

      for entry in &block.fields {
          let DslField::Pair { key, value, .. } = entry else {
              return Err(syn::Error::new(
                  proc_macro2::Span::call_site(),
                  "aggregate! does not support spread or conditional top-level fields",
              ));
          };
          let key_str = key.to_string();
          let block_opt = |v: &DslValue| {
              if let DslValue::Block(b) = v { Ok(b) } else {
                  Err(syn::Error::new(key.span(), format!("`{}` value must be a `{{ ... }}` block", key_str)))
              }
          };
          match key_str.as_str() {
              "where" => where_block = Some(block_opt(value)?),
              "_count" => count_block = Some(block_opt(value)?),
              "_sum" => sum_block = Some(block_opt(value)?),
              "_avg" => avg_block = Some(block_opt(value)?),
              "_min" => min_block = Some(block_opt(value)?),
              "_max" => max_block = Some(block_opt(value)?),
              other => return Err(syn::Error::new(
                  key.span(),
                  format!("unknown key `{}` in aggregate! — expected where/_count/_sum/_avg/_min/_max", other),
              )),
          }
      }

      if count_block.is_none() && sum_block.is_none() && avg_block.is_none()
          && min_block.is_none() && max_block.is_none() {
          return Err(syn::Error::new(
              proc_macro2::Span::call_site(),
              "aggregate! requires at least one of _count, _sum, _avg, _min, _max",
          ));
      }

      let lower_opt = |k: AggKind, b: Option<&DslBlock>| -> syn::Result<TokenStream> {
          match b {
              Some(blk) => {
                  let ts = lower_agg_select(k, blk, &ctx)?;
                  Ok(quote! { ::core::option::Option::Some(#ts) })
              }
              None => Ok(quote! { ::core::option::Option::None }),
          }
      };

      let count_ts = lower_opt(AggKind::Count, count_block)?;
      let sum_ts = lower_opt(AggKind::Sum, sum_block)?;
      let avg_ts = lower_opt(AggKind::Avg, avg_block)?;
      let min_ts = lower_opt(AggKind::Min, min_block)?;
      let max_ts = lower_opt(AggKind::Max, max_block)?;

      let where_ts = match where_block {
          Some(w) => {
              let wts = where_input::lower_where_input(w, &ctx)?.where_input;
              quote! { ::core::option::Option::Some(#wts) }
          }
          None => quote! { ::core::option::Option::None },
      };

      let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
      let args_ident = format_ident!("{}AggregateArgs", ctx.model.name());
      let accessor_expr = &call.accessor_expr;

      Ok(quote! {
          {
              let __args: #module_ident::#args_ident = #module_ident::#args_ident {
                  where_input: #where_ts,
                  _count: #count_ts,
                  _sum: #sum_ts,
                  _avg: #avg_ts,
                  _min: #min_ts,
                  _max: #max_ts,
              };
              (#accessor_expr).aggregate().with_aggregate_args(__args)
          }
      })
  }
  ```

- [ ] **Step 2:** Register the proc-macro entry. In `prax-codegen/src/lib.rs` find where `find_many` is registered and add:

  ```rust
  #[proc_macro]
  pub fn aggregate(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
      crate::macros::ops::aggregate::aggregate_macro_impl(input)
  }
  ```

- [ ] **Step 3:** Re-export from `prax-orm/src/lib.rs` next to other `pub use prax_codegen::*;` re-exports.

- [ ] **Step 4:** Smoke test — workspace builds.

  ```bash
  cargo build --workspace --all-features
  ```

- [ ] **Step 5: Commit**

  ```
  feat(codegen): aggregate! macro entry point (phase 6)
  ```

---

## Task 10: New `group_by!` macro entry point

**Files:**
- Create: `prax-codegen/src/macros/ops/group_by.rs`
- Modify: `prax-codegen/src/macros/ops/mod.rs` — `pub mod group_by;`
- Modify: `prax-codegen/src/lib.rs` — proc-macro registration
- Modify: `prax-orm/src/lib.rs` — re-export

- [ ] **Step 1:** Create `ops/group_by.rs` (mirrors `aggregate.rs` but adds parsing of `by:`, `having:`, `order_by:`):

  ```rust
  //! `group_by!` proc-macro entry point.

  use proc_macro2::TokenStream;
  use quote::{format_ident, quote};
  use convert_case::{Case, Casing};

  use crate::macros::accessor::AccessorCall;
  use crate::macros::dsl::ast::{DslBlock, DslField, DslValue, DslList};
  use crate::macros::lower::aggregate_select::{AggKind, lower_agg_select};
  use crate::macros::lower::having::lower_having;
  use crate::macros::lower::where_input;
  use crate::macros::schema_resolve::resolve_model_for_accessor;

  pub fn group_by_macro_impl(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
      let parsed: AccessorCall = match syn::parse(input) {
          Ok(x) => x,
          Err(e) => return e.to_compile_error().into(),
      };
      match lower_group_by(parsed) {
          Ok(ts) => ts.into(),
          Err(e) => e.to_compile_error().into(),
      }
  }

  fn lower_group_by(call: AccessorCall) -> syn::Result<TokenStream> {
      let ctx = resolve_model_for_accessor(&call)?;
      let block = &call.block;

      let mut by_list: Option<&DslList> = None;
      let mut where_block: Option<&DslBlock> = None;
      let mut count_block: Option<&DslBlock> = None;
      let mut sum_block: Option<&DslBlock> = None;
      let mut avg_block: Option<&DslBlock> = None;
      let mut min_block: Option<&DslBlock> = None;
      let mut max_block: Option<&DslBlock> = None;
      let mut having_block: Option<&DslBlock> = None;
      // order_by handling deferred (see TODO below).

      for entry in &block.fields {
          let DslField::Pair { key, value, .. } = entry else {
              return Err(syn::Error::new(
                  proc_macro2::Span::call_site(),
                  "group_by! does not support spread or conditional top-level fields",
              ));
          };
          let key_str = key.to_string();
          match key_str.as_str() {
              "by" => {
                  let DslValue::List(l) = value else {
                      return Err(syn::Error::new(key.span(), "`by:` value must be a `[col1, col2]` list"));
                  };
                  by_list = Some(l);
              }
              "where" => {
                  let DslValue::Block(b) = value else {
                      return Err(syn::Error::new(key.span(), "`where:` value must be a `{ ... }` block"));
                  };
                  where_block = Some(b);
              }
              "_count" | "_sum" | "_avg" | "_min" | "_max" => {
                  let DslValue::Block(b) = value else {
                      return Err(syn::Error::new(key.span(), format!("`{}` value must be a `{{ ... }}` block", key_str)));
                  };
                  match key_str.as_str() {
                      "_count" => count_block = Some(b),
                      "_sum" => sum_block = Some(b),
                      "_avg" => avg_block = Some(b),
                      "_min" => min_block = Some(b),
                      "_max" => max_block = Some(b),
                      _ => unreachable!(),
                  }
              }
              "having" => {
                  let DslValue::Block(b) = value else {
                      return Err(syn::Error::new(key.span(), "`having:` value must be a `{ ... }` block"));
                  };
                  having_block = Some(b);
              }
              "order_by" => {
                  // Deferred to follow-up; macro-time error.
                  return Err(syn::Error::new(
                      key.span(),
                      "`order_by:` on group_by! is not yet implemented in phase 6 — track as follow-up",
                  ));
              }
              other => return Err(syn::Error::new(
                  key.span(),
                  format!("unknown key `{}` in group_by!", other),
              )),
          }
      }

      let by_list = by_list.ok_or_else(|| syn::Error::new(
          proc_macro2::Span::call_site(),
          "group_by! requires a `by: [...]` list of columns",
      ))?;
      if by_list.items.is_empty() {
          return Err(syn::Error::new(
              proc_macro2::Span::call_site(),
              "group_by! requires at least one column in `by:`",
          ));
      }

      // Validate each by-column and build GroupByColumn variants.
      let column_enum = format_ident!("{}GroupByColumn", ctx.model.name());
      let module_ident = format_ident!("{}", ctx.model.name().to_case(Case::Snake));
      let mut by_variants: Vec<TokenStream> = Vec::new();
      for item in &by_list.items {
          let DslValue::Ident(name) = item else {
              return Err(syn::Error::new(
                  proc_macro2::Span::call_site(),
                  "`by:` items must be bare column identifiers",
              ));
          };
          let col_str = name.to_string();
          let field = ctx.model.get_field(&col_str).ok_or_else(|| {
              let candidates: Vec<String> =
                  ctx.model.fields.keys().map(|k| k.to_string()).collect();
              let suggestion = crate::macros::validate::suggest(&col_str, &candidates);
              let msg = match suggestion {
                  Some(s) => format!(
                      "unknown column `{}`; did you mean `{}`?", col_str, s
                  ),
                  None => format!("unknown column `{}`", col_str),
              };
              syn::Error::new(name.span(), msg)
          })?;
          if field.is_relation() {
              return Err(syn::Error::new(
                  name.span(),
                  format!("by-column `{}` is a relation; group_by requires scalar columns", col_str),
              ));
          }
          let variant = format_ident!("{}", to_pascal(&col_str));
          by_variants.push(quote! { #module_ident::#column_enum::#variant });
      }

      let lower_opt = |k: AggKind, b: Option<&DslBlock>| -> syn::Result<TokenStream> {
          match b {
              Some(blk) => {
                  let ts = lower_agg_select(k, blk, &ctx)?;
                  Ok(quote! { ::core::option::Option::Some(#ts) })
              }
              None => Ok(quote! { ::core::option::Option::None }),
          }
      };
      let count_ts = lower_opt(AggKind::Count, count_block)?;
      let sum_ts = lower_opt(AggKind::Sum, sum_block)?;
      let avg_ts = lower_opt(AggKind::Avg, avg_block)?;
      let min_ts = lower_opt(AggKind::Min, min_block)?;
      let max_ts = lower_opt(AggKind::Max, max_block)?;

      let where_ts = match where_block {
          Some(w) => {
              let wts = where_input::lower_where_input(w, &ctx)?.where_input;
              quote! { ::core::option::Option::Some(#wts) }
          }
          None => quote! { ::core::option::Option::None },
      };

      let having_ts = match having_block {
          Some(h) => {
              let conds = lower_having(h, &ctx)?;
              let having_ty = format_ident!("{}GroupByHaving", ctx.model.name());
              quote! {
                  ::core::option::Option::Some(#module_ident::#having_ty { conditions: #conds })
              }
          }
          None => quote! { ::core::option::Option::None },
      };

      let args_ident = format_ident!("{}GroupByArgs", ctx.model.name());
      let accessor_expr = &call.accessor_expr;

      Ok(quote! {
          {
              let __by: ::std::vec::Vec<#module_ident::#column_enum> = vec![#(#by_variants),*];
              let __args: #module_ident::#args_ident = #module_ident::#args_ident {
                  by: __by.clone(),
                  where_input: #where_ts,
                  _count: #count_ts,
                  _sum: #sum_ts,
                  _avg: #avg_ts,
                  _min: #min_ts,
                  _max: #max_ts,
                  having: #having_ts,
                  order_by: ::core::option::Option::None,
              };
              (#accessor_expr).group_by(__by).with_group_by_args(__args)
          }
      })
  }

  fn to_pascal(snake: &str) -> String {
      let mut out = String::with_capacity(snake.len());
      let mut upper = true;
      for c in snake.chars() {
          if c == '_' { upper = true; }
          else if upper { out.push(c.to_ascii_uppercase()); upper = false; }
          else { out.push(c); }
      }
      out
  }
  ```

  Adjust `AccessorCall.accessor_expr` and `block` field names to match the actual struct (look at `prax-codegen/src/macros/accessor.rs`).

- [ ] **Step 2:** Register proc-macro:

  ```rust
  #[proc_macro]
  pub fn group_by(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
      crate::macros::ops::group_by::group_by_macro_impl(input)
  }
  ```

- [ ] **Step 3:** Re-export from `prax-orm/src/lib.rs`.

- [ ] **Step 4:** Workspace build:

  ```bash
  cargo build --workspace --all-features
  ```

- [ ] **Step 5: Commit**

  ```
  feat(codegen): group_by! macro entry point (phase 6)
  ```

---

## Task 11: trybuild diagnostic fixtures

**Files:**
- Create fixtures under `prax-codegen/tests/ui/aggregate/`:
  - `aggregate_no_blocks_fail.rs` + `.stderr`
  - `aggregate_sum_on_string_fail.rs` + `.stderr`
  - `aggregate_unknown_column_fail.rs` + `.stderr`
  - `aggregate_relation_field_fail.rs` + `.stderr`
  - `aggregate_select_value_not_true_fail.rs` + `.stderr`
- Create fixtures under `prax-codegen/tests/ui/group_by/`:
  - `group_by_empty_by_fail.rs` + `.stderr`
  - `group_by_unknown_by_column_fail.rs` + `.stderr`
  - `group_by_having_bad_operator_fail.rs` + `.stderr`
- Create `count_select_with_value_5_fail.rs` + `.stderr` (in existing `tests/ui/computed_fields/` or new `tests/ui/count/` dir)
- Modify the trybuild harness file (e.g., `tests/ui.rs`) to register the new directories

- [ ] **Step 1:** Create each fixture. Each is a complete `.rs` file that uses derive-style models (since the macro DSL targets schema-defined models — derive-style works for trybuild because the model is in the same crate as the fixture).

- [ ] **Step 2:** Run `TRYBUILD=overwrite cargo test -p prax-codegen --test ui` to materialise `.stderr` baselines. Inspect each baseline matches the expected diagnostic. Fix the macro emission if any baseline says something wrong (wrong span, wrong message).

- [ ] **Step 3:** Run `cargo test -p prax-codegen --test ui` without `TRYBUILD=overwrite` to verify all fixtures match the locked baselines.

- [ ] **Step 4: Commit**

  ```
  test(codegen): trybuild fixtures for aggregate! / group_by! / count! select diagnostics
  ```

---

## Task 12: e2e RecordingEngine tests

**Files:**
- Create: `tests/aggregate_macros_e2e.rs`

- [ ] **Step 1:** Mirror the structure of `tests/nested_writes_e2e.rs` (or `tests/computed_fields_e2e.rs` from phase 5.5). Copy the `RecordingEngine` boilerplate; add `impl SupportsScalarSubqueryInSelect` if relevant (probably not — aggregate queries don't use scalar projections).

- [ ] **Step 2:** Derive models suitable for the tests:

  ```rust
  #[derive(prax_orm::Model, Debug, Clone, Default)]
  #[prax(table = "users")]
  pub struct User {
      #[prax(id, auto)]
      pub id: i32,
      pub team_id: i32,
      pub region: String,
      pub active: bool,
      pub views: i32,
      pub score: i32,
      pub created_at: i64,  // simplified — real DateTime in live tests
  }

  client!(User);
  ```

- [ ] **Step 3:** Tests (each ~25 lines):

  - `count_with_select_emits_per_column_counts` — assert SQL contains
    `COUNT(*) AS "_all"`, `COUNT("email") AS "email"` (use a User with email).
  - `aggregate_emits_sum_avg_min_max_count` — single call with all five
    blocks; assert all five aggregate functions appear in SELECT.
  - `aggregate_where_filters_the_aggregate` — assert WHERE clause is
    emitted on the underlying SELECT.
  - `group_by_emits_by_columns_and_group_by_clause` — assert
    `GROUP BY "team_id", "region"` in SQL.
  - `group_by_having_emits_having_clause` — assert
    `HAVING COUNT(*) > $1` and params include the threshold.
  - `aggregate_omits_unspecified_blocks` — call with only `_sum: { views: true }`;
    assert no `AVG`, `MIN`, `MAX`, `COUNT` appear in SQL.

- [ ] **Step 4:** Run `cargo test --test aggregate_macros_e2e` — all green.

- [ ] **Step 5: Commit**

  ```
  test(query): e2e RecordingEngine coverage for aggregate macros
  ```

---

## Task 13: Live Postgres integration test (`--ignored`)

**Files:**
- Create: `prax-postgres/tests/aggregate_macros.rs`

- [ ] **Step 1:** Mirror `prax-postgres/tests/computed_fields.rs` shape. Use the same `PRAX_E2E=1 + POSTGRES_URL` gating, `unique_table()` helper, `pool()` / `drop_table()` harness.

- [ ] **Step 2:** Tests:

  ```rust
  #[tokio::test]
  #[ignore = "requires postgres container or DATABASE_URL"]
  async fn count_select_round_trip() {
      // CREATE TABLE; INSERT 5 rows where 2 have NULL email;
      // run prax::count!(c.user, { select: { _all: true, email: true } });
      // assert _all == 5 and email == 3.
  }

  #[tokio::test]
  #[ignore = "requires postgres container or DATABASE_URL"]
  async fn aggregate_sum_avg_count_round_trip() {
      // CREATE TABLE users (id, score INT);
      // INSERT scores 10, 20, 30;
      // run prax::aggregate!(c.user, {
      //     _sum: { score: true }, _avg: { score: true }, _count: { _all: true }
      // });
      // assert _sum.score == 60, _avg.score == 20.0, _count._all == 3.
  }

  #[tokio::test]
  #[ignore = "requires postgres container or DATABASE_URL"]
  async fn group_by_with_having_round_trip() {
      // CREATE TABLE users (team_id INT, score INT);
      // INSERT mix where team A has 2 rows, team B has 4 rows;
      // run prax::group_by!(c.user, {
      //     by: [team_id], _count: { _all: true },
      //     having: { _count: { _all: { gt: 3 } } }
      // });
      // assert single returned row, team_id == B, _count._all == 4.
  }
  ```

- [ ] **Step 3:** Compile cleanly. `cargo test -p prax-postgres --test aggregate_macros` (no `--ignored`) should pass with all tests skipped.

- [ ] **Step 4: Commit**

  ```
  test(postgres): live integration for aggregate macros (phase 6)
  ```

---

## Task 14: CHANGELOG + workspace sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1:** Add under `[Unreleased] > Added`:

  ```
  - **Aggregate macros (phase 6).** Three new macros sitting on top of
    the existing `AggregateOperation` / `GroupByOperation` runtime:
    - `count!` `select:` block — Prisma-style per-column non-null
      counts. Returns `<Model>CountSelectResult { _all: i64, <col>: i64,
      ... }`. Without `select:`, behavior is unchanged (returns
      `i64`).
    - `aggregate!` — new macro returning a per-model
      `<Model>AggregateResult` with `_sum`, `_avg`, `_min`, `_max`,
      `_count` substructs populated only when their `_<agg>:` block
      is supplied.
    - `group_by!` — new macro with `by:`, `where:`, `having:`, and
      the same aggregate substructs. Returns
      `Vec<<Model>GroupByResult>`. `order_by:` deferred to a
      follow-up.
  - Per-model codegen surface: `<Model>{Count,Sum,Avg,Min,Max}Select`,
    matching `*Result`, `<Model>AggregateResult`, `<Model>GroupByResult`,
    `<Model>GroupByColumn` enum, `<Model>AggregateArgs`,
    `<Model>GroupByArgs`.
  - Diagnostics: `_sum` / `_avg` on non-numeric column, aggregate on
    relation, unknown by-column with did-you-mean, empty `by:`, empty
    aggregate-block in `aggregate!`, unsupported having operator.

  ### Known limitations
  - The aggregate macros target schema-defined and derive-style models
    that the engine implements `AggregateOperation` /
    `GroupByOperation` for. MongoDB and CQL engines are excluded;
    `$group` and partition-key GROUP BY lowering are separate
    follow-ups.
  - `count_distinct` is in the runtime but not yet exposed via the
    macro shape; use the runtime API for now.
  - `_min`/`_max` against multiple columns at once is not supported in
    a single call.
  - `group_by!` `order_by:` is rejected at macro expansion with a
    follow-up tracking message.
  ```

- [ ] **Step 2:** Full sweep:

  ```bash
  cd /home/joseph/Projects/prax/.worktrees/aggregate-macros
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace --all-features --no-fail-fast
  ```

- [ ] **Step 3: Commit**

  ```
  docs(query): CHANGELOG for phase 6 aggregate macros
  ```

---

## Task 15: Push + PR (manual)

- [ ] `git push -u origin feature/aggregate-macros`
- [ ] `gh pr create --base develop --title "feat: aggregate macros (phase 6)"`
- [ ] `gh pr merge <PR#> --squash --auto`

PR body sketch:

```
Closes phase 6 of the typed-query-traits initiative.

## What ships
- count!(...) with select: { _all, <col>, ... } per-column counts
- aggregate!(...) with _sum / _avg / _min / _max / _count
- group_by!(...) with by, where, having, aggregate blocks

## Deferred follow-ups
- MongoDB $group lowering
- CQL GROUP BY
- count_distinct macro shape
- multi-column _min/_max in one call
- group_by! order_by:
- include: { _count } form

## Test plan
- prax-codegen unit + UI tests
- e2e tests/aggregate_macros_e2e.rs
- prax-postgres/tests/aggregate_macros.rs (--ignored)
```

---

## Out of scope (deferred follow-ups, listed in spec §12)

- MongoDB `$group` pipeline lowering
- CQL `GROUP BY` (partition-key prefix only)
- `count_distinct` shape on `_count`
- Multi-column `_min`/`_max` in one call
- Window-function aggregates, percentiles
- `having:` support for non-aggregate fields
- `include: { _count }` form (analogue of the phase 5.5 deferral)
- `group_by!` `order_by:` (currently rejected at macro time)
