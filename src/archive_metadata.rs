use serde::Deserialize;

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
