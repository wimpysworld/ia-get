# AGENTS.md

Rust CLI for downloading files from archive.org with resume support, integrity checks, and directory preservation.

## Setup

```shell
nix develop              # Enter dev shell (preferred)
cargo build              # Build debug binary
cargo build --release    # Optimised build (stripped, LTO)
```

## Testing

```shell
cargo test               # Unit tests
cargo test --test list -- --ignored  # End-to-end --list against live archive.org (network)
cargo clippy             # Linting
cargo fmt --check        # Format check
```

Manual test URLs:
- `ia-get https://archive.org/details/deftributetozzap64`
- `ia-get https://archive.org/details/zzapp_64_issue_001_600dpi`

## Code Style

- Run `cargo fmt` and `cargo clippy` before committing
- Explicit error handling with `Result<T, E>`
- Use `thiserror` for custom error types
- Prefer idiomatic Rust patterns

## Architecture

```
src/
├── main.rs              # CLI entry point (clap), orchestration and the HTTP client (owns USER_AGENT)
├── lib.rs               # Library exports
├── plan.rs              # Download plan: file selection (whole item vs single file), output-dir prefixing, URL building, collision detection, structured warnings
├── file_filter.rs       # --include/--exclude glob matching (FileFilter, glob_match)
├── archive_metadata.rs  # _files.xml fetch/parse/persist + the archive.org URL contract (parse_archive_url/get_xml_url/encode)
├── check.rs             # --check: verify a directory against the metadata (presence, size, mtime, extras, .part handling, optional md5)
├── cookie.rs            # Cookie header from raw string or Netscape cookies.txt, and applying it to requests
├── display.rs           # Terminal output (spinner, progress bars, banners, status lines, size/duration formatting)
├── filename.rs          # Filename sanitization for cross-platform filesystems
├── fs.rs                # Filesystem write-safety: refuse to write through pre-planted symlinks; free-space lookup (fs2)
├── error.rs             # Custom error types (thiserror)
├── verbose.rs           # Opt-in --verbose diagnostic logging, gated to stderr
├── test_support.rs      # Shared test helpers (scripted local HTTP mock server, TempDir, download fixtures)
└── downloader/
    ├── mod.rs           # Batch orchestration: DownloadTask, .part lifecycle, per-file pipeline
    ├── stream.rs        # Streaming HTTP body to file, Range/resume, retry decisions
    ├── retry.rs         # Exponential backoff + jitter, RetryTracker, Retry-After
    ├── rate.rs          # --limit-rate throughput pacing (RateLimiter) and rate parsing
    ├── verify.rs        # Size + MD5 verification, ExistingFileStatus
    ├── signal.rs        # Ctrl+C handler (graceful stop, then hard exit)
    └── mtime.rs         # Last-Modified / <mtime> parsing and filetime sync

tests/
└── list.rs              # End-to-end --list against live archive.org (ignored by default)
```

## Dependencies

- Keep minimal and well-justified
- Prefer crates with good Nix support
- Update `Cargo.lock` when adding dependencies
- TLS via `rustls` only (no openssl)

## Platform Support

Must build on: `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin`, `x86_64-pc-windows-msvc`

Use `nix build` to verify cross-platform compatibility.

## Commit Guidelines

- Update README.md for new features or usage changes
- Include examples in help text
- Document Internet Archive API specifics or limitations
