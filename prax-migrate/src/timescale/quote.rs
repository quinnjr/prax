//! Identifier, literal and comment quoting for generated TimescaleDB SQL.
//!
//! Every name in generated SQL comes from the schema or the live catalog, so
//! each one is quoted: identifiers with [`quote_ident`], string arguments with
//! [`quote_literal`], and names inside `--` comment lines with
//! [`comment_safe`].

/// Quote an identifier: wrap in double quotes, doubling embedded quotes.
#[must_use]
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Quote a string literal: wrap in single quotes, doubling embedded quotes.
///
/// A value containing a backslash is emitted as an escape string
/// (`E'...'`) with each backslash doubled, so it means the same thing
/// whether or not the server runs with `standard_conforming_strings = on`.
#[must_use]
pub fn quote_literal(s: &str) -> String {
    let quoted = s.replace('\'', "''");
    if s.contains('\\') {
        format!("E'{}'", quoted.replace('\\', "\\\\"))
    } else {
        format!("'{quoted}'")
    }
}

/// A `regclass` argument: the quoted identifier as a string literal.
#[must_use]
pub fn regclass_literal(s: &str) -> String {
    quote_literal(&quote_ident(s))
}

/// A column list for `segmentby`/`orderby`, as one string literal.
///
/// Each entry is a column and whether it sorts descending; descending columns
/// get a `DESC` suffix. An empty list renders `''`.
#[must_use]
pub fn ident_list_literal<'a>(cols: impl IntoIterator<Item = (&'a str, bool)>) -> String {
    let list = cols
        .into_iter()
        .map(|(col, desc)| {
            if desc {
                format!("{} DESC", quote_ident(col))
            } else {
                quote_ident(col)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    quote_literal(&list)
}

/// Make `s` safe to embed in a single-line (`--` or `//`) comment.
///
/// Replaces `\r`, `\n`, every other control character (including U+0085
/// NEXT LINE), U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR with a
/// single space each, so a name can never end the comment and start a
/// statement.
#[must_use]
pub fn comment_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{0085}' | '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
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
        assert_eq!(quote_literal("deviceId"), "'deviceId'");
    }

    #[test]
    fn test_quote_literal_with_backslash_uses_escape_string() {
        assert_eq!(quote_literal("a\\b"), "E'a\\\\b'");
        assert_eq!(quote_literal("it's a\\b"), "E'it''s a\\\\b'");
        assert_eq!(quote_literal("\\'"), "E'\\\\'''");
    }

    #[test]
    fn test_regclass_literal() {
        assert_eq!(regclass_literal("O'Neil"), "'\"O''Neil\"'");
        assert_eq!(regclass_literal("a\\b"), "E'\"a\\\\b\"'");
    }

    #[test]
    fn test_ident_list_literal() {
        assert_eq!(
            ident_list_literal([("deviceId", false), ("time", true)]),
            "'\"deviceId\", \"time\" DESC'"
        );
        assert_eq!(ident_list_literal(Vec::<(&str, bool)>::new()), "''");
    }

    #[test]
    fn test_comment_safe_replaces_line_breaks_and_controls() {
        assert_eq!(comment_safe("plain \"name\""), "plain \"name\"");
        assert_eq!(
            comment_safe("a\nDROP TABLE x;\r\n--"),
            "a DROP TABLE x;  --"
        );
        assert_eq!(comment_safe("a\tb\u{0}c\u{1b}d"), "a b c d");
        assert_eq!(comment_safe("a\u{0085}b\u{2028}c\u{2029}d"), "a b c d");
        assert!(
            !comment_safe("x\u{000b}\u{000c}y")
                .chars()
                .any(char::is_control)
        );
    }
}
