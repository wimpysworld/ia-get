//! Predefined file format categories for Internet Archive content
//!
//! This module provides comprehensive lists of file formats commonly found
//! in Internet Archive collections, organized by content type. These formats
//! are based on the official Internet Archive file format documentation.
//!
//! ## Format Categories
//!
//! - **Documents**: PDF, text files, eBooks, office documents
//! - **Images**: Photos, graphics, artwork in various formats
//! - **Audio**: Music, recordings, audiobooks, podcasts
//! - **Video**: Movies, TV shows, documentaries, clips
//! - **Software**: Applications, games, operating systems
//! - **Data**: Datasets, databases, structured data files
//! - **Web**: Web pages, websites, web archives
//! - **Archives**: Compressed files and archives
//! - **Metadata**: Archive-generated metadata and info files

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Category of file formats
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormatCategory {
    Documents,
    Images,
    Audio,
    Video,
    Software,
    Data,
    Web,
    Archives,
    Metadata,
}

impl FormatCategory {
    /// Get human-readable name for the category
    pub fn display_name(&self) -> &'static str {
        match self {
            FormatCategory::Documents => "Documents",
            FormatCategory::Images => "Images",
            FormatCategory::Audio => "Audio",
            FormatCategory::Video => "Video",
            FormatCategory::Software => "Software",
            FormatCategory::Data => "Data",
            FormatCategory::Web => "Web Content",
            FormatCategory::Archives => "Archives",
            FormatCategory::Metadata => "Metadata",
        }
    }

    /// Get description for the category
    pub fn description(&self) -> &'static str {
        match self {
            FormatCategory::Documents => "PDF files, text documents, eBooks, office documents",
            FormatCategory::Images => "Photographs, graphics, artwork, diagrams",
            FormatCategory::Audio => "Music, recordings, audiobooks, podcasts",
            FormatCategory::Video => "Movies, TV shows, documentaries, video clips",
            FormatCategory::Software => "Applications, games, operating systems, installers",
            FormatCategory::Data => "Datasets, databases, structured data files",
            FormatCategory::Web => "Web pages, websites, web archives",
            FormatCategory::Archives => "Compressed files, archives, backup files",
            FormatCategory::Metadata => "Archive metadata, checksums, indexes",
        }
    }

    /// Get all available categories
    pub fn all() -> Vec<FormatCategory> {
        vec![
            FormatCategory::Documents,
            FormatCategory::Images,
            FormatCategory::Audio,
            FormatCategory::Video,
            FormatCategory::Software,
            FormatCategory::Data,
            FormatCategory::Web,
            FormatCategory::Archives,
            FormatCategory::Metadata,
        ]
    }
}

/// A collection of file format definitions organized by category
#[derive(Debug, Clone)]
pub struct FileFormats {
    formats: HashMap<FormatCategory, Vec<String>>,
}

impl Default for FileFormats {
    fn default() -> Self {
        Self::new()
    }
}

impl FileFormats {
    /// Create a new FileFormats instance with predefined format lists
    pub fn new() -> Self {
        let mut formats = HashMap::new();

        // Document formats commonly found in Internet Archive
        formats.insert(
            FormatCategory::Documents,
            vec![
                // PDF and eBook formats
                "pdf".to_string(),
                "epub".to_string(),
                "mobi".to_string(),
                "azw".to_string(),
                "azw3".to_string(),
                "fb2".to_string(),
                "lit".to_string(),
                "pdb".to_string(),
                "djvu".to_string(),
                "djv".to_string(),
                // Text formats
                "txt".to_string(),
                "rtf".to_string(),
                "md".to_string(),
                "rst".to_string(),
                "tex".to_string(),
                // Office documents
                "doc".to_string(),
                "docx".to_string(),
                "xls".to_string(),
                "xlsx".to_string(),
                "ppt".to_string(),
                "pptx".to_string(),
                "odt".to_string(),
                "ods".to_string(),
                "odp".to_string(),
                // Other document formats
                "ps".to_string(),
                "chm".to_string(),
                "hlp".to_string(),
            ],
        );

        // Image formats
        formats.insert(
            FormatCategory::Images,
            vec![
                // Common image formats
                "jpg".to_string(),
                "jpeg".to_string(),
                "png".to_string(),
                "gif".to_string(),
                "bmp".to_string(),
                "tiff".to_string(),
                "tif".to_string(),
                "webp".to_string(),
                // Raw and professional formats
                "raw".to_string(),
                "cr2".to_string(),
                "nef".to_string(),
                "arw".to_string(),
                "dng".to_string(),
                // Vector graphics
                "svg".to_string(),
                "eps".to_string(),
                "ai".to_string(),
                // Archive image formats
                "jp2".to_string(),
                "jpx".to_string(),
                "pgm".to_string(),
                "ppm".to_string(),
                "pbm".to_string(),
                "pnm".to_string(),
            ],
        );

        // Audio formats
        formats.insert(
            FormatCategory::Audio,
            vec![
                // Compressed audio
                "mp3".to_string(),
                "aac".to_string(),
                "ogg".to_string(),
                "oga".to_string(),
                "m4a".to_string(),
                "wma".to_string(),
                "opus".to_string(),
                // Uncompressed audio
                "wav".to_string(),
                "flac".to_string(),
                "ape".to_string(),
                "aiff".to_string(),
                "au".to_string(),
                // Specialized formats
                "mid".to_string(),
                "midi".to_string(),
                "mod".to_string(),
                "s3m".to_string(),
                "xm".to_string(),
                "it".to_string(),
            ],
        );

        // Video formats
        formats.insert(
            FormatCategory::Video,
            vec![
                // Modern video formats
                "mp4".to_string(),
                "mkv".to_string(),
                "webm".to_string(),
                "avi".to_string(),
                "mov".to_string(),
                "wmv".to_string(),
                "flv".to_string(),
                "f4v".to_string(),
                "m4v".to_string(),
                // Older/specialized formats
                "mpg".to_string(),
                "mpeg".to_string(),
                "3gp".to_string(),
                "ogv".to_string(),
                "asf".to_string(),
                "rm".to_string(),
                "rmvb".to_string(),
                "vob".to_string(),
                "ts".to_string(),
                "m2ts".to_string(),
            ],
        );

        // Software and executable formats
        formats.insert(
            FormatCategory::Software,
            vec![
                // Executables
                "exe".to_string(),
                "msi".to_string(),
                "dmg".to_string(),
                "pkg".to_string(),
                "deb".to_string(),
                "rpm".to_string(),
                "appimage".to_string(),
                // Archives/installers
                "iso".to_string(),
                "img".to_string(),
                "bin".to_string(),
                "cue".to_string(),
                // Source code
                "c".to_string(),
                "cpp".to_string(),
                "h".to_string(),
                "py".to_string(),
                "java".to_string(),
                "rb".to_string(),
                "go".to_string(),
                "rs".to_string(),
            ],
        );

        // Data formats
        formats.insert(
            FormatCategory::Data,
            vec![
                // Structured data
                "csv".to_string(),
                "tsv".to_string(),
                "json".to_string(),
                "xml".to_string(),
                "yaml".to_string(),
                "yml".to_string(),
                "sql".to_string(),
                // Database formats
                "db".to_string(),
                "sqlite".to_string(),
                "sqlite3".to_string(),
                "mdb".to_string(),
                "accdb".to_string(),
                // Configuration
                "ini".to_string(),
                "cfg".to_string(),
                "conf".to_string(),
                "toml".to_string(),
                // Log files
                "log".to_string(),
            ],
        );

        // Web content
        formats.insert(
            FormatCategory::Web,
            vec![
                // Web pages
                "html".to_string(),
                "htm".to_string(),
                "xhtml".to_string(),
                "css".to_string(),
                "js".to_string(),
                "php".to_string(),
                "asp".to_string(),
                "jsp".to_string(),
                // Web archives
                "warc".to_string(),
                "arc".to_string(),
                "har".to_string(),
                // Web formats
                "rss".to_string(),
                "atom".to_string(),
                "sitemap".to_string(),
            ],
        );

        // Archive formats
        formats.insert(
            FormatCategory::Archives,
            vec![
                // Common archives
                "zip".to_string(),
                "rar".to_string(),
                "7z".to_string(),
                "tar".to_string(),
                "gz".to_string(),
                "bz2".to_string(),
                "xz".to_string(),
                "lz".to_string(),
                "lzma".to_string(),
                "zst".to_string(),
                // Combined formats
                "tar.gz".to_string(),
                "tar.bz2".to_string(),
                "tar.xz".to_string(),
                "tar.lz".to_string(),
                "tar.zst".to_string(),
                "tgz".to_string(),
                "tbz".to_string(),
                "tbz2".to_string(),
                "txz".to_string(),
                // Other archives
                "cab".to_string(),
                "ace".to_string(),
                "arj".to_string(),
                "lha".to_string(),
                "lzh".to_string(),
            ],
        );

        // Metadata and system files
        formats.insert(
            FormatCategory::Metadata,
            vec![
                // Archive metadata
                "xml".to_string(),
                "json".to_string(),
                "sqlite".to_string(),
                "marc".to_string(),
                "mrc".to_string(),
                // Checksums and verification
                "md5".to_string(),
                "sha1".to_string(),
                "sha256".to_string(),
                "crc".to_string(),
                "sfv".to_string(),
                // Torrents and links
                "torrent".to_string(),
                "magnet".to_string(),
                // System files
                "tmp".to_string(),
                "temp".to_string(),
                "log".to_string(),
                "bak".to_string(),
                "old".to_string(),
            ],
        );

        Self { formats }
    }

    /// Get all formats for a specific category
    pub fn get_formats(&self, category: &FormatCategory) -> Vec<String> {
        self.formats.get(category).cloned().unwrap_or_default()
    }

    /// Get all formats as a flat list
    pub fn get_all_formats(&self) -> Vec<String> {
        let mut all_formats = Vec::new();
        for formats in self.formats.values() {
            all_formats.extend(formats.clone());
        }
        all_formats.sort();
        all_formats.dedup();
        all_formats
    }

    /// Find which category a format belongs to (returns the most specific category)
    pub fn find_category(&self, format: &str) -> Option<FormatCategory> {
        let format_lower = format.to_lowercase();

        // Define priority order - more specific categories first
        let priority_order = vec![
            FormatCategory::Metadata,  // Most specific
            FormatCategory::Web,       // Web-specific files
            FormatCategory::Software,  // Software/code files
            FormatCategory::Data,      // Structured data
            FormatCategory::Archives,  // Archive formats
            FormatCategory::Documents, // Documents
            FormatCategory::Images,    // Images
            FormatCategory::Audio,     // Audio
            FormatCategory::Video,     // Video
        ];

        for category in priority_order {
            if let Some(formats) = self.formats.get(&category) {
                if formats.iter().any(|f| f == &format_lower) {
                    return Some(category);
                }
            }
        }
        None
    }

    /// Check if a format is valid (exists in any category)
    pub fn is_valid_format(&self, format: &str) -> bool {
        self.find_category(format).is_some()
    }

    /// Get format suggestions based on partial input
    pub fn suggest_formats(&self, partial: &str) -> Vec<String> {
        let partial_lower = partial.to_lowercase();
        let mut suggestions = Vec::new();

        for formats in self.formats.values() {
            for format in formats {
                if format.starts_with(&partial_lower) {
                    suggestions.push(format.clone());
                }
            }
        }

        suggestions.sort();
        suggestions.dedup();
        suggestions
    }

    /// Get common format combinations as presets
    pub fn get_common_presets() -> Vec<(String, String, Vec<String>)> {
        vec![
            (
                "Documents".to_string(),
                "PDF files, eBooks, and text documents".to_string(),
                vec![
                    "pdf".to_string(),
                    "epub".to_string(),
                    "txt".to_string(),
                    "doc".to_string(),
                    "docx".to_string(),
                ],
            ),
            (
                "Images".to_string(),
                "Photos and graphics in common formats".to_string(),
                vec![
                    "jpg".to_string(),
                    "jpeg".to_string(),
                    "png".to_string(),
                    "gif".to_string(),
                    "tiff".to_string(),
                ],
            ),
            (
                "Audio".to_string(),
                "Music and audio recordings".to_string(),
                vec![
                    "mp3".to_string(),
                    "flac".to_string(),
                    "ogg".to_string(),
                    "wav".to_string(),
                    "m4a".to_string(),
                ],
            ),
            (
                "Video".to_string(),
                "Movies and video files".to_string(),
                vec![
                    "mp4".to_string(),
                    "mkv".to_string(),
                    "avi".to_string(),
                    "mov".to_string(),
                    "webm".to_string(),
                ],
            ),
            (
                "Archives".to_string(),
                "Compressed files and archives".to_string(),
                vec![
                    "zip".to_string(),
                    "rar".to_string(),
                    "7z".to_string(),
                    "tar.gz".to_string(),
                    "tar.bz2".to_string(),
                ],
            ),
            (
                "No Metadata".to_string(),
                "Exclude archive metadata and system files".to_string(),
                vec![], // This is an exclude preset
            ),
        ]
    }

    /// Get metadata-only formats (useful for exclusion)
    pub fn get_metadata_formats() -> Vec<String> {
        vec![
            "xml".to_string(),
            "sqlite".to_string(),
            "md5".to_string(),
            "sha1".to_string(),
            "torrent".to_string(),
            "log".to_string(),
            "tmp".to_string(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_categories() {
        let formats = FileFormats::new();

        // Test that all categories have formats
        for category in FormatCategory::all() {
            let category_formats = formats.get_formats(&category);
            assert!(
                !category_formats.is_empty(),
                "Category {:?} should have formats",
                category
            );
        }
    }

    #[test]
    fn test_find_category() {
        let formats = FileFormats::new();

        assert_eq!(
            formats.find_category("pdf"),
            Some(FormatCategory::Documents)
        );
        assert_eq!(formats.find_category("jpg"), Some(FormatCategory::Images));
        assert_eq!(formats.find_category("mp3"), Some(FormatCategory::Audio));
        assert_eq!(formats.find_category("mp4"), Some(FormatCategory::Video));
        assert_eq!(formats.find_category("zip"), Some(FormatCategory::Archives));
        assert_eq!(formats.find_category("nonexistent"), None);
    }

    #[test]
    fn test_format_suggestions() {
        let formats = FileFormats::new();

        let suggestions = formats.suggest_formats("mp");
        assert!(suggestions.contains(&"mp3".to_string()));
        assert!(suggestions.contains(&"mp4".to_string()));

        let pdf_suggestions = formats.suggest_formats("pd");
        assert!(pdf_suggestions.contains(&"pdf".to_string()));
    }

    #[test]
    fn test_common_presets() {
        let presets = FileFormats::get_common_presets();
        assert!(!presets.is_empty());

        // Check that each preset has required fields
        for (name, description, _formats) in presets {
            assert!(!name.is_empty());
            assert!(!description.is_empty());
            // Note: formats can be empty for exclude presets
        }
    }

    #[test]
    fn test_all_formats_unique() {
        let formats = FileFormats::new();
        let all_formats = formats.get_all_formats();
        let mut sorted_formats = all_formats.clone();
        sorted_formats.sort();
        sorted_formats.dedup();

        // Should be the same length if no duplicates
        assert_eq!(all_formats.len(), sorted_formats.len());
    }
}
