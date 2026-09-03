//! Name filtering for the download plan: the `--include`/`--exclude`
//! glob patterns, matched against the archive's original file names
//! (before sanitization).

/// Reports whether `name` matches the glob `pattern`.
///
/// `*` matches any run of characters — including none, and across `/`
/// separators, so `*.pdf` matches `scan/01.pdf`; `?` matches exactly one
/// character; every other character matches literally. Matching is
/// case-sensitive: it runs against the archive's original names, not
/// the local paths (which may be sanitized and case-folded on
/// case-insensitive filesystems).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();

    let (mut i, mut j) = (0, 0);
    // The last committed `*` (pattern position) and the name position it
    // had consumed up to; backtracking makes the star consume one more
    // character of the name.
    let (mut star_i, mut star_j) = (usize::MAX, 0);

    while j < name.len() {
        if i < pattern.len() && (pattern[i] == '?' || pattern[i] == name[j]) {
            i += 1;
            j += 1;
        } else if i < pattern.len() && pattern[i] == '*' {
            star_i = i;
            star_j = j;
            i += 1;
        } else if star_i != usize::MAX {
            star_j += 1;
            j = star_j;
            i = star_i + 1;
        } else {
            return false;
        }
    }

    // Only trailing stars may stay unmatched at the end
    while i < pattern.len() && pattern[i] == '*' {
        i += 1;
    }
    i == pattern.len()
}

/// The `--include`/`--exclude` patterns of one run. An empty filter (no
/// patterns at all) passes every name unchanged.
#[derive(Debug, Default)]
pub struct FileFilter {
    includes: Vec<String>,
    excludes: Vec<String>,
}

impl FileFilter {
    pub fn new(includes: Vec<String>, excludes: Vec<String>) -> Self {
        Self { includes, excludes }
    }

    /// True when no pattern was given: nothing to filter, every name passes
    pub fn is_empty(&self) -> bool {
        self.includes.is_empty() && self.excludes.is_empty()
    }

    /// Whether `name` survives the filter: it must match at least one
    /// `--include` pattern when any were given, and no `--exclude` pattern.
    pub fn matches(&self, name: &str) -> bool {
        let included = self.includes.is_empty()
            || self
                .includes
                .iter()
                .any(|pattern| glob_match(pattern, name));
        included
            && !self
                .excludes
                .iter()
                .any(|pattern| glob_match(pattern, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_match_literal_names() {
        assert!(glob_match("scan.jpg", "scan.jpg"));
        assert!(!glob_match("scan.jpg", "scan.jpeg"));
        assert!(!glob_match("scan.jpg", "x/scan.jpg"));
        // Matching is case-sensitive
        assert!(!glob_match("SCAN.JPG", "scan.jpg"));
        // A '?' in the name is an ordinary character for the wildcard
        assert!(glob_match("a?b.txt", "a?b.txt"));
        assert!(glob_match("a?b.txt", "axb.txt"));
        assert!(!glob_match("aXb.txt", "a?b.txt"));
    }

    #[test]
    fn glob_match_star_crosses_slashes() {
        assert!(glob_match("*.pdf", "scan/01.pdf"));
        assert!(glob_match("*.pdf", "video/parts/02.pdf"));
        assert!(glob_match("*.pdf", "top.pdf"));
        // But the literal characters still anchor the rest of the name
        assert!(!glob_match("*.pdf", "scan/01.jpg"));
    }

    #[test]
    fn glob_match_star_prefix_and_suffix() {
        assert!(glob_match("video/*", "video/a.mp4"));
        assert!(glob_match("video/*", "video/a/b.mp4"));
        assert!(!glob_match("video/*", "audio/a.mp4"));
        // A trailing star swallows the rest, including nothing
        assert!(glob_match("video/*", "video/"));
        assert!(glob_match("*part*", "season_1/part_2.mp4"));
        assert!(glob_match("*.part*", "a.part.b"));
        // A star in the middle spans across separators too
        assert!(glob_match("s*.mp4", "season/1/2.mp4"));
    }

    #[test]
    fn glob_match_question_mark_is_one_character() {
        assert!(glob_match("disk?.img", "disk1.img"));
        assert!(glob_match("disk?.img", "diskA.img"));
        assert!(!glob_match("disk?.img", "disk10.img"));
        // '?' is one character, and '/' is just a character in a name
        assert!(glob_match("disk?.img", "disk/.img"));
        // An empty name matches nothing but a pure star
        assert!(!glob_match("?", ""));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn file_filter_passes_everything_when_empty() {
        let filter = FileFilter::new(vec![], vec![]);
        assert!(filter.is_empty());
        assert!(filter.matches("anything/else.bin"));
    }

    #[test]
    fn file_filter_include_requires_a_match() {
        let filter = FileFilter::new(vec!["*.pdf".to_string()], vec![]);
        assert!(!filter.is_empty());
        assert!(filter.matches("scan/01.pdf"));
        assert!(!filter.matches("cover.jpg"));
    }

    #[test]
    fn file_filter_include_is_or_across_patterns() {
        let filter = FileFilter::new(vec!["*.pdf".to_string(), "cover.jpg".to_string()], vec![]);
        assert!(filter.matches("a/b.pdf"));
        assert!(filter.matches("cover.jpg"));
        assert!(!filter.matches("notes.txt"));
    }

    #[test]
    fn file_filter_exclude_removes_matches() {
        let filter = FileFilter::new(vec![], vec!["*.tmp".to_string(), "video/*".to_string()]);
        assert!(filter.matches("scan/01.pdf"));
        assert!(!filter.matches("work.tmp"));
        assert!(!filter.matches("video/720p.mp4"));
    }

    #[test]
    fn file_filter_include_beats_exclude() {
        // A name matching both an include and an exclude is excluded:
        // the include only narrows the candidates, it does not veto
        // the exclude.
        let filter = FileFilter::new(vec!["*.pdf".to_string()], vec!["*draft*".to_string()]);
        assert!(filter.matches("scan/01.pdf"));
        assert!(!filter.matches("draft/01.pdf"));
        assert!(!filter.matches("scan/01.jpg"));
    }
}
