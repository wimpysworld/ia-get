# ia-get wiki

`ia-get` is a command-line downloader for [archive.org](https://archive.org) written in Rust.
It downloads full items or single files from Internet Archive metadata, resumes partial transfers, verifies MD5 hashes, and keeps writes inside the target directory.

## Guides

| Page | Covers |
| --- | --- |
| [Usage](Usage) | URL formats, CLI options, glob filtering, `--check` verification, bandwidth limits, and verbose logging. |
| [Authentication and Networking](Authentication-and-Networking) | Cookies, redirect handling, proxy configuration, retries, `Retry-After` limits, and polite parallel downloads. |
| [Reliability and Safety](Reliability-and-Safety) | `.part` staging, HTTP range resume, disk-space checks, filename sanitisation, path collision validation, and exit codes. |
| [Development](Development) | Nix development shell, `just` recipes, repository layout, platform targets, releases, and contributing. |

## Project links

- [Project website](https://ia-get.wimpys.world)
- [GitHub repository](https://github.com/wimpysworld/ia-get)
