# Validation status

This is an experimental single-host CLI. Validation demonstrates the implemented contracts, not complete provider billing, every historical format, or every platform.

## Reproducible checks

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cargo build --release --locked
```

The initial public baseline has 81 synthetic implementation tests. They call the actual Rust parser, collector, and CLI; arithmetic examples alone are not treated as parser acceptance.

Coverage includes:

- Codex cumulative/last selection, response identities, mirror exclusion, compaction estimates, cache-write subsets, and conservative fork boundaries.
- Claude streaming updates, request identity ambiguity, cache TTL contradictions, and cross-file/cached uncertainty propagation.
- Missing time, incomplete JSONL tails, malformed records, integer/subset/overflow validation.
- Multiple directories, overlapping paths, duplicate physical files, cache invalidation, deletion, and unchanged-cache reuse.
- Atomic snapshot publication, failed collection preserving the old snapshot, output path protection, summary invariants, and controlled exported fields.

Native development checks were run on macOS arm64 with Rust/Cargo 1.91.1. Windows and macOS x86_64 remain unqualified. Public CI runs synthetic checks only; a CI pass is not evidence of live provider-format coverage.

## Optional local qualification

Run comparisons only on explicitly selected local logs:

```sh
cargo build --locked --examples
python3 scripts/compare-local.py codex-jsonl PRIVATE_CODEX_ROOT --limit 12
python3 scripts/compare-local.py codex-jsonl PRIVATE_CODEX_ROOT --fork-only --limit 100
python3 scripts/compare-local.py claude-jsonl PRIVATE_CLAUDE_ROOT --limit 12
```

Read matched, errors, changed, and oracle_unqualified separately. The Python oracle intentionally excludes cases it cannot qualify; an excluded case is not a pass. No paths, session identities, message contents, or raw model names should be exported. Do not upload private qualification output or logs in bug reports.

Cold/warm performance depends on the selected corpus and filesystem. Measure your own environment; this repository does not publish private corpus sizes, usage totals, inventories, or performance traces.

## Remaining boundaries

Complete upstream tokscale-core execution comparison, broader historical/headless formats, byte-level incremental parsing, concurrent-collection control, native platform qualification, SQLite adapters, and multi-host aggregation remain future work. Recognized unmetered or contradictory fields stay explicit in coverage; read/schema/identity failures are not silently converted to zero.
