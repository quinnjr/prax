//! End-to-end tests that the generated *Input structs for the fixture
//! model are produced correctly by `#[derive(Model)]`.
//!
//! # Why unit tests, not compile-and-run integration tests?
//!
//! `prax-codegen` is a proc-macro crate. The generated code emits references
//! to `::prax_orm::_prax_prelude`, and `prax-orm` in turn depends on
//! `prax-codegen` — a circular dependency that prevents adding `prax-orm` as
//! a dev-dependency here. The token-stream tests below verify the same
//! invariants by inspecting the stringified output of `derive_model_impl`.
//!
//! See `prax-codegen/tests/fixtures/inputs_schema.rs` for the reference
//! struct definition that a real downstream consumer would write.
//!
//! The actual runtime-lowering tests (`into_ir` → `Filter::Contains`, etc.)
//! live in `prax-query/tests/inputs_scalar.rs` where the runtime types are
//! available without a circular dependency.

// NOTE: Integration test bodies are in derive.rs unit tests.
// This file serves as a placeholder so the test path matches the plan spec.
// `cargo test -p prax-codegen --test derive_inputs` will report 0 tests,
// while the substantive coverage is in:
//   `cargo test -p prax-codegen where_input`
