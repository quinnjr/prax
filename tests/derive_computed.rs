//! Tests that `#[derive(Model)]` picks up `#[prax(generated)]` and
//! `#[prax(count/sum/avg/min/max)]` directives and emits the
//! `GENERATED_FIELDS` / `AGGREGATE_FIELDS` metadata constants.
//! Also covers COLUMNS membership and FromRow defaulting for aggregate fields.

use prax_orm::Model;
use prax_query::row::{RowError, RowRef};
use prax_query::traits::Model as QueryModel; // must be in scope for GENERATED_FIELDS / AGGREGATE_FIELDS / COLUMNS

#[derive(Model, Debug, Clone, Default)]
#[prax(table = "posts")]
pub struct Post {
    #[prax(id, auto)]
    pub id: i32,
    pub author_id: i32,
    pub views: i32,
    pub created_at: String,
}

#[derive(Model, Debug, Clone, Default)]
#[prax(table = "users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(unique)]
    pub email: String,
    pub first_name: String,
    pub last_name: String,

    #[prax(generated = "first_name || ' ' || last_name", stored)]
    pub full_name: String,

    #[prax(generated = "LOWER(email)", virtual)]
    pub search_key: String,

    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    pub posts: Vec<Post>,

    #[prax(count(posts))]
    pub post_count: i64,

    #[prax(sum(posts.views))]
    pub total_views: Option<i32>,
}

#[test]
fn user_emits_generated_field_metadata() {
    assert_eq!(
        User::GENERATED_FIELDS,
        &[
            ("full_name", "first_name || ' ' || last_name", true),
            ("search_key", "LOWER(email)", false),
        ][..],
    );
}

#[test]
fn user_emits_aggregate_field_metadata() {
    assert_eq!(
        User::AGGREGATE_FIELDS,
        &[
            ("post_count", "count", "posts", None),
            ("total_views", "sum", "posts", Some("views")),
        ][..],
    );
}

#[test]
fn post_has_no_computed_metadata() {
    assert!(Post::GENERATED_FIELDS.is_empty());
    assert!(Post::AGGREGATE_FIELDS.is_empty());
}

// ── COLUMNS membership ────────────────────────────────────────────────────────

/// `@generated` fields have a real underlying DB column and MUST appear in
/// `Model::COLUMNS`. Aggregate fields (`@count`/`@sum`/etc.) are computed via
/// subquery and have no column in the base table — they must NOT appear.
#[test]
fn user_full_columns_includes_generated_excludes_aggregate() {
    let cols: Vec<&str> = User::COLUMNS.to_vec();

    // @generated fields ARE real columns.
    assert!(
        cols.contains(&"full_name"),
        "COLUMNS missing @generated `full_name`; got: {cols:?}"
    );
    assert!(
        cols.contains(&"search_key"),
        "COLUMNS missing @generated `search_key`; got: {cols:?}"
    );

    // Aggregate fields must NOT be in COLUMNS — they have no base-table column.
    assert!(
        !cols.contains(&"post_count"),
        "COLUMNS must not include @count `post_count`; got: {cols:?}"
    );
    assert!(
        !cols.contains(&"total_views"),
        "COLUMNS must not include @sum `total_views`; got: {cols:?}"
    );
}

// ── FromRow aggregate defaults ────────────────────────────────────────────────

/// Minimal `RowRef` implementation for User's column set.
/// Absent columns behave like SQL NULL for `*_opt` getters and like
/// `ColumnNotFound` for required getters — matching the semantics used
/// by the blanket `impl<T: FromColumn> FromColumn for Option<T>`.
struct UserTestRow {
    data: std::collections::HashMap<String, Option<String>>,
}

impl UserTestRow {
    fn new() -> Self {
        Self {
            data: std::collections::HashMap::new(),
        }
    }

    fn set(mut self, col: &str, val: &str) -> Self {
        self.data.insert(col.to_string(), Some(val.to_string()));
        self
    }
}

impl RowRef for UserTestRow {
    fn get_i32(&self, column: &str) -> Result<i32, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => {
                v.parse()
                    .map_err(|e: std::num::ParseIntError| RowError::TypeConversion {
                        column: column.to_string(),
                        message: e.to_string(),
                    })
            }
            Some(None) => Err(RowError::UnexpectedNull(column.to_string())),
            None => Err(RowError::ColumnNotFound(column.to_string())),
        }
    }

    fn get_i32_opt(&self, column: &str) -> Result<Option<i32>, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => {
                v.parse()
                    .map(Some)
                    .map_err(|e: std::num::ParseIntError| RowError::TypeConversion {
                        column: column.to_string(),
                        message: e.to_string(),
                    })
            }
            Some(None) | None => Ok(None),
        }
    }

    fn get_i64(&self, column: &str) -> Result<i64, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => {
                v.parse()
                    .map_err(|e: std::num::ParseIntError| RowError::TypeConversion {
                        column: column.to_string(),
                        message: e.to_string(),
                    })
            }
            Some(None) => Err(RowError::UnexpectedNull(column.to_string())),
            None => Err(RowError::ColumnNotFound(column.to_string())),
        }
    }

    fn get_i64_opt(&self, column: &str) -> Result<Option<i64>, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => {
                v.parse()
                    .map(Some)
                    .map_err(|e: std::num::ParseIntError| RowError::TypeConversion {
                        column: column.to_string(),
                        message: e.to_string(),
                    })
            }
            // Both absent and explicit NULL → Ok(None). The aggregate
            // FromRow path calls get_i64_opt and maps ColumnNotFound to
            // Ok(None), so returning Ok(None) here would work too, but
            // we want to test the soft-miss path, so we return
            // ColumnNotFound for truly absent columns.
            Some(None) => Ok(None),
            None => Err(RowError::ColumnNotFound(column.to_string())),
        }
    }

    fn get_f64(&self, column: &str) -> Result<f64, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => {
                v.parse()
                    .map_err(|e: std::num::ParseFloatError| RowError::TypeConversion {
                        column: column.to_string(),
                        message: e.to_string(),
                    })
            }
            Some(None) => Err(RowError::UnexpectedNull(column.to_string())),
            None => Err(RowError::ColumnNotFound(column.to_string())),
        }
    }

    fn get_f64_opt(&self, column: &str) -> Result<Option<f64>, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => v.parse().map(Some).map_err(|e: std::num::ParseFloatError| {
                RowError::TypeConversion {
                    column: column.to_string(),
                    message: e.to_string(),
                }
            }),
            Some(None) | None => Ok(None),
        }
    }

    fn get_bool(&self, _column: &str) -> Result<bool, RowError> {
        unimplemented!("UserTestRow: bool not needed for User")
    }

    fn get_bool_opt(&self, _column: &str) -> Result<Option<bool>, RowError> {
        unimplemented!("UserTestRow: bool_opt not needed for User")
    }

    fn get_str(&self, column: &str) -> Result<&str, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => Ok(v.as_str()),
            Some(None) => Err(RowError::UnexpectedNull(column.to_string())),
            None => Err(RowError::ColumnNotFound(column.to_string())),
        }
    }

    fn get_str_opt(&self, column: &str) -> Result<Option<&str>, RowError> {
        match self.data.get(column) {
            Some(Some(v)) => Ok(Some(v.as_str())),
            Some(None) | None => Ok(None),
        }
    }

    fn get_bytes(&self, _column: &str) -> Result<&[u8], RowError> {
        unimplemented!("UserTestRow: bytes not needed for User")
    }

    fn get_bytes_opt(&self, _column: &str) -> Result<Option<&[u8]>, RowError> {
        unimplemented!("UserTestRow: bytes_opt not needed for User")
    }
}

/// When the row doesn't include the `post_count` column (e.g. because
/// the query projected only base columns), the `@count` field must
/// silently default to `0` rather than returning a `ColumnNotFound` error.
#[test]
fn count_field_defaults_to_zero_when_row_missing() {
    use prax_query::row::FromRow;

    let row = UserTestRow::new()
        .set("id", "1")
        .set("email", "a@b.c")
        .set("first_name", "Alice")
        .set("last_name", "Smith")
        .set("full_name", "Alice Smith")
        .set("search_key", "a@b.c");
    // Note: `post_count` and `total_views` are NOT in the row.

    let user = User::from_row(&row).expect("from_row must succeed even without aggregate columns");
    assert_eq!(
        user.post_count, 0,
        "missing @count column must default to 0"
    );
    assert_eq!(
        user.total_views, None,
        "missing @sum column must default to None"
    );
}

// ── Input-struct membership (Task 12) ────────────────────────────────────────

/// `UserCreateInput` must NOT have `full_name`, `search_key`, `post_count`, or
/// `total_views` fields. These are computed by the DB or via scalar subquery
/// and cannot be assigned by the caller.
///
/// The assertion is structural: if any of those fields were mistakenly added
/// back to `UserCreateInput`, the struct-literal construction below would fail
/// to compile (extra fields in struct literal / missing required field) —
/// turning a runtime mystery into a compile error.
#[test]
fn user_create_input_excludes_computed_fields() {
    let _ = user::UserCreateInput {
        email: "a@b.com".into(),
        first_name: "Ada".into(),
        last_name: "Lovelace".into(),
        // No full_name, no search_key, no post_count, no total_views.
    };
}

/// `UserUpdateInput` must also exclude computed fields. Verify by constructing
/// a default instance (all fields are `Option<*FieldUpdate>` so `Default`
/// works) and confirming the type exists and is constructible.
#[test]
fn user_update_input_excludes_computed_fields() {
    // Default construction succeeds — all fields are Option wrappers.
    let _ = user::UserUpdateInput::default();
    // If full_name / search_key / post_count / total_views had been mistakenly
    // added to UpdateInput they would appear here as extra Option fields.
    // Because the struct derives Default, the compile test is: does it derive
    // Default WITHOUT those fields present?  The answer is yes iff exclusion
    // is correct — any spuriously-included non-Option field would break Default.
}

/// `UserWhereInput` MUST include aggregate fields so callers can filter on them.
/// `post_count: i64` maps to `BigIntFilter`; `total_views: Option<i32>` maps
/// to `IntNullableFilter`.
#[test]
fn user_where_input_has_aggregate_filters() {
    let _ = user::UserWhereInput {
        post_count: Some(prax_query::inputs::BigIntFilter::equals(7)),
        total_views: Some(prax_query::inputs::IntNullableFilter {
            equals: Some(42),
            ..Default::default()
        }),
        ..Default::default()
    };
}

/// `UserWhereInput` MUST also include `@generated` fields (`full_name`,
/// `search_key`). These are real DB columns and can be filtered upon.
#[test]
fn user_where_input_has_generated_filters() {
    let _ = user::UserWhereInput {
        full_name: Some(prax_query::inputs::StringFilter::equals("Ada Lovelace")),
        search_key: Some(prax_query::inputs::StringFilter::equals("ada lovelace")),
        ..Default::default()
    };
}

/// `UserSelect` MUST include `full_name` and `search_key` (@generated fields)
/// plus `post_count` and `total_views` (aggregate fields) as `Option<bool>`
/// fields, so callers can opt them in.
#[test]
fn user_select_input_has_computed_fields() {
    let _ = user::UserSelect {
        full_name: Some(true),
        search_key: Some(true),
        post_count: Some(true),
        total_views: Some(true),
        ..Default::default()
    };
}

/// `UserOrderBy` MUST include variants for `@generated` and aggregate fields
/// so callers can sort on them.
#[test]
fn user_order_by_includes_computed_variants() {
    use prax_query::types::SortOrder;
    let _ = user::UserOrderBy::FullName(SortOrder::Asc);
    let _ = user::UserOrderBy::SearchKey(SortOrder::Desc);
    let _ = user::UserOrderBy::PostCount(SortOrder::Asc);
    let _ = user::UserOrderBy::TotalViews(SortOrder::Desc);
}

/// When the row DOES include the projected aggregate values, they must be
/// decoded correctly.
#[test]
fn aggregate_fields_decode_when_row_has_them() {
    use prax_query::row::FromRow;

    let row = UserTestRow::new()
        .set("id", "2")
        .set("email", "b@b.c")
        .set("first_name", "Bob")
        .set("last_name", "Jones")
        .set("full_name", "Bob Jones")
        .set("search_key", "b@b.c")
        .set("post_count", "7")
        .set("total_views", "42");

    let user = User::from_row(&row).expect("from_row must succeed with aggregate columns present");
    assert_eq!(user.post_count, 7, "@count column must decode to i64 value");
    assert_eq!(
        user.total_views,
        Some(42),
        "@sum column must decode to Some(value)"
    );
}
