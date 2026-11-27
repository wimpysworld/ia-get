# Usage

`ia-get` downloads files from [archive.org](https://archive.org) using each item's XML file index.

## Supported URL formats

`ia-get` accepts an archive.org details URL to download every file in an item, or a direct download URL to download one file.

```shell
ia-get https://archive.org/details/<identifier>
ia-get https://archive.org/download/<identifier>/<path>
```

For `/download/<identifier>/<path>` URLs, `ia-get` strips query strings and fragments, percent-decodes the UTF-8 path, and fetches `<identifier>_files.xml` to match the file's size and MD5 metadata.
If that file is absent from the XML metadata, `ia-get` exits with a `FileNotFound` error.
Otherwise, `ia-get` downloads only that file and keeps its relative path inside the output directory.

## Options

| Option | Description |
| --- | --- |
| `-o`, `--output-dir <DIR>` | Directory for downloaded files or `--check` (defaults to the current working directory). |
| `-b`, `--cookies <COOKIES>` | Send a raw `Cookie` header or load cookies from a Netscape `cookies.txt` file. |
| `-l`, `--list` | List files and sizes from archive metadata XML and exit. |
| `--check` | Verify local files against archive metadata without downloading or modifying files (conflicts with `--list`). |
| `-f`, `--filter <GLOB>` | Filter files by glob pattern (repeatable, prefix with `!` to exclude). |
| `--limit-rate <RATE>` | Limit total aggregate download bandwidth across active transfers (raw bytes per second or `B`, `K`/`KB`, `M`/`MB`, `G`/`GB` in 1024-byte units). |
| `-v`, `--verbose` | Write plain-text diagnostic logs to `stderr` while progress output stays on `stdout`. |

## Output directories

Save files to a chosen directory with `-o` or `--output-dir <DIR>`.
When you omit this option, `ia-get` uses the current working directory.

`ia-get` creates missing directories when a download starts, and keeps every downloaded file inside that directory.
It rejects empty paths, broken symlinks, and existing non-directory paths.
When you pass `--list` or `--check`, `ia-get` does not create the output directory.

```shell
ia-get -o ./downloads https://archive.org/details/<identifier>
ia-get --output-dir /tmp/archive https://archive.org/details/<identifier>
```

## Listing files

Preview the files in an item without downloading them with `-l` or `--list`.
For a details URL, `ia-get` prints the names and sizes in original XML metadata order.
For a direct download URL, `ia-get` prints only the matched file and its size.

If archive.org does not provide a size for an entry, `ia-get` reports it as `unknown` and excludes it from the total known size.

```shell
ia-get --list https://archive.org/details/<identifier>
ia-get -l https://archive.org/details/<identifier>
ia-get --list https://archive.org/download/<identifier>/<path>
```

## Filtering files

Filter files by glob pattern with `-f` or `--filter <GLOB>`.
Repeat this option to combine patterns.

- Prefix a pattern with `!` to exclude matching files.
- Prefix a pattern with `\!` to match a literal leading `!`.
- Positive patterns select files that match any positive pattern, and `!` patterns always exclude matching files.
- Patterns match original names from archive metadata without a leading `/`.
- `*` crosses `/`, so `-f '*.pdf'` matches files in nested subdirectories too.

This option works with downloads, `--list`, and `--check`.
`ia-get` exits with an error if a pattern is invalid or matches zero files.

```shell
ia-get -f '*.pdf' https://archive.org/details/<identifier>
ia-get -f '!*.torrent' https://archive.org/details/<identifier>
ia-get -f '*.cbz' -f '!*thumb*' https://archive.org/details/<identifier>
ia-get --list -f '*.pdf' https://archive.org/details/<identifier>
```

## Verifying local files

Verify an existing local directory against archive.org metadata with `--check`.
This mode is read-only, makes zero writes, and fails if the target directory does not exist.

`ia-get` checks files in original XML metadata order.
It verifies file presence, leftover `.part` files, safe regular-file targets, byte sizes, and MD5 hashes when metadata provides them.
At the end of the run, `ia-get` prints a summary and exits with status `0` only when all checked files pass without interruption.

`--check` works with `-o`/`--output-dir`, `-f`/`--filter`, `-b`/`--cookies`, and `/download/<identifier>/<path>` URLs.
`--check` conflicts with `--list`.

```shell
ia-get --check https://archive.org/details/<identifier>
ia-get --check -o ./downloads https://archive.org/details/<identifier>
ia-get --check -o ./downloads -f '*.pdf' https://archive.org/details/<identifier>
ia-get --check https://archive.org/download/<identifier>/<path>
```

## Limiting bandwidth

Cap total aggregate download speed across all active transfers with `--limit-rate <RATE>`.
`<RATE>` accepts a positive whole number in bytes per second, or a case-insensitive binary (1024-based) suffix: `B`, `K`, `KB`, `M`, `MB`, `G`, or `GB`.

`ia-get` rejects `0`, negative or signed values, decimals, whitespace, unknown suffixes, and overflowing values before any transfer starts.
When `ia-get` downloads multiple small or medium files at once, all active transfers share this single rate cap.
See [Parallel downloads](Authentication-and-Networking#parallel-downloads) for how automatic concurrency scales by file size.
Downloads run unthrottled when you omit this option.
Ctrl+C interrupts throttle waits promptly and leaves the `.part` file ready to resume.

```shell
ia-get --limit-rate 500K https://archive.org/details/<identifier>
ia-get --limit-rate 2M https://archive.org/details/<identifier>
ia-get --limit-rate 1024 https://archive.org/download/<identifier>/<path>
```

## Terminal UI and output

When `stdout` is an interactive terminal, `ia-get` draws an inline progress frame without entering the alternate screen.
The frame shows the item identifier, completed file progress, a throughput sparkline, active transfer slots with mirror region badges, and counts of warnings and failures when non-zero.
As each file finishes, `ia-get` writes one permanent completion line above the frame in your native terminal scrollback.
`ia-get` stays in cooked terminal mode, so `Ctrl+C` and `Ctrl+Z` work natively.

When `stdout` is not a terminal or `TERM=dumb`, `ia-get` skips the inline frame and writes plain text with zero ANSI escape codes.
When `NO_COLOR` is non-empty, `ia-get` disables all ANSI colour codes.
Narrow or short terminals hide optional columns (mirror region and sparkline) or collapse the frame to two lines.

At the end of a batch, `ia-get` clears the inline frame and prints a `Completed:` summary line (or `Interrupted:` after `Ctrl+C`).
If any item requires follow-up, `ia-get` prints a `Needs attention:` section grouped by category:

- **MD5 mismatches**: shows the actual and expected hashes and keeps the `.part` file unpromoted on disk, because archive.org metadata can be stale.
- **Skipped private files**: counts `<private>` entries skipped when you do not pass `-b` or `--cookies`.
- **Transfer errors**: lists files that failed to download with each error message.

## Verbose logging

Write diagnostic logs to `stderr` with `-v` or `--verbose`.
The inline UI and status lines write to `stdout`, so `stderr` stays clean when you redirect it to a file.
Without `-v`, `ia-get` writes no verbose diagnostic lines to `stderr`.

With `-v`, `ia-get` writes plain-text lines without ANSI escape codes for:

- HTTP requests, statuses, redirect hops, and retry delays (`retry-after` or `backoff`)
- Elastic scheduler backpressure and recovery events, prefixed with `[<file_path>]` during multi-file batches
- Per-file skip, failure, and verification reasons
- Run summaries

`ia-get` redacts embedded credentials in logged URLs and logs only `cookies=yes` or `cookies=no`, never `Cookie` header names or values.

```shell
ia-get -v https://archive.org/details/<identifier> 2> ia-get.log
ia-get --verbose --check https://archive.org/details/<identifier> 2> check.log
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | All selected files succeeded (or passed `--check`) without interruption. |
| `1` | One or more files failed, an MD5 check mismatched, or another runtime error occurred. |
| `130` | `Ctrl+C` interrupted the run (`SIGINT`). |

## Related guides

- [Authentication and Networking](Authentication-and-Networking) for cookies, redirects, proxies, retries, and polite parallel downloads.
- [Reliability and Safety](Reliability-and-Safety) for resume staging, disk-space checks, filename sanitisation, and collision validation.
