//! File filtering using repeatable glob patterns with `!` negation.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::archive_metadata::XmlFiles;
use crate::{IaGetError, Result};

/// Compiled file filter combining optional include and exclude glob sets.
///
/// Positive patterns form an allow-list: when at least one positive pattern is
/// supplied, a file must match at least one positive pattern. Negative patterns
/// (prefixed with `!`) form a deny-list and always take precedence over positive
/// matches.
#[derive(Debug, Clone)]
pub struct FileFilter {
    patterns: Vec<String>,
    includes: Option<GlobSet>,
    excludes: Option<GlobSet>,
}

impl FileFilter {
    /// Compiles an iterator of user-supplied glob patterns into a [`FileFilter`].
    ///
    /// Patterns starting with `'!'` are treated as exclusions; all other patterns
    /// are inclusions. Prefixing a leading `'!'` with a backslash (`\!`) keeps
    /// the pattern as an inclusion matching a literal `'!'`.
    pub fn compile<I, S>(patterns: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut raw_patterns = Vec::new();
        let mut include_builder = GlobSetBuilder::new();
        let mut exclude_builder = GlobSetBuilder::new();
        let mut has_includes = false;
        let mut has_excludes = false;

        for pattern in patterns {
            let raw = pattern.as_ref();
            if raw.is_empty() {
                return Err(IaGetError::InvalidFilter {
                    pattern: raw.to_string(),
                    reason: "pattern cannot be empty".to_string(),
                });
            }

            let (is_negative, glob_str) = match raw.strip_prefix('!') {
                Some(rest) => {
                    if rest.is_empty() {
                        return Err(IaGetError::InvalidFilter {
                            pattern: raw.to_string(),
                            reason: "negated pattern cannot be empty".to_string(),
                        });
                    }
                    (true, rest)
                }
                None => (false, raw),
            };

            let glob = GlobBuilder::new(glob_str)
                .literal_separator(false)
                .backslash_escape(true)
                .build()
                .map_err(|err| IaGetError::InvalidFilter {
                    pattern: raw.to_string(),
                    reason: err.to_string(),
                })?;

            raw_patterns.push(raw.to_string());
            if is_negative {
                exclude_builder.add(glob);
                has_excludes = true;
            } else {
                include_builder.add(glob);
                has_includes = true;
            }
        }

        let includes = if has_includes {
            Some(
                include_builder
                    .build()
                    .map_err(|err| IaGetError::InvalidFilter {
                        pattern: raw_patterns.join(", "),
                        reason: err.to_string(),
                    })?,
            )
        } else {
            None
        };

        let excludes = if has_excludes {
            Some(
                exclude_builder
                    .build()
                    .map_err(|err| IaGetError::InvalidFilter {
                        pattern: raw_patterns.join(", "),
                        reason: err.to_string(),
                    })?,
            )
        } else {
            None
        };

        Ok(Self {
            patterns: raw_patterns,
            includes,
            excludes,
        })
    }

    /// Returns `true` when no filter patterns were supplied.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Returns `true` if `file_name` passes the compiled include and exclude sets.
    ///
    /// Leading `'/'` characters are stripped before matching to match archive.org
    /// metadata normalisation.
    pub fn matches(&self, file_name: &str) -> bool {
        let normalized = file_name.trim_start_matches('/');

        if let Some(includes) = &self.includes {
            if !includes.is_match(normalized) {
                return false;
            }
        }

        if let Some(excludes) = &self.excludes {
            if excludes.is_match(normalized) {
                return false;
            }
        }

        true
    }

    /// Filters parsed archive metadata entries, returning an error if active
    /// filters exclude every file.
    pub fn apply(&self, files: XmlFiles) -> Result<XmlFiles> {
        if self.is_empty() {
            return Ok(files);
        }

        let matched: Vec<_> = files
            .files
            .into_iter()
            .filter(|entry| self.matches(&entry.name))
            .collect();

        if matched.is_empty() {
            return Err(IaGetError::NoMatchingFiles {
                patterns: self.patterns.join(", "),
            });
        }

        Ok(XmlFiles { files: matched })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_metadata::XmlFile;

    fn xml_file(name: &str) -> XmlFile {
        XmlFile {
            name: name.to_string(),
            source: "original".to_string(),
            mtime: None,
            size: Some(128),
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        }
    }

    #[test]
    fn empty_filter_passes_everything_including_empty_file_list() {
        let filter = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        assert!(filter.is_empty());
        assert!(filter.matches("anything.pdf"));

        let empty = XmlFiles { files: Vec::new() };
        let result = filter.apply(empty).unwrap();
        assert!(result.files.is_empty());
    }

    #[test]
    fn rejects_empty_and_bare_negation_patterns() {
        let err = FileFilter::compile([""]).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::InvalidFilter { ref pattern, .. } if pattern.is_empty()
        ));

        let err = FileFilter::compile(["!"]).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::InvalidFilter { ref pattern, .. } if pattern == "!"
        ));
    }

    #[test]
    fn rejects_invalid_glob_preserving_original_pattern() {
        let err = FileFilter::compile(["!*.[pdf"]).unwrap_err();
        assert!(
            matches!(
                err,
                IaGetError::InvalidFilter { ref pattern, .. } if pattern == "!*.[pdf"
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn positive_and_negative_patterns_combine_and_normalise_leading_slash() {
        let filter = FileFilter::compile(["*.{pdf,cbz}", "!*draft*"]).unwrap();
        assert!(!filter.is_empty());

        assert!(filter.matches("issue01.pdf"));
        assert!(filter.matches("sub/dir/issue02.cbz"));
        assert!(filter.matches("/sub/dir/issue03.pdf"));
        assert!(!filter.matches("sub/issue01-draft.pdf"));
        assert!(!filter.matches("cover.jpg"));
    }

    #[test]
    fn escaped_leading_bang_matches_literal_exclamation_as_include() {
        let filter = FileFilter::compile([r"\!important.txt"]).unwrap();
        assert!(filter.matches("!important.txt"));
        assert!(!filter.matches("important.txt"));
    }

    #[test]
    fn apply_returns_no_matching_files_error_when_nothing_matches() {
        let filter = FileFilter::compile(["*.pdf", "!*sample*"]).unwrap();
        let files = XmlFiles {
            files: vec![xml_file("cover.jpg"), xml_file("sample.pdf")],
        };

        let err = filter.apply(files).unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "*.pdf, !*sample*"
        ));
        assert_eq!(
            err.to_string(),
            "No files matched filter(s): *.pdf, !*sample*"
        );
    }

    fn file_names(files: &XmlFiles) -> Vec<&str> {
        files
            .files
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    #[test]
    fn inactive_filter_preserves_all_files_unchanged() {
        let filter = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        let files = XmlFiles {
            files: vec![
                xml_file("issue01.pdf"),
                xml_file("scans/issue02.cbz"),
                xml_file("/meta/item.torrent"),
            ],
        };

        let result = filter.apply(files).unwrap();
        assert_eq!(
            file_names(&result),
            vec!["issue01.pdf", "scans/issue02.cbz", "/meta/item.torrent"]
        );
    }

    #[test]
    fn positive_only_single_and_multiple_or_patterns() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file("issue01.pdf"),
                xml_file("docs/guide.pdf"),
                xml_file("book.epub"),
                xml_file("cover.jpg"),
                xml_file("item.torrent"),
            ],
        };

        let pdf_only = FileFilter::compile(["*.pdf"]).unwrap();
        let pdf_result = pdf_only.apply(make_files()).unwrap();
        assert_eq!(
            file_names(&pdf_result),
            vec!["issue01.pdf", "docs/guide.pdf"]
        );

        let pdf_or_epub = FileFilter::compile(["*.pdf", "*.epub"]).unwrap();
        let or_result = pdf_or_epub.apply(make_files()).unwrap();
        assert_eq!(
            file_names(&or_result),
            vec!["issue01.pdf", "docs/guide.pdf", "book.epub"]
        );
    }

    #[test]
    fn negative_only_single_and_multiple_exclusions() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file("issue01.pdf"),
                xml_file("docs/guide.pdf"),
                xml_file("item.torrent"),
                xml_file("meta/backup.torrent"),
                xml_file("item_files.sqlite"),
                xml_file("meta/index.sqlite"),
                xml_file("cover.jpg"),
            ],
        };

        let no_torrents = FileFilter::compile(["!*.torrent"]).unwrap();
        let without_torrents = no_torrents.apply(make_files()).unwrap();
        assert_eq!(
            file_names(&without_torrents),
            vec![
                "issue01.pdf",
                "docs/guide.pdf",
                "item_files.sqlite",
                "meta/index.sqlite",
                "cover.jpg",
            ]
        );

        let no_torrents_or_sqlite = FileFilter::compile(["!*.torrent", "!*.sqlite"]).unwrap();
        let without_both = no_torrents_or_sqlite.apply(make_files()).unwrap();
        assert_eq!(
            file_names(&without_both),
            vec!["issue01.pdf", "docs/guide.pdf", "cover.jpg"]
        );
    }

    #[test]
    fn combined_positive_and_negative_is_order_independent() {
        let make_files = || XmlFiles {
            files: vec![
                xml_file("issue01.cbz"),
                xml_file("scans/issue02.cbz"),
                xml_file("issue01_thumb.cbz"),
                xml_file("thumbs/preview.cbz"),
                xml_file("cover.jpg"),
                xml_file("item.torrent"),
            ],
        };

        let positive_first = FileFilter::compile(["*.cbz", "!*thumb*"]).unwrap();
        let negative_first = FileFilter::compile(["!*thumb*", "*.cbz"]).unwrap();

        let result_a = positive_first.apply(make_files()).unwrap();
        let result_b = negative_first.apply(make_files()).unwrap();

        assert_eq!(
            file_names(&result_a),
            vec!["issue01.cbz", "scans/issue02.cbz"]
        );
        assert_eq!(
            file_names(&result_b),
            vec!["issue01.cbz", "scans/issue02.cbz"]
        );
    }

    #[test]
    fn nested_paths_and_leading_slash_handling() {
        let any_pdf = FileFilter::compile(["*.pdf"]).unwrap();
        assert!(any_pdf.matches("doc.pdf"));
        assert!(any_pdf.matches("dir/doc.pdf"));
        assert!(any_pdf.matches("a/b/doc.pdf"));
        assert!(any_pdf.matches("/dir/doc.pdf"));
        assert!(any_pdf.matches("other/doc.pdf"));

        let dir_pdf = FileFilter::compile(["dir/*.pdf"]).unwrap();
        assert!(dir_pdf.matches("dir/doc.pdf"));
        assert!(dir_pdf.matches("/dir/doc.pdf"));
        assert!(dir_pdf.matches("///dir/doc.pdf"));
        assert!(!dir_pdf.matches("a/b/doc.pdf"));
        assert!(!dir_pdf.matches("other/doc.pdf"));
        assert!(!dir_pdf.matches("doc.pdf"));

        let exclude_dir_pdf = FileFilter::compile(["!dir/*.pdf"]).unwrap();
        assert!(!exclude_dir_pdf.matches("dir/doc.pdf"));
        assert!(!exclude_dir_pdf.matches("/dir/doc.pdf"));
        assert!(exclude_dir_pdf.matches("a/b/doc.pdf"));
        assert!(exclude_dir_pdf.matches("other/doc.pdf"));
        assert!(exclude_dir_pdf.matches("doc.pdf"));

        let files = XmlFiles {
            files: vec![
                xml_file("dir/doc.pdf"),
                xml_file("/dir/doc.pdf"),
                xml_file("a/b/doc.pdf"),
                xml_file("other/doc.pdf"),
                xml_file("doc.pdf"),
            ],
        };
        let filtered = dir_pdf.apply(files).unwrap();
        assert_eq!(file_names(&filtered), vec!["dir/doc.pdf", "/dir/doc.pdf"]);
    }

    #[test]
    fn glob_syntax_braces_question_char_class_and_escaped_bang() {
        let braces = FileFilter::compile(["*.{pdf,cbz}"]).unwrap();
        assert!(braces.matches("issue01.pdf"));
        assert!(braces.matches("nested/issue02.cbz"));
        assert!(!braces.matches("book.epub"));
        assert!(!braces.matches("issue01.pdf.bak"));

        let single_char = FileFilter::compile(["page_?.png"]).unwrap();
        assert!(single_char.matches("page_1.png"));
        assert!(single_char.matches("page_a.png"));
        assert!(!single_char.matches("page_10.png"));
        assert!(!single_char.matches("page_.png"));

        let char_class = FileFilter::compile(["issue_0[1-3].pdf"]).unwrap();
        assert!(char_class.matches("issue_01.pdf"));
        assert!(char_class.matches("issue_02.pdf"));
        assert!(char_class.matches("issue_03.pdf"));
        assert!(!char_class.matches("issue_00.pdf"));
        assert!(!char_class.matches("issue_04.pdf"));
        assert!(!char_class.matches("issue_0a.pdf"));

        let escaped_bang = FileFilter::compile([r"\!readme.txt"]).unwrap();
        assert!(escaped_bang.matches("!readme.txt"));
        assert!(escaped_bang.matches("/!readme.txt"));
        assert!(!escaped_bang.matches("readme.txt"));
        let files = XmlFiles {
            files: vec![xml_file("!readme.txt"), xml_file("readme.txt")],
        };
        let filtered = escaped_bang.apply(files).unwrap();
        assert_eq!(file_names(&filtered), vec!["!readme.txt"]);
    }

    #[test]
    fn apply_returns_no_matching_files_for_positive_negative_and_combined_misses() {
        let positive_miss = FileFilter::compile(["*.flac"]).unwrap();
        let err = positive_miss
            .apply(XmlFiles {
                files: vec![xml_file("issue01.pdf"), xml_file("cover.jpg")],
            })
            .unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "*.flac"
        ));
        assert_eq!(err.to_string(), "No files matched filter(s): *.flac");

        let negative_all = FileFilter::compile(["!*.pdf", "!*.jpg"]).unwrap();
        let err = negative_all
            .apply(XmlFiles {
                files: vec![xml_file("issue01.pdf"), xml_file("cover.jpg")],
            })
            .unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "!*.pdf, !*.jpg"
        ));
        assert_eq!(
            err.to_string(),
            "No files matched filter(s): !*.pdf, !*.jpg"
        );

        let combined_miss = FileFilter::compile(["*.cbz", "!*thumb*"]).unwrap();
        let err = combined_miss
            .apply(XmlFiles {
                files: vec![
                    xml_file("cover.jpg"),
                    xml_file("issue01_thumb.cbz"),
                    xml_file("thumbs/issue02.cbz"),
                ],
            })
            .unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "*.cbz, !*thumb*"
        ));
        assert_eq!(
            err.to_string(),
            "No files matched filter(s): *.cbz, !*thumb*"
        );

        let active_on_empty = FileFilter::compile(["*.pdf"]).unwrap();
        let err = active_on_empty
            .apply(XmlFiles { files: Vec::new() })
            .unwrap_err();
        assert!(matches!(
            err,
            IaGetError::NoMatchingFiles { ref patterns } if patterns == "*.pdf"
        ));
    }

    #[test]
    fn rejects_unclosed_brackets_and_mixed_valid_and_invalid_filter_lists() {
        let cases: &[(&[&str], &str)] = &[
            (&["[unclosed"], "[unclosed"),
            (&["![unclosed"], "![unclosed"),
            (&["*.pdf", "[unclosed", "!*.torrent"], "[unclosed"),
            (&["*.pdf", "![unclosed"], "![unclosed"),
            (&["*.pdf", ""], ""),
            (&["*.pdf", "!"], "!"),
        ];

        for (patterns, expected_bad) in cases {
            let err = FileFilter::compile(*patterns).unwrap_err();
            assert!(
                matches!(
                    err,
                    IaGetError::InvalidFilter { ref pattern, .. } if pattern == expected_bad
                ),
                "patterns {patterns:?}: unexpected error {err:?}"
            );
            assert!(
                err.to_string()
                    .contains(&format!("Invalid filter pattern '{expected_bad}':")),
                "patterns {patterns:?}: unexpected display {err}"
            );
        }
    }
}
