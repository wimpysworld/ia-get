# Authentication and Networking

This guide covers session cookies, redirect handling, proxy environment variables, and retry behaviour in `ia-get`.

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
