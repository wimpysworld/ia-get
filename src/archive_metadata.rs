//! Archive metadata structures and processing for ia-get
//!
//! Handles JSON metadata structures from Internet Archive API.
//! Provides traits for working with file collections in a unified way.

use crate::{IaGetError, Result};
use serde::Deserialize;

/// Common trait for file entries from different metadata formats
pub trait FileEntry {
    fn name(&self) -> &str;
    fn source(&self) -> &str;
    fn mtime(&self) -> Option<u64>;
    fn size(&self) -> Option<u64>;
    fn format(&self) -> Option<&str>;
    fn md5(&self) -> Option<&str>;
    fn sha1(&self) -> Option<&str>;
}

/// Common trait for collections of files
pub trait FilesCollection {
    type FileType: FileEntry;
    fn files(&self) -> &[Self::FileType];
}

/// Represents a single file entry from the archive.org JSON metadata
#[derive(Deserialize, Debug)]
pub struct JsonFile {
    pub name: String,
    pub source: String,
    #[serde(deserialize_with = "deserialize_string_to_u64_option")]
    pub mtime: Option<u64>,
    #[serde(deserialize_with = "deserialize_string_to_u64_option")]
    pub size: Option<u64>,
    pub format: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_string_to_u32"
    )]
    pub rotation: Option<u32>,
    pub md5: Option<String>,
    pub crc32: Option<String>,
    pub sha1: Option<String>,
    pub btih: Option<String>,
    pub summation: Option<String>,
    pub original: Option<String>,
}

/// Root structure for parsing the JSON metadata response from archive.org
#[derive(Deserialize, Debug)]
pub struct JsonMetadata {
    pub files: Vec<JsonFile>,
    #[serde(deserialize_with = "deserialize_string_to_u32")]
    pub files_count: u32,
    #[serde(deserialize_with = "deserialize_string_to_u64")]
    pub item_last_updated: u64,
    #[serde(deserialize_with = "deserialize_string_to_u64")]
    pub item_size: u64,
}

impl FileEntry for JsonFile {
    fn name(&self) -> &str {
        &self.name
    }
    fn source(&self) -> &str {
        &self.source
    }
    fn mtime(&self) -> Option<u64> {
        self.mtime
    }
    fn size(&self) -> Option<u64> {
        self.size
    }
    fn format(&self) -> Option<&str> {
        self.format.as_deref()
    }
    fn md5(&self) -> Option<&str> {
        self.md5.as_deref()
    }
    fn sha1(&self) -> Option<&str> {
        self.sha1.as_deref()
    }
}

impl FilesCollection for JsonMetadata {
    type FileType = JsonFile;

    fn files(&self) -> &[Self::FileType] {
        &self.files
    }
}

/// Parses JSON content into JsonMetadata structure
///
/// # Arguments
/// * `json_content` - Raw JSON string from archive.org metadata API
///
/// # Returns
/// * `Ok(JsonMetadata)` if parsing succeeds
/// * `Err(IaGetError)` if parsing fails
pub fn parse_json_files(json_content: &str) -> Result<JsonMetadata> {
    use serde_json::from_str;

    // Provide context for debugging if JSON parsing fails
    match from_str::<JsonMetadata>(json_content) {
        Ok(metadata) => {
            if metadata.files.is_empty() {
                eprintln!("Warning: Parsed JSON metadata but found no files");
            }
            Ok(metadata)
        }
        Err(e) => {
            const DEBUG_TRUNCATE_LEN: usize = 200;
            let preview = if json_content.len() > DEBUG_TRUNCATE_LEN {
                &json_content[..DEBUG_TRUNCATE_LEN]
            } else {
                json_content
            };

            eprintln!(
                "JSON parsing failed.\nError: {}\nContent preview: {}{}",
                e,
                preview,
                if json_content.len() > DEBUG_TRUNCATE_LEN {
                    "..."
                } else {
                    ""
                }
            );
            Err(IaGetError::JsonParsing(format!(
                "Failed to parse JSON metadata: {}",
                e
            )))
        }
    }
}

/// Custom deserializer for string numbers to u64 Option with default support
fn deserialize_string_to_u64_option<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct StringOrU64Visitor;

    impl<'de> Visitor<'de> for StringOrU64Visitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or u64 that can be converted to u64")
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                Ok(None)
            } else {
                value
                    .parse::<u64>()
                    .map(Some)
                    .map_err(|_| de::Error::custom(format!("could not parse '{}' as u64", value)))
            }
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringOrU64Visitor)
}

/// Custom deserializer for string numbers to u64 (required field)
fn deserialize_string_to_u64<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct StringOrU64Visitor;

    impl<'de> Visitor<'de> for StringOrU64Visitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or u64 that can be converted to u64")
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .parse::<u64>()
                .map_err(|_| de::Error::custom(format!("could not parse '{}' as u64", value)))
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }
    }

    deserializer.deserialize_any(StringOrU64Visitor)
}

/// Custom deserializer for optional string numbers to u32
fn deserialize_optional_string_to_u32<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(u32),
    }

    match Option::<StringOrNumber>::deserialize(deserializer)? {
        Some(StringOrNumber::String(s)) => {
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<u32>().map(Some).map_err(|_| {
                    serde::de::Error::custom(format!("could not parse '{}' as u32", s))
                })
            }
        }
        Some(StringOrNumber::Number(n)) => Ok(Some(n)),
        None => Ok(None),
    }
}

/// Custom deserializer for string numbers to u32 (required field)
fn deserialize_string_to_u32<'de, D>(deserializer: D) -> std::result::Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct StringOrU32Visitor;

    impl<'de> Visitor<'de> for StringOrU32Visitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or u32 that can be converted to u32")
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .parse::<u32>()
                .map_err(|_| de::Error::custom(format!("could not parse '{}' as u32", value)))
        }

        fn visit_u32<E>(self, value: u32) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }
    }

    deserializer.deserialize_any(StringOrU32Visitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_parsing_with_valid_data() {
        let json_data = r#"{
            "files": [
                {
                    "name": "test.txt",
                    "source": "original",
                    "mtime": "1234567890",
                    "size": "1024",
                    "format": "Text",
                    "md5": "abcd1234"
                }
            ],
            "files_count": "1",
            "item_last_updated": "1234567890",
            "item_size": "1024"
        }"#;

        let result = parse_json_files(json_data);
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert_eq!(metadata.files.len(), 1);
        assert_eq!(metadata.files[0].name(), "test.txt");
        assert_eq!(metadata.files[0].size(), Some(1024));
        assert_eq!(metadata.files[0].rotation, None); // Should be None when not provided
    }

    #[test]
    fn test_json_parsing_with_string_numbers() {
        // Test case that reproduces the reported error with string numbers for normally u32/u64 fields
        let json_data = r#"{
            "files": [
                {
                    "name": "luigi_episode.mp3",
                    "source": "original", 
                    "mtime": "1756778586",
                    "size": "12345678",
                    "format": "VBR MP3",
                    "rotation": "0",
                    "md5": "abcd1234"
                }
            ],
            "files_count": "1",
            "item_last_updated": "1756778586",
            "item_size": "12345678"
        }"#;

        let result = parse_json_files(json_data);
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert_eq!(metadata.files.len(), 1);
        assert_eq!(metadata.files[0].name(), "luigi_episode.mp3");
        assert_eq!(metadata.files[0].size(), Some(12345678));
        assert_eq!(metadata.files[0].rotation, Some(0));
        assert_eq!(metadata.files_count, 1);
        assert_eq!(metadata.item_last_updated, 1756778586);
        assert_eq!(metadata.item_size, 12345678);
    }

    #[test]
    fn test_json_parsing_real_luigi_error_case() {
        // This test reproduces the exact case from the GitHub comment error
        // where Archive.org returns fields as strings that we expect as numbers
        let luigi_json = r#"{
            "created": 1756778586,
            "d1": "ia902806.us.archive.org",
            "d2": "ia802806.us.archive.org", 
            "dir": "/17/items/luigi",
            "files": [
                {
                    "name": "Life_with_Luigi_49-02-27_ep024_Luigi_Needs_Drivers_License.mp3",
                    "source": "original",
                    "mtime": "1756778586",
                    "size": "23456789",
                    "format": "VBR MP3",
                    "rotation": "0",
                    "md5": "abcd1234"
                }
            ],
            "files_count": "1",
            "item_last_updated": "1756778586",
            "item_size": "23456789"
        }"#;

        let result = parse_json_files(luigi_json);
        assert!(
            result.is_ok(),
            "Luigi JSON should parse successfully now with string-to-number conversion"
        );

        let metadata = result.unwrap();
        assert_eq!(metadata.files.len(), 1);
        assert_eq!(
            metadata.files[0].name(),
            "Life_with_Luigi_49-02-27_ep024_Luigi_Needs_Drivers_License.mp3"
        );
        assert_eq!(metadata.files[0].rotation, Some(0)); // This would have failed before with "invalid type: string "0", expected u32"
        assert_eq!(metadata.files_count, 1);
        assert_eq!(metadata.item_last_updated, 1756778586);
        assert_eq!(metadata.item_size, 23456789);
    }

    #[test]
    fn test_json_parsing_with_invalid_data() {
        let invalid_json = "{ invalid json content";
        let result = parse_json_files(invalid_json);
        assert!(result.is_err());
    }
}
