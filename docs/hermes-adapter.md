# Hermes SQLite adapter

Configure `hermes-sqlite` with explicit database file paths. Directories are not
accepted. The adapter uses the same read-only, committed-WAL-visible transaction
as agy; it does not change source journal settings or checkpoint the database.

`session_model_usage` is authoritative. Rows with a null or empty `task` are main
calls; nonempty tasks are auxiliary calls. Every model row contributes once.
For each session, subtract only its main model-row counters from the `sessions`
counters. Add a nonnegative residual as `usage_kind=residual`, model `unknown`:
the latest session model cannot identify which model generated the residual.
Negative or contradictory residuals are excluded and diagnosed; authoritative
model usage remains visible. Orphan model-session references are diagnosed,
including auxiliary-only references. Duplicate session identities fail closed.

Input excludes cache. Output includes reasoning in the source, so reasoning is
subtracted before producing the mutually exclusive output bucket. Null,
negative, noninteger and contradictory counters fail collection rather than
becoming zero. Only identity, model, task, counters and interval columns are
selected; prompts, tool arguments, billing URLs and costs are never selected.

Rows retain optional `interval` bounds and `usage_kind` (`main`, `auxiliary`,
`residual`). Two known bounds within one Singapore calendar day permit an
`aggregate_interval` day bucket. Cross-day or open intervals are unallocated
with `unallocated_interval`; no timestamp means `unallocated`. There is no
file-mtime fallback. Merged bounds describe an envelope, not a continuous record
of activity. Date filtering excludes unallocated rows; total views retain them.

SQLite sources always requery, including when the main DB metadata is unchanged.
`bytes_read=0` for this adapter means physical page IO was not measured, not that
no database was read. The snapshot reports the linked SQLite runtime version.

Validation: ten synthetic tests call the actual adapter/report path, covering
main/auxiliary/residual accounting, orphan diagnostics, invalid data/schema,
intervals, chronological merging, duplicate sessions and active WAL visibility.
A native macOS x86_64 invocation matched an independent read-only SQL projection
on the source machine, with stable before/after counter projections and SQLite
3.53.2. This qualifies this Hermes check; it does not qualify every source or
platform. Private configuration, source data and measured totals stay outside Git.
