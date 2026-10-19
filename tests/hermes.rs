use agent_usage::*;
use rusqlite::{params, Connection};
use std::{collections::BTreeMap, path::Path};
fn database(path: &Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.execute_batch("CREATE TABLE sessions(id TEXT,input_tokens INTEGER,output_tokens INTEGER,cache_read_tokens INTEGER,cache_write_tokens INTEGER,reasoning_tokens INTEGER,started_at REAL,ended_at REAL); CREATE TABLE session_model_usage(session_id TEXT,model TEXT,task TEXT,input_tokens INTEGER,output_tokens INTEGER,cache_read_tokens INTEGER,cache_write_tokens INTEGER,reasoning_tokens INTEGER,first_seen REAL,last_seen REAL);").unwrap();
    c
}
fn config(path: &Path) -> Config {
    Config {
        schema_version: 1,
        host_id: "fixture".into(),
        model_aliases: BTreeMap::from([("synthetic".into(), "test-model".into())]),
        sources: vec![Source {
            id: "hermes".into(),
            adapter: "hermes-sqlite".into(),
            paths: vec![path.into()],
        }],
    }
}
fn session(c: &Connection, n: i64) {
    c.execute(
        "INSERT INTO sessions VALUES('s',?1,20,30,40,5,1767225600,1767225601)",
        [n],
    )
    .unwrap();
}
fn usage(c: &Connection, task: Option<&str>, n: i64, end: Option<f64>) {
    c.execute(
        "INSERT INTO session_model_usage VALUES('s','synthetic',?1,?2,20,30,40,5,1767225600,?3)",
        params![task, n, end],
    )
    .unwrap();
}
#[test]
fn main_auxiliary_and_residual_are_disjoint() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    session(&c, 15);
    usage(&c, None, 10, Some(1767225601.));
    usage(&c, Some("helper"), 2, Some(1767225601.));
    let s = collect(&config(&p), None).unwrap();
    assert!(s.rows.iter().all(|r| r.hour.is_none()));
    assert_eq!(
        s.totals,
        Tokens {
            input: 17,
            output: 30,
            reasoning: 10,
            cache_read: 60,
            cache_write: 80
        }
    );
    assert_eq!(s.rows.len(), 3);
    assert_eq!(s.sources[0].coverage.auxiliary_usage_records, 1);
    assert_eq!(s.sources[0].coverage.residual_usage_records, 1);
    let r = s
        .rows
        .iter()
        .find(|r| r.usage_kind.as_deref() == Some("residual"))
        .unwrap();
    assert_eq!(r.model, "unknown");
    assert_eq!(r.tokens.input, 5);
    assert_eq!(output::summary(&s).unwrap()["known_total_tokens"], 197);
}
#[test]
fn negative_residual_preserves_authoritative_model_usage() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    session(&c, 1);
    usage(&c, None, 10, Some(1767225601.));
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.totals.input, 10);
    assert_eq!(s.sources[0].coverage.negative_residual_records, 1);
    assert!(!s.coverage_complete);
    assert_eq!(s.sources[0].status, "partial");
}
#[test]
fn cross_day_and_open_intervals_remain_unallocated() {
    for end in [Some(1767312000.), None] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.db");
        let c = database(&p);
        session(&c, 10);
        usage(&c, None, 10, end);
        let s = collect(&config(&p), None).unwrap();
        let r = &s.rows[0];
        assert!(r.day.is_none());
        assert_eq!(r.time_basis, "unallocated_interval");
        assert!(r.interval.as_ref().unwrap().start.is_some());
        assert_eq!(s.sources[0].coverage.unallocated_tokens, 100);
        assert_eq!(output::summary(&s).unwrap()["unallocated_tokens"], 100);
    }
}
#[test]
fn same_day_interval_retains_bounds() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    session(&c, 10);
    usage(&c, None, 10, Some(1767225601.));
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.rows[0].day.as_deref(), Some("2026-01-01"));
    assert_eq!(s.rows[0].time_basis, "aggregate_interval");
    assert!(s.rows[0].interval.is_some());
}
#[test]
fn auxiliary_orphan_is_visible() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    usage(&c, Some("helper"), 10, Some(1767225601.));
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.sources[0].coverage.orphan_usage_records, 1);
    assert_eq!(s.totals.input, 10);
    assert!(!s.coverage_complete);
}
#[test]
fn invalid_counter_schema_and_interval_fail_closed() {
    for sql in [
        "UPDATE session_model_usage SET input_tokens=-1",
        "UPDATE session_model_usage SET input_tokens=NULL",
        "UPDATE session_model_usage SET reasoning_tokens=21",
        "UPDATE session_model_usage SET last_seen=1",
        "DROP TABLE sessions",
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.db");
        let c = database(&p);
        usage(&c, None, 10, Some(1767225601.));
        c.execute_batch(sql).unwrap();
        assert!(collect(&config(&p), None).is_err(), "{sql}");
    }
}
#[test]
fn duplicate_session_identity_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    session(&c, 10);
    session(&c, 10);
    assert_eq!(
        collect(&config(&p), None).unwrap_err().0,
        "duplicate_session_identity"
    );
}
#[test]
fn wal_commit_is_visible_without_main_file_change() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    session(&c, 10);
    let before = std::fs::read(&p).unwrap();
    let cache = d.path().join("cache.json");
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    usage(&c, Some("helper"), 10, Some(1767225601.));
    assert_eq!(collect(&config(&p), Some(&cache)).unwrap().totals.input, 10);
    c.execute_batch("COMMIT").unwrap();
    let wal = std::fs::read(d.path().join("a.db-wal")).unwrap();
    assert_eq!(collect(&config(&p), Some(&cache)).unwrap().totals.input, 20);
    assert_eq!(std::fs::read(&p).unwrap(), before);
    assert_eq!(std::fs::read(d.path().join("a.db-wal")).unwrap(), wal);
}

#[test]
fn interval_merge_compares_instants_not_timestamp_text() {
    let mut a = UsageInterval {
        start: Some("2026-01-01T00:00:00Z".into()),
        end: Some("2026-01-01T00:00:00Z".into()),
    };
    a.merge(&UsageInterval {
        start: Some("2026-01-01T00:00:00.500Z".into()),
        end: Some("2026-01-01T00:00:00.500Z".into()),
    });
    assert_eq!(a.start.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(a.end.as_deref(), Some("2026-01-01T00:00:00.500Z"));
}
#[test]
fn report_rejects_invalid_or_misallocated_interval() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    session(&c, 10);
    usage(&c, None, 10, Some(1767225601.));
    for end in ["invalid", "2025-12-31T00:00:00Z", "2026-01-02T00:00:00Z"] {
        let mut s = collect(&config(&p), None).unwrap();
        s.rows[0].interval.as_mut().unwrap().end = Some(end.into());
        assert!(output::summary(&s).is_err());
    }
    let mut s = collect(&config(&p), None).unwrap();
    s.rows[0].interval = None;
    assert!(output::summary(&s).is_err());
}

#[test]
fn opposite_open_intervals_merge_into_one_unallocated_bucket() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    usage(&c, Some("helper"), 10, None);
    usage(&c, Some("helper"), 10, Some(1767225601.));
    c.execute(
        "UPDATE session_model_usage SET first_seen=NULL WHERE rowid=2",
        [],
    )
    .unwrap();
    usage(&c, Some("helper"), 10, None);
    c.execute(
        "UPDATE session_model_usage SET first_seen=NULL WHERE rowid=3",
        [],
    )
    .unwrap();
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(s.rows.len(), 1);
    assert_eq!(s.rows[0].time_basis, "unallocated");
    assert!(s.rows[0].interval.is_none());
    assert_eq!(s.totals.total().unwrap(), 300);
    assert!(output::summary(&s).is_ok());
}

#[test]
fn optional_api_call_counts_are_read_without_counting_aggregate_rows() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.db");
    let c = database(&p);
    c.execute_batch("ALTER TABLE session_model_usage ADD COLUMN api_call_count INTEGER; INSERT INTO sessions VALUES('s',10,2,0,0,0,1767225600,1767225601); INSERT INTO session_model_usage VALUES('s','fixture-model','',10,2,0,0,0,1767225600,1767225601,7);").unwrap();
    let s = collect(&config(&p), None).unwrap();
    assert_eq!(
        s.rows[0].messages,
        MessageCount {
            known: 7,
            complete: true
        }
    );
    c.execute("UPDATE session_model_usage SET api_call_count=-1", [])
        .unwrap();
    assert!(collect(&config(&p), None).is_err());
}
