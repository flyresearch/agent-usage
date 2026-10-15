# agy SQLite adapter

The `agy-sqlite` adapter reads explicitly configured local database files or the
`.db`/`.sqlite` files immediately inside a configured directory. It does not
recursively discover archives or backups. JSONL source behavior is unchanged.

## Connection contract

The adapter uses rusqlite 0.40.2 with bundled libsqlite3-sys 0.38.2, fixed by
Cargo.lock. Crate metadata identifies rusqlite as MIT; bundled SQLite is public
domain. No load-extension or provider SDK feature is enabled. The actual SQLite
runtime version is returned in `sqlite_version` on snapshots/reports containing a
SQLite source; do not infer it from the system Python or sqlite CLI.

Each scan opens a readonly, non-immutable connection with query_only and
trusted_schema disabled for untrusted schema functions, then uses a deferred
read transaction. No journal-mode change, source checkpoint, backup/copy or
extension loading occurs. Explicit path aliases are resolved before NOFOLLOW
protects the final open. SQLite may require its normal WAL shared-memory/locking
facilities; do not copy just the main DB or discard its WAL.

SQLite sources bypass the file-metadata cache: committed WAL changes need not
modify the main DB's length or mtime. JSONL caches still work. `bytes_read` for a
SQLite source counts selected BLOB payload bytes, not total physical disk IO.

## Field provenance

The usage and timestamp mapping was reviewed against the MIT
[tokscale-core adapter at the pinned revision](https://github.com/Nanako0129/tokscale-core/blob/fcb85923e5544488f98001cfa390740d0009b9da/src/sessions/antigravity_cli.rs).
The existing third-party MIT notice applies to this reviewed adaptation. It is a
reverse-engineered mapping, not an official billing schema.

- `gen_metadata.data.#1.#4` contains usage.
- Noncached input = #1 + #2; cache read = #5; output = #9; reasoning = #10.
- Output total #3 must match output + reasoning when provided; nonzero output
  without a supported total field is rejected.
- Response identity #11 is hashed for deduplication; conflicting observations
  fail. Missing identity uses a file/index fallback and marks coverage.
- Model #19 maps only through private model_aliases, otherwise unknown.
- Time comes from native chatModel.#9.#4 or a unique steps association by
  response ID / generation index. Ambiguous joins remain unallocated. Session
  creation time and file mtime are never used as generation time substitutes.

A usage row lacking both response ID and model may be an unfinished placeholder
with prefilled counters. It is excluded from known tokens and recorded in
`unclassified_usage_records`, not represented as a confirmed zero-use call.
Unknown usage fields remain explicit in coverage. Malformed protobuf, duplicate
singular fields, invalid integer encodings, missing required tables, corrupt DBs,
and SQL/IO errors fail the collection, retaining the previous successful output.

## Validation

Synthetic tests call the actual adapter/collector, including committed and
uncommitted WAL, unchanged main-file metadata, readonly enforcement, schema/type
errors, duplicate response identities, malformed protobuf, missing/ambiguous
time, placeholders, multiple paths, and physical-path overlap.

Readonly source-machine comparison of the selected numeric projection has been
performed locally. Private files, paths, identities, inventories and usage
measurements are not included here. This does not establish native Windows or
Intel-macOS compatibility, or qualify every agy version.
