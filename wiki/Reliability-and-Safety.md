# Reliability and Safety

This guide explains how `ia-get` stages files, resumes downloads, checks disk space, protects local paths, and reports batch outcomes.

## Atomic staging and HTTP range resume

`ia-get` writes every download into a `<file>.part` staging file first.
When archive.org provides an MD5 hash, `ia-get` verifies the completed `.part` file and renames it to the final path atomically only after verification succeeds.

When a destination file already exists on disk and archive metadata provides an expected size, `ia-get` compares the local size first.
If the sizes differ, `ia-get` skips MD5 hashing and re-downloads the file immediately.

When a `.part` file already exists, `ia-get` sends an HTTP `Range` request and validates HTTP `200`, `206`, and `416` responses plus the `Content-Range` header:

- **HTTP `206` with valid `Content-Range`**: appends the remaining bytes to the `.part` file.
- **HTTP `200` or invalid range response**: discards the partial transfer and restarts the download from byte 0.
- **HTTP `416` (Range Not Satisfiable)**: checks the complete remote length from `Content-Range`. If the `.part` file matches the full length and passes MD5 verification, `ia-get` promotes it to the final path. If the `.part` file is oversized, or has the expected length but fails verification (a corrupt equal-length `.part` file), `ia-get` discards it and restarts an unranged download from byte 0.

## Pre-download disk-space checks

Before any file transfer starts, `ia-get` checks free space on the destination volume against the remaining known bytes of the selected files.

- Verified final files already on disk and resumable `.part` prefixes are deducted from the required byte count.
- Entries with unknown sizes in archive metadata are ignored by this calculation.
- When free space is insufficient, `ia-get` stops before issuing any file request and prints an error showing the required and available space.
- `--list` and `--check` skip this check.

## Write safety and filename sanitisation

Every download stays inside the target output directory (the current working directory by default).

- `ia-get` rejects paths that resolve outside the destination root.
- `ia-get` refuses symlinks and non-regular files such as directories or named pipes (FIFOs). On Unix platforms, file opens use `O_NOFOLLOW`.
- For cross-platform safety, `ia-get` sanitises invalid filename characters, dot segments (`.` and `..`), and trailing spaces or dots in path components.
- `ia-get` renames Windows reserved device names: `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `CLOCK$`, `COM0`-`COM9`, `COM¹`-`COM³`, `LPT0`-`LPT9`, and `LPT¹`-`LPT³`. This check ignores case and matches reserved names with or without extensions, even when trailing spaces or dots appear before an extension.

## Pre-download path collision validation

Before creating directories or transferring files, `ia-get` checks all planned entries for local path collisions.

`ia-get` rejects a run if any two entries create:

- Duplicate sanitised destination paths
- `.part` staging collisions, where a `.part` path collides with another entry's final destination path or `.part` path
- File-versus-directory conflicts, where one entry's file path matches another entry's parent directory path

To catch host-specific collisions accurately, `ia-get` probes existing non-symlink parent directories for real filesystem case-folding and Unicode normalisation lookup rules.
When creating a temporary probe subdirectory is unavailable, `ia-get` falls back to prefixed temporary probe files directly inside that directory, and removes all probe artefacts afterwards.
If distinctness cannot be proved on the destination filesystem, validation fails closed so `ia-get` never silently overwrites another file.

## Batch error handling and exit codes

When a file fails to download or fails MD5 verification, `ia-get` prints a warning and continues with the remaining files.
At the end of a run, `ia-get` prints a summary with `ok`, `skipped`, and `failed` counts and lists any failed files in original XML metadata order.

`ia-get` exits with status `0` only when all selected files succeed without interruption.
If any file fails or Ctrl+C interrupts the run, `ia-get` exits with a non-zero status.
