# ia-get

Download files from the Internet Archive.

## Usage

Simply pass the URL of an Archive.org details page you wish to download and `ia-get` will automatically get the XML metadata and download all the files to the current working directory.

```shell
ia-get https://archive.org/details/<identifier>
```

# A.I. driven development

This program is an experiment and has been (*mostly*) written using A.I 🤖🧠
When I started this project I had no experience with Rust and was curious to see if I could use A.I tools to help write a program in a language I do not know.
The [initial version of the program](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was created using ChatGPT-4.
I [discussed that process in Episode 16 of Linux Matters](https://linuxmatters.sh/16/).
I then used [Unfold.ai](https://unfoldai.io/) to refine and improve the code, along with some refactoring from my own brain based on the Rust I picked up along the way.

## Build

```shell
cargo Build
```

## Test

Start a download:

```shell
target/debug/ia-get https://archive.org/details/2020_01_06_fbn
target/debug/ia-get https://archive.org/details/Neo-GeoPocketColorRomCollectionByGhostware
target/debug/ia-get https://archive.org/details/deftributetozzap64
```
