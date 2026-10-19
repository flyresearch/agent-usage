use agent_usage::*;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs};
fn aliases() -> BTreeMap<String, String> {
    [("fixture".into(), "fixture".into())].into()
}
fn claude(id: &str, time: &str, sub: bool) -> Value {
    json!({"type":"assistant","timestamp":time,"isSidechain":sub,"requestId":id,
        "message":{"id":id,"model":"fixture","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}})
}
fn parse(rows: &[Value], adapter: &str) -> Result<parser::Parsed> {
    let data = rows.iter().map(|v| format!("{v}\n")).collect::<String>();
    parser::parse(data.as_bytes(), adapter, &aliases(), "fixture")
}
#[test]
fn timezone_midnight_and_metadata_role_are_preserved() {
    let p = parse(
        &[
            claude("one", "2026-01-01T15:59:59Z", false),
            claude("two", "2026-01-01T16:00:00Z", true),
        ],
        "claude-jsonl",
    )
    .unwrap();
    let first = p
        .records
        .iter()
        .find(|r| r.day.as_deref() == Some("2026-01-01"))
        .unwrap();
    let second = p
        .records
        .iter()
        .find(|r| r.day.as_deref() == Some("2026-01-02"))
        .unwrap();
    assert_eq!(first.hour, Some(23));
    assert_eq!(second.hour, Some(0));
    assert_eq!(first.execution_role.as_deref(), Some("main"));
    assert_eq!(second.execution_role.as_deref(), Some("subagent"));
}
#[test]
fn streamed_response_crossing_hours_remains_one_unallocated_call() {
    let a = claude("one", "2026-01-01T00:59:59Z", false);
    let mut b = claude("one", "2026-01-01T01:00:00Z", true);
    b["message"]["usage"]["output_tokens"] = json!(4);
    for rows in [vec![a.clone(), b.clone()], vec![b, a]] {
        let p = parse(&rows, "claude-jsonl").unwrap();
        assert_eq!(p.records.len(), 1);
        assert_eq!(p.records[0].hour, None);
        assert_eq!(p.records[0].execution_role, None);
        assert_eq!(p.records[0].tokens.total().unwrap(), 14);
    }
}
#[test]
fn missing_timestamp_or_role_is_unknown_not_midnight_or_main() {
    let mut a = claude("one", "2026-01-01T00:00:00Z", false);
    a.as_object_mut().unwrap().remove("timestamp");
    a.as_object_mut().unwrap().remove("isSidechain");
    let p = parse(&[a], "claude-jsonl").unwrap();
    assert_eq!(p.records[0].day, None);
    assert_eq!(p.records[0].hour, None);
    assert_eq!(p.records[0].execution_role, None);
}
#[test]
fn collector_buckets_by_hour_and_role_without_changing_daily_sum_or_cache() {
    let t = tempfile::tempdir().unwrap();
    let logs = t.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let rows = [
        claude("one", "2026-01-01T00:00:00Z", false),
        claude("two", "2026-01-01T01:00:00Z", false),
        claude("three", "2026-01-01T01:00:00Z", true),
    ];
    fs::write(
        logs.join("a.jsonl"),
        rows.iter().map(|v| format!("{v}\n")).collect::<String>(),
    )
    .unwrap();
    let config = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: aliases(),
        sources: vec![Source {
            id: "main".into(),
            adapter: "claude-jsonl".into(),
            paths: vec![logs],
        }],
    };
    let cache = t.path().join("cache.json");
    let a = collect(&config, Some(&cache)).unwrap();
    let b = collect(&config, Some(&cache)).unwrap();
    assert_eq!(a.rows.len(), 3);
    assert_eq!(a.totals.total().unwrap(), 36);
    assert_eq!(b.totals, a.totals);
    assert_eq!(b.sources[0].cache_hits, 1);
    assert_eq!(a.rows.iter().map(|r| r.messages.known).sum::<u64>(), 3);
    for (hour, role, basis) in [
        (Some(24), None, "event_timestamp"),
        (Some(2), None, "unallocated"),
        (None, Some("private-agent"), "event_timestamp"),
    ] {
        let mut malformed: Snapshot =
            serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
        malformed.rows[0].hour = hour;
        malformed.rows[0].execution_role = role.map(str::to_owned);
        malformed.rows[0].time_basis = basis.into();
        assert!(output::summary(&malformed).is_err());
    }
}
#[test]
fn legacy_snapshot_without_dimensions_remains_readable() {
    let t = tempfile::tempdir().unwrap();
    fs::write(
        t.path().join("a.jsonl"),
        format!("{}\n", claude("one", "2026-01-01T00:00:00Z", false)),
    )
    .unwrap();
    let config = Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: aliases(),
        sources: vec![Source {
            id: "main".into(),
            adapter: "claude-jsonl".into(),
            paths: vec![t.path().into()],
        }],
    };
    let snapshot = collect(&config, None).unwrap();
    let mut value = serde_json::to_value(snapshot).unwrap();
    for row in value["rows"].as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("hour");
        row.as_object_mut().unwrap().remove("execution_role");
    }
    let old: Snapshot = serde_json::from_value(value).unwrap();
    assert_eq!(old.rows[0].hour, None);
    assert_eq!(old.rows[0].execution_role, None);
    assert!(output::summary(&old).is_ok());
}

#[test]
fn unsupported_numeric_claude_timestamp_cannot_create_hour_without_day() {
    let mut a = claude("one", "2026-01-01T00:00:00Z", false);
    a["timestamp"] = json!(1767225600000i64);
    let p = parse(&[a], "claude-jsonl").unwrap();
    assert_eq!(p.records[0].day, None);
    assert_eq!(p.records[0].hour, None);
    assert_eq!(p.records[0].tokens.total().unwrap(), 12);
}
