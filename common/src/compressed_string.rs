use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Deserializer, Serializer};
use std::io::Read;

pub fn serialize<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Compress
    let compressed =
        zstd::stream::encode_all(value.as_bytes(), 0).map_err(serde::ser::Error::custom)?;

    // Base64 encode
    let encoded = general_purpose::STANDARD.encode(compressed);

    serializer.serialize_str(&encoded)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;

    // Base64 decode
    let compressed = general_purpose::STANDARD
        .decode(encoded)
        .map_err(serde::de::Error::custom)?;

    // Decompress
    let mut decoder =
        zstd::stream::Decoder::new(&compressed[..]).map_err(serde::de::Error::custom)?;

    let mut result = String::new();
    decoder
        .read_to_string(&mut result)
        .map_err(serde::de::Error::custom)?;

    Ok(result)
}
