//! Shared serde adapters for binary data encoded as base64 (STANDARD) strings.

/// `Vec<u8>` as a base64 string (serialize only).
///
/// Used by [`FilterValue::Bytes`](crate::filter::FilterValue::Bytes), which is
/// never deserialised (it is `#[serde(untagged)]`; a base64 string comes back
/// as `FilterValue::String`).
pub(crate) mod bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::Serializer;

    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
}

/// `Option<Vec<u8>>` as an optional base64 string.
///
/// Absent and `null` both deserialize to `None` (pair with
/// `#[serde(default)]`); `None` serializes as `null`.
pub(crate) mod option {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_some(&STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|s| STANDARD.decode(s).map_err(serde::de::Error::custom))
            .transpose()
    }
}
