//! MongoDB engine capability declarations.
//!
//! MongoDB uses the document/aggregation model rather than SQL primitives, so
//! only a narrow subset of the SQL capability traits apply.
//!
//! `SupportsScalarSubqueryInSelect` is intentionally NOT impl'd here.
//! Relation-aggregate virtual fields require a `$lookup`-lowering pass
//! that is scheduled as a follow-up plan after phase 5.

use prax_query::capabilities::{SupportsNestedWrites, SupportsRelationFilter};

use crate::engine::MongoEngine;

impl SupportsRelationFilter for MongoEngine {}
impl SupportsNestedWrites for MongoEngine {}
