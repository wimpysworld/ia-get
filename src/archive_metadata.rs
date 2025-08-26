use serde::Deserialize;
use crate::{Result, IaGetError};
use crate::constants::XML_DEBUG_TRUNCATE_LEN;

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

/// Common trait for files collections from different metadata formats
pub trait FilesCollection {
    type FileType: FileEntry;
    fn files(&self) -> &[Self::FileType];
}

/// Root structure for parsing the XML files list from archive.org
/// The actual XML structure has a `files` root element containing multiple `file` elements
#[derive(Deserialize, Debug)]
pub struct XmlFiles {
    #[serde(rename = "file", default)]
    pub files: Vec<XmlFile>,
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
    pub rotation: Option<u32>,
    pub md5: Option<String>,
    pub crc32: Option<String>,
    pub sha1: Option<String>,
    pub btih: Option<String>,
    pub summation: Option<String>,
    pub original: Option<String>,
}

/// Represents a single file entry from the archive.org XML metadata (legacy)
///
/// Archive.org XML structure has both attributes and nested elements:
/// ```xml
/// <file name="..." source="...">
///   <mtime>...</mtime>
///   <size>...</size>
///   <md5>...</md5>
///   ...
/// </file>
/// ```
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct XmlFile {
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@source")]
    pub source: String,
    pub mtime: Option<u64>,
    pub size: Option<u64>,
    pub format: Option<String>,
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
    pub files_count: u32,
    pub item_last_updated: u64,
    pub item_size: u64,
    pub server: String,
    pub workable_servers: Vec<String>,
}

// Trait implementations for file entries
impl FileEntry for JsonFile {
    fn name(&self) -> &str { &self.name }
    fn source(&self) -> &str { &self.source }
    fn mtime(&self) -> Option<u64> { self.mtime }
    fn size(&self) -> Option<u64> { self.size }
    fn format(&self) -> Option<&str> { self.format.as_deref() }
    fn md5(&self) -> Option<&str> { self.md5.as_deref() }
    fn sha1(&self) -> Option<&str> { self.sha1.as_deref() }
}

impl FileEntry for XmlFile {
    fn name(&self) -> &str { &self.name }
    fn source(&self) -> &str { &self.source }
    fn mtime(&self) -> Option<u64> { self.mtime }
    fn size(&self) -> Option<u64> { self.size }
    fn format(&self) -> Option<&str> { self.format.as_deref() }
    fn md5(&self) -> Option<&str> { self.md5.as_deref() }
    fn sha1(&self) -> Option<&str> { self.sha1.as_deref() }
}

// Collection trait implementations
impl FilesCollection for JsonMetadata {
    type FileType = JsonFile;
    fn files(&self) -> &[Self::FileType] { &self.files }
}

impl FilesCollection for XmlFiles {
    type FileType = XmlFile;
    fn files(&self) -> &[Self::FileType] { &self.files }
}

/// Parses XML content into XmlFiles structure with improved error context
/// 
/// # Arguments
/// * `xml_content` - Raw XML content string from archive.org
/// 
/// # Returns
/// * `Ok(XmlFiles)` if parsing succeeds
/// * `Err(IaGetError)` with context if parsing fails
pub fn parse_xml_files(xml_content: &str) -> Result<XmlFiles> {
    use serde_xml_rs::from_str;
    from_str(xml_content).map_err(|e| {
        let preview = if xml_content.len() > XML_DEBUG_TRUNCATE_LEN {
            &xml_content[..XML_DEBUG_TRUNCATE_LEN]
        } else {
            xml_content
        };
        
        IaGetError::XmlParsing(format!(
            "Failed to parse _files.xml metadata: {}. Content preview: {}{}",
            e,
            preview,
            if xml_content.len() > XML_DEBUG_TRUNCATE_LEN { "..." } else { "" }
        ))
    })
}

/// Custom deserializer to convert string numbers to u64 Option
fn deserialize_string_to_u64_option<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct StringToU64Visitor;

    impl<'de> Visitor<'de> for StringToU64Visitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or number that can be converted to u64")
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            value.parse::<u64>().map(Some).map_err(de::Error::custom)
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_none<E>(self) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Option<u64>, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringToU64Visitor)
}

/// Parses JSON content into JsonMetadata structure
/// 
/// # Arguments
/// * `json_content` - Raw JSON content string from archive.org
/// 
/// # Returns
/// * `Ok(JsonMetadata)` if parsing succeeds
/// * `Err(IaGetError)` with context if parsing fails
pub fn parse_json_metadata(json_content: &str) -> Result<JsonMetadata> {
    serde_json::from_str(json_content).map_err(|e| {
        let preview = if json_content.len() > XML_DEBUG_TRUNCATE_LEN {
            &json_content[..XML_DEBUG_TRUNCATE_LEN]
        } else {
            json_content
        };
        
        IaGetError::JsonParsing(format!(
            "Failed to parse JSON metadata: {}. Content preview: {}{}",
            e,
            preview,
            if json_content.len() > XML_DEBUG_TRUNCATE_LEN { "..." } else { "" }
        ))
    })
}
