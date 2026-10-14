# agent-usage

**Experimental local usage collector for Codex and Claude Code.**

Read explicitly configured JSONL directories on the source machine, normalize token usage, and export versioned JSON snapshots. Summaries group known usage by agent and model; snapshots also retain daily buckets. Collection does not access login credentials, refresh tokens, contact model providers, or upload session contents.

This is an early single-host CLI, validated locally on macOS arm64. Other platforms and historical log formats are not fully qualified. It reports **known usage**, not an authoritative billing statement.

## Quick start

Requires Rust/Cargo; development checks use Rust 1.91.1.

```sh
cargo build --release --locked
```

Create a private JSON config outside this checkout using the [setup guide](docs/minimal-usage.md). Source directories must be explicit absolute paths; no default HOME discovery is performed.

```sh
./target/release/agent-usage collect --config PRIVATE_CONFIG --cache PRIVATE_CACHE --output PRIVATE_SNAPSHOT --summary
./target/release/agent-usage report --snapshot PRIVATE_SNAPSHOT
```

After configuring the standard private locations, the convenience script supports:

```sh
./scripts/usage.sh refresh
./scripts/usage.sh report
```

A failed collection leaves the saved snapshot unchanged. stdout contains JSON only; diagnostics use fixed error codes on stderr. Cache and output directories must already exist. Run one collection at a time.

## Coverage matters

- Mutually exclusive buckets: non-cached input, cache read, cache write, non-reasoning output, and reasoning.
- Codex response records and token-count mirrors are not added twice. Claude streaming updates are reconciled by message/request identity.
- Missing dates remain unallocated; no file-mtime substitution.
- Missing historical usage, ambiguous fork prefixes, contradictory cache fields, and unknown models remain visible in coverage.
- `complete=true` means configured sources were read successfully. It does **not** imply `coverage_complete=true` or complete historical billing.
- Deleted source files disappear from the next successful snapshot. This is not a permanent ledger.

The current release does not implement SQLite adapters, multi-host SSH aggregation, a GUI, background scheduling, or a concurrent-collection lock. Changed files are reparsed; unchanged files use the local cache.

## Development

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

Tests use synthetic data and exercise the actual parser, collector, and CLI.

[Setup](docs/minimal-usage.md) · [Protocol](docs/protocol-v1.md) · [Data contract](docs/data-contract.md) · [Validation](docs/minimal-validation.md) · [Roadmap](docs/development-plan.md) · [Contributor handoff](docs/handoff.md)

## License and provenance

MIT — see [LICENSE](LICENSE). The token-selection rules were reviewed against pinned upstream sources; adaptations and differences are documented in [dependency-review.md](docs/dependency-review.md). Preserve the [tokscale-core MIT notice](third_party/tokscale-core/LICENSE). This project is independent and is not an official billing or usage product of the supported providers.
