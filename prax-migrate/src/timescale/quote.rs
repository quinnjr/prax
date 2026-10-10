//! Identifier and literal quoting for generated TimescaleDB SQL (spec §4.7).

/// Quote an identifier: wrap in double quotes, doubling embedded quotes.
#[must_use]
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Quote a string literal: wrap in single quotes, doubling embedded quotes.
#[must_use]
pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A `regclass` argument: the quoted identifier as a string literal.
#[must_use]
pub fn regclass_literal(s: &str) -> String {
    quote_literal(&quote_ident(s))
}

/// A column name passed as a `name` argument, as a string literal.
#[must_use]
pub fn name_literal(s: &str) -> String {
    quote_literal(s)
}

/// A column list for `segmentby`/`orderby`, as one string literal.
///
/// Each entry is a column and whether it sorts descending; descending columns
/// get a `DESC` suffix. An empty list renders `''`.
#[must_use]
pub fn ident_list_literal(cols: &[(String, bool)]) -> String {
    let list = cols
        .iter()
        .map(|(col, desc)| {
            if *desc {
                format!("{} DESC", quote_ident(col))
            } else {
                quote_ident(col)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    quote_literal(&list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_ident() {
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    #[test]
    fn test_quote_literal() {
        assert_eq!(quote_literal("a'b"), "'a''b'");
    }

    #[test]
    fn test_regclass_literal() {
        assert_eq!(regclass_literal("O'Neil"), "'\"O''Neil\"'");
    }

    #[test]
    fn test_name_literal() {
        assert_eq!(name_literal("deviceId"), "'deviceId'");
    }

    #[test]
    fn test_ident_list_literal() {
        let cols = vec![("deviceId".to_string(), false), ("time".to_string(), true)];
        assert_eq!(ident_list_literal(&cols), "'\"deviceId\", \"time\" DESC'");
        assert_eq!(ident_list_literal(&[]), "''");
    }
}
