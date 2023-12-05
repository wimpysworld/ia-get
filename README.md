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

<ul style="list-style-type:none; padding-left:0;">
<li>🔽 Reliably download files from the Internet Archive</li>
<li>🌳 Preserves the original directory structure</li>
<li>🔄 Automatically resumes partial or failed downloads</li>
<li>🔏 Hash checks to confirm file integrity</li>
<li>🌱 Can be run multiple times to update existing downloads</li>
<li>📦️ Available for <b>Linux</b> 🐧 <b>macOS</b> 🍏 and <b>Windows</b> 🪟</li>
</ul>

# A.I. Driven Development 🤖

This program is an experiment 🧪 and has been (*mostly*) written using AI.
When I started this project I had no experience 👶 with [Rust](https://www.rust-lang.org/) and was curious to see if I could use AI tools to assist in developing a program in a language I do not know.
The [initial version of the program](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was created using ChatGPT-4.
I [discussed that process in Episode 16 of Linux Matters](https://linuxmatters.sh/16/).
Since that initial MVP, I've used [Unfold.ai](https://unfoldai.io/) to add features and improve the code 🧑‍💻
All commits since Oct 27, 2023 that were co-authored by AI have full details of the AI contribution in the commit message.
I've picked up some Rust along way, and some refactoring came directly from my own brain 🧠

## Build 🏗️

```shell
cargo build
```

### Tests 🤞

I used these commands to test `ia-get` during development.

```shell
ia-get https://archive.org/details/deftributetozzap64
ia-get https://archive.org/details/zzapp_64_issue_001_600dpi
```
