<h1 align="center">
  <img src="assets/ia-get.png" width="256" height="256" alt="ia-get">
  <br />
  ia-get
</h1>

<p align="center"><b>File downloader for archive.org</b></p>
<p align="center"><a href="https://ia-get.wimpys.world">Project website</a></p>
<p align="center">
<img alt="GitHub all releases" src="https://img.shields.io/github/downloads/wimpysworld/ia-get/total?logo=github&label=Downloads">
</p>

<p align="center">Made with 💝 by 🤖</p>

# Install 📦

| | Platform | Repository |
| :-: | --- | --- |
| <img src="https://apt.wimpys.world/logo.svg" width="32" height="32" alt=""> | Debian and Ubuntu | [apt.wimpys.world](https://apt.wimpys.world/#ia-get) |
| <img src="https://rpm.wimpys.world/logo.svg" width="32" height="32" alt=""> | Fedora and openSUSE | [rpm.wimpys.world](https://rpm.wimpys.world/#ia-get) |
| <img src="https://pkg.wimpys.world/logo.svg" width="32" height="32" alt=""> | Arch Linux | [pkg.wimpys.world](https://pkg.wimpys.world/#ia-get) |
| <img src="https://apk.wimpys.world/logo.svg" width="32" height="32" alt=""> | Alpine Linux | [apk.wimpys.world](https://apk.wimpys.world/#ia-get) |
| <img src="https://brew.wimpys.world/logo.svg" width="32" height="32" alt=""> | macOS (Homebrew) | [brew.wimpys.world](https://brew.wimpys.world/#ia-get) |
| <img src="https://scoop.wimpys.world/logo.svg" width="32" height="32" alt=""> | Windows (Scoop) | [scoop.wimpys.world](https://scoop.wimpys.world/#ia-get) |
| <img src="https://nix.wimpys.world/logo.svg" width="32" height="32" alt=""> | Nix and NixOS | [nix.wimpys.world](https://nix.wimpys.world/#ia-get) |

Each repository page gives the setup steps for installing and updating ia-get.

## Any platform

[bin](https://github.com/marcosnils/bin) installs and updates the release binary:

```shell
bin install github.com/wimpysworld/ia-get
```

# Usage 📖

`ia-get` accepts an [archive.org](https://archive.org) details URL to download every file in an item, or a direct download URL to download one file.
`ia-get` fetches the item's XML metadata and saves files to the current working directory.

```shell
ia-get https://archive.org/details/<identifier>
ia-get https://archive.org/download/<identifier>/<path>
```

For `/download/<identifier>/<path>` URLs, `ia-get` strips query strings and fragments, percent-decodes the path, and fetches `<identifier>_files.xml` to match the file's size and MD5 metadata.
If that file is absent from the XML metadata, `ia-get` exits with an error.
Otherwise, `ia-get` downloads only that file and keeps its relative path in the output directory.

`ia-get` downloads each file into `<file>.part`.
When a `.part` file exists, `ia-get` sends a range request and validates HTTP `200`, `206`, and `416` responses plus the `Content-Range` header.
If the server returns HTTP `200` or an invalid range response, `ia-get` restarts the download from byte 0.
`ia-get` verifies the MD5 hash when archive.org provides one, and renames the `.part` file to the final path atomically only after verification succeeds.

All downloads stay inside the target output directory (the current working directory by default).
`ia-get` refuses symlinks, paths outside that directory, and non-regular files such as directories or named pipes (FIFOs).
For cross-platform safety, `ia-get` sanitises invalid characters, dot segments (`.` and `..`), and trailing spaces or dots in path components.
It also renames reserved device names on Windows (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `CLOCK$`, `COM0`-`COM9`, `COM¹`-`COM³`, `LPT0`-`LPT9`, and `LPT¹`-`LPT³`).
This check ignores case and matches reserved names with or without extensions, even with trailing spaces or dots before an extension.

For both metadata fetches and file downloads, `ia-get` retries transient transport errors and HTTP `408`, `429`, and `5xx` responses up to 3 times.
Retries use exponential backoff with jitter, or honour `Retry-After` headers (delta-seconds or HTTP-date) capped at 60 seconds.
Other `4xx` errors fail immediately, and Ctrl+C interrupts retry waits promptly.

When a file fails to download or fails MD5 verification, `ia-get` prints a warning and continues with the remaining files.
At the end of a run, `ia-get` prints a summary with `ok`, `skipped`, and `failed` counts and lists any failed files.
If any file fails or Ctrl+C interrupts the run, `ia-get` exits with a non-zero status.

Save files to a chosen directory with `-o` or `--output-dir <DIR>`.
`ia-get` creates missing directories when a download starts, and defaults to the current working directory when you omit this option.
`ia-get` rejects empty paths, broken symlinks, and existing non-directory paths, and keeps every downloaded file inside that directory.
When you pass `--list`, `ia-get` does not create the output directory.

```shell
ia-get -o ./downloads https://archive.org/details/<identifier>
ia-get --output-dir /tmp/archive https://archive.org/details/<identifier>
```

Pass cookies for archive.org items that require a logged-in session.
`--cookies`/`-b` accepts either a raw `Cookie` header or a Netscape `cookies.txt` file exported from your browser.
Raw cookie strings attach only to HTTPS requests for `archive.org` and its subdomains.
For `cookies.txt` files, `ia-get` matches host, path (longest path first per RFC 6265), the `Secure` flag, and live expiry on every request and retry.
`ia-get` follows up to 10 redirects manually and rejects redirect loops and HTTPS-to-HTTP downgrades.
`ia-get` re-evaluates cookies on every hop so that credentials never leak to unrelated hosts or mismatched paths.

```shell
ia-get --cookies cookies.txt https://archive.org/details/<identifier>
ia-get -b 'logged-in-user=...; logged-in-sig=...' https://archive.org/details/<identifier>
```

Preview the files first with `--list` or `-l`.
For a details URL, `ia-get` prints the names and sizes from archive.org metadata and does not download files.
For a direct download URL, `ia-get` prints only the matched file and its size.
If archive.org does not provide a size for an entry, `ia-get` reports it as `unknown` and excludes it from the total known size.

```shell
ia-get --list https://archive.org/details/<identifier>
ia-get -l https://archive.org/details/<identifier>
ia-get --list https://archive.org/download/<identifier>/<path>
```

## Options

| Option | Description |
| --- | --- |
| `-o`, `--output-dir <DIR>` | Save downloaded files to `<DIR>` (defaults to the current working directory). |
| `-b`, `--cookies <COOKIES>` | Send a raw `Cookie` header or load cookies from a Netscape `cookies.txt` file. |
| `-l`, `--list` | List files and sizes from archive metadata XML and exit. |

## Why? 🤔💭

I wanted to download high-quality scans of [ZZap!64 magazine](https://en.wikipedia.org/wiki/Zzap!64) and some read-only memory from archive.org.
Archives of this type often include many large files, torrents are not always provided and when they are available they do not index all the available files in the archive.

Archive.org publishes XML documents for every page that indexes every file available.
So I co-authored `ia-get` to automate the download process.

### Features ✨

- 🎯 Downloads full items from `/details/<identifier>` URLs or single files from `/download/<identifier>/<path>` URLs
- 🔽 Retries transient errors on metadata and file downloads with exponential backoff, jitter, `Retry-After` support, and prompt Ctrl+C interruption
- 🌳 Preserves the original directory structure
- 📁 Saves downloads to a chosen directory with `-o`/`--output-dir <DIR>` and creates missing directories automatically
- 🛡️ Keeps downloads inside the target output directory, refuses symlinks and non-regular files, and sanitises unsafe paths and reserved device names on Windows
- 🔄 Stages downloads in `.part` files, resumes partial transfers, and validates HTTP range responses
- 🔏 Verifies MD5 hashes and renames `.part` files atomically only after verification succeeds
- 📋 Continues past individual file failures, prints a batch summary, and exits with a non-zero status on failed or interrupted runs
- 🍪 Scopes cookies per request and redirect hop, checks live expiry, and rejects redirect loops and HTTPS-to-HTTP downgrades
- 🌱 Can be run multiple times to update existing downloads
- 📊 Gets all the metadata for the archive
- 📦️ Available for **Linux** 🐧 **macOS** 🍏 and **Windows** 🪟

### Sharing is caring 🤝

You can use `ia-get` to download files from archive.org, including all the metadata and the `.torrent` file, if there is one.
You can the start seeding the torrent using a pristine copy of the archive, and a complete file set.

# Demo 🧑‍💻

<div align="center"><img alt="ia-get demo" src="assets/ia-get.gif" width="1024" /></div>

# Development 🏗️

Such as it is.

```shell
cargo build
```

## Unit Tests 🧪

You can run the built-in unit tests with:

```shell
cargo test
```

This will run tests that verify URL pattern validation and other core functionality.

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
