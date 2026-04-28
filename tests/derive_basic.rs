use prax_orm::Model;
use prax_query::row::{RowError, RowRef};
use prax_query::traits::Model as QueryModel;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Model)]
#[prax(table = "authors")]
struct Author {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

fn assert_impls<T: prax_query::row::FromRow + prax_query::traits::Model>() {}

#[test]
fn author_has_model_and_fromrow_impls() {
    assert_impls::<Author>();
    assert_eq!(Author::TABLE_NAME, "authors");
    assert_eq!(Author::PRIMARY_KEY, &["id"]);
}

// Minimal in-file RowRef impl for round-tripping through the generated
// `FromRow` derive. We only implement the getters the derive emits for
// Author (`get_i32`, `get_str`/`get_string`, and `get_str_opt`/`get_string_opt`
// via the default trait methods). A missing entry in the HashMap maps to
// SQL NULL semantics for `*_opt` getters and to `ColumnNotFound` for the
// required getters — matching the other MockRow in prax-query/src/row.rs.
struct TestRow {
    data: HashMap<String, Option<String>>,
}

impl TestRow {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    fn set(mut self, col: &str, val: &str) -> Self {
        self.data.insert(col.to_string(), Some(val.to_string()));
        self
    }

    fn set_null(mut self, col: &str) -> Self {
        self.data.insert(col.to_string(), None);
        self
    }
}

impl RowRef for TestRow {
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

    fn get_i64(&self, _column: &str) -> Result<i64, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }

    fn get_i64_opt(&self, _column: &str) -> Result<Option<i64>, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }

    fn get_f64(&self, _column: &str) -> Result<f64, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }

    fn get_f64_opt(&self, _column: &str) -> Result<Option<f64>, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }

    fn get_bool(&self, _column: &str) -> Result<bool, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }

    fn get_bool_opt(&self, _column: &str) -> Result<Option<bool>, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
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
            // Present-but-NULL and absent-entirely both map to None. The
            // derive calls FromColumn<Option<String>>::from_column → the
            // blanket get_string_opt default → get_str_opt, so only the
            // `Option<&str>` outcome matters to the caller.
            Some(None) | None => Ok(None),
        }
    }

    fn get_bytes(&self, _column: &str) -> Result<&[u8], RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }

    fn get_bytes_opt(&self, _column: &str) -> Result<Option<&[u8]>, RowError> {
        unimplemented!("TestRow only used for Author's FromRow impl")
    }
}

#[test]
fn author_fromrow_decodes_mockrow() {
    let row = TestRow::new()
        .set("id", "42")
        .set("email", "a@b.c")
        .set("name", "alice");

    let author = <Author as prax_query::row::FromRow>::from_row(&row)
        .expect("Author::from_row must succeed with all columns populated");

    assert_eq!(author.id, 42);
    assert_eq!(author.email, "a@b.c");
    assert_eq!(author.name.as_deref(), Some("alice"));
}

#[test]
fn author_fromrow_maps_null_optional_to_none() {
    // Absent column → get_str_opt returns Ok(None) → Option<String> becomes
    // None. This is what drivers report when the underlying SQL column is
    // NULL, so it's the round-trip we care about in production.
    let row = TestRow::new().set("id", "7").set("email", "b@b.c");

    let author = <Author as prax_query::row::FromRow>::from_row(&row)
        .expect("absent optional column must round-trip to None");
    assert_eq!(author.id, 7);
    assert_eq!(author.email, "b@b.c");
    assert!(author.name.is_none());

    // Present-but-NULL (explicit NULL marker) should also map to None.
    let row_null = TestRow::new()
        .set("id", "8")
        .set("email", "c@b.c")
        .set_null("name");
    let author_null = <Author as prax_query::row::FromRow>::from_row(&row_null)
        .expect("present-NULL optional column must round-trip to None");
    assert_eq!(author_null.id, 8);
    assert!(author_null.name.is_none());
}
