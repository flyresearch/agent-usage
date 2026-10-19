//! Recent observed usage, never an estimate of model generation speed.
use crate::{parser::Record, protocol::*};
use chrono::{DateTime, FixedOffset, Utc};
use std::collections::BTreeMap;

pub const WINDOW_SECONDS: i64 = 600;
pub fn agent(adapter: &str) -> &'static str {
    match adapter {
        "codex-jsonl" => "codex",
        "claude-jsonl" => "claude",
        "workbuddy-jsonl" => "workbuddy",
        "agy-sqlite" => "agy",
        "hermes-sqlite" => "hermes",
        "openclaw-sqlite" | "openclaw-codex-jsonl" => "openclaw",
        _ => "unknown",
    }
}
pub fn project(
    config: &Config,
    records: &BTreeMap<(usize, String), Record>,
    statuses: &[SourceStatus],
    now: i64,
) -> Result<RecentActivity> {
    let start = now
        .checked_sub(WINDOW_SECONDS)
        .ok_or(Error("invalid_activity_time"))?;
    let start_day = DateTime::<Utc>::from_timestamp(start, 0)
        .ok_or(Error("invalid_activity_time"))?
        .with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d")
        .to_string();
    let unsupported_sources = config
        .sources
        .iter()
        .filter(|s| s.adapter == "hermes-sqlite")
        .map(|s| s.id.clone())
        .collect::<Vec<_>>();
    let mut complete = unsupported_sources.is_empty()
        && statuses.iter().all(|s| {
            let c = &s.coverage;
            c.pending_files == 0
                && c.unmetered_files == 0
                && c.unclassified_usage_records == 0
                && c.missing_backend_threads == 0
                && c.orphan_usage_records == 0
                && c.negative_residual_records == 0
                && c.unclassified_fork_prefix_records == 0
                && c.invalid_cache_ttl_records == 0
                && c.unknown_total_carry == 0
                && c.missing_identity_records == 0
                && c.unsupported_fields == 0
                && c.counter_resets == 0
        });
    let mut grouped: BTreeMap<(i64, String), (u64, MessageCount)> = BTreeMap::new();
    let mut unlocated_records = 0u64;
    let mut future_records = 0u64;
    for ((source, _), r) in records {
        if r.claude_usage_stage == 1 {
            complete = false;
            continue;
        }
        let Some(time) = r.event_time else {
            // A dated old record cannot be in this window. Undated/recent
            // aggregate records are unknown, not zero recent consumption.
            if r.day.as_ref().is_none_or(|day| day >= &start_day) {
                unlocated_records = unlocated_records
                    .checked_add(1)
                    .ok_or(Error("coverage_overflow"))?;
                complete = false;
            }
            continue;
        };
        if time > now {
            future_records = future_records
                .checked_add(1)
                .ok_or(Error("coverage_overflow"))?;
            complete = false;
            continue;
        }
        if time <= start {
            continue;
        }
        let entry = grouped
            .entry((time, agent(&config.sources[*source].adapter).into()))
            .or_insert((
                0,
                MessageCount {
                    known: 0,
                    complete: true,
                },
            ));
        entry.0 = entry
            .0
            .checked_add(r.tokens.total()?)
            .ok_or(Error("token_overflow"))?;
        entry.1 = entry.1.checked_add(r.messages)?;
    }
    Ok(RecentActivity {
        as_of: now,
        window_seconds: WINDOW_SECONDS as u64,
        coverage_complete: complete,
        unlocated_records,
        future_records,
        unsupported_sources,
        buckets: grouped
            .into_iter()
            .map(|((timestamp, agent), (tokens, messages))| ActivityBucket {
                timestamp,
                agent,
                tokens,
                messages,
            })
            .collect(),
    })
}

pub fn validate(value: &RecentActivity, snapshot: &Snapshot) -> Result<()> {
    let start = value
        .as_of
        .checked_sub(WINDOW_SECONDS)
        .ok_or(Error("invalid_snapshot"))?;
    let generated = DateTime::parse_from_rfc3339(&snapshot.generated_at)
        .map_err(|_| Error("invalid_snapshot"))?
        .timestamp();
    if value.window_seconds != WINDOW_SECONDS as u64
        || value.as_of > generated
        || value.coverage_complete
            && (value.unlocated_records > 0
                || value.future_records > 0
                || !value.unsupported_sources.is_empty())
        || value
            .unsupported_sources
            .iter()
            .any(|id| !snapshot.sources.iter().any(|s| &s.source_id == id))
    {
        return Err(Error("invalid_snapshot"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for bucket in &value.buckets {
        if bucket.timestamp <= start
            || bucket.timestamp > value.as_of
            || !["codex", "claude", "agy", "hermes", "openclaw", "workbuddy"]
                .contains(&bucket.agent.as_str())
            || !seen.insert((bucket.timestamp, &bucket.agent))
        {
            return Err(Error("invalid_snapshot"));
        }
        total = total
            .checked_add(bucket.tokens)
            .ok_or(Error("token_overflow"))?;
    }
    if total > snapshot.totals.total()? {
        return Err(Error("invalid_snapshot"));
    }
    Ok(())
}
