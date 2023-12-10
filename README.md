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

## Why? 🤔💭

I wanted to download high-quality scans of [ZZap!64 magazine](https://en.wikipedia.org/wiki/Zzap!64) and some read-only memory from archive.org.
Archives of this type often include many large files, torrents are not always provided and when they are available they do not index all the available files in the archive.

Archive.org publishes XML documents for every page that indexes every file available.
So I co-authored `ia-get` to automate the download process.

### Features ✨

- 🔽 Reliably download files from the Internet Archive
- 🌳 Preserves the original directory structure
- 🔄 Automatically resumes partial or failed downloads
- 🔏 Hash checks to confirm file integrity
- 🌱 Can be run multiple times to update existing downloads
- 📊 Gets all the metadata for the archive
- 📦️ Available for **Linux** 🐧 **macOS** 🍏 and **Windows** 🪟

### Sharing is caring 🤝

You can use `ia-get` to download files from archive.org, including all the metadata and the `.torrent` file, if there is one.
You can the start seeding the torrent using a pristine copy of the archive, and a complete file set.

# A.I. Driven Development 🤖

This program is an experiment 🧪 and has been (*mostly*) written using [Chatty Jeeps](https://ubuntu.social/@popey/111527182881051626).
When I started this project I had no experience 👶 with [Rust](https://www.rust-lang.org/) and was curious to see if I could use AI tools to assist in developing a program in a language I do not know.

**As featured on [Linux Matters](https://linuxmatters.sh) podcast!** 🎙️ I am a presenter on Linux Matters and we discussed how the [initial version of the program](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was created using Chatty Jeeps (ChatGPT-4) in [Episode 16 - Blogging to the Fediverse](https://linuxmatters.sh/16/).

I discussed that process, and the successes and drawbacks. We will be discussing the latest version of the project in a future episode.

<div align="center">
  <a href="https://linuxmatters.sh" target="_blank"><img src="https://raw.githubusercontent.com/wimpysworld/nix-config/main/.github/screenshots/linuxmatters.png" alt="Linux Matters Podcast"/></a>
  <br />
  <em>Linux Matters Podcast</em>
</div>

Since that initial MVP, I've used [Unfold.ai](https://unfoldai.io/) to add features and improve the code 🧑‍💻
All commits since Oct 27, 2023 that were co-authored by AI have full details of the AI contribution in the commit message.
I've picked up some Rust along the way and some refactoring came directly from my own brain 🧠

# Demo 🧑‍💻

<div align="center"><a href="https://asciinema.org/a/BO3vMD8rJawKhM1iZ3SZqVx0F?autoplay=1" target="_blank"><img src="https://asciinema.org/a/BO3vMD8rJawKhM1iZ3SZqVx0F.svg" /></a></div>


# Development 🏗️

Such as it is.

```shell
cargo build
```

## Tests 🤞

I used these commands to test `ia-get` during development.

```shell
ia-get https://archive.org/details/deftributetozzap64
ia-get https://archive.org/details/zzapp_64_issue_001_600dpi
```
