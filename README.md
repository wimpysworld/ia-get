<h1 align="center">
  <img src="pages/icon.svg" width="256" height="256" alt="ia-get">
  <br />
  ia-get
</h1>

<p align="center"><b>Command-line downloader for archive.org: preview, filter, resume, verify, and keep folders.</b></p>

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

# Features

- **One URL, whole archive:** `ia-get` downloads every file with its original folders from an archive.org `/details/` link, or one file from a `/download/` link.
- **Fast and polite automatically:** `ia-get` downloads small and medium files in parallel with zero flags and slows down for large files. It backs off on rate limits to respect the Internet Archive's wishes and [automated access guidelines](https://archive.org/developers/bots.html).
- **Live terminal UI:** The inline display shows active parallel downloads, a throughput sparkline, mirror hosts, and overall progress. Calm warnings appear at the end, and your terminal scrollback stays intact.
- **Preview and cherry-pick:** Run `--list` to inspect every file and size before you download. Use `-f` glob patterns to include or exclude files.
- **Resume and verify:** Interrupted downloads pick up where they left off, and finished files pass automatic MD5 checks. Run `--check` to verify a local folder without changing a file.
- **Private items, speed caps, and proxies:** Pass browser cookies with `-b` for login-only items, or cap total speed with `--limit-rate`. `ia-get` follows system proxies automatically and skips private files when you do not pass cookies.
- **Native and safe everywhere:** `ia-get` is a fast standalone binary for Linux, macOS, and Windows. It checks free disk space first and keeps every write inside your target folder.

# Quick start

```shell
# Download an entire archive item with its original folders
ia-get https://archive.org/details/<identifier>

# Download a single file directly
ia-get https://archive.org/download/<identifier>/<path>

# Preview files and sizes without downloading anything
ia-get --list https://archive.org/details/<identifier>

# Save files into a chosen folder
ia-get -o ./downloads https://archive.org/details/<identifier>

# Download only PDFs and exclude *_text.pdf files
ia-get -f '*.pdf' -f '!*_text.pdf' https://archive.org/details/<identifier>

# Verify an existing local folder without downloading or changing files
ia-get --check -o ./downloads https://archive.org/details/<identifier>
```

Read the full guides on the wiki:

- [Usage](https://github.com/wimpysworld/ia-get/wiki/Usage) for all options, glob filtering, bandwidth limits, and verbose logging.
- [Authentication and Networking](https://github.com/wimpysworld/ia-get/wiki/Authentication-and-Networking) for cookies, redirect handling, proxies, retries, and polite parallel downloads.
- [Reliability and Safety](https://github.com/wimpysworld/ia-get/wiki/Reliability-and-Safety) for `.part` resume staging, disk-space checks, filename sanitisation, and collision validation.

# Demo 🧑‍💻

<div align="center"><img alt="ia-get demo" src="pages/demo.webp" width="872" height="536" /></div>

# Why? 🤔💭

I wanted to download high-quality scans of [ZZap!64 magazine](https://en.wikipedia.org/wiki/Zzap!64) and some read-only memory from archive.org.
Archives of this type often include many large files, torrents are not always provided and when they are available they do not index all the available files in the archive.

Archive.org publishes XML documents for every page that indexes every file available.
So I co-authored `ia-get` to automate the download process.

## Sharing is caring 🤝

You can use `ia-get` to download files from archive.org, including all the metadata and the `.torrent` file, if there is one.
You can then start seeding the torrent using a pristine copy of the archive, and a complete file set.
For multi-terabyte archives of multi-gigabyte files, use the item's `.torrent` file (BitTorrent with webseeds) instead of overloading archive.org HTTP datanodes.

# A.I. Driven Development 🤖

This program is an experiment 🧪 In late 2023, it was initially co-authored using [Chatty Jeeps](https://ubuntu.social/@popey/111527182881051626).
When I started this project, I had no experience 👶 with [Rust](https://www.rust-lang.org/) and was curious to see if I could use AI tools to assist in developing a program in a language I do not know.

**As featured on [Linux Matters](https://linuxmatters.sh) podcast!** 🎙️ I am a presenter on Linux Matters and we discussed how the [initial version of the program](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was created using Chatty Jeeps (ChatGPT-4) in [Episode 16 - Blogging to the Fediverse](https://linuxmatters.sh/16/).

I discussed that process, its successes, and drawbacks. In a future episode, we will discuss the latest version of the project.

<div align="center">
  <a href="https://linuxmatters.sh" target="_blank"><img src="https://raw.githubusercontent.com/wimpysworld/nix-config/main/assets/linuxmatters.png" alt="Linux Matters Podcast"/></a>
  <br />
  <em>Linux Matters Podcast</em>
</div>

Since that initial MVP, I used [Unfold.ai](https://unfoldai.io/) to add features and improve the code 🧑‍💻.
All commits from October 27, 2023, until the end of December 2023 that were AI co-authored have full details of the AI contribution in the commit messages.
Linux Matters listener [Daniel Dewberry](https://github.com/DanielDewberry) submitted a [*"peer review"* of ia-get](https://github.com/wimpysworld/ia-get/issues/7) in January 2024.
The project had little development activity until May 2025, when I incorporated the improvements Daniel had suggested.

I've picked up some Rust along the way, and some of the refactoring and redesign comes directly from my brain 🧠 and some assistance from GitHub CoPilot using Claude Sonnet 3.7 and Gemini Pro 2.5.

# Documentation and development

See the [Wiki](https://github.com/wimpysworld/ia-get/wiki) for detailed guides and command reference.
See the [Development](https://github.com/wimpysworld/ia-get/wiki/Development) page for building, testing, and contributing.
