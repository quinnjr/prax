//! Client-side checks for `LISTEN` / `NOTIFY` arguments, shared by the
//! sending helpers ([`PgConnection::notify`](crate::connection::PgConnection::notify),
//! [`PgTransaction::notify`](crate::connection::PgTransaction::notify)) and
//! [`PgListener`](crate::listener::PgListener).

use crate::error::{PgError, PgResult};

/// Longest channel name PostgreSQL accepts (`NAMEDATALEN - 1`), in bytes.
pub(crate) const MAX_CHANNEL_LEN: usize = 63;

/// PostgreSQL's notification payload limit in bytes; a payload must be
/// strictly shorter than this.
pub(crate) const PAYLOAD_LIMIT: usize = 8000;

/// Validate a `LISTEN` / `NOTIFY` channel name: non-empty, at most 63 bytes,
/// no NUL. Channel names are identifiers and cannot be bound as parameters
/// in `LISTEN`, so this runs before the name is quoted into SQL.
pub(crate) fn validate_channel(name: &str) -> PgResult<()> {
    if name.is_empty() {
        return Err(PgError::query(
            "notification channel name must not be empty",
        ));
    }
    if name.len() > MAX_CHANNEL_LEN {
        return Err(PgError::query(format!(
            "notification channel name is {} bytes; PostgreSQL allows at most {MAX_CHANNEL_LEN}",
            name.len()
        )));
    }
    if name.contains('\0') {
        return Err(PgError::query(
            "notification channel name must not contain NUL",
        ));
    }
    Ok(())
}

/// Validate a notification payload against PostgreSQL's limit (it must be
/// shorter than 8000 bytes).
pub(crate) fn validate_payload(payload: &str) -> PgResult<()> {
    if payload.len() >= PAYLOAD_LIMIT {
        return Err(PgError::query(format!(
            "notification payload is {} bytes; PostgreSQL requires fewer than {PAYLOAD_LIMIT}",
            payload.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_channel_rejects_empty_too_long_and_nul() {
        assert!(matches!(validate_channel(""), Err(PgError::Query(_))));
        assert!(matches!(
            validate_channel(&"a".repeat(MAX_CHANNEL_LEN + 1)),
            Err(PgError::Query(_))
        ));
        assert!(matches!(validate_channel("a\0b"), Err(PgError::Query(_))));
    }

    #[test]
    fn validate_channel_accepts_max_length_and_quotes() {
        assert!(validate_channel(&"a".repeat(MAX_CHANNEL_LEN)).is_ok());
        assert!(validate_channel("Mixed\"Case").is_ok());
    }

    #[test]
    fn validate_payload_enforces_8000_byte_limit() {
        assert!(validate_payload(&"x".repeat(PAYLOAD_LIMIT - 1)).is_ok());
        assert!(matches!(
            validate_payload(&"x".repeat(PAYLOAD_LIMIT)),
            Err(PgError::Query(_))
        ));
    }
}
