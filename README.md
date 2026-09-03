<h1 align="center">
  <img src="assets/ia-get.png" width="256" height="256" alt="ia-get">
  <br />
  ia-get
</h1>

<p align="center"><b>File downloader for archive.org</b></p>
<p align="center">
<img alt="GitHub all releases" src="https://img.shields.io/github/downloads/wimpysworld/ia-get/total?logo=github&label=Downloads">
</p>

<p align="center">Made with 💝 by 🤖</p>

# Usage 📖

Simply pass the URL of an [archive.org](https://archive.org) details page you want to download and `ia-get` will automatically get the XML metadata and download all files to the current working directory.

```shell
ia-get https://archive.org/details/<identifier>
```

To download a single file, pass a download URL that names it. The path is percent-decoded and matched against the archive's metadata; its parent directories are recreated locally. If the item does not contain the file, `ia-get` fails without downloading anything.

```shell
ia-get https://archive.org/download/<identifier>/<path/to/file.ext>
```

Narrow a whole-item download with `--include` and `--exclude` — repeatable globs matched against the archive's original file names, before sanitization. `*` matches any run of characters, across `/` separators, and `?` matches one character. With no `--include`, every file is a candidate; the `--exclude` patterns then remove from it. A run whose filters keep nothing fails without downloading anything.

```shell
ia-get --include "*.pdf" --exclude "*draft*" https://archive.org/details/<identifier>
```

Write the files somewhere else than the current directory with `-o`/`--output-dir`; the directory (and any missing parents) is created if needed. In a whole-item download the saved `<id>_files.xml` lands in the same directory as the files.

```shell
ia-get -o ia-get-<identifier> https://archive.org/details/<identifier>
```

Pass cookies for archive.org items that require a logged-in session.
`--cookies`/`-b` accepts either a raw Cookie header string or a Netscape `cookies.txt` file exported from your browser.

```shell
ia-get --cookies cookies.txt https://archive.org/details/<identifier>
ia-get -b 'logged-in-user=...; logged-in-sig=...' https://archive.org/details/<identifier>
```

Preview the files first with `--list` or `-l`.
This lists the names and sizes reported by archive.org metadata without downloading anything.
If archive.org does not provide a size for an entry, `ia-get` reports it as `unknown` and excludes it from the total known size.
The archive's own `<id>_files.xml` entry is marked as `(metadata)`: it is saved locally as file #1 during a download rather than fetched as one of the archive's files, which is why a download reports one fewer file than `--list` shows.

```shell
ia-get --list https://archive.org/details/<identifier>
ia-get -l https://archive.org/details/<identifier>
```

Archive.org's edge servers occasionally return error pages, empty responses, or drop connections mid-transfer. `ia-get` is built to ride out those outages:

- Server errors (timeouts, connection resets, HTTP 5xx/429) are retried with exponential backoff — 5s base, doubling per attempt up to a 60s cap, with ±20% jitter — and a `Retry-After` header is honored when present.
- Error response bodies are never written into your files, and an empty or truncated response is retried rather than saved.
- Each file is downloaded to a `<name>.part` temporary file and renamed to its final name only after the size (when known) and MD5 hash have been verified. A failed verification triggers a re-download from scratch, up to three attempts per file.
- If a file ultimately fails, the remaining files in the archive still download; `ia-get` exits with a non-zero status and prints a list of the failures. Use `--stop-on-error` to abort at the first failure instead.
- When the batch completes, `ia-get` prints a closing summary line in the style of the `--check` report: `Σ downloaded N files: X ok, Y failed`.

```shell
ia-get --stop-on-error https://archive.org/details/<identifier>
```

## Session options 🧰

A few flags shape how a download behaves on your network and disk. None of them change what gets downloaded — only how.

**Disk-space pre-check.** Before any bytes cross the network, `ia-get` adds up the size of the files that still need downloading (files already present at their expected size are skipped, and a leftover `<name>.part` only counts its missing remainder). If the target volume clearly cannot hold them, the run stops immediately with a `Not enough disk space` error rather than failing mid-transfer. Files whose size the metadata does not report are not counted, so the check is a lower-bound guard, not a hard guarantee.

**`--limit-rate`.** Cap the throughput to be polite on a metered or shared connection. The value is bytes/second, with an optional `K`/`M`/`G` suffix (case-insensitive, `B` allowed):

```shell
ia-get --limit-rate 1M https://archive.org/details/<identifier>   # ~1 MiB/s
ia-get --limit-rate 512K https://archive.org/details/<identifier>
```

Unlimited when omitted. `--limit-rate 0` also disables the cap.

**`--proxy`.** Route requests through a proxy. A bare `host:port` is treated as an `http://` proxy. When the flag is omitted, `ia-get` falls back to the `HTTPS_PROXY` (or `https_proxy`) environment variable:

```shell
ia-get --proxy http://127.0.0.1:3128 https://archive.org/details/<identifier>
```

**`--verbose`.** Log diagnostics to `stderr` — the resolved proxy and rate limit, the free-space figures, each request URL and HTTP status code. The normal progress output still goes to `stdout`, so the two can be captured separately:

```shell
ia-get --verbose https://archive.org/details/<identifier> 2> ia-get.log
```

## Verifying a download 🔎

`--check` verifies that a directory holds what a download of the item would have produced, without downloading or writing anything. It fetches the same `_files.xml`, maps each entry to the local path it would take (sanitized names, under the `-o` directory), and compares with what is on disk:

```shell
ia-get --check https://archive.org/details/<identifier>
ia-get --check -o my-downloads https://archive.org/details/<identifier>
```

- By default every file is checked for **presence**, **size** and **last-modified time**, and the directory is scanned for **unexpected files**. `--include`/`--exclude` narrow the check the same way they narrow a download.
- A file that is listed but absent, or present at the wrong size, fails the run (non-zero exit).
- **Date** and **extra-file** findings are warnings by default: the downloader stores the server's `Last-Modified`, which can legitimately differ from the `_files.xml` `<mtime>`. Add `--strict` to make those fail too.
- `--md5` additionally hashes each file against the metadata's MD5 (off by default, as it is the slow check).
- `.part` files are understood: a `.part` whose final file is missing is reported as *incomplete* (not missing, not extra), and a `.part` next to a complete file is reported as a stale leftover.
- The archive's own `<id>_files.xml` entry is excluded from the size/date/hash comparison (its self-referencing metadata is unreliable) but is still recognized, so it is not flagged as an unexpected file.

A `--check` run that finds problems exits non-zero and prints a per-file report plus a summary; a clean run exits `0`.

## Why? 🤔💭

I wanted to download high-quality scans of [ZZap!64 magazine](https://en.wikipedia.org/wiki/Zzap!64) and some read-only memory from archive.org.
Archives of this type often include many large files, torrents are not always provided and when they are available they do not index all the available files in the archive.

Archive.org publishes XML documents for every page that indexes every file available.
So I co-authored `ia-get` to automate the download process.

### Features ✨

- 🔽 Reliably downloads files from the Internet Archive
- 🎯 Downloads a single file via a download URL, or narrows a whole-item download with `--include`/`--exclude` globs
- 📂 `-o`/`--output-dir` writes the files into a target directory (created if missing)
- 🌳 Preserves the archive's directory structure
- 🔄 Resumes interrupted downloads from `<name>.part` files
- 🔏 Verifies size and MD5 hash before installing a file
- 🔁 Retries transient server errors with exponential backoff, honoring `Retry-After`
- 🚦 Continues after a failed file, exiting non-zero; `--stop-on-error` aborts at the first failure
- 🕓 Preserves last-modified times (`Last-Modified` header, falling back to `_files.xml`'s `<mtime>`)
- 🌱 Safe to re-run: verified files are kept, stale ones are re-downloaded
- 🔑 Supports private items via `--cookies` (a raw header or a Netscape `cookies.txt`)
- 📄 `--list` previews the files and sizes without downloading
- 🔎 `--check` verifies a directory against the archive's metadata (size, mtime, extras; `--md5` for hashes, `--strict` for hard dates/extras)
- 📊 Saves the archive's own `<id>_files.xml` locally alongside the files
- 💾 Fails fast when the target volume clearly lacks the space for the planned download
- 🐌 `--limit-rate` caps the download throughput (e.g. `1M`, `512K`); unlimited by default
- 🌐 `--proxy` routes requests through a proxy, falling back to the `HTTPS_PROXY` env var
- 🔎 `--verbose` logs request URLs and HTTP status codes to stderr for diagnosis
- 📦️ Available for **Linux** 🐧 **macOS** 🍏 and **Windows** 🪟

### Sharing is caring 🤝

You can use `ia-get` to download files from archive.org, including all the metadata and the `.torrent` file, if there is one.
You can start seeding the torrent using a pristine copy of the archive, and a complete file set.

# Demo 🧑‍💻

<div align="center"><img alt="ia-get demo" src="assets/ia-get.gif" width="1024" /></div>

# Development 🏗️

The repository ships a [`justfile`](./justfile) for the common tasks. With [just](https://github.com/casey/just) installed, `just check` runs the format check, clippy (warnings treated as errors) and the test suite in one go:

```shell
just check        # fmt-check + clippy + test
just test         # cargo test
just lint         # cargo clippy --all-targets --all-features -- -D warnings
just fmt          # cargo fmt
just build        # cargo build
```

Without `just`, the equivalent cargo commands are:

```shell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

A release build optimises for size (LTO, stripped):

```shell
cargo build --release
```

## Manual Tests 🤞

I used these commands to test `ia-get` during development.

```shell
ia-get https://archive.org/details/deftributetozzap64
ia-get https://archive.org/details/zzapp_64_issue_001_600dpi
```

# A.I. Driven Development 🤖

This program is an experiment 🧪 In late 2023, it was initially co-authored using [Chatty Jeeps](https://ubuntu.social/@popey/111527182881051626).
When I started this project, I had no experience 👶 with [Rust](https://www.rust-lang.org/) and was curious to see if I could use AI tools to assist in developing a program in a language I do not know.

**As featured on [Linux Matters](https://linuxmatters.sh) podcast!** 🎙️ I am a presenter on Linux Matters and we discussed how the [initial version of the program](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was created using Chatty Jeeps (ChatGPT-4) in [Episode 16 - Blogging to the Fediverse](https://linuxmatters.sh/16/).

I discussed that process, its successes, and drawbacks. In a future episode, we will discuss the latest version of the project.

<div align="center">
  <a href="https://linuxmatters.sh" target="_blank"><img src="https://raw.githubusercontent.com/wimpysworld/nix-config/main/.github/screenshots/linuxmatters.png" alt="Linux Matters Podcast"/></a>
  <br />
  <em>Linux Matters Podcast</em>
</div>

Since that initial MVP, I used [Unfold.ai](https://unfoldai.io/) to add features and improve the code 🧑‍💻.
All commits from October 27, 2023, until the end of December 2023 that were AI co-authored have full details of the AI contribution in the commit messages.
Linux Matters listner [Daniel Dewberry](https://github.com/DanielDewberry) submitted a [*"peer review"* of ia-get](https://github.com/wimpysworld/ia-get/issues/7) in January 2024.
The project had little development activity until May 2025, when I incorporated the improvements Daniel had suggested.

I've picked up some Rust along the way, and some of the refactoring and redesign comes directly from my brain 🧠 and some assistance from GitHub CoPilot using Claude Sonnet 3.7 and Gemini Pro 2.5.
