# Contributor handoff

Start with AGENTS.md, then [setup](minimal-usage.md), [validation](minimal-validation.md), and [protocol](protocol-v1.md).

## Current baseline

Package 0.1.0 is experimental; parser version p2b-9, snapshot schema v1, and app-owned fleet store schema 2. The implemented surface includes Codex/Claude/WorkBuddy JSONL and agy/Hermes/OpenClaw SQLite collection, local/SSH fleet orchestration, atomic last-good aggregation, and a separate native macOS UI fork. Source directories and host configurations are explicit.

Tests use synthetic data and exercise the actual implementation. Native qualification covers macOS arm64/x86_64 and Windows x86_64 GNU collectors; the native UI is a macOS application. Hosted Windows MSVC CI has not been executed for these unpushed changes. A fresh checkout has no private config, cache, snapshot, model mapping, or ready-to-run source directories; follow the setup guide.

The active macOS app adapts the original Syrtis target, with Agent Usage branding, machine/Agent filters, daily/model views, metered response/API-call counts and versioned offline price estimates. Computer Use verified filtering and metric consistency. The latest branding build compiles and starts; its updated popover appearance has not yet been rechecked. Shared Swift core checks total 23 and original-target projection checks total 9.

The full trial and final evidence audit remain incomplete; the extended trial goal is paused. Earlier observer samples do not qualify the replacement original-target UI. Read [first-release.md](first-release.md) and [validation-matrix.md](validation-matrix.md). Chronological checkpoints below record earlier states rather than current acceptance. This is a source checkpoint, not a notarized or distribution release.

## Important invariants

- Preserve mutually exclusive token buckets and explicit coverage gaps.
- Do not sum Codex response usage and its token-count mirrors.
- Keep fork owner identity fixed across inherited metadata; uncertain legacy prefixes are not child usage.
- Claude identity or streaming conflicts must not become independent duplicate calls.
- Cache-write uncertainty propagates across streamed versions and cache hits.
- Failed collection does not overwrite the saved successful snapshot.
- CLI collections sharing config/cache/output directories hold OS locks through publication. Keep empty lock markers in place; library callers must hold CollectionLocks explicitly.
- Configuration is rechecked after locking and before publication; this is not a transaction with non-cooperating editors.
- Never use session text, tool arguments, credentials, file mtime, or guessed billing values to fill missing usage.
- No private raw logs, machine inventories, absolute user paths, or credentials belong in commits or bug reports.

## Next work and deferred scope

Menu-bar filtering and metric consistency have direct Computer Use evidence. Resume the extended trial only when requested, then audit observer completion, state continuity, failure recovery and resource trends before declaring the first release complete. Do not restart an observer merely because a poll times out.

Broader historical/headless formats, byte-level tail parsing and platforms beyond the qualified targets remain deferred. SQLite adapters, remote per-host single-flight collection, multi-host aggregation and native UI integration are already implemented within the documented scope.

Keep behavior and cache/parser revisions synchronized. Run the documented checks before changes are submitted. MIT applies to this repository; preserve third-party attribution in third_party and the provenance review. Do not claim full upstream-engine compatibility: the complete upstream Rust engine has not been executed as part of this baseline.


## Chronological implementation checkpoints

The pending items and test counts in these checkpoints describe their respective development stages. Use the current baseline and linked completion ledger above for present status.

## First-release goal checkpoint

The requested end state includes the UI, five source families and multi-host
operation. Track the complete requirements in [first-release.md](first-release.md);
do not redefine completion around the currently implemented local collector.
The agy adapter and SQLite runtime metadata are implemented; remote source and
platform checks, OpenClaw, fleet aggregation, full native UI qualification and the trial
window remain open. A separate local Syrtis fork at v2.7.0 contains an offline SwiftUI target
under offline-ui. It builds into an independent locally signed app bundle, and
local configuration/refresh, restored snapshots, source diagnostics and agent
filtering have been exercised through the native UI. Fleet/host flows remain
pending; no commit, push, or distribution release is implied by this work.

## Hermes checkpoint

The Hermes adapter and optional interval/category protocol fields are implemented; see [hermes-adapter.md](hermes-adapter.md). Ten actual-path tests pass, including committed WAL and residual handling. Native macOS x86_64 source-machine counter comparison passed. The separate offline Swift consumer accepts and validates interval rows; nine data-layer checks pass. OpenClaw, fleet aggregation and remaining first-release qualification are still pending.

## OpenClaw checkpoint

OpenClaw SQLite/Zstd and dedicated Codex attribution are implemented in parser p2b-3; see [openclaw-adapter.md](openclaw-adapter.md). Eleven actual-path tests and a native Intel source-machine comparison passed. Raw runtime usage is never added to totals. Historical ambiguous records remain unclassified; do not present this as complete billing coverage. Multi-host collection/store, full UI fleet flows and the trial remain open.

## Fleet CLI checkpoint

`fleet-sync` and read-only `fleet-report` implement bounded local/SSH execution and transactional SQLite last-good state. Configuration generations plus attempt serials reject stale callbacks; failed hosts retain snapshots, uncollected hosts remain absent, and changed-config snapshots are excluded. Fourteen new actual store/runner/CLI tests pass; real two-host collection includes all five agent families. Remote config-lock conflict/recovery was verified. See [fleet.md](fleet.md). The separate native UI still consumes the single-host envelope and must be extended; four-host qualification and trial remain open.

## Native fleet UI checkpoint

The separate local UI fork now accepts fleet envelopes, shows host freshness/error states, filters host/agent/model/date before aggregation, and offers five-minute auto-refresh. Fifteen Swift data/process checks pass, including nonblocking timeout and output bounds. Actual local + SSH refresh, remote conflict/recovery and filtering were verified in the native app. Bundle notices are copied from an explicitly provided collector source checkout. Four-host/platform checks and the trial remain open.

## Additional-machine format checkpoint

Actual Intel-host collection exposed in-scope Claude subagent symlinks, explicit Codex task-boundary resets and Claude terminal input/cache repartitioning. These are now handled under narrow evidence-based rules in parser p2b-4; scope-expanding links and unexplained counter conflicts remain errors. Synthetic regressions cover both arrival orders, cross-file/warm-cache reconciliation, unresolved provisional records and forged/replayed task boundaries. Native Windows GNU test execution has passed; real source qualification is being completed. Test bundles can set `AGENT_USAGE_TEST_BINARY` to an explicit relocated executable; production does not read this override. Windows was added to the CI matrix, but no new remote CI run is implied without a push.

## Four-host qualification checkpoint

All four configured machines now collect successfully using parser p2b-4, and the native UI has shown all four as fresh in one refresh. Current test evidence: 151 native Rust tests on macOS arm64 and Intel, 143 native tests on Windows x86_64 GNU (Unix-specific cases are excluded), and 16 Swift decoder/process checks. Windows formatting and Clippy passed. The Windows release executable was checked with a system-only PATH and imports only Windows system DLLs; actual Codex/Claude collection and warm-cache reuse passed. Intel agy counters matched an independent readonly projection. No hosted Windows CI run or publication is implied.

A local private observer is reading the UI fleet store once per minute for the planned 24-hour trial; the app owns five-minute collection. It records only numeric/state metadata and checks aggregate consistency. Do not claim the observation window complete until its actual completion record and samples are audited. The goal remains active; remaining work includes the trial, resource/rollback audit, precise failed-source reporting and native menu-bar interaction qualification.

## Operations closeout checkpoint

Collector source attribution now reaches optional fleet `failed_source_id` and visible host/source error labels. App-owned store schema 2 upgrades transactionally; readonly schema-1 reports remain supported. An isolated native UI source failure retained exactly the prior known total, identified the source, and cleared after recovery; real four-host settings were restored. A paired app/state rollback backup was created privately, and the old bundled collector successfully read that paired store. See [operations.md](operations.md).

Cached projections now pass outbound validation before publication, open intervals that lose both aggregate bounds normalize into one unallocated bucket, and per-worker receipt time is captured before waiting for slower batch peers. Regression tests cover these boundaries. Menu-bar reopening is confirmed by the user and subsequent native inspection. The live 24-hour state observer and a separate resource observer remain running; initial profiles pass the documented targets, but the full-window trend is not yet qualified.

## Local-mode identity and trial continuity

The native local mode now checks collector configuration identity on restore
and before publishing successful output. The UI writes only validated output
under its own refresh lock, retaining prior snapshots on failure. A real
collector integration check covers changed configuration, failure retention and
recovery; run `make offline-integration COLLECTOR_BINARY=ABSOLUTE_BINARY_PATH` in
the UI fork. Standard standalone checks report explicitly when that integration
check was not requested.

Private host labels now match the approved inventory; source paths were not
changed. The trial records this configuration transition. A source inventory
decrease was observed and independently reproduced without cache; the UI now
explains that moving/removing source logs can reduce successful historical
aggregation. Rollback backups include fleet and source configurations as well
as the app/state pair. The full 24-hour observation remains pending.

Latest closeout verification: 158 local Rust tests pass; the Windows CLI identity
check also passed, and configuration identity matched all four deployed
collectors. Swift checks passed with the real collector integration enabled
(18 checks total). The trial is still in progress; no full-window completion
claim, commit, push or distribution release has been made.

## Trial audit tool

`scripts/audit-trial.py` summarizes the private state/resource observations
without emitting host IDs or usage totals. Six synthetic tests exercise its
window, recovery, label-mapping, resource and diagnostic boundaries. In-flight
unfinished markers that later succeed in the same attempt are separated from
actual error/retry recovery. See [validation-matrix.md](validation-matrix.md)
for the completion evidence map; elapsed time alone is not an acceptance result.

## Syrtis UI adaptation correction

The separate offline-ui executable is now a transition fallback. Active UI work
adapts the original Syrtis target at the pinned fork revision: its native status
item, popover, cards, charts, client tabs and settings remain the implementation.
The existing UsageDataSource seam now has a collector/fleet adapter and an
explicit offline startup lane. The original Rust/FFI and Sparkle are linked by
that target; do not repeat the transition app's no-linkage claim for it.
Provider/account/updater/Discord initialization is bypassed in the fleet lane.

The original target builds, its unbundled selftest passes, and eight adapter
projection checks pass both unbundled and in the local bundle. The user confirmed original-popover opening, normal rendering, readable models,
and machine/Agent/model/daily filtering. Message counts and amount are still
unavailable; the user explicitly called out this gap. Prior independent-UI tests and trial samples do not qualify the
new UI build. See the UI fork's canonical offline-agent-usage document for the
integration files, build commands and unsupported-metric presentation rules.

## Message counts and price estimates

p2b-5 exports additive row `messages` counts with explicit partial coverage;
parser revision invalidates older cached projections. The current-price estimate
lives in the UI consumer, using a bundled versioned USD directory and exact
controlled model-label matches. No provider calls or price auto-updates run at
collection/UI startup. Historical actual cost and subscription charges are not
inferred. Unknown model/rate rows remain unpriced; the UI shows priced-token
coverage and the baseline assumptions. See data-contract.md and the UI fork's
pricing-estimates.md for semantics and provenance.

Latest metric verification: 162 local Rust tests and formatting/Clippy pass;
Intel native full suite passed before the final zero-token response refinement,
and the affected Hermes/OpenClaw native suites passed again afterward. Windows
native final suite passes 153 checks. Swift core has 22 passing checks including
real collector integration; original Syrtis projection checks pass 9 cases and
its unbundled selftest passes. All four configured collectors have been updated
to p2b-5 and returned fresh snapshots with known counts. The original-target
actual-report check confirms available estimates and explicit partial counting
coverage; private percentages and usage values remain outside Git.

Computer Use verified the real original Syrtis popover after finding and fixing
an outdated configured collector path that stripped additive count fields.
Actual Agent/host switches, daily/model views and the rendered screenshot now
show consistent metered counts and estimates. The app-owned collector rebinding
regression is implemented and tested; custom CLI paths remain untouched. Swift
core checks now total 23; original-target projection checks remain 9. Runtime
config and rendered private figures are not stored in Git.

## WorkBuddy source checkpoint

Parser p2b-7 adds the bounded [WorkBuddy JSONL adapter](workbuddy-adapter.md).
The native UI consumer accepts the new agent label. Private source configuration
and installed/remote collectors are not changed by this source checkpoint.

## Offline analysis dimensions

Parser p2b-8 retains optional hourly and controlled execution-role dimensions.
Daily totals and deduplicated response counts are unchanged. UI hourly analysis
must report unallocated-hour tokens, and Agents analysis must retain unknown
attribution. Older deployed collectors remain readable with unknown dimensions.
The native fork enables Hourly, Stats and Agents with offline estimates and
scoped filtering; these views do not provide subscription quotas or live rates.

## Recent activity and optional online quota

Parser p2b-9 adds the bounded [recent activity report](recent-activity.md).
Historical collection remains local-only. The native UI now has separate opt-in
live-metrics and Codex app-server quota settings. The latter is an independent
UI bridge; it is not a collector/provider-login dependency. Unsupported quota
providers and old collectors must remain unavailable/partial, never zero.

## Fleet p2b-9 rollout qualification

The configured macOS arm64, two macOS Intel and Windows GNU collectors now run
parser p2b-9 from collector revision `d140eb4`. Each remote candidate passed
native configuration-identity and real-source collection checks before replacing
the existing executable. The Windows native suite passed 178 tests; its candidate
also executed with only Windows system directories on PATH. Remote executable,
configuration and cache rollback copies, plus the central store backup, remain
private. Source configurations and configured source sets were not expanded.

A subsequent four-host refresh returned fresh p2b-9 snapshots with recent-activity
reports on all hosts. The native UI's actual-report projection check passed for
hourly/role totals and coverage reporting. This closes the old-collector protocol
gap, not historical attribution gaps: missing timestamps/roles, unsupported recent
Hermes interval activity and other source coverage limits remain explicitly partial.
