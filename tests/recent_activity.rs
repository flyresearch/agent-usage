use agent_usage::*;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};
fn row(id: &str, time: i64) -> Value {
    json!({"type":"assistant","timestamp":DateTime::<Utc>::from_timestamp(time,0).unwrap().to_rfc3339(),"message":{"id":id,"model":"fixture","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}})
}
fn parsed(rows: &[Value]) -> parser::Parsed {
    let data = rows.iter().map(|v| format!("{v}\n")).collect::<String>();
    parser::parse(
        data.as_bytes(),
        "claude-jsonl",
        &[("fixture".into(), "fixture".into())].into(),
        "fixture",
    )
    .unwrap()
}
fn project(rows: &[Value], now: i64) -> RecentActivity {
    let p = parsed(rows);
    let config = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        sources: vec![Source {
            id: "source".into(),
            adapter: "claude-jsonl".into(),
            paths: vec![],
        }],
        model_aliases: BTreeMap::new(),
    };
    let status = SourceStatus {
        source_id: "source".into(),
        status: "ok".into(),
        files: 1,
        cache_hits: 0,
        bytes_read: 0,
        coverage: p.coverage,
    };
    let records = p
        .records
        .into_iter()
        .map(|r| ((0, r.key.clone()), r))
        .collect();
    activity::project(&config, &records, &[status], now).unwrap()
}
#[test]
fn window_edges_future_records_and_replay_are_not_double_counted() {
    let now = 1767225600;
    let a = row("one", now - 5);
    let r = project(
        &[
            row("old", now - 600),
            a.clone(),
            a,
            row("two", now),
            row("future", now + 1),
        ],
        now,
    );
    assert_eq!(r.buckets.iter().map(|b| b.tokens).sum::<u64>(), 24);
    assert_eq!(r.future_records, 1);
    assert!(!r.coverage_complete);
    assert_eq!(r.buckets.iter().map(|b| b.messages.known).sum::<u64>(), 2);
}
#[test]
fn missing_or_conflicting_event_time_is_partial_not_zero_complete() {
    let now = 1767225600;
    let a = row("one", now - 5);
    let b = row("one", now - 4);
    let r = project(&[a, b], now);
    assert!(r.buckets.is_empty());
    assert_eq!(r.unlocated_records, 1);
    assert!(!r.coverage_complete);
    let mut missing = row("missing", now);
    missing.as_object_mut().unwrap().remove("timestamp");
    assert!(!project(&[missing], now).coverage_complete);
    let empty = project(&[], now);
    assert!(empty.coverage_complete && empty.buckets.is_empty());
}
#[test]
fn warm_cache_reprojects_time_without_freezing_recent_usage() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("logs");
    fs::create_dir(&root).unwrap();
    let now = Utc::now().timestamp();
    fs::write(root.join("a.jsonl"), format!("{}\n", row("one", now - 5))).unwrap();
    let cfg = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        sources: vec![Source {
            id: "source".into(),
            adapter: "claude-jsonl".into(),
            paths: vec![root],
        }],
        model_aliases: [("fixture".into(), "fixture".into())].into(),
    };
    let cache = d.path().join("cache.json");
    let first = collect(&cfg, Some(&cache)).unwrap();
    let second = collect(&cfg, Some(&cache)).unwrap();
    assert_eq!(second.sources[0].cache_hits, 1);
    assert_eq!(
        second.recent_activity.as_ref().unwrap().buckets[0].tokens,
        12
    );
    assert_eq!(first.totals, second.totals);
    let mut bad: Snapshot = serde_json::from_value(serde_json::to_value(&second).unwrap()).unwrap();
    bad.recent_activity.as_mut().unwrap().buckets[0].tokens = 13;
    assert!(output::summary(&bad).is_err());
    bad = serde_json::from_value(serde_json::to_value(&second).unwrap()).unwrap();
    let activity = bad.recent_activity.as_mut().unwrap();
    activity.buckets.push(activity.buckets[0].clone());
    assert!(output::summary(&bad).is_err());
}
