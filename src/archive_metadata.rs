use crate::constants::XML_DEBUG_TRUNCATE_LEN;
use crate::{IaGetError, Result};
use quick_xml::de::from_str;
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
#[derive(Deserialize, Debug)]
pub struct XmlFile {
    #[serde(rename = "@name")]
    pub name: String,
    pub size: Option<u64>,
    pub md5: Option<String>,
    #[serde(default)]
    pub private: Option<String>,
}

impl XmlFile {
    /// Returns `true` when `<private>` metadata marks this file as `"true"` or `"1"`
    /// (trimmed, ASCII case-insensitive).
    pub fn is_private(&self) -> bool {
        self.private.as_deref().is_some_and(|value| {
            let trimmed = value.trim();
            trimmed.eq_ignore_ascii_case("true") || trimmed == "1"
        })
    }
}

/// Truncates `content` to at most `max_bytes` on a valid UTF-8 character boundary.
fn truncate_preview(content: &str, max_bytes: usize) -> (&str, bool) {
    if content.len() <= max_bytes {
        return (content, false);
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    (&content[..end], true)
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
        let (preview, truncated) = truncate_preview(xml_content, XML_DEBUG_TRUNCATE_LEN);

        IaGetError::XmlParsing(format!(
            "Failed to parse _files.xml metadata: {}. Content preview: {}{}",
            e,
            preview,
            if truncated { "..." } else { "" }
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xml_files_handles_attributes_optional_fields_unknown_tags_and_empty_root() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<files>
  <file name="dir/book.pdf" source="original" extra="ignored">
    <mtime>1700000000</mtime>
    <size>123456</size>
    <md5>d41d8cd98f00b204e9800998ecf8427e</md5>
    <sha1>da39a3ee5e6b4b0d3255bfef95601890afd80709</sha1>
    <format>Text PDF</format>
  </file>
  <file name="notes.txt" source="metadata">
    <crc32>00000000</crc32>
  </file>
</files>"#;

        let parsed = parse_xml_files(xml).expect("valid archive.org XML should parse");
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].name, "dir/book.pdf");
        assert_eq!(parsed.files[0].size, Some(123_456));
        assert_eq!(
            parsed.files[0].md5.as_deref(),
            Some("d41d8cd98f00b204e9800998ecf8427e")
        );
        assert_eq!(parsed.files[1].name, "notes.txt");
        assert_eq!(parsed.files[1].size, None);
        assert_eq!(parsed.files[1].md5, None);

        let empty_self_closing = parse_xml_files("<files/>").expect("empty <files/> should parse");
        assert!(empty_self_closing.files.is_empty());

        let empty_pair =
            parse_xml_files("<files></files>").expect("empty <files></files> should parse");
        assert!(empty_pair.files.is_empty());
    }

    #[test]
    fn test_parse_xml_files_truncates_multibyte_utf8_preview_without_panicking() {
        // Place a 4-byte UTF-8 character straddling byte offset XML_DEBUG_TRUNCATE_LEN (1000).
        let mut malformed = "a".repeat(XML_DEBUG_TRUNCATE_LEN - 1);
        malformed.push_str("🦀🦀<unclosed");
        assert!(malformed.len() > XML_DEBUG_TRUNCATE_LEN);
        assert!(!malformed.is_char_boundary(XML_DEBUG_TRUNCATE_LEN));

        let err = parse_xml_files(&malformed).unwrap_err();
        assert!(matches!(err, IaGetError::XmlParsing(_)));
        let msg = err.to_string();
        let (_, preview) = msg
            .split_once("Content preview: ")
            .expect("error message should include Content preview");
        assert!(
            preview.ends_with("..."),
            "expected ellipsis in preview: {preview}"
        );
        assert_eq!(preview.len(), XML_DEBUG_TRUNCATE_LEN - 1 + 3);
        assert!(
            !preview.contains('🦀'),
            "character straddling byte limit should not be split or partially included in preview"
        );
    }

    #[test]
    fn test_parse_xml_files_does_not_expand_doctype_entities() {
        let xxe_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE files [
  <!ENTITY lol "expanded_secret">
  <!ENTITY lol2 "&lol;&lol;&lol;&lol;">
]>
<files>
  <file name="&lol2;">
    <size>1</size>
  </file>
</files>"#;

        match parse_xml_files(xxe_xml) {
            Ok(parsed) => {
                for file in &parsed.files {
                    assert!(
                        !file.name.contains("expanded_secret"),
                        "custom DOCTYPE entity must never be expanded: {:?}",
                        file.name
                    );
                }
            }
            Err(err) => {
                assert!(
                    matches!(err, IaGetError::XmlParsing(_)),
                    "expected XmlParsing rejection for custom entity, got {err:?}"
                );
            }
        }
    }

    #[test]
    fn test_parse_xml_files_parses_private_tag_variants() {
        let xml = r#"<files>
  <file name="public.pdf"><size>10</size></file>
  <file name="priv_true.pdf"><private>true</private></file>
  <file name="priv_upper.pdf"><private>  TrUe  </private></file>
  <file name="priv_one.pdf"><private> 1 </private></file>
  <file name="pub_false.pdf"><private>false</private></file>
  <file name="pub_zero.pdf"><private>0</private></file>
  <file name="pub_empty.pdf"><private/></file>
  <file name="pub_other.pdf"><private>yes</private></file>
</files>"#;

        let parsed = parse_xml_files(xml).expect("XML with <private> variants should parse");
        let results: Vec<(&str, bool)> = parsed
            .files
            .iter()
            .map(|f| (f.name.as_str(), f.is_private()))
            .collect();
        assert_eq!(
            results,
            vec![
                ("public.pdf", false),
                ("priv_true.pdf", true),
                ("priv_upper.pdf", true),
                ("priv_one.pdf", true),
                ("pub_false.pdf", false),
                ("pub_zero.pdf", false),
                ("pub_empty.pdf", false),
                ("pub_other.pdf", false),
            ]
        );
    }
}
