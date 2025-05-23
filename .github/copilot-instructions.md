# GitHub Copilot Instructions for ia-get

## Project Overview
This is a Rust CLI tool for downloading files from the Internet Archive, built with Nix flakes for reproducible builds.

## Development Guidelines

### Rust Standards
- Follow standard Rust conventions and idiomatic patterns
- Use `cargo fmt` and `cargo clippy` for code formatting and linting
- Prefer explicit error handling with `Result<T, E>` types
- Use `anyhow` or `thiserror` for error handling consistency

### Nix Integration
- This project uses Nix flakes for development environment and packaging
- When suggesting build commands, prefer `nix develop` for entering the dev shell
- Build with `nix build` rather than direct cargo commands when possible
- Maintain compatibility with the supported systems: x86_64-linux, aarch64-darwin, x86_64-darwin, aarch64-linux

### Dependencies
- Keep dependencies minimal and well-justified
- Update Cargo.lock when adding new dependencies
- Prefer crates that are well-maintained and have good Nix support

### Code Structure
- Follow CLI best practices with clear subcommands and help text
- Use structured logging for better debugging
- Implement proper signal handling for long-running downloads
- Include comprehensive error messages for user-facing operations

### Testing
- Write unit tests for core functionality
- Include integration tests for CLI behavior
- Test cross-platform compatibility where relevant

### Documentation
- Update README.md for any new features or usage changes
- Include examples in help text and documentation
- Document any Internet Archive API specifics or limitations