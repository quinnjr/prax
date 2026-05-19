//! Test fixture for the input-codegen tests.
//!
//! NOTE: This file is a documentation-only fixture. The `#[derive(Model)]`
//! macro generates code that references `::prax_orm::_prax_prelude`, which
//! creates a circular dependency when used directly in `prax-codegen` integration
//! tests (since `prax-orm` depends on `prax-codegen`).
//!
//! The equivalent integration tests live in `prax-codegen/src/generators/derive.rs`
//! as `#[cfg(test)]` unit tests that verify the generated token stream without
//! needing to compile and execute the generated code.
//!
//! The struct definition below is kept as a reference for what a real consumer
//! of the macro would write.

/*
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
    pub role: String,
}
*/
