# Contributor handoff

Start with AGENTS.md, then [setup](minimal-usage.md), [validation](minimal-validation.md), and [protocol](protocol-v1.md).

## Current baseline

Package 0.1.0 is experimental; parser version p2a-5, snapshot schema v1. The implemented surface is a single-host Codex/Claude JSONL collector with explicit directory configuration, a private file cache, atomic snapshot publication, and numeric JSON summaries.

Tests use synthetic data and exercise the actual implementation. Native development validation is limited to macOS arm64. A fresh checkout has no private config, cache, snapshot, model mapping, or ready-to-run source directories; follow the setup guide.

## Important invariants

- Preserve mutually exclusive token buckets and explicit coverage gaps.
- Do not sum Codex response usage and its token-count mirrors.
- Keep fork owner identity fixed across inherited metadata; uncertain legacy prefixes are not child usage.
- Claude identity or streaming conflicts must not become independent duplicate calls.
- Cache-write uncertainty propagates across streamed versions and cache hits.
- Failed collection does not overwrite the saved successful snapshot.
- Never use session text, tool arguments, credentials, file mtime, or guessed billing values to fill missing usage.
- No private raw logs, machine inventories, absolute user paths, or credentials belong in commits or bug reports.

## Next work

Broader historical/headless formats, byte-level tail parsing, per-host single-flight collection, additional platforms, SQLite adapters, multi-host aggregation, and optional UI integration. None is implied by the current CLI being usable on its qualified scope.

Keep behavior and cache/parser revisions synchronized. Run the documented checks before changes are submitted. MIT applies to this repository; preserve third-party attribution in third_party and the provenance review. Do not claim full upstream-engine compatibility: the complete upstream Rust engine has not been executed as part of this baseline.
