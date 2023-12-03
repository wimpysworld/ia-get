# ia-get

Downloads files from the Internet Archive.

This program is an experiment and has been entirely written using ChatGPT.
It is probably not suitable for real use, yet.

## Build

```shell
cargo Build
```

## Test

Start a download:

```shell
target/debug/ia-get https://archive.org/download/Neo-GeoPocketColorRomCollectionByGhostware/Neo-GeoPocketColorRomCollectionByGhostware_files.xml
target/debug/ia-get https://archive.org/download/deftributetozzap64/deftributetozzap64_files.xml
```

Profile it with `htop`:

```shell
top -p $(pidof ia-get)
```
