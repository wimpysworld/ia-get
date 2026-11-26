# Authentication and Networking

This guide covers session cookies, redirect handling, proxy environment variables, retries, and polite parallel downloads in `ia-get`.

## Cookies

Pass cookies with `-b` or `--cookies <COOKIES>` when an archive.org item requires a logged-in session.
`--cookies` accepts either a raw `Cookie` header string or a path to a Netscape `cookies.txt` file exported from your browser.

```shell
ia-get --cookies cookies.txt https://archive.org/details/<identifier>
ia-get -b 'logged-in-user=...; logged-in-sig=...' https://archive.org/details/<identifier>
```

`ia-get` scopes cookies strictly:

- Raw cookie strings attach only to HTTPS requests for `archive.org` and its subdomains.
- Netscape `cookies.txt` files match host, path (longest path first per RFC 6265), the `Secure` flag, and live expiry on every request and retry.

When verbose logging (`-v`) is active, `ia-get` logs only `cookies=yes` or `cookies=no` and never prints cookie names or values.

## Redirect handling

`ia-get` follows HTTP redirects manually instead of relying on automatic client redirects.
Each request can follow at most 10 redirect hops.

On every hop, `ia-get`:

- Re-evaluates cookies against the new target URL so credentials never leak to unrelated hosts or mismatched paths
- Rejects redirect loops
- Rejects non-HTTP and non-HTTPS schemes
- Rejects HTTPS-to-HTTP downgrades

## Proxies

`ia-get` reads standard proxy environment variables automatically: `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY`.

Precedence rules:

- Lowercase variable names (`http_proxy`, `https_proxy`, `all_proxy`, `no_proxy`) take precedence over uppercase names.
- Scheme-specific variables (`HTTP_PROXY`, `HTTPS_PROXY`) take precedence over `ALL_PROXY`.

Supported proxy schemes are `http://`, `https://`, `socks4://`, `socks4a://`, `socks5://`, and `socks5h://`.
`ia-get` validates proxy configuration before any request starts, rejects malformed proxy URLs, and redacts embedded credentials in error messages.

```shell
HTTPS_PROXY=http://proxy.example:8080 ia-get https://archive.org/details/<identifier>
ALL_PROXY=socks5h://127.0.0.1:1080 NO_PROXY=localhost,127.0.0.1 ia-get https://archive.org/details/<identifier>
```

## Retries and rate-limit handling

For both metadata fetches and file downloads, `ia-get` retries transient transport errors and HTTP `408`, `429`, and `5xx` responses up to 3 times.

- Retry waits use exponential backoff with jitter.
- When a response includes an RFC 9110 `Retry-After` header (delta-seconds or HTTP-date), `ia-get` honours that delay up to a 60-second cap.
- Other `4xx` responses fail immediately without retrying.
- Pressing Ctrl+C interrupts retry waits promptly.

## Parallel downloads

`ia-get` parallelises downloads automatically with no `--jobs` or `-j` flag to configure.
It uses a 4-permit funnel with weighted first-in, first-out (FIFO) scheduling based on each file's expected size.

| Expected file size | Permits per file | Maximum concurrent downloads |
| --- | --- | --- |
| Under 8 MiB | 1 | 4 |
| 8 MiB to under 64 MiB | 2 | 2 |
| 64 MiB and larger, or unknown | 4 | 1 (sequential) |

### Queue order and FIFO drain

Before transfers start, `ia-get` sorts the download queue in ascending order of expected file size and places entries with unknown sizes at the tail.
Small files such as metadata, thumbnails, page scans, and MP3s finish quickly and do not wait behind large transfers.
Because scheduling is FIFO, active small and medium transfers drain and release their permits before a large file starts.
A file of 64 MiB or more requires all 4 permits.

Queue sorting applies only to download order.
`--list`, `--check`, and end-of-run failure summaries preserve the original XML metadata order from archive.org.

### Host backpressure and global rate limiting

When archive.org returns HTTP `429 Too Many Requests` or `503 Service Unavailable`, `ia-get` honours any `Retry-After` header up to the 60-second cap.
During cooldown, `ia-get` pauses new dispatches across the batch and steps the active permit ceiling down (`4 -> 2 -> 1`).
The ceiling steps back up gradually only after 4 clean network completions without `429` or `503` responses.

When you pass `--limit-rate`, `ia-get` caps total aggregate bandwidth across all active transfers rather than per file.

### Internet Archive automated access guidelines

The 4-permit ceiling, sequential transfer of large files, and automatic `429` and `503` backoff comply with Internet Archive's wishes and [automated access guidelines](https://archive.org/developers/bots.html).
For multi-terabyte archives of multi-gigabyte files, use the item's `.torrent` file (BitTorrent with webseeds) instead of overloading archive.org HTTP datanodes.
