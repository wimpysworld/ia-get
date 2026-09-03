//! Last-modified time handling: parsing the server's `Last-Modified` header
//! and the `_files.xml` mtime, and applying them to files on disk.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, LAST_MODIFIED};

/// Parses the server-provided `Last-Modified` header into a `SystemTime`.
///
/// `reqwest::Response` has no typed accessor for this header, so the raw
/// HTTP-date is parsed with `httpdate`.
pub fn parse_last_modified(headers: &HeaderMap) -> Option<SystemTime> {
    headers
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| httpdate::parse_http_date(s).ok())
}

/// Converts the `<mtime>` value from `_files.xml` (unix seconds) into a
/// `SystemTime`, or `None` when absent or out of representable range.
pub(crate) fn mtime_from_xml(mtime: Option<u64>) -> Option<SystemTime> {
    mtime.and_then(|secs| UNIX_EPOCH.checked_add(Duration::from_secs(secs)))
}

/// Sets the file's last-modified time to `target` when the current time
/// differs at second granularity.
///
/// Returns whether the time was set. Setting it may fail (a locked file, a
/// read-only filesystem); the error is returned to the caller, which treats
/// the sync as best-effort — a failure warns and the batch continues.
pub fn sync_file_mtime(file_path: impl AsRef<Path>, target: SystemTime) -> std::io::Result<bool> {
    // A pre-1970 timestamp has no Unix representation; stamping the epoch
    // would fabricate a wrong mtime, so the time is left untouched instead.
    let Ok(target_duration) = target.duration_since(UNIX_EPOCH) else {
        return Ok(false);
    };
    let target_secs = target_duration.as_secs();

    let current_secs = fs::metadata(&file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    if current_secs == Some(target_secs) {
        return Ok(false);
    }

    // u64 seconds only overflow i64 for dates around the year 292 million
    // AD; nothing representable exists there, so leave the time as is.
    let Ok(target_secs) = i64::try_from(target_secs) else {
        return Ok(false);
    };

    let file_time = filetime::FileTime::from_unix_time(target_secs, 0);

    filetime::set_file_mtime(&file_path, file_time)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn mtime_from_xml_converts_and_rejects_overflow() {
        assert_eq!(mtime_from_xml(None), None);
        assert_eq!(
            mtime_from_xml(Some(1_735_965_174)),
            Some(UNIX_EPOCH + Duration::from_secs(1_735_965_174))
        );
        assert_eq!(mtime_from_xml(Some(u64::MAX)), None);
    }

    #[test]
    fn sync_file_mtime_skips_pre_epoch_times() {
        // A Last-Modified before 1970 has no Unix representation: the mtime
        // must be left untouched instead of being stamped to the epoch.
        let dir = TempDir::new("mtime_pre_epoch");
        let path = dir.join("f.txt");
        fs::write(&path, "x").unwrap();

        let pre_epoch = UNIX_EPOCH - Duration::from_secs(1);
        assert!(!sync_file_mtime(&path, pre_epoch).unwrap());

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(
            mtime.duration_since(UNIX_EPOCH).unwrap().abs_diff(now) < Duration::from_secs(60),
            "the mtime must stay at the write time"
        );
    }
}
