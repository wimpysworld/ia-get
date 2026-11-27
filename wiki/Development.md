# Development

This guide covers building, testing, linting, releasing, and maintaining `ia-get`.

## Prerequisites and development shell

Enter the Nix development shell with `nix develop`, or run `direnv allow` if you use direnv.

The flake development shell provides:

- Rust toolchain (`rustc`, `cargo`, `rustfmt`, `clippy`, `rust-analyzer`)
- `just`, `actionlint`, `cargo-audit`, `cargo-edit`, `cargo-watch`
- `cargo-zigbuild`, `goreleaser`, `zig`, `gh`
- `tailor` (on platforms with a Tailor package), `miniserve`, `ntrecord`, `fira-code`, and `ffmpeg-headless`

Plain `cargo` commands also work outside Nix if you have a local Rust toolchain installed.

## Common `just` recipes

| Command | Action |
| --- | --- |
| `just build` | Runs `cargo build --workspace --locked`, then runs `just/project/build.sh` with `BUILD_VERSION` from `git describe`. |
| `just test` | Runs `cargo test --workspace --locked`. |
| `just lint` | Runs `actionlint`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo audit --file Cargo.lock`. |
| `just lint correct` | Formats code with `cargo fmt --all`, then runs the full lint checks. |
| `just check` | Runs `fmt-check`, `lint`, and `test`. |
| `just release-build` | Builds the stripped, size-optimised LTO binary with `cargo build --release`. |
| `just run -- <args>` | Runs `ia-get` via `cargo run` with the given CLI arguments. |
| `just pages` | Serves `pages/` locally with `miniserve` at `http://127.0.0.1:18473`. |
| `just record` | Builds the release binary and records `pages/demo.webp` and `pages/demo-still.png` from `ia-get.tape` with `ntrecord` and `ffmpeg`. |
| `just update` | Updates flake inputs (`nix flake update`) and Cargo dependencies (`cargo update`). |
| `nix build` | Builds the Nix package from `nix/package.nix` to verify Nix packaging. |

## Manual integration testing

Use these archive.org items to test live downloads during development:

```shell
ia-get https://archive.org/details/deftributetozzap64
ia-get https://archive.org/details/zzapp_64_issue_001_600dpi
```

## Repository layout

```text
src/
├── main.rs                  # CLI entry point (clap) and run orchestration
├── lib.rs                   # Library module exports
├── downloader/
│   ├── mod.rs               # Core download loop, range resume, rate limiting, and batch orchestration
│   ├── scheduler.rs         # Elastic 4-permit concurrency funnel and reactive backpressure
│   ├── retry.rs             # Retry policy, backoff, Retry-After parsing, and shutdown signal
│   ├── proxy.rs             # Proxy configuration, validation, and redaction
│   ├── redirect.rs          # Manual redirect handling and per-hop cookie scoping
│   ├── fs_safety.rs         # Path containment, symlink-safe I/O, and atomic .part promotion
│   └── collision.rs         # Pre-write path collision detection and filesystem probing
├── ui/
│   ├── mod.rs               # Inline Ratatui TTY driver, cooked-mode status spinner, and non-TTY plain output
│   ├── palette.rs           # Archival colour tokens, Unicode/ASCII glyph sets, and inline style helpers
│   ├── state.rs             # Terminal-independent UI state, critically damped easing, and sparkline ring buffer
│   └── render.rs            # Pure frame renderer, responsive breakpoints, completion lines, and post-run summary
├── archive_metadata.rs      # XML metadata parsing for archive.org
├── cookie.rs                # Raw Cookie header and Netscape cookies.txt parsing and scoping
├── disk_space.rs            # Destination volume free-space estimation and checks
├── filter.rs                # Glob include and exclude pattern matching
├── verify.rs                # Read-only --check local directory verification
├── utils.rs                 # URL parsing, filename sanitisation, verbose logger, and formatting helpers
├── error.rs                 # Custom IaGetError types (thiserror)
├── constants.rs             # Timeouts, retry limits, and redirect limits
└── test_support.rs          # Shared unit test helpers and mock HTTP servers
tests/
└── cli_smoke.rs             # CLI integration smoke tests
pages/                   # Static project website (ia-get.wimpys.world)
wiki/                    # Source Markdown files synced to the GitHub Wiki
nix/                     # Nix package definition and development shell modules
just/                    # Tailor-managed just recipes and just/project/build.sh
```

## Dependencies and platform support

- Keep crate dependencies minimal, well-justified, and Nix-friendly.
- Update `Cargo.lock` whenever dependencies change.
- Use `rustls` for TLS (`reqwest` with `default-features = false` and `rustls-tls`) with no OpenSSL dependency.
- Supported build targets are `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin`, and Windows (`x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` in CI).

## Releases and Tailor maintenance

1. Update `version` in `Cargo.toml` to the bare `x.y.z` release number and commit on a clean working tree.
2. Run `just release x.y.z`. The Tailor-managed recipe checks that the working tree is clean and creates an annotated tag `v<x.y.z>`.
3. Push the tag with `git push origin v<x.y.z>`. This triggers `.github/workflows/builder.yml`, which verifies `Cargo.toml` against the tag version in GoReleaser, builds and publishes release artefacts, and updates `nix/version` on the `latest` tag.

Edit wiki documentation inside the `wiki/` directory of the main repository.
Pushes to `main` publish changes to the GitHub Wiki through `.github/workflows/tailor-wiki.yml`.
Do not edit `wiki/.tailor-wiki-base`.

## Contributing

Read [CONTRIBUTING.md](https://github.com/wimpysworld/ia-get/blob/main/CONTRIBUTING.md) before opening a pull request.
Contributions must follow Conventional Commits, meet the project Code of Conduct, report security issues privately, and keep a responsible person accountable for any AI-assisted work.
