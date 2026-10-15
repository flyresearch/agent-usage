# On-demand fleet collection

The CLI supports an app-owned SQLite last-good store and at most two concurrent
collector processes. Local hosts execute the configured collector directly;
remote hosts use system OpenSSH with BatchMode, strict host-key verification,
connect timeout and keepalives. No remote daemon, credential copy or provider
request is introduced. Existing SSH identities and agent sockets may be used.

Create a private fleet configuration and state directory outside the checkout.
Keep both separate from each local collector's config/cache directory: CLI locks
are directory-scoped, and the parent must not hold its child's collector lock.
Each host also retains its own existing collector source configuration. Example
paths and hosts below are fictional; substitute explicit absolute paths. Remote
paths are quoted literally, without HOME or shell-variable expansion.

```json
{
  "schema_version": 1,
  "timeout_seconds": 120,
  "stale_after_seconds": 900,
  "hosts": [
    {
      "host_id": "local-one",
      "configuration_fingerprint": "0000000000000000000000000000000000000000000000000000000000000000",
      "transport": {
        "kind": "local",
        "executable": "/opt/agent-usage/bin/agent-usage",
        "config": "/var/lib/agent-usage/collector/config.json",
        "cache": "/var/lib/agent-usage/collector/cache.json"
      }
    },
    {
      "host_id": "remote-one",
      "configuration_fingerprint": "0000000000000000000000000000000000000000000000000000000000000000",
      "transport": {
        "kind": "ssh",
        "destination": "operator@synthetic-host",
        "executable": "/opt/agent-usage/bin/agent-usage",
        "config": "/var/lib/agent-usage/collector/config.json",
        "cache": "/var/lib/agent-usage/collector/cache.json",
        "shell": "posix"
      }
    }
  ]
}
```

Replace every zero fingerprint with the exact `configuration_fingerprint` from
that host's successful full `collect` snapshot. It pins the source configuration,
not the binary version. Change the pin explicitly when source configuration
changes. `cache` may be null. Windows SSH invocation supports `shell=powershell`,
with a UTF-16LE encoded command to avoid outer-shell interpretation; native
Windows GNU execution and the PowerShell SSH transport have been qualified. POSIX path quoting is exercised against
an actual shell with spaces, quotes and metacharacters.

```sh
agent-usage fleet-sync --config PRIVATE_FLEET_CONFIG --store PRIVATE_FLEET_DB
agent-usage fleet-report --config PRIVATE_FLEET_CONFIG --store PRIVATE_FLEET_DB
```

`fleet-sync` activates the configuration, invokes collectors, replaces successful
host snapshots transactionally and returns a `fleet_snapshot`. Per-host failure
still returns a report with exit 0; inspect `complete` and host states. A command
or storage/configuration failure returns the ordinary error envelope and nonzero
exit. `fleet-report` opens an existing store read-only, does not invoke collectors
and does not create missing databases. It requires the currently activated fleet
configuration. Config files are rechecked before accepting each batch and before
returning the report; noncooperating editors do not participate in a file-system
transaction.

## Snapshot and failure semantics

- Each host has `state`, `included`, `last_success_at`, `last_attempt_at`, `error`
  an optional `failed_source_id`, and an optional full normalized `snapshot`.
- `fresh` means a successful receipt within the configured age. Source history
  coverage still comes from the contained snapshot; freshness is not billing
  completeness.
- `stale` retains and includes the last-good snapshot after error or expiry.
  Remote timestamps do not establish freshness; the center's receipt time does.
  Center clock rollback also marks data stale.
- `never_collected` has no snapshot and contributes no invented zero. Fleet
  `totals` is null if no current-generation host snapshot exists.
- `configuration_changed` may retain the prior snapshot for inspection, but
  `included=false` excludes it from current totals until a matching success.
- Fleet `complete` requires every configured host to be fresh. Fleet
  `coverage_complete` additionally requires all host coverage to be complete.
- Repeated import replaces the host snapshot, never increments it. Different
  hosts add independently. Removing a host excludes it on activation.

Configuration generations and per-host attempt serials fence old successes and
failures, including A-to-B-to-A config changes. Interrupted attempts remain
explicit after restart. Source errors and unknown protocol responses cannot
become successful zero snapshots. Fixed, allowlisted failure codes are retained;
raw stdout/stderr, SSH addresses and remote command text are not exported.

Output is bounded to 64 MiB per host and each invocation has a deadline. Killing
a timed-out SSH client does not prove the remote collector stopped immediately;
its existing source-config lock protects subsequent refreshes until it exits.
Remote collector results are never accepted after the timeout. No broad process
cleanup is used. The GUI must run orchestration away from its main actor.

## Validation and remaining work

Fourteen synthetic tests exercise the actual store, process runner and CLI:
replacement/addition, restart/last-good, absent state, config/attempt fencing,
malformed or mismatched results, clock/staleness, readonly report, unrelated DB
protection, timeout, literal POSIX quoting and destination option rejection.
Native four-host collection (local, macOS SSH and Windows PowerShell SSH) has produced one report containing
all five agent families. A remote collector lock conflict preserved the identical
last-good snapshot and recovered on the following successful refresh. Private
configuration, database files and measured usage remain outside the repository.

The separate native UI now consumes this envelope, exposes host states and filters,
and defaults to five-minute auto-refresh. Four-host/native Windows execution has passed; the planned trial window and remaining operational audit are still required.

See [operation, store compatibility and rollback](operations.md) for failed-source
presentation, per-worker freshness, private trial evidence and the remaining
observation window.
