# ia-get

Download files from the Internet Archive.

## Synthetic development

This program is an experiment and has been (*mostly*) written using A.I 🤖🧠
When I started this project I had no experience with Rust and was curious to see if I could use A.I to help write a program in a language I do not know.
The [initial version of the program](https://github.com/wimpysworld/ia-get/tree/5f2b356e7d841f2756780e2a101cf8be4041a7f6) was created using ChatGPT-4.
I [discussed the process in Episode 16 of Linux Matters](https://linuxmatters.sh/16/).
I then used Unfold.ai to refine and improve the code, along with some refactoring from my own brain based on the little Rust I have picked up along the way.


## Build

```shell
cargo Build
```

## Test

Start a download:

```shell
target/debug/ia-get https://ia801800.us.archive.org/14/items/2020_01_06_fbn/2020_01_06_fbn_files.xml
target/debug/ia-get https://archive.org/download/Neo-GeoPocketColorRomCollectionByGhostware/Neo-GeoPocketColorRomCollectionByGhostware_files.xml
target/debug/ia-get https://archive.org/download/deftributetozzap64/deftributetozzap64_files.xml
```

Profile it with `htop`:

```shell
top -p $(pidof ia-get)
```
