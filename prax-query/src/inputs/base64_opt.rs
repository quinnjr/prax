//! serde adapter: `Option<Vec<u8>>` as an optional base64 (STANDARD) string.
//!
//! Absent and `null` both deserialize to `None` (pair with
//! `#[serde(default)]`); `None` serializes as `null`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serializer};

pub(crate) fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(b) => s.serialize_some(&STANDARD.encode(b)),
        None => s.serialize_none(),
    }
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|s| STANDARD.decode(s).map_err(serde::de::Error::custom))
        .transpose()
}
