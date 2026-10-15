# Local operation, qualification and rollback

The native app and collectors are a local trial build. No hosted service or
remote daemon is installed. The native app owns five-minute refresh; collectors
run only when invoked. Closing the dashboard keeps the menu-bar app available.
Use the menu-bar **Quit Agent Usage** action to stop automatic refresh.

## Failure and freshness

A failed host retains its last-good snapshot. `failed_source_id`, when known,
identifies a controlled configured source label; it never contains a path,
session ID or body. The source view explicitly says it is displaying the previous
successful snapshot. Configuration/state/transport failures may have no source
ID. A subsequent success clears the failure details.

An active log can change during a scan. `source_changed_during_scan` is a
retryable failure: the previous snapshot remains available and the next refresh
retries. Unknown/missing sources are not replaced by zero. Removing logs followed
by a successful scan can reduce known historical usage; this is not a permanent
ledger.

Freshness uses the center's per-worker completion time. A slow host in the same
batch cannot renew the receipt time of a result that completed earlier. Hosts
are not sampled at one global instant. Changing fleet configuration excludes
old-generation snapshots from current totals until matching collection succeeds.

## Store compatibility

The app-owned fleet store has SQLite `application_id=0x41555347`. Store schema 2
adds optional failed-source metadata. Opening for refresh upgrades schemas 0/1
transactionally while retaining snapshots. Read-only reporting can read schema
1 without migrating it. Future unsupported schema versions are rejected.
Source databases are separate: this migration never applies to agent databases.

The JSON fleet envelope remains schema 1; `failed_source_id` is additive and
optional. Collector failure envelopes add optional `source_id`. Known tokens,
source status and completeness retain their existing meaning.

## Rollback procedure

1. Quit the custom app and let any in-flight collector invocation end. No broad
   process termination is needed; source-config locks are released on exit.
2. Preserve the current app, private connection configuration, fleet configuration,
   per-host collector configurations and app-owned fleet store together. For an online state copy, use SQLite's backup operation; do
   not copy a live main DB while ignoring its WAL. Do not copy or alter source
   agent databases for this procedure.
3. Restore a previously qualified app and its paired private state backup to
   their original local locations. Keep the newer state separately until the
   rollback is verified. An older collector that cannot open schema 2 needs its
   paired schema-1 backup, not an attempted in-place schema downgrade.
4. Open the restored app and check host identities, last-success times and known
   totals before refreshing. A read-only report with the older bundled collector
   and its paired backup has been exercised successfully.
5. To stop using this fork entirely, quit it and use the original application.
   The custom bundle identifier, app-support directory and collector directories
   are separate; the original app/settings have not been replaced. Collector
   configuration and generated state can remain private for later reuse.

## Qualification evidence and remaining window

Current actual-path evidence covers macOS arm64/Intel and Windows x86_64 GNU,
four-host native UI refresh, model/date/host filters, menu-bar reopening, source
failure attribution/recovery, changed-config exclusion, committed WAL visibility,
last-good restart and source-output privacy. A synthetic source was temporarily
used with an isolated UI state directory to verify the visible failure marker
and unchanged known total, then the real four-host settings were restored.
The user confirmed the menu-bar icon/action, and the reopened native dashboard
was inspected afterwards.

A private 24-hour observer reads the real UI store once per minute. It checks
that included host totals sum exactly to the report, retains freshness/error
states and never reads raw agent histories. A separate temporary observer
samples only the owned UI/collector processes. Development updates and isolated
qualification work are recorded separately. Both observers are finite local
processes, not installed services.

Acceptance targets for the configured trial workload are a warm successful
four-host refresh within 30 seconds, idle UI CPU below 5% after startup and user
interaction settle, UI RSS below 512 MiB, and local collector peak RSS below
1 GiB. Cold parsing and explicit offline timeouts are assessed separately;
per-host invocation deadlines remain configured independently. Initial warm
profiling and idle samples are within these targets. Startup/interaction CPU
spikes are not idle samples. The full observation window and resource trend must
still be audited before claiming trial completion.

Private configurations, host inventories, samples, usage values, snapshots,
rollback artifacts and screenshots remain outside the public repository.

Single-host UI snapshots are also bound to the current configuration fingerprint.
The readonly `config-id --config PRIVATE_CONFIG` command computes the collector
identity without opening source directories or databases. The UI validates a
received/restored snapshot against that identity and only then atomically
publishes its local snapshot. Failed reads preserve the prior file; a changed
configuration cannot reuse it as current data. Local UI refreshes share an OS
lock through validation/publication.
