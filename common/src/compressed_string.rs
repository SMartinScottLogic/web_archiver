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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestStruct {
        #[serde(serialize_with = "serialize", deserialize_with = "deserialize")]
        value: String,
    }

    #[test]
    fn test_round_trip_basic() {
        let original = TestStruct {
            value: "Hello, world!".to_string(),
        };

        let json = serde_json::to_string(&original).unwrap();
        let decoded: TestStruct = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_empty_string() {
        let original = TestStruct {
            value: "".to_string(),
        };

        let json = serde_json::to_string(&original).unwrap();
        let decoded: TestStruct = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_large_string() {
        let large_text = "abc123".repeat(10_000);

        let original = TestStruct { value: large_text };

        let json = serde_json::to_string(&original).unwrap();
        let decoded: TestStruct = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_invalid_base64() {
        let invalid_json = r#"{"value":"!!!not_base64!!!"}"#;

        let result: Result<TestStruct, _> = serde_json::from_str(invalid_json);

        assert!(result.is_err());
    }

    #[test]
    fn test_corrupted_compressed_data() {
        // Valid base64, but not valid zstd-compressed data
        let corrupted = base64::engine::general_purpose::STANDARD.encode("not compressed");

        let json = format!(r#"{{"value":"{}"}}"#, corrupted);

        let result: Result<TestStruct, _> = serde_json::from_str(&json);

        assert!(result.is_err());
    }

    #[test]
    fn test_unicode_string() {
        let original = TestStruct {
            value: "こんにちは🌍🚀".to_string(),
        };

        let json = serde_json::to_string(&original).unwrap();
        let decoded: TestStruct = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
    }
}
