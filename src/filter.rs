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
    use crate::test_support::xml_file;

    fn xml_files(names: &[&str]) -> XmlFiles {
        XmlFiles {
            files: names
                .iter()
                .map(|name| xml_file(name, Some(128), None))
                .collect(),
        }
    }

    fn file_names(files: &XmlFiles) -> Vec<&str> {
        files
            .files
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    #[test]
    fn empty_filter_is_inactive_and_preserves_empty_or_non_empty_lists() {
        let filter = FileFilter::compile::<[&str; 0], _>([]).unwrap();
        assert!(filter.is_empty());
        assert!(filter.matches("anything.pdf"));

        let empty_result = filter.apply(xml_files(&[])).unwrap();
        assert!(empty_result.files.is_empty());

        let all = ["issue01.pdf", "scans/issue02.cbz", "/meta/item.torrent"];
        let preserved = filter.apply(xml_files(&all)).unwrap();
        assert_eq!(file_names(&preserved), all);
    }

    #[test]
    fn rejects_invalid_empty_bare_negation_and_malformed_glob_patterns() {
        let cases: &[(&[&str], &str)] = &[
            (&[""], ""),
            (&["!"], "!"),
            (&["!*.[pdf"], "!*.[pdf"),
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

    #[test]
    fn apply_supports_positive_negative_and_order_independent_combined_patterns() {
        let catalog = [
            "issue01.pdf",
            "docs/guide.pdf",
            "book.epub",
            "cover.jpg",
            "item.torrent",
            "meta/backup.torrent",
            "item_files.sqlite",
            "meta/index.sqlite",
        ];
        let comic_catalog = [
            "issue01.cbz",
            "scans/issue02.cbz",
            "issue01_thumb.cbz",
            "thumbs/preview.cbz",
            "cover.jpg",
            "item.torrent",
        ];
        let draft_catalog = [
            "issue01.pdf",
            "sub/dir/issue02.cbz",
            "/sub/dir/issue03.pdf",
            "sub/issue01-draft.pdf",
            "cover.jpg",
        ];

        let cases: &[(&[&str], &[&str], &[&str])] = &[
            (&["*.pdf"], &catalog, &["issue01.pdf", "docs/guide.pdf"]),
            (
                &["*.pdf", "*.epub"],
                &catalog,
                &["issue01.pdf", "docs/guide.pdf", "book.epub"],
            ),
            (
                &["!*.torrent"],
                &catalog,
                &[
                    "issue01.pdf",
                    "docs/guide.pdf",
                    "book.epub",
                    "cover.jpg",
                    "item_files.sqlite",
                    "meta/index.sqlite",
                ],
            ),
            (
                &["!*.torrent", "!*.sqlite"],
                &catalog,
                &["issue01.pdf", "docs/guide.pdf", "book.epub", "cover.jpg"],
            ),
            (
                &["*.cbz", "!*thumb*"],
                &comic_catalog,
                &["issue01.cbz", "scans/issue02.cbz"],
            ),
            (
                &["!*thumb*", "*.cbz"],
                &comic_catalog,
                &["issue01.cbz", "scans/issue02.cbz"],
            ),
            (
                &["*.{pdf,cbz}", "!*draft*"],
                &draft_catalog,
                &["issue01.pdf", "sub/dir/issue02.cbz", "/sub/dir/issue03.pdf"],
            ),
        ];

        for (patterns, inputs, expected) in cases {
            let filter = FileFilter::compile(*patterns).unwrap();
            assert!(!filter.is_empty(), "patterns {patterns:?} must be active");
            let result = filter.apply(xml_files(inputs)).unwrap();
            assert_eq!(file_names(&result), *expected, "patterns {patterns:?}");
        }
    }

    #[test]
    fn nested_paths_and_leading_slash_handling() {
        let cases: &[(&str, &[(&str, bool)])] = &[
            (
                "*.pdf",
                &[
                    ("doc.pdf", true),
                    ("dir/doc.pdf", true),
                    ("a/b/doc.pdf", true),
                    ("/dir/doc.pdf", true),
                    ("other/doc.pdf", true),
                ],
            ),
            (
                "dir/*.pdf",
                &[
                    ("dir/doc.pdf", true),
                    ("/dir/doc.pdf", true),
                    ("///dir/doc.pdf", true),
                    ("a/b/doc.pdf", false),
                    ("other/doc.pdf", false),
                    ("doc.pdf", false),
                ],
            ),
            (
                "!dir/*.pdf",
                &[
                    ("dir/doc.pdf", false),
                    ("/dir/doc.pdf", false),
                    ("a/b/doc.pdf", true),
                    ("other/doc.pdf", true),
                    ("doc.pdf", true),
                ],
            ),
        ];

        for (pattern, checks) in cases {
            let filter = FileFilter::compile([*pattern]).unwrap();
            for (path, expected) in *checks {
                assert_eq!(
                    filter.matches(path),
                    *expected,
                    "pattern={pattern:?}, path={path:?}"
                );
            }
        }

        let dir_pdf = FileFilter::compile(["dir/*.pdf"]).unwrap();
        let filtered = dir_pdf
            .apply(xml_files(&[
                "dir/doc.pdf",
                "/dir/doc.pdf",
                "a/b/doc.pdf",
                "other/doc.pdf",
                "doc.pdf",
            ]))
            .unwrap();
        assert_eq!(file_names(&filtered), vec!["dir/doc.pdf", "/dir/doc.pdf"]);
    }

    #[test]
    fn glob_syntax_braces_question_char_class_and_escaped_bang() {
        let cases: &[(&str, &[(&str, bool)])] = &[
            (
                "*.{pdf,cbz}",
                &[
                    ("issue01.pdf", true),
                    ("nested/issue02.cbz", true),
                    ("book.epub", false),
                    ("issue01.pdf.bak", false),
                ],
            ),
            (
                "page_?.png",
                &[
                    ("page_1.png", true),
                    ("page_a.png", true),
                    ("page_10.png", false),
                    ("page_.png", false),
                ],
            ),
            (
                "issue_0[1-3].pdf",
                &[
                    ("issue_01.pdf", true),
                    ("issue_02.pdf", true),
                    ("issue_03.pdf", true),
                    ("issue_00.pdf", false),
                    ("issue_04.pdf", false),
                    ("issue_0a.pdf", false),
                ],
            ),
            (
                r"\!readme.txt",
                &[
                    ("!readme.txt", true),
                    ("/!readme.txt", true),
                    ("readme.txt", false),
                ],
            ),
        ];

        for (pattern, checks) in cases {
            let filter = FileFilter::compile([*pattern]).unwrap();
            for (path, expected) in *checks {
                assert_eq!(
                    filter.matches(path),
                    *expected,
                    "pattern={pattern:?}, path={path:?}"
                );
            }
        }

        let escaped_bang = FileFilter::compile([r"\!readme.txt"]).unwrap();
        let filtered = escaped_bang
            .apply(xml_files(&["!readme.txt", "readme.txt"]))
            .unwrap();
        assert_eq!(file_names(&filtered), vec!["!readme.txt"]);
    }

    #[test]
    fn apply_returns_no_matching_files_for_positive_negative_and_combined_misses() {
        let cases: &[(&[&str], &[&str], &str)] = &[
            (&["*.flac"], &["issue01.pdf", "cover.jpg"], "*.flac"),
            (
                &["!*.pdf", "!*.jpg"],
                &["issue01.pdf", "cover.jpg"],
                "!*.pdf, !*.jpg",
            ),
            (
                &["*.cbz", "!*thumb*"],
                &["cover.jpg", "issue01_thumb.cbz", "thumbs/issue02.cbz"],
                "*.cbz, !*thumb*",
            ),
            (&["*.pdf"], &[], "*.pdf"),
        ];

        for (patterns, inputs, expected_patterns) in cases {
            let filter = FileFilter::compile(*patterns).unwrap();
            let err = filter.apply(xml_files(inputs)).unwrap_err();
            assert!(matches!(
                err,
                IaGetError::NoMatchingFiles { ref patterns } if patterns == expected_patterns
            ));
            assert_eq!(
                err.to_string(),
                format!("No files matched filter(s): {expected_patterns}")
            );
        }
    }
}
