//! CQL type conversions to Rust types.

use crate::error::{CassandraError, CassandraResult};

/// Decode a big-endian i32 from a 4-byte slice.
pub fn decode_int(bytes: &[u8]) -> CassandraResult<i32> {
    if bytes.len() != 4 {
        return Err(CassandraError::Deserialization(format!(
            "expected 4 bytes for int, got {}",
            bytes.len()
        )));
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(bytes);
    Ok(i32::from_be_bytes(buf))
}

/// Decode a big-endian i64 from an 8-byte slice.
pub fn decode_bigint(bytes: &[u8]) -> CassandraResult<i64> {
    if bytes.len() != 8 {
        return Err(CassandraError::Deserialization(format!(
            "expected 8 bytes for bigint, got {}",
            bytes.len()
        )));
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bytes);
    Ok(i64::from_be_bytes(buf))
}

/// Decode a UTF-8 string from bytes.
pub fn decode_text(bytes: &[u8]) -> CassandraResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|e| {
        CassandraError::Deserialization(format!("invalid UTF-8 in text column: {}", e))
    })
}

/// Decode a boolean (1 byte: 0 = false, nonzero = true).
pub fn decode_bool(bytes: &[u8]) -> CassandraResult<bool> {
    if bytes.len() != 1 {
        return Err(CassandraError::Deserialization(format!(
            "expected 1 byte for bool, got {}",
            bytes.len()
        )));
    }
    Ok(bytes[0] != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_int_valid() {
        let bytes = 42i32.to_be_bytes();
        assert_eq!(decode_int(&bytes).unwrap(), 42);
    }

    #[test]
    fn test_decode_int_wrong_size() {
        assert!(decode_int(&[1, 2, 3]).is_err());
    }

    #[test]
    fn test_decode_bigint_valid() {
        let bytes = (-1234567890123i64).to_be_bytes();
        assert_eq!(decode_bigint(&bytes).unwrap(), -1234567890123i64);
    }

    #[test]
    fn test_decode_text_valid() {
        assert_eq!(decode_text(b"hello").unwrap(), "hello");
    }

    #[test]
    fn test_decode_text_invalid_utf8() {
        assert!(decode_text(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn test_decode_bool() {
        assert!(!decode_bool(&[0]).unwrap());
        assert!(decode_bool(&[1]).unwrap());
        assert!(decode_bool(&[42]).unwrap());
        assert!(decode_bool(&[]).is_err());
    }
}
