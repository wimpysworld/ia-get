//! End-to-end check of the `--list` flag against the real archive.org.
//!
//! Ignored by default (it hits the network with live data); run it
//! explicitly:
//!
//! ```sh
//! cargo test --test list -- --ignored
//! ```

#[test]
#[ignore]
fn list_flag_reports_archive_contents() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ia-get"))
        .args(["--list", "https://archive.org/details/deftributetozzap64"])
        .output()
        .expect("failed to spawn ia-get");

    assert!(
        output.status.success(),
        "ia-get --list must succeed, got {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    // Output is piped, so `colored` stays plain: the glyphs and labels
    // below must be present verbatim. The "✔ Archive has ..." banner is
    // the spinner's final line, which indicatif only draws when stderr is
    // a TTY: it is absent from a piped run, so only the stdout rows are
    // asserted here.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("deftributetozzap64_files.xml"),
        "the archive's own metadata file must be listed:\n{stdout}"
    );
    assert!(
        stdout.contains("(metadata)"),
        "the metadata entry must carry its marker:\n{stdout}"
    );
    assert!(
        stdout.contains("Note: deftributetozzap64_files.xml is the archive's own metadata"),
        "the bridge note between list and plan counts is missing:\n{stdout}"
    );
}
