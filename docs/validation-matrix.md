# First-release evidence map

This maps the requested scope to actual implementation and verification. It is
not a completed-release declaration. The remaining live observation window is
tracked in [first-release.md](first-release.md).

| Requirement | Implementation | Direct verification | Qualification boundary |
|---|---|---|---|
| Five source families | JSONL parser and sqlite/agy, hermes, openclaw adapters | Actual Rust parser/collector suites; readonly source-machine comparisons | Historical unmetered, unclassified and unknown fields remain explicit |
| Mutually exclusive token buckets | protocol::Tokens and adapter normalization | Subset, overflow, reset, stream/replay and residual fixtures | Known usage, not a complete billing statement |
| Multi-directory source isolation | collector path validation and physical-file tracking | Overlap/hardlink rejection, in-scope alias dedup and out-of-scope link rejection | No automatic archive/VM/WSL discovery |
| WAL-aware readonly sources | sqlite::open_readonly and read transactions | Committed/uncommitted WAL tests on native platforms; source-machine reads | App-owned store writes are separate from source reads |
| Host collection and deadlines | fleet::transport and batches of two | Actual local/SSH/PowerShell collection; timeout, bounded output and quoting checks | Killing SSH does not prove immediate remote termination |
| Atomic last-good replacement | fleet::store | Repeated import, independent-host addition, restart, generation/attempt fencing and schema upgrade tests | Successful source deletion/move can lower historical totals |
| Failure/source reporting | collect_detailed, failure envelope and failed_source_id | CLI-to-fleet tests and isolated native UI failure/recovery | Source ID may be absent for configuration/state/transport errors |
| Freshness and configuration identity | per-worker receipt time, config pins, config-id | Slow-peer timestamp regression; local restore/config-change real-collector integration | Noncooperating file editors are not an atomic FS/DB transaction |
| Native UI | Separate Syrtis-fork offline-ui target | Native four-host view, host/agent/model/date filters, source diagnostics and restart | Locally signed trial app; no distribution/notarization claim |
| Menu-bar lifecycle | MenuBarExtra and shared DashboardStore | User confirmed menu reopening; reopened native panel inspected | Closing the panel retains the app; Quit ends auto-refresh |
| No provider/updater startup | Independent Swift package and explicit CLI entrypoints | Reachable-source and runtime linkage inspection | Existing SSH identities are used only for authorized transport |
| Three platform targets | macOS arm64/x86_64 and Windows x86_64 GNU | Native suites and real configured-source scans; Windows system-only PATH check | Hosted MSVC CI is configured but not claimed executed |
| Rollback | Paired private app, configuration and store backup | Older bundled collector reads its paired fleet config/store | Source logs are not restored by an aggregate-state backup |
| Performance | Cached JSONL scans, bounded subprocess output, async UI | Cold/warm measurements, four-host warm profile and ongoing process samples | Targets apply to the configured workload; final trend audit pending |
| 24-hour trial | Finite private state/resource observers | Minute-level state samples, aggregate consistency and failure/recovery evidence | Window still running; process completion and final samples must be checked |

## Final observation audit

Run the actual audit utility against the private observation files:

```sh
python3 scripts/audit-trial.py --observations PRIVATE_OBSERVATIONS --resources PRIVATE_RESOURCES --events PRIVATE_EVENTS
python3 -m unittest discover -s tests -p 'test_trial_audit.py'
```

The utility emits JSON without host IDs or usage totals. It reports elapsed time,
observation gaps, mismatch/error counts, verified label changes and resource
samples. A transient unfinished-attempt marker followed by success of that same
attempt is distinguished from a failed attempt followed by a retry. An elapsed
window alone is never a success verdict: `review_required` remains true.

Before completion, verify the live observer handles have ended normally, inspect
the completion record, assess any gaps or errors and evaluate resource trends.
Keep all private observations and artifacts outside Git. Synthetic tests and a
short healthy run do not substitute for the requested observation window.
