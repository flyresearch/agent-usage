# WorkBuddy JSONL adapter

`workbuddy-jsonl` projects per-call `providerData.usage` from explicitly configured
project transcript directories. It emits agent `workbuddy`, uses parser p2b-7,
and does not read credentials, the WorkBuddy database, trace logs or online
billing. Configure one source containing all intended transcript roots to dedup
copied calls across those roots. Separate source IDs remain independent.

```json
{"id":"workbuddy-main","adapter":"workbuddy-jsonl","paths":["REPLACE_WITH_PRIVATE_PROJECTS_DIRECTORY"]}
```

Paths must be absolute in the private configuration. No HOME discovery runs.
Nested subagent JSONL files use the same parser. Models require explicit private
`model_aliases`, as with the other adapters; unrecognized models remain unknown.

## Supported accounting

- `message` and `function_call` records may carry usage; tool results and metadata
  without usage do not count. A `message.usage` mirror is never added again.
- Assistant messages (including unknown-role messages) and function calls without
  usage are matched to metered records by response identity within the file, in
  either arrival order. Unmatched responses mark `unclassified_usage_records`;
  a file with only such responses is `unmetered`. User/system/developer/tool
  messages and tool results do not introduce this gap. A missing terminal usage
  record remains a gap even when the response is still streaming. Matching only
  across separate files remains conservatively incomplete.
- `requests` must be exactly 1. A metered zero-token response still counts once.
- Input includes cache read; output includes reasoning. Required detail arrays
  provide `cached_tokens` and `reasoning_tokens`. Subtract those subsets before
  publishing the five mutually exclusive token buckets. Empty arrays mean zero;
  missing or malformed arrays fail rather than inventing noncached input.
- `totalTokens` must equal input plus output, with checked integer arithmetic.
- Raw prompt/completion/total/cache-hit fields, when present, must agree with the
  normalized usage. Legacy `cache_read_input_tokens` and
  `completion_thinking_tokens` are not authoritative aliases: observed records
  can report zero there while the normalized details contain nonzero values.
- Nonzero `cache_creation_input_tokens` or `prompt_cache_write_tokens` fail with
  `unsupported_cache_write`; positive-write semantics are not yet qualified.
  Missing write fields also fail with `unsupported_cache_write`; unknown write
  usage must not be assigned to noncached input or passed to pricing.
- `providerData.messageId` is the source-scoped, hashed identity. Identical copied
  calls count once; conflicting tokens, model or day fail. `traceId` and
  `conversationRequestId` identify multi-call rounds and are not dedup keys.
- Millisecond event timestamps determine Asia/Singapore days. Missing/invalid
  timestamps remain unallocated; neither mtime nor collection time substitutes.

Only complete newline-terminated records are parsed. A partial final line is
retried on a subsequent scan. Existing source locks, stamp checks, caches,
last-good publication and symlink restrictions apply unchanged. Unknown record
shapes and unsupported usage fail explicitly; failures never publish zero.

## Scope and evidence

Synthetic tests invoke the real parser and collector, including copied prefixes,
nested subagents, warm-cache conflicts, missing fields, malformed values,
unsupported writes, pending lines and zero-token response counts. Native local
WorkBuddy 5.8.3 transcript structure informed this implementation. Live subagents,
positive cache writes, older formats, cloud-only sessions and other operating
systems are not qualified. This is known local usage, not a complete historical
billing ledger. No credit-to-USD conversion is performed.

The implementation is independently written. Structural research included the
MIT-licensed [wb-token-meter collector at a fixed revision](https://github.com/FightZhanAng/wb-token-meter/blob/2209d290fa9258028f4754d4b5cd133871258135/src/shared/collector.ts).
No upstream code was copied. Its permissive error/time handling and database
copy fallback are not used. Raw sessions, identifiers, private configurations,
model maps and local validation outputs must remain outside Git.
