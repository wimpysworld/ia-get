use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ia-get"))
        .args(args)
        .output()
        .expect("run ia-get")
}

#[test]
fn help_exits_zero_and_documents_key_flags() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--list",
        "--check",
        "--cookies",
        "--output-dir",
        "--filter",
        "--limit-rate",
        "--verbose",
    ] {
        assert!(stdout.contains(flag), "help missing {flag}");
    }
}

#[test]
fn conflicting_list_and_check_exits_non_zero() {
    let out = run(&["--list", "--check", "https://archive.org/details/item"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--list") || stderr.contains("--check"));
}

#[test]
fn invalid_limit_rate_exits_non_zero_before_network() {
    let out = run(&["--limit-rate", "0", "https://archive.org/details/item"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Invalid rate limit '0'"));
}

#[test]
fn check_missing_output_dir_exits_non_zero_without_creating_dir() {
    let sandbox = tempfile::tempdir().expect("create smoke sandbox");
    let missing = sandbox.path().join("never/created");

    let out = run(&[
        "--check",
        "-o",
        missing.to_str().unwrap(),
        "https://archive.org/details/item",
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!missing.exists() && !sandbox.path().join("never").exists());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains('\x1b'),
        "non-TTY stderr must emit zero ANSI escapes: {stderr:?}"
    );
    assert!(
        stderr.contains("Error: "),
        "non-TTY fatal errors must use plain 'Error: ' prefix: {stderr:?}"
    );
    assert!(
        !stderr.contains("FileSystem("),
        "main() must not emit raw Rust Debug error dumps: {stderr:?}"
    );
}

#[test]
fn invalid_url_non_tty_with_no_color_and_dumb_term_exits_one_without_ansi_or_debug_dump() {
    let out = Command::new(env!("CARGO_BIN_EXE_ia-get"))
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["https://example.com/not-archive"])
        .output()
        .expect("run ia-get");

    assert_eq!(out.status.code(), Some(1));
    assert!(
        !out.stdout.contains(&0x1b),
        "stdout must emit zero ANSI bytes under NO_COLOR/non-TTY"
    );
    assert!(
        !out.stderr.contains(&0x1b),
        "stderr must emit zero ANSI bytes under NO_COLOR/non-TTY"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Invalid archive.org URL"));
    assert!(!stderr.contains("UrlFormat("));
}
