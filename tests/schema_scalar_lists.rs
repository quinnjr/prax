//! `prax_schema!` on a model with `String[]` fields: the generated struct
//! holds `Vec<String>` / `Option<Vec<String>>`, decodes them through
//! `FromColumn` and encodes them through `ToFilterValue`. Before
//! `Vec<String>` had those impls, this file did not compile (E0277).

prax_orm::prax_schema!("tests/fixtures/scalar_lists.prax");

use prax_query::filter::FilterValue;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::ModelWithPk;

/// A row with an `id`, and text-array columns stored as comma-joined
/// strings; `None` is SQL NULL.
struct ArrayRow {
    id: i32,
    arrays: Vec<(&'static str, Option<&'static str>)>,
}

impl ArrayRow {
    fn cell(&self, column: &str) -> Result<Option<&'static str>, RowError> {
        self.arrays
            .iter()
            .find(|(name, _)| *name == column)
            .map(|(_, value)| *value)
            .ok_or_else(|| RowError::ColumnNotFound(column.to_string()))
    }
}

fn unused(column: &str) -> RowError {
    RowError::TypeConversion {
        column: column.to_string(),
        message: "not used by this test".into(),
    }
}

impl RowRef for ArrayRow {
    fn get_i32(&self, column: &str) -> Result<i32, RowError> {
        if column == "id" {
            Ok(self.id)
        } else {
            Err(unused(column))
        }
    }
    fn get_i32_opt(&self, column: &str) -> Result<Option<i32>, RowError> {
        self.get_i32(column).map(Some)
    }
    fn get_i64(&self, column: &str) -> Result<i64, RowError> {
        Err(unused(column))
    }
    fn get_i64_opt(&self, column: &str) -> Result<Option<i64>, RowError> {
        Err(unused(column))
    }
    fn get_f64(&self, column: &str) -> Result<f64, RowError> {
        Err(unused(column))
    }
    fn get_f64_opt(&self, column: &str) -> Result<Option<f64>, RowError> {
        Err(unused(column))
    }
    fn get_bool(&self, column: &str) -> Result<bool, RowError> {
        Err(unused(column))
    }
    fn get_bool_opt(&self, column: &str) -> Result<Option<bool>, RowError> {
        Err(unused(column))
    }
    fn get_str(&self, column: &str) -> Result<&str, RowError> {
        Err(unused(column))
    }
    fn get_str_opt(&self, column: &str) -> Result<Option<&str>, RowError> {
        Err(unused(column))
    }
    fn get_bytes(&self, column: &str) -> Result<&[u8], RowError> {
        Err(unused(column))
    }
    fn get_bytes_opt(&self, column: &str) -> Result<Option<&[u8]>, RowError> {
        Err(unused(column))
    }
    fn is_null(&self, column: &str) -> Result<bool, RowError> {
        if column == "id" {
            return Ok(false);
        }
        self.cell(column).map(|value| value.is_none())
    }
    fn get_string_array(&self, column: &str) -> Result<Vec<String>, RowError> {
        match self.cell(column)? {
            None => Err(RowError::UnexpectedNull(column.to_string())),
            Some("") => Ok(vec![]),
            Some(joined) => Ok(joined.split(',').map(str::to_string).collect()),
        }
    }
}

#[test]
fn string_list_fields_are_vec_string() {
    let grant = access_grant::AccessGrant {
        id: 1,
        scopes: vec!["read".into(), "write".into()],
        allowed_scopes: vec![],
        labels: None,
    };
    assert_eq!(grant.scopes.len(), 2);
}

#[test]
fn string_list_fields_decode_from_a_row() {
    let row = ArrayRow {
        id: 7,
        arrays: vec![
            ("scopes", Some("read,write")),
            ("allowed_scopes", Some("")),
            ("labels", None),
        ],
    };
    let grant = access_grant::AccessGrant::from_row(&row).expect("row decodes");
    assert_eq!(grant.id, 7);
    assert_eq!(grant.scopes, vec!["read".to_string(), "write".to_string()]);
    assert!(grant.allowed_scopes.is_empty());
    assert_eq!(grant.labels, None);

    let row = ArrayRow {
        id: 8,
        arrays: vec![
            ("scopes", Some("read")),
            ("allowed_scopes", Some("admin")),
            ("labels", Some("vip")),
        ],
    };
    let grant = access_grant::AccessGrant::from_row(&row).expect("row decodes");
    assert_eq!(grant.labels, Some(vec!["vip".to_string()]));
}

#[test]
fn string_list_fields_encode_as_lists_of_strings() {
    let grant = access_grant::AccessGrant {
        id: 1,
        scopes: vec!["read".into(), "write".into()],
        allowed_scopes: vec![],
        labels: None,
    };
    assert_eq!(
        grant.get_column_value("scopes"),
        Some(FilterValue::List(vec![
            FilterValue::String("read".into()),
            FilterValue::String("write".into()),
        ]))
    );
    assert_eq!(
        grant.get_column_value("allowed_scopes"),
        Some(FilterValue::List(vec![]))
    );
    assert_eq!(grant.get_column_value("labels"), Some(FilterValue::Null));
}
