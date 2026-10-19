# Recent observed token rate

Parser p2b-9 retains an optional event second in the private normalized cache and
adds `recent_activity` to full snapshots. The ordinary historical rows and totals
are unchanged. No new network or provider access is added to the collector.

The report covers `(as_of - 600, as_of]`, grouping deduplicated events by second
and controlled agent label. Tokens include the five mutually exclusive input,
cache and output buckets. The UI's tokens/min is the observed ten-minute sum
divided by ten, not output-generation throughput or an inference-time estimate.
A host's rate is measured at that host's collection time; fleet samples are not
claimed to be simultaneous.

Unknown event time, conflicting replay time, incomplete trailing lines,
unsupported usage fields and unclassified usage remain coverage gaps. Future
observations are excluded and counted as a gap. Hermes interval totals cannot
establish per-event time and are explicitly unsupported for recent rates.
Records provably dated before the window are not treated as recent missing use.
Old snapshots without this optional report mean unavailable, not zero activity.

The UI enables thirty-second collection only when its explicit live-metrics
setting is enabled and automatic collection is enabled. Concurrent consumers
share a single collection; failed automatic attempts also observe the cadence.
No scan is started per animation frame. Existing readonly SQLite/WAL behavior,
config locks, source-mutation detection and atomic last-good storage still apply.

The UI accepts samples at most 75 seconds old (with at most five seconds of
positive clock skew). A recent last-good sample after a failed refresh is
marked retained/partial; older samples are unavailable. Missing hosts are never
added as zero, and old collectors remain visibly unrepresented. Activity from
different hosts is additive; subscription quotas are a separate account view.

Only seconds, controlled labels, aggregate token counts and metered counts are
exported. No response/session identifiers, bodies, tool arguments or account
information are included. Regression fixtures are synthetic and call the actual
parsers, collector, decoder and rate projection.
