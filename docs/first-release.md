# First usable release — completion ledger

> Historical milestone, not the current product completion gate. As of 2026-10-20,
> the target is complete Syrtis replacement; see [development-plan.md](development-plan.md)
> and its linked UI capability ledger. Older checkpoint versions/test counts below
> are historical evidence and must not be presented as current qualification.

Goal: deliver the first usable version, including the native UI. The existing
Codex/Claude CLI alone does not satisfy this goal.

| Requirement | Evidence required | Current state |
|---|---|---|
| Codex and Claude local collection | Actual parsers, boundary fixtures, readonly runtime validation | Implemented; preserve coverage limitations |
| agy SQLite/protobuf | Actual adapter tests, committed WAL visibility, source-machine comparison | Adapter implemented; arm64 and Intel source-machine independent readonly comparisons plus native WAL tests passed |
| Hermes main/auxiliary/residual usage | Actual adapter, interval/unallocated fixtures and runtime comparison | Implemented; actual-path tests and native macOS x86_64 readonly source-machine comparison passed |
| OpenClaw SQLite/Zstd and Codex backend attribution | Explicit source linkage, non-overlap tests and runtime comparison | Implemented; eleven actual-path tests and native macOS x86_64 readonly runtime/non-Codex comparison passed; historical unclassified gaps remain explicit |
| Multi-directory/physical-path isolation | Reject duplicate/overlapping sources and preserve private configuration | JSONL and DB physical duplicates/overlap protected; in-scope regular JSONL aliases explicitly deduplicated |
| Multi-host collection | On-demand SSH, bounded concurrency/timeout, no remote daemon | Implemented; all four configured hosts passed actual local/SSH collection, including Windows PowerShell transport |
| Last-good aggregation | SQLite atomic per-host replacement, config fencing, stale/error/never-collected states | Implemented; actual-path store/CLI tests and remote conflict/recovery verification passed |
| Native UI derived from pinned Syrtis | Independent bundle/storage, overview/daily/models/source/host filters | Native fleet target built and exercised with real local + SSH snapshots, host/agent/model/date filters and remote failure recovery; four-host native UI refresh passed; trial pending |
| No online credentials/provider/update initialization | Audit reachable entrypoints and runtime behavior | Collector and separate UI entrypoints inspected; UI links only system frameworks and uses explicit CLI/SSH transport, without provider/FFI/Sparkle startup |
| Platform qualification | Native macOS arm64/x86_64 and Windows x86_64 checks | macOS arm64/x86_64 and Windows x86_64 GNU native tests and real configured-source collection passed; hosted Windows MSVC CI remains unexecuted |
| End-to-end usability | Real local/remote refresh, offline recovery, restart persistence, responsive UI | Four-host refresh, filtering, restart and visible failed-source recovery exercised; user confirmed menu-bar reopening, then native panel inspected |
| Menu-bar usage experience | Readable usage summary, per-agent/host detail and visible error state with actual interaction checks | Syrtis-style usage popover implemented: today status-item total, period/host/Agent filters, buckets/trend/model/host views and footer actions. Local bundle builds; 20 Swift checks including real collector integration pass. User confirmed actual popover opening, correct layout and working period/Agent/model/host switching; automation cannot target the popover, so those interaction checks are manual |
| Readable model names | Reviewed private aliases and actual model-list rendering | Removed qualification hash labels from four-host private configuration; updated configuration pins and verified readable names in the native panel; unidentified models remain unknown |
| Trial operation | Planned multi-host observation window, failure/recovery and rollback evidence | 24-hour state/resource observation running; initial warm/idle checks and paired-backup rollback report passed; full-window audit pending |

Private host inventories, addresses, config, source logs, databases, and measured
usage stay outside the public repository. UI work is kept in a separate local
fork; pinned source provenance belongs in public docs, private machine details do
not. No goal-completion claim is allowed until the full ledger has authoritative
evidence. A green synthetic suite is necessary but not full business acceptance.

The requirement-to-evidence mapping and final observation procedure are in
[validation-matrix.md](validation-matrix.md). The audit utility reports evidence
only; it cannot mark the goal complete.

## UI baseline correction

The final UI must adapt the original Syrtis application rather than extend the
independent transition UI. The original-target fleet bundle is now built and
running, with original selftests and eight adapter projection checks passing.
Original-popover opening is user-confirmed; its detailed visual/filter checks
are separate from the earlier transition UI acceptance. The earlier trial must
not be presented as a full-window qualification of this replacement build.

## Metered counts and directory pricing checkpoint

All configured collectors now export p2b-5 metered response/API-call counts with
partial coverage. The original Syrtis graph/model adapter receives those counts
and estimates from a versioned offline USD catalog. Actual fleet-envelope checks
prove nonempty counts and estimated amounts, and show unpriced-token coverage
explicitly. This does not claim complete historical message counts or actual
billed spend. Local Rust/Swift and native Intel/Windows checks passed; detailed
pricing assumptions and official sources are in the UI fork pricing document.
