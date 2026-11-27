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
├── main.rs                  # CLI entry point (clap)
├── lib.rs                   # Library exports
├── downloader/
│   ├── mod.rs               # Core download loop, range resume, rate limiting, batch orchestration
│   ├── scheduler.rs         # Elastic 4-permit concurrency funnel and reactive backpressure
│   ├── retry.rs             # Retry policy, backoff, Retry-After parsing, shutdown signal
│   ├── proxy.rs             # Proxy configuration, validation, and redaction
│   ├── redirect.rs          # Manual redirect handling and per-hop cookie scoping
│   ├── fs_safety.rs         # Path containment, symlink-safe I/O, atomic .part promotion
│   └── collision.rs         # Pre-write path collision detection and filesystem probing
├── ui/
│   ├── mod.rs               # Inline Ratatui TTY driver, cooked-mode status spinner, non-TTY plain output
│   ├── palette.rs           # Archival colour tokens, Unicode/ASCII glyph sets, inline style helpers
│   ├── state.rs             # Terminal-independent UI state, critically damped easing, sparkline ring buffer
│   ├── render.rs            # Pure frame renderer, responsive breakpoints, Archival Receipt, adaptive ledger
│   ├── list.rs              # Interactive --list Manifest Card, flat-grouped micro-bars, large-archive breakdown
│   └── check.rs             # Interactive --check Verification Card, status rows, and failure ledger
├── archive_metadata.rs      # XML parsing for archive.org
├── cookie.rs                # Netscape cookies.txt and raw Cookie header scoping
├── disk_space.rs            # Free-space estimation and pre-download checks
├── filter.rs                # Include/exclude glob filtering
├── verify.rs                # Offline MD5 verification (--check)
├── utils.rs                 # Helpers (filename sanitisation, formatting, verbose logging)
├── error.rs                 # Custom error types
├── constants.rs             # Timeouts, retries, etc.
└── test_support.rs          # Shared unit test helpers and mock HTTP server
tests/
└── cli_smoke.rs             # CLI integration smoke tests
```

## Dependencies

- Keep minimal and well-justified
- Prefer crates with good Nix support
- Update `Cargo.lock` when adding dependencies
- TLS via `rustls` only (no openssl)

## Platform Support

Must build on: `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin`

Use `nix build` to verify cross-platform compatibility.

## Commit Guidelines

- Update README.md for new features or usage changes
- Include examples in help text
- Document Internet Archive API specifics or limitations
