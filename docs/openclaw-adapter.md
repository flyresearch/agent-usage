# OpenClaw SQLite and dedicated Codex sources

Configure `openclaw-sqlite` with explicit database files and
`openclaw-codex-jsonl` with the dedicated backend's session directories. Do not
also configure the dedicated directory as ordinary Codex: physical overlapping
sources are rejected. General user Codex and dedicated OpenClaw Codex remain
separate sources, not globally deduplicated by model name.

Dedicated backend JSONL uses the existing Codex parser and cache, but produces
`agent=openclaw`, `backend=codex`. Those logs are scanned before SQLite regardless
of source configuration order. Hashed thread identities remain internal.

The SQLite adapter reads `transcript_events` and `trajectory_runtime_events` in
one read-only transaction, without loading extensions or modifying WAL settings.
Plain JSON and Zstd events are supported; encoded and decoded event sizes are
bounded at 16 MiB. Invalid JSON, compression, sizes, numeric fields or required
schema fail collection. Main DB metadata never substitutes for reading WAL.

Runtime `session.started` and `model.completed` records with `data.threadId`
establish a `(session_id, run_id)` to backend-thread mapping. A transcript's
`message.__openclaw.runId` selects that mapping. If its thread exists in the
configured dedicated logs, the transcript is an excluded mirror. Otherwise it
is unclassified and `missing_backend_threads` reports the gap. Conflicting
runtime linkage fails. `sourceSeq`, model names and current `session_windows`
harness labels are not historical backend evidence. Runtime usage is counted
only as diagnostic records, never added to transcript/backend totals.

For messages without Codex linkage, the qualified non-Codex transport APIs are
`openai-completions`, `anthropic-messages`, and `google-generative-ai`. Assistant
usage on these paths is counted only without mirror-origin metadata. Other
historical APIs, including ambiguous OpenAI Responses paths, remain explicitly
unclassified. This is a conservative supported-format boundary, not a claim of
complete historical OpenClaw billing. Keep unknown future APIs unclassified
until their producer and backend ownership have been investigated.

Input/cache buckets are independent; reasoning is a subset of output. Optional
reported totals must equal the mutually exclusive sum. Message timestamps are
RFC3339 or epoch milliseconds; no database creation-time or file-mtime fallback
is used. A session-scoped idempotency key deduplicates replay; otherwise the
source-file/session/sequence identifies the event. Conflicting replay fails.
API is retained as `backend`, provider as an opaque `provider-` hash label, and
model only via the configured controlled aliases. No raw provider/model strings,
run IDs, thread IDs, body text or tool arguments enter snapshots or diagnostics.

Eleven synthetic actual-path tests cover JSON/Zstd mapping, dedicated source order
and cache, missing linkage, non-Codex isolation, replay/conflicts, invalid fields,
size bounds, session-scoped linkage, active WAL and source-journal destination protection. Native macOS x86_64 real
collection succeeded with SQLite 3.53.2; an independent readonly JSON/Zstd SQL
projection matched the non-Codex counters with stable before/after observations.
All observed runtime backend threads were found in the dedicated source. Known
historical unclassified records remain visible; they were not declared covered.
