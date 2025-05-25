use serde::Deserialize;
use serde_xml_rs::from_str;
use crate::{Result, IaGetError};
use crate::constants::XML_DEBUG_TRUNCATE_LEN;

/// Root structure for parsing the XML files list from archive.org
/// The actual XML structure has a `files` root element containing multiple `file` elements
#[derive(Deserialize, Debug)]
pub struct XmlFiles {
    #[serde(rename = "file", default)]
    pub files: Vec<XmlFile>,
}

/// Represents a single file entry from the archive.org XML metadata
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

/// Parses XML content into XmlFiles structure with improved error context
/// 
/// # Arguments
/// * `xml_content` - Raw XML content string from archive.org
/// 
/// # Returns
/// * `Ok(XmlFiles)` if parsing succeeds
/// * `Err(IaGetError)` with context if parsing fails
pub fn parse_xml_files(xml_content: &str) -> Result<XmlFiles> {
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
